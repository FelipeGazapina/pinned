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
  assert.equal(tabListAction(true, false), "refresh");
});

test("offline clears tabs and a snapshot is applied as given", () => {
  const tabListAction = loadActions();
  assert.equal(tabListAction(false, false), "clear");
  assert.equal(tabListAction(true, true), "apply");
});

function loadMissingTab() {
  const src = readFileSync(new URL("./main.js", import.meta.url), "utf8");
  const start = src.indexOf("const missingTabId");
  const end = src.indexOf("const callOn");
  const context = {};
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}\nglobalThis.missingTabId = missingTabId;`, context);
  return context.missingTabId;
}

test("a no-tab error names the id the node rejected", () => {
  const missingTabId = loadMissingTab();
  assert.equal(missingTabId("no tab tab-5"), "tab-5");
  assert.equal(missingTabId("tab-5 has no running session"), null);
  const src = readFileSync(new URL("./main.js", import.meta.url), "utf8");
  const callOn = src.slice(src.indexOf("const callOn"), src.indexOf("const IS_MAC"));
  assert.match(callOn, /missingTabId\(error\)/);
  const start = src.slice(src.indexOf("async function startClaude"), src.indexOf("async function setBypass"));
  assert.match(start, /tabs\.includes\(tab\)/);
});

test("showMachines uses the tab list action", () => {
  const src = readFileSync(new URL("./main.js", import.meta.url), "utf8");
  const body = src.slice(src.indexOf("function showMachines"), src.indexOf("function disconnectHub"));
  assert.match(body, /tabListAction\(/);
});
