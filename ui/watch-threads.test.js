import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function publishThreadWatch() {
  const src = readFileSync(new URL("./gui.js", import.meta.url), "utf8");
  const start = src.indexOf("let watchedSignature");
  const end = src.indexOf("setInterval(() => {", start);
  const calls = [];
  const state = { gui: true, hidden: false, tabs: [], active: null };
  const sandbox = {
    calls,
    guiOn: () => state.gui,
    terminalOnly: (tab) => !!tab.terminalOnly,
    turnLive: (tab) => !!tab.live,
    phaseOf: (tab) => tab.phase || "idle",
    callOn(machine, call) {
      calls.push({ machine, tabs: [...call.tabs] });
      return Promise.resolve();
    },
  };
  Object.defineProperty(sandbox, "document", { get: () => ({ hidden: state.hidden }) });
  Object.defineProperty(sandbox, "tabs", { get: () => state.tabs, set: (value) => { state.tabs = value; } });
  Object.defineProperty(sandbox, "active", { get: () => state.active, set: (value) => { state.active = value; } });
  vm.createContext(sandbox);
  vm.runInContext(src.slice(start, end), sandbox);
  return { sandbox, state, calls };
}

test("a background tab is not watched, and a hidden page watches nothing", () => {
  const { sandbox, state, calls } = publishThreadWatch();
  const active = { machine: "m", view: { id: "tab-1", program: "agent" }, phase: "idle" };
  const background = { machine: "m", view: { id: "tab-2", program: "agent" }, phase: "idle" };
  state.tabs.push(active, background);
  state.active = active;
  sandbox.publishThreadWatch();
  assert.deepEqual(calls, [{ machine: "m", tabs: ["tab-1"] }]);
  state.hidden = true;
  sandbox.publishThreadWatch();
  assert.deepEqual(calls[1], { machine: "m", tabs: [] });
  sandbox.publishThreadWatch();
  assert.equal(calls.length, 2);
});

test("a hub hello sends the current watch again", () => {
  const { sandbox, state, calls } = publishThreadWatch();
  const active = { machine: "m", view: { id: "tab-1", program: "agent" }, phase: "idle" };
  state.tabs.push(active);
  state.active = active;
  sandbox.account = { email: "" };
  sandbox.showMachines = () => sandbox.publishThreadWatch();
  sandbox.hub = { hub: () => Promise.resolve() };
  sandbox.onHubSettings = () => {};
  const main = readFileSync(new URL("./main.js", import.meta.url), "utf8");
  const start = main.indexOf("    hello(frame) {");
  const end = main.indexOf("    machines:", start);
  assert.notEqual(start, -1);
  assert.notEqual(end, -1);
  vm.runInContext(`function connectHello(){ return {\n${main.slice(start, end)}\n}; }`, sandbox);
  const hello = sandbox.connectHello().hello;
  const frame = { account: { email: "a@b.c" }, machines: [], tabs: {} };
  hello(frame);
  assert.deepEqual(calls, [{ machine: "m", tabs: ["tab-1"] }]);
  hello(frame);
  assert.deepEqual(calls, [
    { machine: "m", tabs: ["tab-1"] },
    { machine: "m", tabs: ["tab-1"] },
  ]);
});

test("a watch the hub rejects is sent again", async () => {
  const { sandbox, state, calls } = publishThreadWatch();
  sandbox.callOn = (machine, call) => {
    calls.push({ machine, tabs: [...call.tabs] });
    return Promise.reject(new Error("unknown variant `watch_threads`"));
  };
  const active = { machine: "m", view: { id: "tab-1", program: "agent" }, phase: "idle" };
  state.tabs.push(active);
  state.active = active;
  sandbox.publishThreadWatch();
  await new Promise((resolve) => setTimeout(resolve, 0));
  sandbox.publishThreadWatch();
  assert.deepEqual(calls, [
    { machine: "m", tabs: ["tab-1"] },
    { machine: "m", tabs: ["tab-1"] },
  ]);
});

test("a rejection from before hello does not undo the watch hello sent", async () => {
  const { sandbox, state, calls } = publishThreadWatch();
  let rejectFirst;
  sandbox.callOn = (machine, call) => {
    calls.push({ machine, tabs: [...call.tabs] });
    return new Promise((_, reject) => {
      rejectFirst = reject;
    });
  };
  const active = { machine: "m", view: { id: "tab-1", program: "agent" }, phase: "idle" };
  state.tabs.push(active);
  state.active = active;
  sandbox.publishThreadWatch();
  sandbox.forgetThreadWatch();
  sandbox.callOn = (machine, call) => {
    calls.push({ machine, tabs: [...call.tabs] });
    return Promise.resolve();
  };
  sandbox.publishThreadWatch();
  rejectFirst(new Error("unknown variant `watch_threads`"));
  await new Promise((resolve) => setTimeout(resolve, 0));
  sandbox.publishThreadWatch();
  assert.deepEqual(calls, [
    { machine: "m", tabs: ["tab-1"] },
    { machine: "m", tabs: ["tab-1"] },
  ]);
});

function slice(src, startAt, endAt) {
  const start = src.indexOf(startAt);
  const end = src.indexOf(endAt, start + startAt.length);
  assert.ok(start >= 0 && end >= 0, `missing slice ${startAt}`);
  return src.slice(start, end);
}

