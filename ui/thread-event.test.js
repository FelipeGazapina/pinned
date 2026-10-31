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
