#!/usr/bin/env node

const fs = require("fs");
const net = require("net");
const os = require("os");
const path = require("path");

const args = process.argv.slice(2);
const flag = (name, fallback) => {
  const i = args.indexOf(name);
  if (i < 0) return fallback;
  const value = args[i + 1];
  if (value === undefined) throw new Error(`missing value for ${name}`);
  return value;
};

const repo = flag("--repo");
const hubWs = flag("--hub-ws");
const token = flag("--token");
const machine = flag("--machine");
const state = flag("--state");
const work = flag("--work");
const rtt = Number(flag("--rtt-ms", "150"));
const iterations = Number(flag("--iterations", "40"));
const warmup = Number(flag("--warmup", "2"));
const clientStats = flag("--client-stats", "");
const nodeStats = flag("--node-stats", "");
const outPath = flag("--out");
const heavy = args.includes("--heavy");

if (typeof WebSocket !== "function") {
  console.error("this node has no global WebSocket");
  process.exit(2);
}

const pollMs = readPollMs(repo);
const oneWay = rtt;
const callMs = rtt * 2;
const openFloor = callMs * 3 + oneWay;

const result = {
  rtt_ms: rtt,
  iterations,
  warmup,
  poll_ms: pollMs,
  heavy,
  load: os.loadavg(),
  cpus: os.cpus().length,
  floor_ms: { one_way: oneWay, call: callMs, open_remote: openFloor },
  errors: 0,
  error_messages: [],
  thread_nudged: 0,
  work: {},
  proxy: {},
  metrics: {},
};

let sent = 0;
let seq = 0;
let tokenId = 0;

function readPollMs(root) {
  const text = fs.readFileSync(path.join(root, "ui/gui.js"), "utf8");
  const match = text.match(/^const POLL_MS = (\d+)/m);
  if (!match) throw new Error("ui/gui.js has no POLL_MS");
  return Number(match[1]);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function percentile(sorted, p) {
  if (sorted.length === 0) return null;
  const index = Math.ceil((p / 100) * sorted.length) - 1;
  return sorted[Math.max(0, Math.min(sorted.length - 1, index))];
}

function summarize(samples, calls) {
  const sorted = [...samples].sort((a, b) => a - b);
  return {
    n: sorted.length,
    min: sorted[0] ?? null,
    max: sorted[sorted.length - 1] ?? null,
    p50: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    calls,
    samples: sorted,
  };
}

function readStats(file) {
  if (!file) return null;
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return null;
  }
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
  const tabsWaiters = [];
  const threadListeners = new Set();
  const state = { machines: [], tabs: [], hello: null };
  let opened;
  const ready = new Promise((resolve, reject) => {
    opened = { resolve, reject };
  });

  ws.addEventListener("open", () => {
    ws.send(JSON.stringify({ type: "auth", token }));
  });
  ws.addEventListener("error", () => opened.reject(new Error(`websocket failed for ${hubWs}`)));
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
      state.hello = frame;
      state.machines = frame.machines || [];
      opened.resolve();
    } else if (frame.type === "machines") {
      state.machines = frame.machines || [];
    } else if (frame.type === "event" && frame.event && frame.event.kind === "tabs") {
      state.tabs = frame.event.tabs || [];
      for (const waiter of tabsWaiters.splice(0)) waiter(state.tabs);
    } else if (frame.type === "event" && frame.event && frame.event.kind === "thread") {
      for (const listen of threadListeners) listen(frame.event);
    } else if (frame.type === "reply") {
      const waiter = waiters.get(frame.id);
      if (!waiter) return;
      waiters.delete(frame.id);
      if (Object.prototype.hasOwnProperty.call(frame, "err")) waiter.reject(new Error(frame.err));
      else waiter.resolve(frame.ok);
    }
  });

  function call(body) {
    sent += 1;
    const id = ++seq;
    const sentAt = Date.now();
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
    return { sentAt, promise };
  }

  function waitTabs(pred, timeoutMs) {
    const existing = state.tabs;
    if (pred(existing)) return Promise.resolve(existing);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        const at = tabsWaiters.indexOf(onTabs);
        if (at >= 0) tabsWaiters.splice(at, 1);
        reject(new Error("timed out waiting for a tabs event"));
      }, timeoutMs);
      function onTabs(tabs) {
        if (!pred(tabs)) return;
        clearTimeout(timer);
        resolve(tabs);
      }
      tabsWaiters.push(onTabs);
    });
  }

  return { ready, state, call, waitTabs, threadListeners, close: () => ws.close() };
}