const gui = readFileSync(new URL("./gui.js", import.meta.url), "utf8");
const main = readFileSync(new URL("./main.js", import.meta.url), "utf8");
const settle = () => new Promise((resolve) => setImmediate(resolve));

function page() {
  const state = { tabs: [], active: null, polls: [], hubClosed: false };
  const sent = [];
  const attempts = [];
  const transport = (machine) => ({
    call: async (call) => {
      if (machine !== "local" && state.hubClosed) {
        await settle();
        throw "lost the connection to the hub";
      }
      if (call.cmd === "watch_threads") sent.push([machine, [...call.tabs]]);
      if (call.cmd === "tabs") return state.listed[machine] ?? [];
      return null;
    },
  });
  const sandbox = {
    sent,
    attempts,
    state,
    LOCAL: "local",
    machines: new Map([["local", { view: { id: "local", online: true }, tabs: [], transport: transport("local"), usage: {} }]]),
    account: { machine_id: "me" },
    hub: {
      close() {
        state.hubClosed = true;
      },
      transport,
    },
    closing: new Set(),
    viewListeners: [],
    document: { hidden: false, body: { dataset: {} } },
    guiOn: () => true,
    terminalOnly: (tab) => tab.view.program === "terminal" || tab.view.program === "sign-in",
    turnLive: (tab) => tab.view.status === "working",
    phaseOf: (tab) => (tab.view.agents ? "delegating" : tab.view.status ?? "idle"),
    keyOf: (machine, id) => `${machine}:${id}`,
    tabByKey: (key) => state.tabs.find((tab) => tab.key === key),
    addTab(machine, view) {
      state.tabs.push({ key: `${machine}:${view.id}`, machine, view });
    },
    removeTab(tab) {
      state.tabs.splice(state.tabs.indexOf(tab), 1);
      if (state.active === tab) state.active = state.tabs[0] ?? null;
    },
    updateView(tab, view) {
      tab.view = view;
    },
    fetchThread(tab) {
      state.polls.push(tab.view.id);
    },
    placeTabs() {},
    refreshProjects() {},
    retryUploadImages() {},
    settingsOpen: () => false,
    render: () => sandbox.publishThreadWatch(),
    POLL_MS: 1200,
    setInterval: (tick) => (state.tick = tick),
    setTimeout,
    clearTimeout,
  };
  Object.defineProperty(sandbox, "tabs", { get: () => state.tabs });
  Object.defineProperty(sandbox, "active", { get: () => state.active });
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(
    `${slice(main, "const missingTabId", "const IS_MAC").replace("const callOn = async", "const realCallOn = async")}
const callOn = (machine, call) => {
  attempts.push(machine);
  return realCallOn(machine, call);
};
${slice(main, "function setTabs", "/** The machine's accounts")}
${slice(main, "function showMachines", "async function bootApp")}
${slice(gui, "let watchedSignature", "setInterval(() => guiOn() && renderSidebar()")}
globalThis.showMachines = showMachines;
globalThis.disconnectHub = disconnectHub;
globalThis.setTabs = setTabs;`,
    sandbox,
  );
  return sandbox;
}

test("the poll reads exactly the tabs the page watches", () => {
  const app = page();
  app.setTabs("local", [
    { id: "shown", program: "agent", status: "idle" },
    { id: "working", program: "agent", status: "working" },
    { id: "agents", program: "agent", status: "done", agents: true },
    { id: "shell", program: "terminal" },
    { id: "idle", program: "agent", status: "idle" },
    { id: "login", program: "sign-in" },
  ]);
  app.state.active = app.state.tabs[0];
  app.sent.length = 0;
  app.forgetThreadWatch();
  app.state.tick();
  assert.deepEqual(app.sent, [["local", ["shown", "working", "agents", "shell"]]]);
  assert.deepEqual(app.state.polls, ["shown", "working", "agents", "shell"]);
});

test("a machine that drops off the hub is unwatched, and watched again once its tabs are back", async () => {
  const app = page();
  app.state.listed = { box: [{ id: "tab-1", program: "agent", status: "working" }] };
  app.showMachines([{ id: "box", name: "box", online: true }], { box: app.state.listed.box });
  await settle();
  app.showMachines([{ id: "box", name: "box", online: false }]);
  await settle();
  app.showMachines([{ id: "box", name: "box", online: true }]);
  await settle();
  await settle();
  assert.deepEqual(app.sent, [
    ["box", ["tab-1"]],
    ["box", []],
    ["box", ["tab-1"]],
  ]);
});

test("signing out leaves the watch of this computer's tabs and stops calling the hub's machines", async () => {
  const app = page();
  app.setTabs("local", [{ id: "mine", program: "agent", status: "working" }]);
  app.showMachines([{ id: "box", name: "box", online: true }], { box: [{ id: "tab-1", program: "agent", status: "idle" }] });
  app.state.active = app.tabByKey("box:tab-1");
  app.publishThreadWatch();
  await settle();
  app.sent.length = 0;
  app.attempts.length = 0;
  app.disconnectHub();
  for (let tick = 0; tick < 3; tick++) {
    await settle();
    await settle();
    app.state.tick();
  }
  await settle();
  assert.deepEqual(app.sent, [], "this computer's unchanged watch was sent again");
  assert.deepEqual(app.attempts, ["box"], "one unwatch into the closing hub, then none");
});
