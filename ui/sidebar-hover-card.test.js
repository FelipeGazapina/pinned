import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function loadCase() {
  const src = readFileSync(new URL("./gui-sidebar.js", import.meta.url), "utf8");
  const start = src.indexOf("const cardFitsBeside");
  const end = src.indexOf("function showCard");
  const context = {};
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}\nglobalThis.cardFitsBeside = cardFitsBeside;`, context);
  return context.cardFitsBeside;
}

test("a row card opens beside a row only when it fits in the window", () => {
  const fits = loadCase();
  assert.equal(fits(700, 320, 1200), true);
  assert.equal(fits(480, 320, 520), false);
  assert.equal(fits(868, 320, 1200), true);
  assert.equal(fits(869, 320, 1200), false);
});
