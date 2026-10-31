#!/usr/bin/env node

const fs = require("fs");
const vm = require("vm");

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  if (i < 0 || args[i + 1] === undefined) throw new Error(`missing ${name}`);
  return args[i + 1];
};

const repo = flag("--repo");
const hubWs = flag("--hub-ws");
const token = flag("--token");
const machine = flag("--machine");
const state = flag("--state");
const work = flag("--work");
const phasePath = flag("--phase");
const outPath = flag("--out");
const rtt = Number(flag("--rtt-ms"));

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function phase(name) {
  fs.writeFileSync(phasePath, name);
  console.error(`${Date.now()} phase ${name}`);
}

async function waitFor(label, pred, ms) {
  const start = Date.now();
  while (!pred()) {
    if (Date.now() - start > ms) throw new Error(`timed out waiting for ${label}`);
    await sleep(20);
  }
}

function appendMessage(file, text) {
  const line = {
    type: "assistant",
    uuid: `u${Date.now()}${Math.random().toString(16).slice(2)}`,
    timestamp: new Date().toISOString(),
    message: { role: "assistant", content: [{ type: "text", text }] },
  };
  fs.appendFileSync(file, `${JSON.stringify(line)}\n`);
}

function transcriptPath() {
  const file = `${state}/transcript`;
  if (!fs.existsSync(file)) return "";
  return fs.readFileSync(file, "utf8").trim();
}

const sandbox = {
  WebSocket,
  setTimeout,
  clearTimeout,
  console,
  atob,
  btoa,
  JSON,
  Map,
  Set,
  Promise,
  Uint8Array,
  account: { email: "", machine_id: "phone" },
  tabs: [],
  active: null,
  watches: [],
  threads: [],
  nodeOnline: false,
  guiOn: () => true,
  terminalOnly: () => false,
  turnLive: () => false,
  phaseOf: () => "idle",
  onHubSettings() {},
  document: { hidden: false, body: { dataset: {} } },
};
sandbox.callOn = (id, call) => {
  const pending = sandbox.hub.call(id, call);
  if (call.cmd === "watch_threads") sandbox.watches.push({ at: Date.now(), tabs: [...call.tabs], pending });
  return pending;
};
vm.createContext(sandbox);
vm.runInContext(`${fs.readFileSync(`${repo}/ui/hub.js`, "utf8")}\nthis.Hub = Hub;\n`, sandbox);
const gui = fs.readFileSync(`${repo}/ui/gui.js`, "utf8");
const guiStart = gui.indexOf("let watchedSignature");
const guiEnd = gui.indexOf("setInterval(() => {", guiStart);
vm.runInContext(gui.slice(guiStart, guiEnd), sandbox);
const mainSrc = fs.readFileSync(`${repo}/ui/main.js`, "utf8");
const helloStart = mainSrc.indexOf("    hello(frame) {");
const helloEnd = mainSrc.indexOf("    machines:", helloStart);
vm.runInContext(`function connectHello(){ return {\n${mainSrc.slice(helloStart, helloEnd)}\n}; }`, sandbox);
sandbox.showMachines = (list, tabsByMachine) => {
  for (const view of list) {
    if (view.id !== machine) continue;
    sandbox.nodeOnline = !!view.online;
    if (!view.online) {
      sandbox.tabs = [];
      sandbox.active = null;
    } else if (tabsByMachine && Object.prototype.hasOwnProperty.call(tabsByMachine, view.id)) {
      sandbox.setTabs(view.id, tabsByMachine[view.id]);
    }
  }
  sandbox.publishThreadWatch();
};
sandbox.setTabs = (id, views) => {
  sandbox.tabs = views.map((view) => ({ machine: id, view, phase: "idle" }));
  sandbox.active = sandbox.tabs[0] || null;
  sandbox.publishThreadWatch();
};
const hello = sandbox.connectHello().hello;
sandbox.hub = new sandbox.Hub(hubWs.replace(/^ws/, "http").replace(/\/ws\/client$/, ""), token, {
  hello(frame) {
    hello(frame);
  },
  machines(list, tabs) {
    sandbox.showMachines(list, tabs);
  },
  event(id, event) {
    if (id === sandbox.account.machine_id) return;
    if (event.kind === "tabs") sandbox.setTabs(id, event.tabs);
    else if (event.kind === "thread") sandbox.threads.push({ at: Date.now(), event });
  },
  settings() {},
  online() {},
  refused() {
    throw new Error("hub refused the phone client");
  },
});