function waitLine(file, pred, timeoutMs) {
  const started = Date.now();
  return new Promise((resolve, reject) => {
    const timer = setInterval(() => {
      let text = "";
      try {
        text = fs.readFileSync(file, "utf8");
      } catch {}
      const line = text.split("\n").find((row) => row && pred(row));
      if (line) {
        clearInterval(timer);
        resolve(line);
      } else if (Date.now() - started > timeoutMs) {
        clearInterval(timer);
        reject(new Error(`timed out waiting on ${path.basename(file)}`));
      }
    }, 2);
  });
}

function stampAfter(file, t0) {
  const started = Date.now();
  let prior = "";
  try {
    prior = fs.readFileSync(file, "utf8");
  } catch {}
  return new Promise((resolve, reject) => {
    const timer = setInterval(() => {
      let text = "";
      try {
        text = fs.readFileSync(file, "utf8");
      } catch {}
      const extra = text.startsWith(prior) ? text.slice(prior.length) : text;
      const line = extra.split("\n").find((row) => row && Number(row.split(" ")[1]) >= t0);
      if (line) {
        clearInterval(timer);
        resolve(Number(line.split(" ")[1]));
      } else if (Date.now() - started > 20000) {
        clearInterval(timer);
        reject(new Error(`timed out waiting on ${path.basename(file)}`));
      }
    }, 2);
  });
}

function latestPid(file) {
  const rows = fs.readFileSync(file, "utf8").trim().split("\n").filter(Boolean);
  return Number(rows[rows.length - 1].split(" ")[0]);
}

function waitGone(pid) {
  const started = Date.now();
  return new Promise((resolve, reject) => {
    const timer = setInterval(() => {
      let alive = true;
      try {
        process.kill(pid, 0);
      } catch (err) {
        alive = err.code !== "ESRCH";
      }
      if (!alive) {
        clearInterval(timer);
        resolve(Date.now());
      } else if (Date.now() - started > 20000) {
        clearInterval(timer);
        reject(new Error(`pid ${pid} stayed alive`));
      }
    }, 2);
  });
}

function hitsFile() {
  return path.join(state, "hits");
}
function readyFile() {
  return path.join(state, "ready");
}

async function measure(fn) {
  const before = sent;
  const ms = await fn();
  if (!Number.isFinite(ms) || ms < 0) throw new Error(`bad sample ${ms}`);
  return { ms, calls: sent - before };
}

async function repeat(name, fn) {
  const samples = [];
  let calls = null;
  for (let i = 0; i < warmup + iterations; i += 1) {
    const sample = await fn(i);
    calls = sample.calls;
    if (i >= warmup) samples.push(sample.ms);
    process.stderr.write(`${name} ${i >= warmup ? i - warmup + 1 : "warmup"} ${sample.ms}ms calls ${sample.calls}\n`);
  }
  result.metrics[name] = summarize(samples, calls);
  result.work[name] = samples.length;
}

async function whenListed(client, id) {
  if ((client.state.tabs || []).some((tab) => tab.id === id)) return;
  try {
    await client.waitTabs((tabs) => tabs.some((tab) => tab.id === id), 2000);
  } catch {
    await client.call({ cmd: "tabs" }).promise;
  }
}

async function startSession(client) {
  const t0 = Date.now();
  const opened = client.call({ cmd: "open_tab", path: null, bypass: false });
  const id = await opened.promise;
  const started = client.call({ cmd: "start_claude", tab: id, cols: 80, rows: 24 });
  await stampAfter(readyFile(), t0);
  await started.promise;
  const transcript = await waitFile(path.join(state, "transcript"));
  const sock = await waitFile(path.join(state, "sock"));
  const page = await client.call({ cmd: "thread", tab: id, from: 0 }).promise;
  if (!page || !page.session) throw new Error("the session never reported a transcript");
  return { id, transcript, sock, next: page.next };
}

