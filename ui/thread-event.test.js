import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function takeThreadPage() {
  const src = readFileSync(new URL("./gui.js", import.meta.url), "utf8");
  const start = src.indexOf("function takeThreadPage");
  const end = src.indexOf("async function fetchThread");
  const sandbox = {
    threadOf: (tab) => tab.thread,
    guiOn: () => false,
    active: null,
    takeAgents() {
      throw new Error("a line push must not clear agents");
    },
  };
  vm.createContext(sandbox);
  vm.runInContext(src.slice(start, end), sandbox);
  return sandbox.takeThreadPage;
}

test("a thread push appends the new line and a repeated push does not duplicate it", () => {
  const take = takeThreadPage();
  const tab = { thread: { session: "s", next: 10, items: [{ kind: "text", text: "ready" }], loaded: true, history: true } };
  const page = (from, text, next) => ({ session: "s", from, next, reset: false, items: [{ kind: "text", text }] });
  take(tab, page(10, "visible", 20), 10);
  assert.equal(tab.thread.items.length, 2);
  assert.equal(tab.thread.items[1].text, "visible");
  assert.equal(tab.thread.next, 20);
  assert.equal(take(tab, page(10, "visible", 20), 10), undefined);
  assert.equal(tab.thread.items.length, 2);
  assert.equal(take(tab, page(40, "later", 50), 40), "gap");
  assert.equal(tab.thread.next, 20);
  assert.equal(take(tab, { session: "s2", items: [{ kind: "text", text: "new" }], next: 5, reset: false }, 20), "restart");
  assert.equal(tab.thread.session, "s2");
  assert.equal(tab.thread.items.length, 0);
  assert.equal(tab.thread.next, 0);
});

test("a rewritten transcript replaces the lines, the tasks, and the branch", () => {
  const take = takeThreadPage();
  const tab = {
    thread: {
      session: "s",
      next: 40,
      items: [{ kind: "text", text: "gone" }],
      loaded: true,
      history: true,
      todos: { at: "t0", steps: [{ step: "old", status: "pending" }] },
      branch: "main",
      context: { tokens: 500, at: "t0" },
    },
  };
  take(tab, {
    session: "s",
    next: 12,
    reset: true,
    items: [{ kind: "text", text: "ready" }],
    context: { tokens: 0, at: "t2" },
  }, 40);
  assert.equal(JSON.stringify(tab.thread.items.map((item) => item.text)), JSON.stringify(["ready"]));
  assert.equal(tab.thread.todos, null);
  assert.equal(tab.thread.branch, null);
  assert.equal(tab.thread.next, 12);
  assert.equal(JSON.stringify(tab.thread.context), JSON.stringify({ tokens: 0, at: "t2" }));
});

test("pushed pages and one polled page leave the same tasks, context, and branch", () => {
  const take = takeThreadPage();
  const fresh = () => ({
    thread: {
      session: "s",
      next: 10,
      items: [{ kind: "user", text: "start" }],
      loaded: true,
      history: true,
      branch: "main",
      todos: null,
      context: null,
    },
  });
  const pushed = fresh();
  const polled = fresh();
  take(pushed, { session: "s", next: 20, reset: false, items: [], todos: { at: "t1", steps: [{ step: "Ship the fix", status: "in_progress" }] } }, 10);
  take(pushed, { session: "s", next: 40, reset: false, items: [{ kind: "text", text: "done" }], context: { tokens: 120, at: "t2" } }, 20);
  take(pushed, { session: "s", next: 55, reset: false, items: [{ kind: "user", text: "switch" }], branch: "feature" }, 40);
  take(polled, {
    session: "s",
    next: 55,
    reset: false,
    items: [{ kind: "text", text: "done" }, { kind: "user", text: "switch" }],
    todos: { at: "t1", steps: [{ step: "Ship the fix", status: "in_progress" }] },
    context: { tokens: 120, at: "t2" },
    branch: "feature",
  }, 10);
  const view = (tab) => ({
    items: tab.thread.items.map((item) => item.text),
    todos: tab.thread.todos,
    context: tab.thread.context,
    branch: tab.thread.branch,
    next: tab.thread.next,
  });
  const expected = {
    items: ["start", "done", "switch"],
    todos: { at: "t1", steps: [{ step: "Ship the fix", status: "in_progress" }] },
    context: { tokens: 120, at: "t2" },
    branch: "feature",
    next: 55,
  };
  assert.deepEqual(view(pushed), expected);
  assert.deepEqual(view(polled), expected);
});
