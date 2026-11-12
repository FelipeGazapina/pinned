import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui-sidebar.js", import.meta.url), "utf8");
const context = {};
vm.createContext(context);
const agents = readFileSync(new URL("./gui-agents.js", import.meta.url), "utf8");
context.localIdOf = (id) => (id === "hub-m" ? "m" : id);
vm.runInContext(agents.slice(agents.indexOf("function spawnerKey"), agents.indexOf("/** The worker threads")), context);
vm.runInContext(`${src.slice(src.indexOf("function spawnTree"), src.indexOf("function toggleFold"))}\nthis.spawnTree = spawnTree; this.spawnRows = spawnRows;`, context);

const tab = (machine, id, opened, spawned_by) => ({ key: `${machine}:${id}`, machine, view: { id, opened, ...(spawned_by ? { spawned_by } : {}) } });
const lines = (list, folded = new Set(), { done = new Set(), opened = new Set(), active = null } = {}) => {
  const { roots, kids } = context.spawnTree(list);
  const look = { folded: (t) => folded.has(t.key), finished: (t) => done.has(t.key), opened: (t) => opened.has(t.key), shown: (t) => t === active };
  return Array.from(roots)
    .flatMap((root) => Array.from(context.spawnRows(root, kids, look)))
    .map(({ tab, depth, under, finished, of }) => (finished ? `${"  ".repeat(depth)}Finished under ${of.view.id} (${finished.length})` : `${"  ".repeat(depth)}${tab.view.id}${under ? ` (${under})` : ""}`));
};

const plain = tab("m", "tab-1", 1);
const capitao = tab("m", "tab-2", 2);
const worker = tab("m", "tab-3", 3, "tab-1");
const nested = tab("m", "tab-4", 4, "tab-3");
const second = tab("m", "tab-5", 5, "tab-2");
const early = tab("m", "tab-6", 6, "tab-1");
const elsewhere = tab("other", "tab-7", 7, "tab-1");

test("each worker follows the thread that started it, oldest first, as deep as it nests", () => {
  assert.deepEqual(lines([second, nested, plain, early, capitao, worker]), ["tab-1 (3)", "  tab-3 (1)", "    tab-4", "  tab-6", "tab-2 (1)", "  tab-5"]);
});

test("a folded thread hides everything under it and still counts it", () => {
  assert.deepEqual(lines([plain, worker, nested, early], new Set(["m:tab-1"])), ["tab-1 (3)"]);
  assert.deepEqual(lines([plain, worker, nested, early], new Set(["m:tab-3"])), ["tab-1 (3)", "  tab-3 (1)", "  tab-6"]);
});

test("a worker whose parent is not listed, or lives on another machine, stands on its own", () => {
  assert.deepEqual(lines([worker, nested, elsewhere]), ["tab-3 (1)", "  tab-4", "tab-7"]);
});

test("a worker another machine's thread delegated nests under that thread, with the local ones", () => {
  const remote = { ...tab("mini", "tab-9", 9, "tab-1"), view: { id: "tab-9", opened: 9, spawned_by: "tab-1", spawned_on: "hub-m" } };
  const boxed = { ...tab("box", "tab-1", 10, "tab-1"), view: { id: "tab-1", opened: 10, spawned_by: "tab-1", spawned_on: "hub-m" } };
  assert.deepEqual(lines([plain, worker, remote, boxed]), ["tab-1 (3)", "  tab-3", "  tab-9", "  tab-1"]);
});

test("a thread that names itself as its spawner, or two that name each other, still show once", () => {
  const self = tab("m", "tab-8", 8, "tab-8");
  const a = tab("m", "tab-9", 9, "tab-10");
  const b = tab("m", "tab-10", 10, "tab-9");
  const under = tab("m", "tab-11", 11, "tab-9");
  assert.deepEqual(lines([self]), ["tab-8"]);
  assert.deepEqual(lines([a, b]), ["tab-9", "tab-10"]);
  assert.deepEqual(lines([a, b, under]), ["tab-9 (1)", "  tab-11", "tab-10"]);
});

test("a thread's finished workers fold into one Finished group after the ones still at work", () => {
  const third = tab("m", "tab-12", 12, "tab-1");
  const done = new Set(["m:tab-3", "m:tab-12"]);
  assert.deepEqual(lines([plain, worker, nested, early, third], new Set(), { done }), ["tab-1 (4)", "  tab-6", "  Finished under tab-1 (2)"]);
  assert.deepEqual(lines([plain, worker, nested, early, third], new Set(), { done, opened: new Set(["m:tab-1"]) }), ["tab-1 (4)", "  tab-6", "  Finished under tab-1 (2)", "  tab-3 (1)", "    tab-4", "  tab-12"]);
});

test("a closed Finished group still lists the finished worker that is open, and a folded thread hides the group", () => {
  const done = new Set(["m:tab-3", "m:tab-6"]);
  assert.deepEqual(lines([plain, worker, early], new Set(), { done, active: early }), ["tab-1 (2)", "  Finished under tab-1 (2)", "  tab-6"]);
  assert.deepEqual(lines([plain, worker, early], new Set(["m:tab-1"]), { done }), ["tab-1 (2)"]);
  assert.deepEqual(lines([plain, worker, nested, early], new Set(), { done, active: nested }), ["tab-1 (3)", "  Finished under tab-1 (2)", "  tab-3 (1)", "    tab-4"], "the open thread under a finished worker shows too");
});

test("a worker at work, waiting or failed stays in its place, and with none finished there is no group", () => {
  assert.deepEqual(lines([plain, worker, early], new Set(), { done: new Set() }), ["tab-1 (2)", "  tab-3", "  tab-6"]);
});