function waitFile(file) {
  return waitLine(file, () => true, 20000).then(() => fs.readFileSync(file, "utf8").trim());
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

function tell(sock, message) {
  return new Promise((resolve, reject) => {
    const client = net.createConnection(sock);
    const timer = setTimeout(() => {
      client.destroy();
      reject(new Error("hook socket timed out"));
    }, 5000);
    client.on("error", (err) => {
      clearTimeout(timer);
      reject(err);
    });
    client.on("connect", () => {
      client.end(`${JSON.stringify(message)}\n`, () => {
        clearTimeout(timer);
        resolve();
      });
    });
  });
}

const PHASE_STRIDE_MS = 47;
function phaseOffset(i) {
  return (i * PHASE_STRIDE_MS) % pollMs;
}

async function waitPhase(anchor, offset) {
  const into = (Date.now() - anchor) % pollMs;
  const wait = (offset - into + pollMs) % pollMs;
  if (wait > 0) await sleep(wait);
}

function startThreadPoll(client, tab, from) {
  let cursor = from;
  let fetching = false;
  let again = false;
  let againByNudge = false;
  let anchor = 0;
  const pending = [];
  function deliver(items, byNudge) {
    const keep = [];
    for (const waiter of pending.splice(0)) {
      if (!waiter(items, byNudge)) keep.push(waiter);
    }
    pending.push(...keep);
  }
  function read(byNudge) {
    if (fetching) {
      again = true;
      againByNudge = againByNudge || byNudge;
      return;
    }
    fetching = true;
    client
      .call({ cmd: "thread", tab, from: cursor })
      .promise.then((page) => {
        if (page && Number.isFinite(page.next)) cursor = page.next;
        deliver((page && page.items) || [], byNudge);
      })
      .catch((err) => {
        for (const waiter of pending.splice(0)) waiter.reject(err);
      })
      .finally(() => {
        fetching = false;
        if (!again) return;
        const next = againByNudge;
        again = againByNudge = false;
        read(next);
      });
  }
  const onNudge = (event) => {
    if (event.tab === tab) read(true);
  };
  client.threadListeners.add(onNudge);
  const timer = setInterval(() => {
    if (anchor === 0) anchor = Date.now();
    read(false);
  }, pollMs);
  return {
    anchor: () => anchor,
    stop() {
      clearInterval(timer);
      client.threadListeners.delete(onNudge);
    },
    seen(text) {
      return new Promise((resolve, reject) => {
        let settled = false;
        const timerId = setTimeout(() => {
          if (settled) return;
          settled = true;
          reject(new Error(`thread poll missed ${text}`));
        }, pollMs + callMs + 5000);
        const waiter = (items, byNudge) => {
          if (!items.some((item) => item.kind === "text" && item.text === text)) return false;
          if (!settled) {
            settled = true;
            clearTimeout(timerId);
            if (byNudge) result.thread_nudged += 1;
            resolve(Date.now());
          }
          return true;
        };
        waiter.reject = (err) => {
          if (settled) return;
          settled = true;
          clearTimeout(timerId);
          reject(err);
        };
        pending.push(waiter);
      });
    },
  };
}

function checkFloors() {
  if (rtt < 50) {
    const write = result.metrics.write_input;
    if (write && write.p50 > 250) {
      throw new Error(`0ms run write_input p50 ${write.p50}ms is still delayed`);
    }
    return;
  }
  const checks = [
    ["write_input", oneWay],
    ["close_tab", oneWay],
    ["tab_status", oneWay],
    ["set_model", oneWay],
    ["thread_message", oneWay],
    ["open_remote", oneWay],
  ];
  for (const [name, floor] of checks) {
    const metric = result.metrics[name];
    if (!metric || metric.p50 == null) continue;
    if (metric.p50 < floor * 0.7) {
      throw new Error(`${name} p50 ${metric.p50}ms is under 70% of its ${floor}ms network floor, so the delay was not on the path`);
    }
  }
}

async function main() {
  const client = connect();
  await client.ready;
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const online = (client.state.machines || []).some((item) => item.id === machine && item.online);
    if (online) break;
    await sleep(50);
  }
  if (!(client.state.machines || []).some((item) => item.id === machine && item.online)) {
    throw new Error(`machine ${machine} did not come online`);
  }

  const session = await startSession(client);
  const probe = ++tokenId;
  const probed = client.call({ cmd: "write_input", tab: session.id, data: `W${probe}Z\r` });
  await waitLine(hitsFile(), (line) => line.startsWith(`${probe} `), 20000);
  await probed.promise;

  await repeat("write_input", async () => {
    const id = ++tokenId;
    return measure(async () => {
      const t0 = Date.now();
      const issued = client.call({ cmd: "write_input", tab: session.id, data: `W${id}Z\r` });
      const stamp = await waitLine(hitsFile(), (line) => line.startsWith(`${id} `), 20000).then((line) => Number(line.split(" ")[1]));
      await issued.promise;
      return stamp - t0;
    });
  });

  let titleN = 0;
  await repeat("tab_status", async () => {
    titleN += 1;
    const title = `bench-${titleN}-${Date.now()}`;
    return measure(async () => {
      const wait = client.waitTabs((tabs) => tabs.some((tab) => tab.id === session.id && tab.title === title), 20000);
      const t0 = Date.now();
      await tell(session.sock, {
        kind: "agent",
        status: "needs-input",
        cwd: work,
        message: null,
        tab: session.id,
        session: fs.readFileSync(path.join(state, "session"), "utf8").trim(),
        title,
        resume_at: null,
        prompt: null,
        snapshot: null,
        hook_feedback: false,
      });
      await wait;
      return Date.now() - t0;
    });
  });

  const poll = startThreadPoll(client, session.id, session.next);
  try {
    await client.call({ cmd: "watch_threads", tabs: [session.id] }).promise;
  } catch (err) {
    console.error(`watch_threads unavailable: ${err}`);
  }
  const anchorDeadline = Date.now() + pollMs + callMs + 5000;
  while (poll.anchor() === 0) {
    if (Date.now() > anchorDeadline) throw new Error("thread poll never started");
    await sleep(10);
  }
  await sleep(pollMs);
  await repeat("thread_message", async (i) => {
    const text = `visible-${i}-${Date.now()}`;
    await waitPhase(poll.anchor(), phaseOffset(i));
    return measure(async () => {
      const seeing = poll.seen(text);
      const t0 = Date.now();
      appendMessage(session.transcript, text);
      const seenAt = await seeing;
      return seenAt - t0;
    });
  });
  poll.stop();

  if (heavy) {
    const pad = "x".repeat(200);
    const target = 400 * 1024;
    let bytes = 0;
    while (bytes < target) {
      appendMessage(session.transcript, `pad ${pad}`);
      bytes += pad.length + 80;
    }
    const cold = [];
    for (let i = 0; i < 8; i += 1) {
      const t0 = Date.now();
      const page = await client.call({ cmd: "thread", tab: session.id, from: 0 }).promise;
      cold.push(Date.now() - t0);
      if (!page || !page.items) throw new Error("cold thread read failed");
    }
    result.metrics.thread_cold = summarize(cold, 1);
    result.work.thread_cold = cold.length;
  }

  await repeat("set_model", async (i) => {
    return measure(async () => {
      const t0 = Date.now();
      const issued = client.call({ cmd: "set_model", tab: session.id, model: `b${i}`, cols: 80, rows: 24 });
      const stamp = await stampAfter(readyFile(), t0);
      await issued.promise;
      return stamp - t0;
    });
  });

  await repeat("close_tab", async () => {
    const t0 = Date.now();
    const opened = client.call({ cmd: "open_tab", path: work, bypass: false });
    const id = await opened.promise;
    const started = client.call({ cmd: "start_claude", tab: id, cols: 80, rows: 24 });
    await stampAfter(readyFile(), t0);
    await started.promise;
    const pid = latestPid(readyFile());
    return measure(async () => {
      const began = Date.now();
      const issued = client.call({ cmd: "close_tab", tab: id });
      const gone = await waitGone(pid);
      await issued.promise;
      return gone - began;
    });
  });

  await repeat("open_remote", async () => {
    let id = null;
    const sample = await measure(async () => {
      const t0 = Date.now();
      const opened = client.call({ cmd: "open_tab", path: work, bypass: false });
      id = await opened.promise;
      await whenListed(client, id);
      const started = client.call({ cmd: "start_claude", tab: id, cols: 80, rows: 24 });
      const stamp = await stampAfter(readyFile(), t0);
      await started.promise;
      return stamp - t0;
    });
    if (id) await client.call({ cmd: "close_tab", tab: id }).promise;
    return sample;
  });

  client.close();
}

