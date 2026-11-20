import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
const line = (start) => {
  const from = src.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  return src.slice(from, src.indexOf("\n", from));
};
const decl = (start) => {
  const from = src.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  const close = src.slice(from).search(/\n\}\n/);
  assert.ok(close > 0, `missing the end of ${start}`);
  return src.slice(from, from + close + 2);
};

/** traitEntries as the menu builder runs it, with `efforts` on the model the tab picked. */
function entries(efforts) {
  const context = {
    modelOf: () => ({ efforts, fast: false }),
    effortChoice: () => null,
    ultrathinkInText: () => false,
    ultrathinking: () => false,
  };
  vm.createContext(context);
  vm.runInContext(
    `${line("const REASONING")}\n${line("const ALL_REASONING")}\n${decl("function traitEntries")}\nglobalThis.traitEntries = traitEntries;`,
    context,
  );
  return context.traitEntries({ traits: {} });
}

test("the Reasoning menu offers the gateway's levels with their labels", () => {
  const levels = Array.from(entries(["low", "medium", "high", "xhigh", "max"])).filter((e) => typeof e.run === "function");
  assert.deepEqual(levels.map((e) => e.label), ["Low", "Medium", "High", "Extra High", "Max"]);
  assert.ok(levels.every((e) => typeof e.label === "string" && e.label.length > 0), "every entry the menu draws has a label");
});

test("a model the gateway offers no usable level for draws no Reasoning entries", () => {
  assert.deepEqual(Array.from(entries([])), []);
});
