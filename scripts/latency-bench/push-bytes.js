#!/usr/bin/env node
// Bytes the hub sends while several transcripts grow and one client watches
// one tab. Without --watch the node still pushes every tab, which is the
// flood this measures. With --watch the client names that one tab first.

const fs = require("fs");
const path = require("path");

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
const state = flag("--state");
const work = flag("--work");
const clientStats = flag("--client-stats");
const nodeStats = flag("--node-stats");
const outPath = flag("--out");
const background = Number(flag("--background", "4"));
const seconds = Number(flag("--seconds", "3"));
const watch = args.includes("--watch");

if (typeof WebSocket !== "function") {
  console.error("this node has no global WebSocket");
  process.exit(2);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function readStats(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
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
  const counts = new Map();
  let seq = 0;
  let bytes = 0;
  const state = { machines: [] };
  const ready = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("hello timed out")), 20000);
    ws.addEventListener("open", () => ws.send(JSON.stringify({ type: "auth", token })));
    ws.addEventListener("error", () => reject(new Error(`websocket failed for ${hubWs}`)));
    ws.addEventListener("message", async (event) => {
      const text = await textOf(event.data);
      bytes += Buffer.byteLength(text);
      let frame;
      try {
        frame = JSON.parse(text);
      } catch {
        return;
      }
      if (frame.type === "hello") {
        state.machines = frame.machines || [];
        clearTimeout(timer);
        resolve();
      } else if (frame.type === "machines") {
        state.machines = frame.machines || [];
      } else if (frame.type === "event" && frame.event && frame.event.kind === "thread") {
        const tab = frame.event.tab;
        const items = (frame.event.items || []).length;
        const row = counts.get(tab) || { events: 0, items: 0 };
        row.events += 1;
        row.items += items;
        counts.set(tab, row);
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
      }, 20000);
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
  return { ready, state, call, counts, bytes: () => bytes, close: () => ws.close() };
}

function waitReady(before) {
  const file = path.join(state, "ready");
  const started = Date.now();
  return new Promise((resolve, reject) => {
    const timer = setInterval(() => {
      let lines = [];
      try {
        lines = fs.readFileSync(file, "utf8").split("\n").filter(Boolean);
      } catch {}
      if (lines.length > before) {
        clearInterval(timer);
        resolve(lines.length);
      } else if (Date.now() - started > 20000) {
        clearInterval(timer);
        reject(new Error("claude never became ready"));
      }
    }, 20);
  });
}

function appendMessage(file, text) {
  const line = {
    parentUuid: "p",
    isSidechain: false,
    type: "assistant",
    uuid: `u${Date.now()}${Math.random().toString(16).slice(2)}`,
    timestamp: new Date().toISOString(),
    message: { role: "assistant", content: [{ type: "text", text }] },
  };
  fs.appendFileSync(file, `${JSON.stringify(line)}\n`);
}

async function startSession(client, readyLines) {
  const id = await client.call({ cmd: "open_tab", path: work, bypass: false });
  const starting = client.call({ cmd: "start_claude", tab: id, cols: 80, rows: 24 });
  const lines = await Promise.all([starting, waitReady(readyLines)]).then(([, next]) => next);
  const transcript = fs.readFileSync(path.join(state, "transcript"), "utf8").trim();
  const page = await client.call({ cmd: "thread", tab: id, from: 0 });
  if (!page || !page.session) throw new Error("the session never reported a transcript");
  return { id, transcript, readyLines: lines };
}

async function main() {
  const client = connect();
  await client.ready;
  const onlineAt = Date.now() + 20000;
  while (!(client.state.machines || []).some((item) => item.id === machine && item.online)) {
    if (Date.now() > onlineAt) throw new Error(`machine ${machine} did not come online`);
    await sleep(50);
  }
  let readyLines = 0;
  const sessions = [];
  for (let i = 0; i < background + 1; i += 1) {
    const session = await startSession(client, readyLines);
    readyLines = session.readyLines;
    sessions.push(session);
  }
  const active = sessions[0];
  const rest = sessions.slice(1);
  if (watch) {
    await client.call({ cmd: "watch_threads", tabs: [active.id] });
  }
  await sleep(300);
  for (const tab of [active.id, ...rest.map((session) => session.id)]) client.counts.delete(tab);
  const clientBefore = readStats(clientStats).transferred;
  const nodeBefore = readStats(nodeStats).transferred;
  const framesBefore = client.bytes();
  const until = Date.now() + seconds * 1000;
  let n = 0;
  while (Date.now() < until) {
    n += 1;
    appendMessage(active.transcript, `active-${n}`);
    for (const session of rest) appendMessage(session.transcript, `background-${session.id}-${n}`);
    await sleep(80);
  }
  await sleep(800);
  const byTab = {};
  for (const [tab, row] of client.counts) byTab[tab] = row;
  const body = {
    watch,
    background,
    seconds,
    lines_each: n,
    active_tab: active.id,
    background_tabs: rest.map((session) => session.id),
    client_transferred: readStats(clientStats).transferred - clientBefore,
    node_transferred: readStats(nodeStats).transferred - nodeBefore,
    client_frame_bytes: client.bytes() - framesBefore,
    thread_events: byTab,
  };
  fs.writeFileSync(outPath, `${JSON.stringify(body, null, 2)}\n`);
  console.log(JSON.stringify(body));
  client.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
