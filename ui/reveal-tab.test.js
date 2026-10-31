import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function load(callOn) {
  const src = readFileSync(new URL("./main.js", import.meta.url), "utf8");
  const start = src.indexOf("function setTabs");
  const end = src.indexOf("/** The machine's accounts");
  const tabs = [];
  const calls = [];
  const sandbox = {
    tabs,
    calls,
    machines: new Map([["remote", { tabPaths: "" }]]),
    closing: new Set(),
    keyOf: (machine, id) => `${machine}:${id}`,
    tabByKey: (key) => tabs.find((tab) => tab.key === key),
    removeTab() {},
    updateView(tab, view) {
      tab.view = view;
    },
    addTab(machine, view) {
      tabs.push({ key: `${machine}:${view.id}`, machine, view });
    },
    placeTabs() {},
    refreshProjects() {},
    render() {},
    activate(tab) {
      sandbox.active = tab;
    },
    callOn: async (machine, call) => {
      calls.push({ machine, call });
      return callOn(call);
    },
    setTimeout,
    clearTimeout,
  };
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(`${src.slice(start, end)}\nglobalThis.revealTab = revealTab;\nglobalThis.setTabs = setTabs;`, sandbox);
  return sandbox;
}

const view = (id) => ({ id, cwd: "/work" });

test("a tab the machine already pushed is shown without asking for the list again", async () => {
  const app = load(() => {
    throw new Error("tabs was fetched");
  });
  app.setTabs("remote", [view("tab-1")]);
  const tab = await app.revealTab("remote", "tab-1");
  assert.equal(tab.view.id, "tab-1");
  assert.equal(app.active, tab);
  assert.deepEqual(app.calls, []);
});

test("a tab push that arrives after the open answers is still not a second fetch", async () => {
  const app = load(() => {
    throw new Error("tabs was fetched");
  });
  const shown = app.revealTab("remote", "tab-2");
  await new Promise((resolve) => setTimeout(resolve, 20));
  app.setTabs("remote", [view("tab-2")]);
  const tab = await shown;
  assert.equal(tab.view.id, "tab-2");
  assert.deepEqual(app.calls, []);
});

test("opening a remote folder asks the machine to open that folder, then shows it and starts it", async () => {
  const src = readFileSync(new URL("./main.js", import.meta.url), "utf8");
  const start = src.indexOf("async function openRemote");
  const end = src.indexOf("async function moveTab");
  const calls = [];
  const sandbox = {
    calls,
    callOn: async (_machine, call) => {
      calls.push(call);
      if (call.cmd !== "open_tab") throw new Error(`unexpected ${call.cmd}`);
      return "tab-3";
    },
    revealTab: async (_machine, id) => {
      calls.push({ reveal: id });
      return { view: { id } };
    },
    startTab: async (tab) => {
      calls.push({ start: tab.view.id });
    },
    setTimeout,
    clearTimeout,
  };
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(`${src.slice(start, end)}\nglobalThis.openRemote = openRemote;`, sandbox);
  const tab = await sandbox.openRemote("remote", "/work", true);
  assert.equal(tab.view.id, "tab-3");
  assert.equal(calls[0].cmd, "open_tab");
  assert.equal(calls[0].path, "/work");
  assert.equal(calls[0].bypass, true);
  assert.equal(calls[1].reveal, "tab-3");
  assert.equal(calls[2].start, "tab-3");
  assert.equal(calls.length, 3);
});

test("a node that never pushes the tab falls back to one tabs fetch", async () => {
  const app = load((call) => {
    assert.equal(call.cmd, "tabs");
    return [view("tab-9")];
  });
  const tab = await app.revealTab("remote", "tab-9");
  assert.equal(tab.view.id, "tab-9");
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0].machine, "remote");
  assert.equal(app.calls[0].call.cmd, "tabs");
});
