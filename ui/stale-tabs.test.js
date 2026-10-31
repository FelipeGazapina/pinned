import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./main.js", import.meta.url), "utf8");

const snapshot = (value) => JSON.parse(JSON.stringify(value));

function runSlice(startAt, endAt, extra = "") {
  const start = src.indexOf(startAt);
  const end = src.indexOf(endAt);
  const context = { console, LOCAL: "local" };
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}\n${extra}`, context);
  return context;
}

function showMachinesContext() {
  const applied = [];
  const refreshed = [];
  const retried = [];
  const context = runSlice("function showMachines", "function disconnectHub");
  context.account = { machine_id: "this-mac" };
  context.page = "";
  context.machinesUi = { renaming: false };
  context.settingsOpen = () => false;
  context.render = () => {};
  context.hub = { transport: () => ({}) };
  context.machines = new Map([
    ["box", { view: { id: "box", name: "railway", online: true }, tabs: [{ id: "tab-old" }] }],
  ]);
  context.setTabs = (machine, views) => applied.push({ machine, ids: views.map((view) => view.id) });
  context.refreshTabs = (machine) => {
    refreshed.push(machine);
    return Promise.resolve();
  };
  context.retryUploadImages = () => retried.push("retry");
  return { context, applied, refreshed, retried };
}

function whenListedHarness() {
  const context = runSlice("const tabListed", "async function revealTab");
  context.tabs = [];
  context.asked = [];
  context.applied = [];
  context.keyOf = (machine, id) => `${machine}:${id}`;
  context.tabByKey = (key) => context.tabs.find((tab) => tab.key === key) ?? null;
  context.setTabs = (machine, views) => {
    context.applied.push(views.map((view) => view.id));
    context.tabs = views.map((view) => ({ key: context.keyOf(machine, view.id), view }));
  };
  context.callOn = async (machine, call) => {
    context.asked.push(`${machine}:${call.cmd}`);
    if (call.cmd === "tabs") return [{ id: "tab-1" }];
    return null;
  };
  context.refreshTabs = async (machine) => {
    context.asked.push(`${machine}:refresh`);
    context.setTabs(machine, [{ id: "tab-1" }]);
  };
  context.setTimeout = setTimeout;
  context.clearTimeout = clearTimeout;
  return context;
}

test("an older node is asked for its tabs without a two second wait", async () => {
  const context = whenListedHarness();
  const waits = [];
  context.setTimeout = (fn, ms) => {
    waits.push(ms);
    return 1;
  };
  context.clearTimeout = () => {};
  const pending = context.whenListed("box", "tab-1");
  assert.deepEqual(waits, []);
  assert.deepEqual(context.asked, ["box:tabs"]);
  await pending;
  assert.equal(context.tabByKey("box:tab-1").view.id, "tab-1");
});

test("a newer node that pushes the tab does not wait, and a late ask does not replace it", async () => {
  const context = whenListedHarness();
  let answer;
  context.callOn = () => new Promise((resolve) => {
    answer = resolve;
  });
  const started = Date.now();
  const pending = context.whenListed("box", "tab-new");
  context.tabs.push({ key: "box:tab-new", view: { id: "tab-new" } });
  context.noteTabs("box");
  await pending;
  assert.ok(Date.now() - started < 500, `the push took ${Date.now() - started} ms`);
  if (answer) {
    answer([{ id: "stale" }]);
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  assert.deepEqual(context.applied, []);
  assert.equal(context.tabByKey("box:tab-new").view.id, "tab-new");
});

test("an already-online machine keeps its tabs when the update has no snapshot", () => {
  const { context, applied, refreshed } = showMachinesContext();
  context.showMachines([{ id: "box", name: "railway", online: true }]);
  assert.deepEqual(snapshot(applied), []);
  assert.deepEqual(snapshot(refreshed), []);
});

test("an already-online machine applies a tab snapshot without asking again", () => {
  const { context, applied, refreshed } = showMachinesContext();
  context.showMachines([{ id: "box", name: "railway", online: true }], { box: [{ id: "tab-new" }] });
  assert.deepEqual(snapshot(applied), [{ machine: "box", ids: ["tab-new"] }]);
  assert.deepEqual(snapshot(refreshed), []);
});

test("a machine going offline drops its tabs", () => {
  const { context, applied } = showMachinesContext();
  context.showMachines([{ id: "box", name: "railway", online: false }]);
  assert.deepEqual(snapshot(applied), [{ machine: "box", ids: [] }]);
});

test("a machine that just came online asks for its tabs when the hub sent none", () => {
  const { context, refreshed, retried } = showMachinesContext();
  context.machines.get("box").view.online = false;
  context.showMachines([{ id: "box", name: "railway", online: true }]);
  assert.deepEqual(snapshot(refreshed), ["box"]);
  assert.deepEqual(snapshot(retried), ["retry"]);
});

function startClaudeContext() {
  const context = runSlice("async function startClaude", "async function setBypass");
  context.started = [];
  context.notes = [];
  context.writes = [];
  context.guiOn = () => true;
  context.refit = () => {};
  context.render = () => {};
  context.activate = (tab) => {
    context.activated = tab.key;
  };
  context.showComposerNote = (text, error) => context.notes.push({ text, error });
  context.startTab = (tab) => context.started.push(tab.view.id);
  context.callOn = async () => {
    throw "no tab tab-old";
  };
  return context;
}

test("a machine that just came online uses the snapshot instead of asking", () => {
  const { context, applied, refreshed, retried } = showMachinesContext();
  context.machines.get("box").view.online = false;
  context.showMachines([{ id: "box", name: "railway", online: true }], { box: [{ id: "tab-new" }] });
  assert.deepEqual(snapshot(applied), [{ machine: "box", ids: ["tab-new"] }]);
  assert.deepEqual(snapshot(refreshed), []);
  assert.deepEqual(snapshot(retried), ["retry"]);
});

test("Enter starts the restored session when the old tab id is gone", async () => {
  const context = startClaudeContext();
  const old = { key: "box:tab-old", machine: "box", view: { id: "tab-old", session: "conv-1", running: false }, attached: Promise.resolve(), term: { cols: 80, rows: 24, write() {} } };
  const restored = { key: "box:tab-new", machine: "box", view: { id: "tab-new", session: "conv-1", running: true }, attached: Promise.resolve(), term: { cols: 80, rows: 24, write() {} } };
  context.tabs = [restored];
  await context.startClaude(old);
  assert.deepEqual(snapshot(context.started), []);
  assert.deepEqual(snapshot(context.notes), []);
  assert.equal(context.activated, "box:tab-new");
});

test("Enter says the session ended when the machine has no tab for it", async () => {
  const context = startClaudeContext();
  const old = { key: "box:tab-old", machine: "box", view: { id: "tab-old", session: "conv-1", running: false }, attached: Promise.resolve(), term: { cols: 80, rows: 24, write(text) { context.writes.push(text); } } };
  context.tabs = [];
  await context.startClaude(old);
  assert.deepEqual(snapshot(context.notes), [{ text: "That session ended when the machine restarted. Open it again to continue.", error: true }]);
  assert.deepEqual(snapshot(context.writes), []);
});

test("Enter reports a start error on a tab the machine still has", async () => {
  const context = startClaudeContext();
  const tab = {
    key: "box:tab-old",
    machine: "box",
    view: { id: "tab-old", running: false },
    attached: Promise.resolve(),
    term: { cols: 80, rows: 24, write(text) { context.writes.push(text); } },
  };
  context.tabs = [tab];
  context.guiOn = () => false;
  await context.startClaude(tab);
  assert.equal(context.writes.length, 1);
  assert.match(context.writes[0], /could not start claude: no tab tab-old/);
});

test("a no-tab error reloads that machine's tabs and is still reported", async () => {
  const context = runSlice("const missingTabId", "const IS_MAC", "globalThis.callOn = callOn;");
  const calls = [];
  context.machines = new Map([["box", { transport: { call: async (call) => { calls.push(call.cmd); throw "no tab tab-9"; } } }]]);
  context.refreshed = [];
  context.refreshTabs = async (machine) => context.refreshed.push(machine);
  await assert.rejects(context.callOn("box", { cmd: "start_claude", tab: "tab-9" }), (error) => error === "no tab tab-9");
  assert.deepEqual(context.refreshed, ["box"]);
  context.refreshed.length = 0;
  await assert.rejects(context.callOn("box", { cmd: "tabs" }), (error) => error === "no tab tab-9");
  assert.deepEqual(context.refreshed, []);
  assert.deepEqual(calls, ["start_claude", "tabs"]);
});

test("an already-online machine keeps its tabs when the snapshot omits that machine", () => {
  const { context, applied, refreshed } = showMachinesContext();
  context.showMachines([{ id: "box", name: "railway", online: true }], {});
  assert.deepEqual(snapshot(applied), []);
  assert.deepEqual(snapshot(refreshed), []);
});

test("Enter on a stale tab activates the restored one and does not start it", async () => {
  const context = runSlice("const starting", "async function setBypass");
  context.calls = [];
  context.notes = [];
  context.guiOn = () => true;
  context.refit = () => {};
  context.render = () => {};
  context.activate = (tab) => {
    context.activated = tab.key;
  };
  context.showComposerNote = (text, error) => context.notes.push({ text, error });
  context.callOn = async (_machine, call) => {
    context.calls.push(call);
    if (call.tab === "tab-old") throw "no tab tab-old";
    return "/work";
  };
  const old = {
    key: "box:tab-old",
    machine: "box",
    view: { id: "tab-old", session: "conv-1", running: false },
    attached: Promise.resolve(),
    term: { cols: 80, rows: 24, write() {} },
  };
  const restored = {
    key: "box:tab-new",
    machine: "box",
    view: { id: "tab-new", session: "conv-1", running: false },
    attached: Promise.resolve(),
    term: { cols: 80, rows: 24, write() {} },
  };
  context.tabs = [restored];
  await context.startTab(old);
  assert.equal(context.activated, "box:tab-new");
  assert.deepEqual(
    context.calls.map((call) => [call.cmd, call.tab]),
    [["start_claude", "tab-old"]],
  );
});

test("overlapping no-tab errors refresh a machine once", async () => {
  const context = runSlice("const missingTabId", "const IS_MAC", "globalThis.callOn = callOn;");
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  context.machines = new Map([["box", { transport: { call: async () => { throw "no tab tab-9"; } } }]]);
  context.refreshed = [];
  context.refreshTabs = (machine) => {
    context.refreshed.push(machine);
    return gate;
  };
  const first = context.callOn("box", { cmd: "start_claude", tab: "tab-9" });
  const second = context.callOn("box", { cmd: "write_input", tab: "tab-9", data: "x" });
  const third = context.callOn("box", { cmd: "list_accounts" });
  const settled = Promise.allSettled([first, second, third]);
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.deepEqual(context.refreshed, ["box"]);
  release();
  const results = await settled;
  assert.deepEqual(
    results.map((result) => (result.status === "rejected" ? result.reason : result.value)),
    ["no tab tab-9", "no tab tab-9", "no tab tab-9"],
  );
  assert.deepEqual(context.refreshed, ["box"]);
});
