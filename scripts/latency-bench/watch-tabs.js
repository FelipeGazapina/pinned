#!/usr/bin/env node

const fs = require("fs");

const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const i = args.indexOf(name);
  if (i < 0) return fallback;
  const value = args[i + 1];
  if (value === undefined) throw new Error(`missing value for ${name}`);
  return value;
};

const hubWs = flag("--hub-ws");
const token = flag("--token");
const machine = flag("--machine");
const transcript = flag("--transcript");
const outPath = flag("--out");
const tabsExpected = Number(flag("--tabs", "15"));
const hold = args.includes("--hold");

if (typeof WebSocket !== "function") {
  console.error("this node has no global WebSocket");
  process.exit(2);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function textOf(data) {
  if (typeof data === "string") return data;
  if (Buffer.isBuffer(data)) return data.toString("utf8");
  if (data instanceof ArrayBuffer) return Buffer.from(data).toString("utf8");
  if (data && typeof data.text === "function") return data.text();
  return String(data);
}

function connect() {
  const ws = new WebSocket(hubWs);
  const waiters = new Map();
  const threads = [];
  let seq = 0;
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("hello timed out")), 15000);
    ws.addEventListener("open", () => ws.send(JSON.stringify({ type: "auth", token })));
    ws.addEventListener("error", () => reject(new Error(`websocket failed for ${hubWs}`)));
    ws.addEventListener("close", () => {
      for (const waiter of waiters.values()) waiter.reject(new Error("websocket closed"));
      waiters.clear();
    });
    ws.addEventListener("message", async (event) => {
      let frame;
      try {
        frame = JSON.parse(await textOf(event.data));
      } catch {
        return;
      }
      if (frame.type === "hello") {
        clearTimeout(timer);
        resolve();
      } else if (frame.type === "event" && frame.event && frame.event.kind === "thread") {
        threads.push(frame.event);
      } else if (frame.type === "reply") {
        const waiter = waiters.get(frame.id);
        if (!waiter) return;
        waiters.delete(frame.id);
        if (Object.prototype.hasOwnProperty.call(frame, "err")) waiter.reject(new Error(frame.err));
        else waiter.resolve(frame.ok);
      }
    });
  });

  function call(body) {
    const id = ++seq;
    const promise = new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        waiters.delete(id);
        reject(new Error(`call ${body.cmd} timed out`));
      }, 15000);
      waiters.set(id, {
        resolve: (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        reject: (err) => {
          clearTimeout(timer);
          reject(err);
        },
      });
    });
    ws.send(JSON.stringify({ type: "call", id, machine, call: body }));
    return promise;
  }

  return { ready, call, threads, close: () => ws.close() };
}

async function main() {
  const client = connect();
  await client.ready;

  let tabs = [];
  const listed = Date.now() + 10000;
  while (Date.now() < listed) {
    const page = await client.call({ cmd: "tabs" });
    if (Array.isArray(page) && page.length >= tabsExpected) {
      tabs = page;
      break;
    }
    await sleep(50);
  }
  if (tabs.length < tabsExpected) {
    console.error(`expected ${tabsExpected} tabs, saw ${tabs.length}`);
    process.exit(1);
  }

  const ids = tabs.map((tab) => tab.id);
  await client.call({ cmd: "watch_threads", tabs: ids });
  await sleep(300);
  const marker = `watched-cost-${Date.now()}`;
  const before = client.threads.length;
  const started = Date.now();
  fs.appendFileSync(
    transcript,
    JSON.stringify({
      type: "assistant",
      uuid: marker,
      timestamp: "2020-01-01T00:00:00Z",
      message: { role: "assistant", content: [{ type: "text", text: marker }] },
    }) + "\n",
  );
  const deadline = started + 800;
  while (Date.now() < deadline && client.threads.length === before) await sleep(10);
  const hit = client.threads[before];
  if (!hit) {
    console.error(`no nudge for ${marker} within 800 ms`);
    process.exit(1);
  }
  const nudgeMs = Date.now() - started;
  const page = await client.call({ cmd: "thread", tab: hit.tab, from: 0 });
  if (!(page.items || []).some((item) => item.text === marker)) {
    console.error(`the nudged tab ${hit.tab} has no ${marker}`);
    process.exit(1);
  }
  const result = {
    nudged: true,
    tabs: ids.length,
    tab: hit.tab,
    nudge_ms: nudgeMs,
    read_ms: Date.now() - started,
    marker,
  };
  fs.writeFileSync(outPath, JSON.stringify(result, null, 2) + "\n");
  console.log(JSON.stringify(result));
  if (hold) {
    await new Promise(() => {});
  }
  client.close();
}

main().catch((err) => {
  console.error(err && err.stack ? err.stack : err);
  process.exit(1);
});
