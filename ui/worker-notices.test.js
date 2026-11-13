import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
const agents = read("./gui-agents.js");
const context = {};
vm.createContext(context);
vm.runInContext(`${agents.slice(agents.indexOf("const NOTICES"), agents.indexOf("/** Whether a prompt is only"))}\nObject.assign(globalThis, { noticesIn });`, context);

const machine = read("../crates/core/src/machine.rs");
const registry = read("../crates/core/src/delegate/registry.rs");

test("the notices the machine types are the ones the divider reads", () => {
  for (const words of [
    'format!("[pinned] Worker {worker} \\"{title}\\" {what}")',
    'format!("finished. Its last message:',
    'format!("needs input: {}"',
    'format!("stopped on an error: {}"',
    '"[pinned] Worker \\"{}\\" on {place}{}{what}: {}.',
  ]) assert.ok(machine.includes(words), `machine.rs no longer says ${words}`);
  assert.ok(registry.includes('pub const ASK_NOTICE: &str = " waits on the user for ";'));
  for (const words of ['format!("[pinned] Delegated task {} \\"{}\\" ({who}) finished: {outcome}.', 'format!("failed: {}"', 'format!("cancelled: {}"', '"result available".into()', '"[pinned] Delegated task {} \\"{}\\" ({}) is still working after', '"[pinned] Delegated task {} \\"{}\\" ({}) replied and still runs']) {
    assert.ok(registry.includes(words), `registry.rs no longer says ${words}`);
  }
});

test("each notice in a prompt reads as its title and what became of it", () => {
  const prompt = [
    "Please look at these.",
    '[pinned] Worker tab-7 "JSON output" needs input: "Claude needs your permission to use Bash"',
    '[pinned] Worker tab-8 "Board check" finished. Its last message:',
    "",
    "Added --json in src/cli.rs; all 12 tests pass.",
    '[pinned] Worker tab-9 "Flaky" stopped on an error: "exit 2"',
    '[pinned] Worker "Slow suite" on mini waits on the user for approval to use "Bash": "git push origin main". The user answers in its thread on mini; it keeps its place, so do not cancel it for that.',
    '[pinned] Delegated task task-1 "Ping" (Claude Code · claude-haiku-5-5) finished: result available. Its thread is tab-4. Call task_status with taskId "task-1".',
    '[pinned] Delegated task task-2 "Lint" (Cursor) finished: failed: "exit 1". Call task_status with taskId "task-2".',
    '[pinned] Delegated task task-3 "Docs" (Codex) finished: cancelled: "by the user". Call task_status with taskId "task-3".',
    '[pinned] Delegated task task-4 "Slow" (Codex) is still working after 12 min, past its budget of 10 min. pinned cancels it at its limit of 30 min. Call task_status with taskId "task-4" to read its recent steps, or task_cancel to stop it.',
  ].join("\n");
  const found = context.noticesIn(prompt);
  assert.equal(found.mine, "Please look at these.");
  assert.deepEqual(Array.from(found.notices, (n) => `${n.title} ${n.what}`), [
    "JSON output needs input",
    "Board check reported back",
    "Flaky stopped on an error",
    "Slow suite needs input",
    "Ping reported back",
    "Lint failed",
    "Docs was cancelled",
    "Slow is still working",
  ]);
  assert.equal(found.notices[1].text, '[pinned] Worker tab-8 "Board check" finished. Its last message:\n\nAdded --json in src/cli.rs; all 12 tests pass.');
  assert.equal(context.noticesIn("Ship [pinned] Worker notes as written."), null);
});
