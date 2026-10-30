import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function loadRunOn(callOn) {
  const src = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
  const start = src.indexOf("async function runOn");
  const end = src.indexOf("const pickPreviousWorktree");
  const calls = [];
  const context = {
    callOn,
    calls,
    threadOf: () => ({ workspace: { project: "/Users/me/app" } }),
    showComposerNote: (text) => calls.push({ note: String(text) }),
    openRemote: async (machine, path, bypass) => calls.push({ openRemote: { machine, path, bypass } }),
    revealTab: async (machine, id) => {
      calls.push({ reveal: { machine, id } });
      return { view: { id, running: false, role: "orchestrator", bypass: false } };
    },
    setBypass: async (tab, on) => calls.push({ setBypass: { id: tab.view.id, on } }),
    startTab: async (tab) => calls.push({ start: tab.view.id }),
    closeTab: (tab) => calls.push({ close: tab.view.id }),
  };
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}\nglobalThis.runOn = runOn;`, context);
  return { runOn: context.runOn, calls };
}

test("running an orchestrator on another machine opens an orchestrator there", async () => {
  const { runOn, calls } = loadRunOn(async (machine, call) => {
    calls.push({ machine, call });
    if (call.cmd === "git_remote") return "git@github.com:acme/app.git";
    if (call.cmd === "clone") return "/home/dev/app";
    if (call.cmd === "open_orchestrator") return "tab-9";
    throw new Error(`unexpected ${call.cmd}`);
  });
  await runOn({ view: { id: "tab-1", role: "orchestrator", bypass: true, cwd: "/Users/me/app" } }, "machine-b");
  const orchestrator = calls.find((entry) => entry.call?.cmd === "open_orchestrator");
  assert.ok(orchestrator, `the remote machine was not asked to open an orchestrator: ${JSON.stringify(calls)}`);
  assert.equal(orchestrator.machine, "machine-b");
  assert.equal(orchestrator.call.path, "/home/dev/app");
  assert.equal(calls.some((entry) => entry.openRemote), false);
  assert.deepEqual(calls.filter((entry) => entry.setBypass), [{ setBypass: { id: "tab-9", on: true } }]);
  assert.equal(calls.some((entry) => entry.start), false);
  assert.deepEqual(calls.filter((entry) => entry.close), [{ close: "tab-1" }]);
});

test("running a plain thread on another machine stays a plain thread", async () => {
  const { runOn, calls } = loadRunOn(async (machine, call) => {
    calls.push({ machine, call });
    if (call.cmd === "git_remote") return "git@github.com:acme/app.git";
    if (call.cmd === "clone") return "/home/dev/app";
    throw new Error(`unexpected ${call.cmd}`);
  });
  await runOn({ view: { id: "tab-2", bypass: true, cwd: "/Users/me/app" } }, "machine-b");
  assert.equal(calls.some((entry) => entry.call?.cmd === "open_orchestrator"), false);
  assert.deepEqual(calls.find((entry) => entry.openRemote).openRemote, { machine: "machine-b", path: "/home/dev/app", bypass: true });
  assert.deepEqual(calls.filter((entry) => entry.close), [{ close: "tab-2" }]);
});
