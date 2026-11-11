import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui-worker-banner.js", import.meta.url), "utf8");
const context = {
  icon: (name) => `<svg data-icon="${name}"></svg>`,
  esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
  workersOf: () => context.rows,
  rows: [],
};
vm.createContext(context);
vm.runInContext(`${src.slice(src.indexOf("const waitingWorkers"), src.indexOf("function renderWorkerBanner"))}\nObject.assign(globalThis, { waitingWorkers, workerBannerHtml });`, context);

const text = (html) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
const buttons = (html) => [...html.matchAll(/<button[^>]*>([^<]*)<\/button>/g)].map((m) => m[1]);
const approval = (choices) => ({ kind: "approval", text: null, command: "git push origin readme-tidy\nsecond line", choices: choices.map((choice) => ({ label: choice, about: null, choice })), typed: true });

test("an approval names the worker and the command, with Decline, Approve and Open", () => {
  const html = context.workerBannerHtml({ title: "Tidy the README", ask: approval(["approve", "decline", "always"]) }, 0, 1, false);
  assert.equal(text(html), "Worker waiting. Tidy the README needs your approval to run git push origin readme-tidy Decline Approve Open");
  assert.deepEqual(buttons(html), ["Decline", "Approve", "Open"]);
});

test("a question, a plan, or an ask no tap answers offers Open alone", () => {
  const question = context.workerBannerHtml({ title: "Pick a color", ask: { kind: "question", text: "Which?", command: null, choices: null, typed: false } }, 0, 1, false);
  assert.equal(text(question), "Worker waiting. Pick a color has a question Open");
  const plan = context.workerBannerHtml({ title: "Plan the port", ask: { kind: "plan", text: "Plan", command: null, choices: [{ label: "Yes", about: null, choice: { option: 1 } }], typed: true } }, 0, 1, false);
  assert.equal(text(plan), "Worker waiting. Plan the port has a plan to approve Open");
  const unparsed = context.workerBannerHtml({ title: "Run CI", ask: { kind: "approval", text: null, command: null, choices: null, typed: false } }, 0, 1, false);
  assert.equal(text(unparsed), "Worker waiting. Run CI needs your approval Open");
});

test("several waiting workers show the oldest first and count them", () => {
  context.rows = [
    { key: "b", title: "Newer", since: 2000, tab: {}, ask: approval(["approve"]) },
    { key: "c", title: "Done", since: 500, tab: {}, ask: null },
    { key: "a", title: "Older", since: 1000, tab: {}, ask: approval(["approve", "decline"]) },
    { key: "d", title: "Task", since: 100, tab: null, ask: null },
  ];
  const rows = context.waitingWorkers({});
  assert.deepEqual(rows.map((r) => r.key), ["a", "b"]);
  const html = context.workerBannerHtml(rows[1], 1, rows.length, true);
  assert.deepEqual(buttons(html), ["2 of 2", "Approve", "Open"]);
  assert.match(html, /data-worker-answer="approve" disabled/);
});