async function seen(marker, file) {
  const tab = sandbox.tabs[0].view.id;
  const from = (await sandbox.hub.call(machine, { cmd: "thread", tab, from: 0 })).next;
  const nudges = sandbox.threads.length;
  const started = Date.now();
  appendMessage(file, marker);
  await waitFor(`nudge for ${marker}`, () => sandbox.threads.slice(nudges).some((row) => row.event.tab === tab), 800);
  const nudge = sandbox.threads.slice(nudges).find((row) => row.event.tab === tab);
  const page = await sandbox.hub.call(machine, { cmd: "thread", tab, from });
  if (!page.items.some((item) => item.text === marker)) throw new Error(`the nudged read of ${tab} missed ${marker}`);
  return { marker, tab, nudge_ms: nudge.at - started, read_ms: Date.now() - started };
}

async function main() {
  phase("connecting");
  await waitFor("node online", () => sandbox.nodeOnline, 20000);
  const opened = await sandbox.hub.call(machine, { cmd: "open_tab", path: work, bypass: false });
  await sandbox.hub.call(machine, { cmd: "start_claude", tab: opened, cols: 80, rows: 24 });
  await waitFor("transcript", () => transcriptPath() !== "", 20000);
  await waitFor("watch", () => sandbox.watches.some((row) => row.tabs.includes(opened)), 10000);
  await sandbox.watches.find((row) => row.tabs.includes(opened)).pending;
  await sleep(400);
  const firstFile = transcriptPath();
  const first = await seen(`again-client-${Date.now()}`, firstFile);
  const watchesAfterFirst = sandbox.watches.length;
  phase("cut");

  await waitFor("watch resent on hello", () => sandbox.watches.length > watchesAfterFirst, 15000);
  const resent = sandbox.watches[sandbox.watches.length - 1];
  if (!resent.tabs.includes(opened)) throw new Error(`hello resent ${JSON.stringify(resent.tabs)}`);
  await resent.pending;
  await sleep(400);
  const second = await seen(`again-phone-${Date.now()}`, transcriptPath());
  phase("restart");

  await waitFor("node offline", () => !sandbox.nodeOnline, 20000);
  const restartedAt = Date.now();
  await waitFor("node online", () => sandbox.nodeOnline, 20000);
  await waitFor("new transcript", () => {
    const path = transcriptPath();
    if (!path || path === firstFile) return false;
    try {
      return fs.statSync(path).mtimeMs >= restartedAt - 1000;
    } catch {
      return false;
    }
  }, 20000);
  await waitFor("restored tab", () => {
    const id = sandbox.tabs[0] && sandbox.tabs[0].view.id;
    return !!id && id !== opened;
  }, 20000);
  const restored = sandbox.tabs[0].view.id;
  await waitFor("watch of the restored tab", () => sandbox.watches.some((row) => row.tabs.includes(restored)), 15000);
  await sandbox.watches.find((row) => row.tabs.includes(restored)).pending;
  await sleep(500);
  const third = await seen(`again-machine-${Date.now()}`, transcriptPath());
  const report = {
    rtt_ms: rtt,
    watches: sandbox.watches.map((row) => ({ at: row.at, tabs: row.tabs })),
    lines: [first, second, third],
    poll_ms: 1200,
  };
  fs.writeFileSync(outPath, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify(report));
  phase("done");
  sandbox.hub.close();
}

main().catch((err) => {
  console.error(err && err.stack ? err.stack : err);
  phase("failed");
  process.exit(1);
});
