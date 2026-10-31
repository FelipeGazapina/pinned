import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function loadActions() {
  const src = readFileSync(new URL("./main.js", import.meta.url), "utf8");
  const start = src.indexOf("function tabListAction");
  const end = src.indexOf("function showMachines");
  const context = {};
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}\nglobalThis.tabListAction = tabListAction;`, context);
  return context.tabListAction;
}

test("a machine that is already online still reloads its tabs when the update has no snapshot", () => {
  const tabListAction = loadActions();
  assert.equal(tabListAction(true, true, false), "refresh");
});

test("offline clears tabs and a snapshot is applied as given", () => {
  const tabListAction = loadActions();
  assert.equal(tabListAction(true, false, false), "clear");
  assert.equal(tabListAction(false, true, true), "apply");
});
