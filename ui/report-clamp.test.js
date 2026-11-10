import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const timeline = readFileSync(new URL("./gui-timeline.js", import.meta.url), "utf8");
function between(src, startAt, endAt) {
  const start = src.indexOf(startAt);
  const end = src.indexOf(endAt, start + startAt.length);
  if (start < 0 || end < 0) throw new Error(`missing slice ${startAt}`);
  return src.slice(start, end);
}

const context = {
  esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
  markdown: (text) => `<p>${text}</p>`,
  whenHtml: () => "",
};
vm.createContext(context);
vm.runInContext(between(timeline, "function reportRowHtml", "/** The prompt bubble"), context);
const row = (text, open = false) => context.reportRowHtml({ project: "site", machineName: "node", machineId: "m1", tabId: "t1", text }, "t0", { open: new Set(open ? ["full:u1:0"] : []) }, "full:u1:0");

const table = Array.from({ length: 60 }, (_, i) => `| ${i} | PR #${i} | merged |`).join("\n");

test("a long report folds to a few lines with Show more", () => {
  const html = row(table);
  assert.match(html, /class="body md clamped"/);
  assert.match(html, /<button type="button" class="show-full" data-toggle="full:u1:0" aria-expanded="false">Show more<\/button>/);
});

test("opening it shows the whole text and offers Show less", () => {
  const html = row(table, true);
  assert.doesNotMatch(html, /clamped/);
  assert.ok(html.includes(table), "the full text is in the row");
  assert.match(html, />Show less<\/button>/);
});

test("a short report, even one a few lines long, is drawn in full with no button", () => {
  for (const text of ["CI is green.", "one\ntwo\nthree", "x".repeat(600)]) {
    const html = row(text);
    assert.doesNotMatch(html, /clamped|show-full/, text.slice(0, 20));
  }
});

test("a report folds past 600 characters or 8 lines", () => {
  assert.match(row("x".repeat(601)), /clamped/);
  assert.match(row(Array(9).fill("line").join("\n")), /clamped/);
  assert.doesNotMatch(row(Array(8).fill("line").join("\n")), /clamped/);
});

test("an old report with no reply says so in words, with the Captain's thread one click away", () => {
  const html = row("(no reply)");
  assert.ok(html.includes("Captain didn't write a reply. Open its thread."));
  assert.match(html, /data-report-tab="t1"/);
  assert.doesNotMatch(html, /\(no reply\)|show-full/);
});

test("a row drawn without a view state still draws", () => {
  assert.match(context.reportRowHtml({ project: "p", machineName: "n", machineId: "m", tabId: "t", text: "ok" }, "t0"), /class="report-row"/);
});
