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

test("a preview question with the cursor on its unnumbered last option still reads as a menu", () => {
  const screen = CLAUDE_2_1_319_PREVIEW_SCREEN.map((l) => l.replace("❯ 1.", "  1.").replace(/^  Chat about this/, "❯ Chat about this"));
  assert.deepEqual(menuOn(screen).options.map((o) => [o.n, o.head, o.chosen]), [
    [1, "North route", false],
    [2, "South route", false],
    [3, "Chat about this", true],
  ]);
});

test("a box in earlier output leaves a permission request below it whole", () => {
  const screen = [
    "⏺ ┌──────────┐",
    "  │ client   │",
    "  └──────────┘",
    RULE,
    " Bash command",
    "   echo sweep",
    " Do you want to proceed?",
    " ❯ 1. Yes",
    "   2. Yes, and don't ask again for echo commands in this folder",
    "   3. No, and tell Claude what to do differently (esc)",
    "",
    " Esc to cancel · Tab to amend",
  ];
  assert.deepEqual(menuOn(screen).options.map((o) => o.head), ["Yes", "Yes, and don't ask again for echo commands in this folder", "No, and tell Claude what to do differently (esc)"]);
});

test("a box in tool output keeps the descriptions of a question below it", () => {
  const screen = ["  ⎿  ┌──────────┐", "     │ client   │", "     └──────────┘", ...PLAIN_SCREEN];
  assert.equal(menuOn(screen).options[0].more, "Go up the hill.");
});

test("a draft that starts with a number in claude's prompt box is not a menu", () => {
  assert.equal(context.menuOn(["⏺ Done.", "", RULE, "❯ 1. fix the bug", RULE, "  ⏵⏵ accept edits on (shift+tab to cycle)"]), null);
});

test("numbered options that skip a number are not a menu", () => {
  assert.equal(context.menuOn([RULE, "Pick one", "❯ 1. One", "  3. Three", "Enter to select"]), null);
});

test("a numbered line with a box beside it in earlier output is not a preview for the menu below", () => {
  const output = "  1. Client sends   ┌──────────┐";
  assert.equal(context.menuOn([output, RULE, "❯ 1. fix the bug", RULE, "  ⏵⏵ accept edits on (shift+tab to cycle)"]), null);
  const model = menuOn([output, RULE, " Select model", "", " ❯ Default", "   Opus", "   Sonnet", "", " Enter to confirm · Esc to exit"]);
  assert.deepEqual(model.options.map((o) => o.head), ["Default", "Opus", "Sonnet"]);
});
