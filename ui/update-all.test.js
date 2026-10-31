import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("./update-all.js", import.meta.url), "utf8");

function open(account) {
  const machines = new Map();
  const calls = [];
  const pending = [];
  const hub = {
    call(id, body) {
      calls.push(id);
      assert.deepEqual(body, { cmd: "update_self" });
      let resolve;
      let reject;
      const promise = new Promise((res, rej) => {
        resolve = res;
        reject = rej;
      });
      pending.push({ resolve, reject });
      return promise;
    },
  };
  const viewListeners = [];
  const api = new Function(
    "machines",
    "hub",
    "viewListeners",
    "LOCAL",
    "seedAccount",
    `let account = seedAccount;\n${source}\nreturn { updateEveryMachine, updateLabel, rowUpdate };`,
  )(machines, hub, viewListeners, "local", account);
  return {
    machines,
    calls,
    viewListeners,
    ...api,
    settle() {
      for (const item of pending.splice(0)) item.resolve(null);
    },
    finish(status) {
      for (const item of pending.splice(0)) item.resolve(status);
    },
    release(ids, status = { state: "up_to_date", version: "1" }) {
      for (const machine of machines.values()) {
        const view = machine.view ?? machine.hubView;
        if (view && ids.includes(view.id)) view.update = status;
      }
      for (const listen of viewListeners.slice()) listen();
    },
    fail(message) {
      const item = pending.shift();
      item.reject(new Error(message));
    },
  };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

test("other online machines are asked before this one, and a box is left out", async () => {
  const page = open({ machine_id: "self" });
  page.machines.set("a", { view: { id: "a", online: true } });
  page.machines.set("box", { view: { id: "box", online: true, upstash_box: "box-1" } });
  page.machines.set("off", { view: { id: "off", online: false } });
  page.machines.set("b", { view: { id: "b", online: true } });
  page.machines.set("local", { hubView: { id: "self", online: true } });
  const done = page.updateEveryMachine();
  assert.deepEqual(page.calls, ["a", "b"]);
  const ignored = page.updateEveryMachine();
  assert.deepEqual(page.calls, ["a", "b"]);
  page.release(["a", "b"]);
  await tick();
  assert.deepEqual(page.calls, ["a", "b", "self"]);
  page.release(["self"]);
  await done;
  await ignored;
  const again = page.updateEveryMachine();
  assert.deepEqual(page.calls, ["a", "b", "self", "a", "b"]);
  page.finish({ state: "failed", to: "", reason: "done" });
  await tick();
  page.finish({ state: "up_to_date", version: "1" });
  await again;
});

test("a call that finishes before the row changes drops its listener", async () => {
  const page = open({ machine_id: "self" });
  page.machines.set("a", { view: { id: "a", online: true } });
  const done = page.updateEveryMachine();
  assert.equal(page.viewListeners.length, 1);
  page.finish({ state: "up_to_date", version: "1" });
  await done;
  assert.equal(page.viewListeners.length, 0);
});

test("a page with no machine id does not update itself", async () => {
  const page = open({ machine_id: null });
  page.machines.set("a", { view: { id: "a", online: true } });
  page.machines.set("b", { view: { id: "b", online: true } });
  const done = page.updateEveryMachine();
  assert.deepEqual(page.calls, ["a", "b"]);
  page.release(["a", "b"]);
  await done;
  assert.deepEqual(page.calls, ["a", "b"]);
});

test("a phase already on the row does not count as acceptance", async () => {
  const page = open({ machine_id: "self" });
  page.machines.set("a", { view: { id: "a", online: true, update: { state: "updating", to: "1" } } });
  page.machines.set("local", { hubView: { id: "self", online: true } });
  const done = page.updateEveryMachine();
  assert.deepEqual(page.calls, ["a"]);
  for (const listen of page.viewListeners.slice()) listen();
  await tick();
  assert.deepEqual(page.calls, ["a"]);
  page.machines.get("a").view.update = { state: "restarting", to: "1" };
  for (const listen of page.viewListeners.slice()) listen();
  await tick();
  assert.deepEqual(page.calls, ["a", "self"]);
  page.release(["self"]);
  await done;
});

test("a rejected call is shown on that machine", async () => {
  const page = open({ machine_id: "self" });
  page.machines.set("a", { view: { id: "a", online: true, name: "vps" } });
  page.machines.set("local", { hubView: { id: "self", online: true, name: "this mac" } });
  const done = page.updateEveryMachine();
  page.fail("vps did not answer");
  await tick();
  assert.equal(page.machines.get("a").view.update.state, "failed");
  assert.equal(page.machines.get("a").view.update.reason, "vps did not answer");
  await tick();
  assert.deepEqual(page.calls, ["a", "self"]);
  page.release(["self"]);
  await done;
});

test("an updating phase does not start this machine", async () => {
  const page = open({ machine_id: "self" });
  page.machines.set("a", { view: { id: "a", online: true } });
  page.machines.set("local", { hubView: { id: "self", online: true } });
  const done = page.updateEveryMachine();
  page.machines.get("a").view.update = { state: "updating", to: "9.9.9" };
  for (const listen of page.viewListeners.slice()) listen();
  page.settle();
  await tick();
  await tick();
  assert.deepEqual(page.calls, ["a"]);
  page.machines.get("a").view.update = { state: "failed", to: "9.9.9", reason: "the hub did not answer the sign in" };
  for (const listen of page.viewListeners.slice()) listen();
  await tick();
  assert.deepEqual(page.calls, ["a", "self"]);
  page.release(["self"]);
  await done;
});

test("a machine that never reports a final phase does not hold this one", async () => {
  const page = open({ machine_id: "self" });
  page.updateEveryMachine.phaseWait = 20;
  page.machines.set("a", { view: { id: "a", online: true } });
  page.machines.set("local", { hubView: { id: "self", online: true } });
  const done = page.updateEveryMachine();
  assert.deepEqual(page.calls, ["a"]);
  await done;
  assert.deepEqual(page.calls, ["a", "self"]);
});

test("a row prints the phase it was given", () => {
  const page = open(null);
  assert.equal(page.updateLabel({ state: "up_to_date", version: "0.16.0" }), "up to date · v0.16.0");
  assert.equal(page.updateLabel({ state: "updating", to: "0.17.0" }), "updating to v0.17.0");
  assert.equal(page.updateLabel({ state: "updating", to: "" }), "updating");
  assert.equal(page.updateLabel({ state: "restarting", to: "0.17.0" }), "restarting into v0.17.0");
  assert.equal(page.updateLabel({ state: "failed", reason: "came back on 0.16.0" }), "update failed: came back on 0.16.0");
  assert.equal(page.updateLabel({ state: "cannot", reason: "update by hand" }), "update by hand");
  assert.equal(page.updateLabel({ state: "elsewhere" }), "");
  assert.equal(page.rowUpdate({ version: "0.16.0" }), "v0.16.0");
  assert.equal(page.rowUpdate({ version: "" }), "");
  assert.equal(page.rowUpdate({ version: "0.16.0", update: { state: "cannot", reason: "an Upstash box is replaced by the hub" } }), "an Upstash box is replaced by the hub");
});
