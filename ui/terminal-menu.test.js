import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const context = {};
vm.createContext(context);
vm.runInContext(readFileSync(new URL("./terminal-menu.js", import.meta.url), "utf8"), context);
const menuOn = (lines) => JSON.parse(JSON.stringify(context.menuOn(lines)));

const RULE = "─".repeat(100);
const BOX = "─".repeat(42);

const CLAUDE_2_1_319_PREVIEW_SCREEN = [
  "  ⎿  sweep",
  "  Thought for 3s, ran 1 shell command",
  "",
  RULE,
  " ☐ Direction",
  "",
  "Which way should the sweep go?",
  "",
  `❯ 1. North route                  ┌${BOX}┐`,
  "  2. South route                  │ North                                    │",
  "                                  │   step one                               │",
  "                                  │   step two                               │",
  `                                  └${BOX}┘`,
  "",
  "                                  Notes: press n to add notes",
  "",
  RULE,
  "  Chat about this",
  "",
  "Enter to select · ↑/↓ to navigate · n to add notes · Esc to cancel",
];

const PLAIN_SCREEN = [
  RULE,
  " ☐ Direction ",
  "Which way should the sweep go?",
  "❯ 1. North route",
  "     Go up the hill.",
  "  2. South route",
  "     Go down to the river.",
  "  3. Type something.",
  RULE,
  "  4. Chat about this",
  "Enter to select · ↑/↓ to navigate · Esc to cancel",
];

test("a question whose options carry previews reads as its options, without the preview box", () => {
  assert.deepEqual(menuOn(CLAUDE_2_1_319_PREVIEW_SCREEN), {
    question: "☐ Direction\nWhich way should the sweep go?",
    box: "",
    amend: false,
    options: [
      { n: 1, label: "North route", head: "North route", more: "", chosen: true },
      { n: 2, label: "South route", head: "South route", more: "", chosen: false },
      { n: 3, label: "Chat about this", head: "Chat about this", more: "", chosen: false },
    ],
  });
});

test("a question without previews keeps its numbered options and their descriptions", () => {
  assert.deepEqual(menuOn(PLAIN_SCREEN).options, [
    { n: 1, label: "North route Go up the hill.", head: "North route", more: "Go up the hill.", chosen: true },
    { n: 2, label: "South route Go down to the river.", head: "South route", more: "Go down to the river.", chosen: false },
    { n: 3, label: "Type something.", head: "Type something.", more: "", chosen: false },
    { n: 4, label: "Chat about this", head: "Chat about this", more: "", chosen: false },
  ]);
});

test("numbered options that skip a number are not a menu", () => {
  assert.equal(context.menuOn([RULE, "Pick one", "❯ 1. One", "  3. Three", "Enter to select"]), null);
});
