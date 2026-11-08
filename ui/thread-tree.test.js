import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui-sidebar.js", import.meta.url), "utf8");
const context = {};
vm.createContext(context);
vm.runInContext(`${src.slice(src.indexOf("function spawnTree"), src.indexOf("function toggleFold"))}\nthis.spawnTree = spawnTree; this.spawnRows = spawnRows;`, context);

const tab = (machine, id, opened, spawned_by) => ({ key: `${machine}:${id}`, machine, view: { id, opened, ...(spawned_by ? { spawned_by } : {}) } });
const lines = (list, folded = new Set()) => {
  const { roots, kids } = context.spawnTree(list);
  return Array.from(roots).flatMap((root) => Array.from(context.spawnRows(root, kids, (t) => folded.has(t.key)))).map(({ tab, depth, under }) => `${"  ".repeat(depth)}${tab.view.id}${under ? ` (${under})` : ""}`);
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
