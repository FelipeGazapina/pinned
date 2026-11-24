import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// The header's label helpers, with the sidebar's statsLabel, the delegation page's token format and the settings page's esc they build on.
function load() {
  const read = (file) => readFileSync(new URL(`./${file}`, import.meta.url), "utf8");
  const between = (src, from, to) => {
    const start = src.indexOf(from);
    return src.slice(start, src.indexOf(to, start));
  };
  const sidebar = read("gui-sidebar.js");
  const durationLabel = between(sidebar, "const durationLabel", "/** The thread's active time and cost");
  const statsLabel = between(sidebar, "function statsLabel", "function statsHtml");
  const tokens = between(read("delegation.js"), "function delegationTokens", "const delegationPercent");
  const header = between(read("gui-header.js"), "function headStatsLabel", "function headStatsHtml");
  const esc = between(read("settings.js"), "const esc = ", "\n");
  const context = { statusOf: () => "idle", nowS: () => 0 };
  vm.createContext(context);
  vm.runInContext(`${durationLabel}\n${statsLabel}\n${tokens}\n${esc}\n${header}\nthis.api = { headStatsLabel, headCountsLabel, headLongLabel, headLongHtml };`, context);
  return context.api;
}
const tab = (view) => ({ key: "m:tab", view });

test("the header shows the thread's time and cost, then its turns and tokens in and out", () => {
  const { headLongLabel } = load();
  const thread = tab({ stats: { active_ms: 4320000, cost_usd: 3.42 }, turns: 14, tokens_in: 2400000, tokens_out: 52000 });
  assert.equal(headLongLabel(thread), "1h 12m · $3.42 · 14 turns · 2.4M in · 52k out");
});

test("one turn reads as a turn", () => {
  const { headCountsLabel, headLongLabel } = load();
  assert.equal(headCountsLabel(tab({ turns: 1 })), "1 turn");
  assert.equal(headLongLabel(tab({ stats: { active_ms: 5000, cost_usd: 0.42 }, turns: 1 })), "5s · $0.42 · 1 turn");
});

test("a Captain keeps the cost of its workers too and drops their count before its own counts", () => {
  const { headLongLabel } = load();
  const captain = tab({ stats: { active_ms: 60000, cost_usd: 0.84 }, work: { usd: 0.9, unknown: 0, workers: 1 }, turns: 3, tokens_in: 120000, tokens_out: 4000 });
  assert.equal(headLongLabel(captain), "1m · $0.90 · 3 turns · 120k in · 4k out");
});

test("a thread with no counts shows the header's label as before", () => {
  const { headStatsLabel, headLongLabel } = load();
  const thread = tab({ stats: { active_ms: 5000, cost_usd: 0.42 } });
  assert.equal(headLongLabel(thread), "5s · $0.42");
  assert.equal(headLongLabel(thread), headStatsLabel(thread));
  assert.equal(headLongLabel(tab({ stats: { active_ms: 5000, cost_usd: 0.42 }, turns: null, tokens_in: null, tokens_out: null })), "5s · $0.42");
  assert.equal(headLongLabel(tab({})), "");
});

test("a first turn with tokens but no time or cost yet shows the tokens alone", () => {
  const { headLongLabel } = load();
  assert.equal(headLongLabel(tab({ tokens_in: 2400000, tokens_out: 52000 })), "2.4M in · 52k out");
});

test("headStatsLabel, which the phone's second line reads, never shows the counts", () => {
  const { headStatsLabel } = load();
  const views = [
    { stats: { active_ms: 4320000, cost_usd: 3.42 }, turns: 14, tokens_in: 2400000, tokens_out: 52000 },
    { stats: { active_ms: 60000, cost_usd: 0.84 }, work: { usd: 0.9, unknown: 0, workers: 1 }, turns: 3, tokens_in: 120000, tokens_out: 4000 },
    { turns: 1, tokens_in: 2400000, tokens_out: 52000 },
  ];
  for (const view of views) {
    const label = headStatsLabel(tab(view));
    for (const part of ["turn", " in", " out"]) assert.ok(!label.includes(part), `${JSON.stringify(label)} has ${JSON.stringify(part)}`);
  }
  assert.equal(headStatsLabel(tab(views[0])), "1h 12m · $3.42");
  assert.equal(headStatsLabel(tab(views[2])), "");
});

test("the header's markup keeps the counts in their own span, and reads as the label", () => {
  const { headLongLabel, headLongHtml } = load();
  const text = (html) => html.replace(/<[^>]+>/g, "");
  const full = tab({ stats: { active_ms: 4320000, cost_usd: 3.42 }, turns: 14, tokens_in: 2400000, tokens_out: 52000 });
  assert.equal(headLongHtml(full), `1h 12m · $3.42<span class="counts"> · 14 turns · 2.4M in · 52k out</span>`);
  const tokensOnly = tab({ tokens_in: 2400000, tokens_out: 52000 });
  assert.equal(headLongHtml(tokensOnly), `<span class="counts">2.4M in · 52k out</span>`);
  const plain = tab({ stats: { active_ms: 5000, cost_usd: 0.42 } });
  assert.equal(headLongHtml(plain), "5s · $0.42", "no counts, no span");
  for (const view of [full, tokensOnly, plain]) assert.equal(text(headLongHtml(view)), headLongLabel(view));
});
