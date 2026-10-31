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
