import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const usageSrc = read("gui-usage.js");
const settingsSrc = read("settings.js");
const between = (src, from, to) => {
  const start = src.indexOf(from);
  assert.ok(start >= 0, `${from} is in the source`);
  return src.slice(start, src.indexOf(to, start));
};
const line = (src, prefix) => src.split("\n").find((l) => l.startsWith(prefix));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const text = (html) => html.replace(/<[^>]*>/g, " ").replace(/&#60;/g, "<").replace(/&#62;/g, ">").replace(/&#38;/g, "&").replace(/\s+/g, " ").trim();

const NOW = 1793491210;
const AT = NOW - 3600 - ((NOW - 3600) % 900);
const ID_A = "a1b2c3d4-0000-4000-8000-00000000000a";
const ID_B = "b5c6d7e8-0000-4000-8000-00000000000b";
const ID_U = "c9d0e1f2-0000-4000-8000-00000000000c";
/** A `token_usage` row of `tokens` input tokens costing `usd`, unpriced when `usd` is null; `session_id` left out when `id` is null, as an older machine sends it. */
const row = (id, usd, tokens, fields = {}) => ({
  at: AT, model: "claude-opus-5-5", session: 0, speed: "standard", input: tokens, cached: 0, cache_write: 0, output: 0,
  cost: usd == null ? null : { input: usd, cache_read: 0, cache_write: 0, output: 0 }, premium: 0, savings: 0, ...(id == null ? {} : { session_id: id }), ...fields,
});

/** The spend view as the Usage page draws it, over each machine's `rows`, the `selected` machines, the `metric`, the `breakdown` and the open `tabs`. */
function usagePage({ rows, selected = null, metric = "cost", breakdown = "session", tabs = [] }) {
  const context = {
    machineName: (m) => (m === "macbook" ? "MacBook" : m === "macmini" ? "Mac mini" : m),
    machines: new Map(Object.keys(rows).map((m) => [m, {}])),
    usageState: { metric, days: 30, breakdown, now: NOW, selected,
      envs: new Map(Object.entries(rows).map(([m, list]) => [m, { rows: list.map((r) => ({ ...r, machine: m })), loaded: true, loading: false, error: "", threads: [] }])) },
    tabs,
    threadTitle: (tab) => tab.title,
    icon: () => "",
    popAttr: () => "",
    sharesHtml: () => "",
    costByType: () => [],
    speedParts: () => [],
    tokensByType: () => [],
    threadsHtml: () => "",
  };
  vm.createContext(context);
  vm.runInContext([
    line(settingsSrc, "const esc = "),
    line(usageSrc, "const CLAUDE_COLOR = "),
    line(usageSrc, "const usd = "),
    fn(usageSrc, "tokensText"),
    line(usageSrc, "const percentText = "),
    line(usageSrc, "const rowCost = "),
    line(usageSrc, "const rowTokens = "),
    line(usageSrc, "const monthDay = "),
    line(usageSrc, "const hourText = "),
    between(usageSrc, "const localDay = ", "/** Rows come in quarter hours"),
    line(usageSrc, "const bucketOf = "),
    line(usageSrc, "const usageEnvs = "),
    line(usageSrc, "const usageSelected = "),
    fn(usageSrc, "summarize"),
    line(usageSrc, "const costUnknown = "),
    between(usageSrc, "const segmented = ", "const pickButton = "),
    fn(usageSrc, "spendHtml"),
    line(usageSrc, "const sessionThread = "),
    fn(usageSrc, "sessionRowsHtml"),
    "this.summarize = summarize; this.spendHtml = spendHtml;",
  ].join("\n"), context);
  const s = context.summarize();
  const html = context.spendHtml(s);
  const table = html.match(/<table class="by-model by-session">[\s\S]*?<\/table>/)?.[0] ?? null;
  return { html, s, table };
}
/** The by-session table's body rows: each one's class, its cells' markup and its text. */
function sessionRows(table) {
  const body = table.match(/<tbody>([\s\S]*)<\/tbody>/)[1];
  return [...body.matchAll(/<tr([^>]*)>([\s\S]*?)<\/tr>/g)].map(([whole, attrs, inner]) => ({
    whole, more: / class="more"/.test(attrs), cells: [...inner.matchAll(/<td[^>]*>([\s\S]*?)<\/td>/g)].map(([, cell]) => cell), text: text(inner),
  }));
}
const figures = (r) => [text(r.cells[0]), text(r.cells[1]), text(r.cells[2]), text(r.cells[3]), text(r.cells[4])];

const TWO_MACHINES = {
  macbook: [row(ID_A, 1, 100), row(ID_A, 2, 100, { model: "claude-haiku-5" }), row(ID_B, 1, 5000)],
  macmini: [row(ID_A, 0.5, 50), row(ID_U, null, 10000)],
};

test("the Breakdown control offers Model, Day and Session, and Session draws its table in both views", () => {
  for (const metric of ["cost", "tokens"]) {
    const { html, table } = usagePage({ rows: TWO_MACHINES, metric });
    const control = html.match(/<span class="seg" role="radiogroup" aria-label="Usage breakdown" data-seg="breakdown">[\s\S]*?<\/span>/)[0];
    assert.deepEqual([...control.matchAll(/data-value="([^"]+)"[^>]*>([^<]+)</g)].map(([, value, label]) => [value, label]), [["model", "Model"], ["day", "Day"], ["session", "Session"]]);
    assert.match(control, /aria-checked="true" data-value="session"/);
    assert.ok(table, `${metric}: the Session table is drawn`);
    assert.equal(text(table.match(/<thead>([\s\S]*?)<\/thead>/)[1]), "# Session Cost Share Tokens");
    assert.doesNotMatch(html, /<table class="by-model">|<table class="by-time">/, `${metric}: in place of the other two`);
  }
  for (const breakdown of ["model", "day"]) assert.equal(usagePage({ rows: TWO_MACHINES, breakdown }).table, null, `${breakdown} draws no Session table`);
});

test("rows group by machine and session id, ordered by cost in the Cost view and by tokens in the Tokens view", () => {
  const cost = sessionRows(usagePage({ rows: TWO_MACHINES }).table);
  assert.deepEqual(cost.map(figures), [
    ["1", "a1b2c3d4 MacBook · Thread not open", "$3.00", "66.7%", "200"],
    ["2", "b5c6d7e8 MacBook · Thread not open", "$1.00", "22.2%", "5K"],
    ["3", "a1b2c3d4 Mac mini · Thread not open", "$0.50", "11.1%", "50"],
    ["4", "c9d0e1f2 Mac mini · Thread not open", "Unpriced", "", "10K"],
  ], "one id on two machines is two sessions; a session's models add up; an unpriced session has no cost share");
  assert.match(cost[3].cells[2], /<span class="muted">Unpriced<\/span>/);
  assert.match(cost[0].cells[3], /^66\.7%$/);
  assert.match(cost[0].whole, /<td class="num fig share-col">66\.7%<\/td>/, "Share hides on phones as the Model table's does");

  const tokens = sessionRows(usagePage({ rows: TWO_MACHINES, metric: "tokens" }).table);
  assert.deepEqual(tokens.map(figures), [
    ["1", "c9d0e1f2 Mac mini · Thread not open", "Unpriced", "65.6%", "10K"],
    ["2", "b5c6d7e8 MacBook · Thread not open", "$1.00", "32.8%", "5K"],
    ["3", "a1b2c3d4 MacBook · Thread not open", "$3.00", "1.3%", "200"],
    ["4", "a1b2c3d4 Mac mini · Thread not open", "$0.50", "0.3%", "50"],
  ], "in the Tokens view an unpriced session still has its token share");

  const ties = { macbook: [row(ID_A, 1, 10), row(ID_B, 1, 20), row(ID_U, 2, 5)] };
  assert.deepEqual(sessionRows(usagePage({ rows: ties }).table).map((r) => text(r.cells[1]).slice(0, 8)), ["c9d0e1f2", "b5c6d7e8", "a1b2c3d4"], "equal costs go by tokens");
  const tokenTies = { macbook: [row(ID_A, 1, 10), row(ID_B, 2, 10), row(ID_U, 0, 20)] };
  assert.deepEqual(sessionRows(usagePage({ rows: tokenTies, metric: "tokens" }).table).map((r) => text(r.cells[1]).slice(0, 8)), ["c9d0e1f2", "b5c6d7e8", "a1b2c3d4"], "equal tokens go by cost");
});

test("a session open in a thread is named by it and opens it; one with no open thread shows its id's head; one with no id says so", () => {
  const rows = { macbook: [row(ID_A, 3, 100), row(ID_B, 2, 100), row(null, 1, 100)], macmini: [row(ID_A, 0.5, 100)] };
  const tabs = [{ key: "macbook:tab-1", machine: "macbook", view: { session: ID_A }, title: `Fix the <form> "login"` }];
  const [open, shut, none, elsewhere] = sessionRows(usagePage({ rows, tabs }).table);

  assert.match(open.cells[1], /^<button type="button" data-usage-thread="macbook:tab-1"><span>Fix the &#60;form&#62; &#34;login&#34;<\/span><\/button><small class="muted">MacBook<\/small>$/);
  assert.doesNotMatch(open.text, /Thread not open/);

  assert.match(shut.cells[1], new RegExp(`<span class="id" title="${ID_B}">b5c6d7e8</span>`));
  assert.equal(text(shut.cells[1]), "b5c6d7e8 MacBook · Thread not open");
  assert.doesNotMatch(shut.whole, /<button/);

  assert.equal(text(none.cells[1]), "No session id MacBook");
  assert.match(none.cells[1], /<span class="muted">No session id<\/span>/);
  assert.doesNotMatch(none.whole, /<button|Thread not open/);

  assert.equal(text(elsewhere.cells[1]), "a1b2c3d4 Mac mini · Thread not open", "the same session id open on another machine is not this machine's thread");
  assert.doesNotMatch(elsewhere.whole, /<button/);
});

test("the machine's name shows only with more than one environment selected", () => {
  const tabs = [{ key: "macbook:tab-1", machine: "macbook", view: { session: ID_A }, title: "Ship it" }];
  const rows = { macbook: [row(ID_A, 3, 100), row(ID_B, 2, 100), row(null, 1, 100)], macmini: [row(ID_U, 0.5, 100)] };
  const both = sessionRows(usagePage({ rows, tabs }).table);
  assert.deepEqual(both.map((r) => text(r.cells[1])), ["Ship it MacBook", "b5c6d7e8 MacBook · Thread not open", "No session id MacBook", "c9d0e1f2 Mac mini · Thread not open"]);
  const one = sessionRows(usagePage({ rows, tabs, selected: new Set(["macbook"]) }).table);
  assert.deepEqual(one.map((r) => text(r.cells[1])), ["Ship it", "b5c6d7e8 Thread not open", "No session id"]);
  assert.ok(one.every((r) => !/MacBook|Mac mini|·/.test(r.text)));
  assert.doesNotMatch(one[0].cells[1], /<small/, "an open thread on one environment has nothing below its name");
});

test("the table shows the top 50 sessions and says how many it left out", () => {
  const many = (n) => ({ macbook: Array.from({ length: n }, (_, i) => row(`${String(i).padStart(8, "0")}-0000-4000-8000-000000000000`, n - i, 10)) });
  const capped = sessionRows(usagePage({ rows: many(53) }).table);
  const ranked = capped.filter((r) => !r.more);
  assert.equal(ranked.length, 50);
  assert.deepEqual(ranked.map((r) => text(r.cells[0])), Array.from({ length: 50 }, (_, i) => String(i + 1)));
  assert.equal(text(ranked[0].cells[1]), "00000000 Thread not open");
  assert.equal(text(ranked[49].cells[1]), "00000049 Thread not open", "the 50 dearest stay");
  assert.equal(capped.length, 51);
  assert.equal(capped.at(-1).whole, `<tr class="more"><td class="muted" colspan="5">3 more sessions not shown</td></tr>`);
  assert.equal(sessionRows(usagePage({ rows: many(51) }).table).at(-1).text, "1 more session not shown");
  assert.ok(sessionRows(usagePage({ rows: many(50) }).table).every((r) => !r.more), "fifty sessions leave nothing out");
  assert.equal(sessionRows(usagePage({ rows: many(1200) }).table).at(-1).text, "1,150 more sessions not shown");
});

test("an empty window says so", () => {
  const { table } = usagePage({ rows: { macbook: [] } });
  assert.match(table, /<tbody><tr class="empty"><td colspan="5">No activity in this window\.<\/td><\/tr><\/tbody>/);
});

test("rows from an older machine with no session ids make one row per machine and still count as today", () => {
  const rows = {
    macbook: [row(ID_A, 2, 100, { session: 0 }), row(null, 1, 100, { session: 1 })],
    macmini: [row(null, 1, 100, { session: 0 }), row(null, 2, 100, { session: 1, model: "claude-haiku-5" }), row(null, 0.5, 100, { session: 2 })],
  };
  const { html, s, table } = usagePage({ rows });
  const sessions = sessionRows(table);
  assert.deepEqual(sessions.map(figures), [
    ["1", "No session id Mac mini", "$3.50", "53.8%", "300"],
    ["2", "a1b2c3d4 MacBook · Thread not open", "$2.00", "30.8%", "100"],
    ["3", "No session id MacBook", "$1.00", "15.4%", "100"],
  ]);
  assert.doesNotMatch(text(html), /Update required|Unavailable|error/i);
  assert.equal(s.sessions.size, 5, "the page counts each machine's session numbers as before");
  assert.match(html, /<span class="fig">5 sessions<\/span>/);
  assert.equal(s.cost, 6.5);
  assert.equal(s.tokens, 500);
});

test("opening a usage row's thread shows it, and one closed since the page drew redraws the page", () => {
  const calls = [];
  const tab = { key: "macbook:tab-1" };
  const context = { tabByKey: (key) => [tab].find((t) => t.key === key), activate: (t) => calls.push(["activate", t]), renderUsagePage: () => calls.push(["render"]) };
  vm.createContext(context);
  vm.runInContext(`${fn(usageSrc, "openUsageThread")}\nthis.openUsageThread = openUsageThread;`, context);
  context.openUsageThread("macbook:tab-1");
  assert.equal(calls.length, 1);
  assert.equal(calls[0][0], "activate");
  assert.equal(calls[0][1], tab);
  calls.length = 0;
  assert.doesNotThrow(() => context.openUsageThread("macbook:gone"));
  assert.deepEqual(calls, [["render"]]);
});

test("a click on a session row, or on its button, opens its thread before the Model check", () => {
  const listener = between(usageSrc, `pageView.addEventListener("click", (e) => {`, "\n});\n");
  assert.ok(listener.indexOf("openUsageThread") < listener.indexOf("tr[data-model]"), "threads come before models");
  const calls = [];
  const context = { pageView: {}, shellPage: () => "usage", openUsageThread: (key) => calls.push(["thread", key]), openModel: (model) => calls.push(["model", model]) };
  context.pageView.addEventListener = (type, f) => (context.click = f);
  vm.createContext(context);
  vm.runInContext(`${listener}\n});`, context);
  const button = { dataset: { usageThread: "macbook:tab-1" } };
  const sessionRow = { querySelector: (selector) => (selector === "[data-usage-thread]" ? button : null) };
  const click = (closest) => context.click({ target: { closest: (selector) => closest[selector] ?? null } });
  click({ tr: sessionRow });
  click({ "[data-usage-thread]": button, tr: sessionRow });
  click({ tr: { querySelector: () => null }, "tr[data-model]": { dataset: { model: "claude-opus-5-5" } } });
  click({ tr: { querySelector: () => null } });
  assert.deepEqual(calls, [["thread", "macbook:tab-1"], ["thread", "macbook:tab-1"], ["model", "claude-opus-5-5"]]);
});

test("rows that open a thread take the Model rows' pointer and hover, and the Threads button covers its cell", () => {
  const css = read("gui-usage.css");
  assert.match(css, /\.usage :is\(tr\[data-model\], tr:has\(\[data-usage-thread\]\)\) \{ position: relative; cursor: pointer; \}/);
  assert.match(css, /\.usage :is\(tr\[data-model\], tr:has\(\[data-usage-thread\]\)\):is\(:hover, :has\(:focus-visible\)\) td \{ background: color-mix\(in srgb, var\(--g-fg\) 2\.4%, transparent\); \}/);
  assert.match(css, /\.usage table\.threads \[data-usage-thread\]::after \{ content: ""; position: absolute; inset: 0; \}/);
});
