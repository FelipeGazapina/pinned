import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function loadMenuOn() {
  const src = readFileSync(new URL("./terminal-menu.js", import.meta.url), "utf8");
  const context = {};
  vm.createContext(context);
  vm.runInContext(`${src}\nglobalThis.menuOn = menuOn;`, context);
  return context.menuOn;
}

test("a question menu longer than sixteen lines still parses", () => {
  const menuOn = loadMenuOn();
  const lines = ["", "────────────────────────", "Which option?", ""];
  for (let i = 1; i <= 24; i++) lines.push(`${i === 1 ? "❯" : " "} ${i}. Option ${i}`);
  lines.push("", "Enter to select · Esc to cancel");
  const menu = menuOn(lines);
  assert.equal(menu?.options.length, 24);
  assert.equal(menu?.options[0].head, "Option 1");
  assert.equal(menu?.question, "Which option?");
});

test("a long typed prompt with a cursor does not read as a menu", () => {
  const menuOn = loadMenuOn();
  const lines = ["❯ Use the question tool", "  with two questions"];
  for (let i = 0; i < 20; i++) lines.push(`assistant output ${i}`);
  lines.push("", "Enter to select · Esc to cancel");
  assert.equal(menuOn(lines), null);
});