main()
  .catch((err) => {
    result.errors += 1;
    result.error_messages.push(String(err && err.stack ? err.stack : err));
    console.error(err);
  })
  .finally(() => {
    result.proxy = { client: readStats(clientStats), node: readStats(nodeStats) };
    try {
      checkFloors();
    } catch (err) {
      result.errors += 1;
      result.error_messages.push(String(err.message || err));
      console.error(err.message || err);
    }
    const nodeBytes = result.proxy.node && result.proxy.node.transferred;
    if (!(nodeBytes > 1000)) {
      result.errors += 1;
      result.error_messages.push("the node proxy carried almost no bytes, so the node was not behind the delay");
    }
    fs.mkdirSync(path.dirname(outPath), { recursive: true });
    fs.writeFileSync(outPath, `${JSON.stringify(result, null, 2)}\n`);
    const names = Object.keys(result.metrics);
    console.error(
      `rtt_ms ${rtt} iterations ${iterations} errors ${result.errors} work ${Object.values(result.work).reduce((sum, n) => sum + n, 0)} thread_nudged ${result.thread_nudged} load ${result.load.map((n) => n.toFixed(2)).join(" ")}`,
    );
    console.error("metric            p50     p95     min     max   n  calls");
    for (const name of names) {
      const metric = result.metrics[name];
      const cell = (value) => String(Math.round(value) || value === 0 ? Math.round(value) : "-").padStart(6);
      console.error(
        `${name.padEnd(16)} ${cell(metric.p50)} ${cell(metric.p95)} ${cell(metric.min)} ${cell(metric.max)} ${String(metric.n).padStart(3)} ${String(metric.calls).padStart(6)}`,
      );
    }
    process.exit(result.errors > 0 ? 1 : 0);
  });
