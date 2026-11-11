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
vm.runInContext(`${src.slice(src.indexOf("const waitingWorkers"), src.indexOf("function renderWorkerBanner"))}\nObject.assign(globalThis, { waitingWorkers, workerBannerHtml, phoneWorkerBannerHtml });`, context);

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

test("a phone's banner names the worker and what it waits on, with Review", () => {
  const html = context.phoneWorkerBannerHtml({ title: "Tidy the README", ask: approval(["approve"]) });
  assert.equal(text(html), "Tidy the README needs approval Review");
  assert.deepEqual(buttons(html), ["Review"]);
  assert.match(html, /data-worker-review/);
  const question = context.phoneWorkerBannerHtml({ title: "Pick a color", ask: { kind: "question", text: "Which?", command: null, choices: null, typed: false } });
  assert.equal(text(question), "Pick a color has a question Review");
});

test("an answer goes out once, for the ask the surface drew, and a refused one says why", async () => {
  const sent = [];
  const notes = [];
  let release;
  context.answerAsk = (tab, choice, seen) => (sent.push([tab.key, choice, seen.command]), new Promise((r) => (release = r)));
  context.showComposerNote = (text) => notes.push(text);
  vm.runInContext(`${src.slice(src.indexOf("async function answerWorker"), src.indexOf("function renderWorkerBanner"))}\nglobalThis.answerWorker = answerWorker;`, context);
  const drawn = { key: "w", title: "Tidy the README", tab: { key: "mini:w" }, ask: approval(["approve", "decline"]) };
  const busy = new Set();
  let paints = 0;
  const first = context.answerWorker(drawn, "approve", busy, () => paints++);
  const second = context.answerWorker({ ...drawn, ask: approval(["approve"]) }, "approve", busy, () => paints++);
  assert.deepEqual(sent, [["mini:w", "approve", "git push origin readme-tidy\nsecond line"]], "a second tap while the first is on its way sends nothing");
  assert.deepEqual([...busy], ["w"]);
  release(true);
  await Promise.all([first, second]);
  assert.deepEqual([...busy], []);
  assert.equal(paints, 2, "the buttons wait, then redraw");
  assert.deepEqual(notes, []);
  const refused = context.answerWorker(drawn, "decline", busy, () => {});
  release(false);
  await refused;
  assert.deepEqual(notes, ["Tidy the README changed its question. Open it to answer."]);
});
