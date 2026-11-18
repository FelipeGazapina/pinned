import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const composer = read("gui-composer.js");
const phone = read("gui-phone-thread.js");
const fn = (src, name) => {
  const at = src.indexOf(`function ${name}(`);
  assert.ok(at >= 0, `missing ${name}`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const esc = (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const text = (html) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();

const menu = { question: "Which color?", options: [{ n: 1, head: "Red", label: "Red" }, { n: 2, head: "Blue", label: "Blue" }] };
/** What claude asks when it sends several questions at once: its row of question tabs, `at` on the shown one. */
const three = { headers: ["Color", "Sizes", "Shipping"], answered: [false, false, false], at: 0 };

function drawn(tabs, extra = {}) {
  const approval = { innerHTML: "", toggleAttribute() {} };
  const context = { approval, threadOf: () => ({}), esc, icon: (name) => `<i data-icon="${name}"></i>`, multiSelect: () => false, ticked: () => false, ...extra };
  vm.createContext(context);
  vm.runInContext(fn(composer, "renderQuestion"), context);
  context.renderQuestion({ key: "m:t" }, menu, tabs, ["Which color?"]);
  return approval.innerHTML;
}

test("a questionnaire of three reads Question 1 of 3 on the desktop card, then 2 of 3 on the next", () => {
  assert.match(text(drawn(three)), /Color Question 1 of 3/);
  assert.match(text(drawn({ ...three, at: 1 })), /Sizes Question 2 of 3/);
});

test("one question, and the review of the answers, add no count", () => {
  const counted = (html) => html.includes("q-count");
  assert.equal(counted(drawn(null)), false);
  assert.equal(counted(drawn({ headers: ["Color"], answered: [false], at: 0 })), false);
  assert.equal(counted(drawn({ ...three, at: 3 })), false);
});

function sheet(tabs) {
  const context = {
    esc,
    icon: (name) => `<i data-icon="${name}"></i>`,
    phoneSvg: () => "",
    menuKind: () => "question",
    menuIsQuestion: () => true,
    askPlace: () => ({ at: 0, count: 1, next: null }),
    questionTabs: () => tabs,
    freeOption: () => null,
    threadOf: () => ({}),
    multiSelect: () => false,
    ticked: () => false,
    approvalChoices: () => ({}),
    TAB_ROW: /^←\s+(.*?)\s+→\s*$/,
  };
  vm.createContext(context);
  vm.runInContext(fn(phone, "askSheetHtml"), context);
  return context.askSheetHtml({ menu });
}

test("the phone sheet's header says Question 1 of 3 for a questionnaire, and 1 of 1 stays the thread count of a single question", () => {
  assert.match(text(sheet(three)), /Color Question 1 of 3/);
  assert.doesNotMatch(sheet(null), /ask-count">Question/);
  assert.doesNotMatch(sheet({ headers: ["Color"], answered: [false], at: 0 }), /ask-count">Question/);
  assert.match(text(sheet(null)), /1 of 1/, "the thread count is as it was");
});
