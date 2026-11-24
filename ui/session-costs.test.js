import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const usageSrc = read("gui-usage.js");
const sidebarSrc = read("gui-sidebar.js");
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
const plain = (value) => JSON.parse(JSON.stringify(value));
const text = (html) => html.replace(/<[^>]*>/g, " ").replace(/&#60;/g, "<").replace(/&#62;/g, ">").replace(/&#38;/g, "&").replace(/\s+/g, " ").trim();
const rowsOf = (html) => [...html.matchAll(/<tr>([\s\S]*?)<\/tr>/g)].slice(1).map(([, row]) => text(row));
const headOf = (html) => text(html.match(/<thead>([\s\S]*?)<\/thead>/)[1]);

const NOW = 1793491210;
const DAY = 86400;
const saved = (session, fields = {}) => ({ session, title: `Thread ${session}`, project: "/w/site", account: "claude", turns: 2, active_ms: 150000, cost_usd: 1, first_at: NOW - 7200, last_at: NOW - 3600, ...fields });

/** The Threads section as the Cost view draws it, over each machine's `threads`, the `selected` machines and the open `tabs`. */
function page({ threads, selected = null, days = 30, tabs = [] }) {
  const context = {
    tabs,
    machineName: (m) => (m === "macbook" ? "MacBook" : m === "macmini" ? "Mac mini" : m),
    machines: new Map(Object.keys(threads).map((m) => [m, {}])),
    usageState: { days, now: NOW, selected, envs: new Map(Object.entries(threads).map(([m, list]) => [m, { rows: [], loaded: true, loading: false, error: "", threads: list.map((r) => ({ ...r, machine: m })) }])) },
  };
  vm.createContext(context);
  vm.runInContext([
    line(settingsSrc, "const esc = "),
    line(sidebarSrc, "const durationLabel = "),
    line(sidebarSrc, "const costLabel = "),
    between(usageSrc, "const localDay = ", "/** Rows come in quarter hours"),
    line(usageSrc, "const bucketOf = "),
    line(usageSrc, "const usageEnvs = "),
    line(usageSrc, "const usageSelected = "),
    line(usageSrc, "const sessionThread = "),
    fn(usageSrc, "threadsHtml"),
    "this.draw = () => threadsHtml(usageBuckets(usageState.days, usageState.now));",
  ].join("\n"), context);
  return context.draw();
}

test("rows go costliest first, unknown costs last, and a tie goes to the latest", () => {
  const html = page({ threads: { macbook: [
    saved("cheap", { cost_usd: 0.5 }),
    saved("unknown", { cost_usd: null }),
    saved("dear", { cost_usd: 3 }),
    saved("tie-old", { cost_usd: 1, last_at: NOW - 7200 }),
    saved("tie-new", { cost_usd: 1, last_at: NOW - 600 }),
  ] } });
  assert.deepEqual(rowsOf(html).map((row) => row.split(" ")[1]), ["dear", "tie-new", "tie-old", "cheap", "unknown"]);
});

test("each figure reads as the tab showed it, and an unknown one says so", () => {
  const html = page({ threads: { macbook: [saved("known", { cost_usd: 3, active_ms: 150000, turns: 4 }), saved("blank", { title: undefined, cost_usd: null, active_ms: null, project: "/w/docs" })] } });
  const [known, blank] = rowsOf(html);
  assert.equal(known, "Thread known site 4 2m $3.00");
  assert.equal(blank, "New thread docs 2 — Unknown");
  assert.match(html, /<td class="opt" title="\/w\/site">site<\/td>/, "the project shows its folder, with the whole path on hover");
  assert.match(html, /<td class="num fig muted">Unknown<\/td>/);
  assert.match(html, /<td class="num fig muted">—<\/td>/);
  assert.equal(rowsOf(page({ threads: { macbook: [saved("tiny", { cost_usd: 0.004 })] } }))[0], "Thread tiny site 2 2m <$0.01");
});

test("the Machine column shows only when more than one machine is selected", () => {
  const threads = { macbook: [saved("a", { cost_usd: 2 })], macmini: [saved("b")] };
  const both = page({ threads });
  assert.equal(headOf(both), "Thread Project Machine Turns Active time Cost");
  assert.deepEqual(rowsOf(both), ["Thread a site MacBook 2 2m $2.00", "Thread b site Mac mini 2 2m $1.00"]);
  const one = page({ threads, selected: new Set(["macmini"]) });
  assert.equal(headOf(one), "Thread Project Turns Active time Cost");
  assert.deepEqual(rowsOf(one), ["Thread b site 2 2m $1.00"]);
});

test("a record last seen outside the window is not listed", () => {
  const threads = { macbook: [saved("today"), saved("old", { last_at: NOW - 40 * DAY })] };
  assert.deepEqual(rowsOf(page({ threads })).map((row) => row.split(" ")[1]), ["today"]);
  assert.deepEqual(rowsOf(page({ threads, days: 90 })).map((row) => row.split(" ")[1]), ["today", "old"]);
});

test("a machine whose call failed adds no rows and no error, and an empty window says so", () => {
  const html = page({ threads: { macbook: [saved("a")], macmini: [] } });
  assert.deepEqual(rowsOf(html), ["Thread a site MacBook 2 2m $1.00"]);
  assert.doesNotMatch(text(html), /Update required|Unavailable|error/i);
  const empty = page({ threads: { macbook: [], macmini: [] } });
  assert.match(empty, /<tr class="empty"><td colspan="6">No saved threads in this window\.<\/td><\/tr>/);
});

test("a title and a project are escaped", () => {
  const html = page({ threads: { macbook: [saved("x", { title: `<img src=x onerror="alert(1)">`, project: `/w/<b>"q"` })] } });
  assert.doesNotMatch(html, /<img|<b>/);
  assert.match(html, /&#60;img src=x onerror=&#34;alert\(1\)&#34;&#62;/);
  assert.match(html, /title="\/w\/&#60;b&#62;&#34;q&#34;"/);
});

test("a long title is cut short with the whole of it on hover, and a phone leaves out the project and machine", () => {
  const long = "Rework the onboarding flow so a new account lands on its first project with the sample data loaded";
  const html = page({ threads: { macbook: [saved("long", { title: long })], macmini: [] } });
  assert.match(html, new RegExp(`<td title="${long}"><span class="clip">${long}</span></td>`));
  assert.match(html, /<table class="by-time threads">/);
  assert.match(html, /<th class="opt">Project<\/th><th class="opt">Machine<\/th>/);
  assert.match(html, /<td class="opt" title="\/w\/site">site<\/td><td class="opt">MacBook<\/td>/);
  const css = read("gui-usage.css");
  assert.match(css, /\.usage table\.threads \.clip \{[^}]*max-width: [^;]+; overflow: hidden; text-overflow: ellipsis;/);
  assert.match(css, /@media \(max-width: 639px\) \{\n  \.usage table\.threads \.opt \{ display: none; \}/);
});

test("a record whose session is open in a thread on its machine opens it, and the others keep their markup", () => {
  const threads = { macbook: [saved("open", { cost_usd: 3 }), saved("shut", { cost_usd: 2 })], macmini: [saved("elsewhere")] };
  const before = page({ threads });
  const tabs = [{ key: "macbook:t1", machine: "macbook", view: { session: "open" } }, { key: "macbook:t2", machine: "macbook", view: { session: "elsewhere" } }];
  const html = page({ threads, tabs });
  assert.match(html, /<td title="Thread open"><button type="button" data-usage-thread="macbook:t1"><span class="clip">Thread open<\/span><\/button><\/td>/);
  assert.equal((html.match(/data-usage-thread/g) ?? []).length, 1, "a session open on another machine opens nothing");
  const rows = (markup) => [...markup.matchAll(/<tr>([\s\S]*?)<\/tr>/g)].map(([row]) => row);
  assert.deepEqual(rows(html).slice(2), rows(before).slice(2), "the other records render as before");
  assert.deepEqual(rowsOf(html), rowsOf(before), "the row's text is unchanged");
});

test("the Cost view alone adds the Threads section", () => {
  const spend = fn(usageSrc, "spendHtml");
  assert.match(spend, /const cost = metric === "cost";/);
  assert.match(spend, /\$\{cost \? threadsHtml\(s\.buckets\) : ""\}/);
});

/** Each machine's entry after `fetchUsage`, where `answers` gives what each machine answers to each call, or throws. */
async function fetched(answers) {
  const context = {
    usageState: { seq: 0, now: 0, envs: new Map() },
    moveBrowserPrices: async () => {},
    onlineMachines: () => Object.keys(answers),
    shellPage: () => "settings",
    shellDialog: { dataset: {} },
    callOn: async (m, call) => {
      const answer = answers[m][call.cmd];
      if (answer instanceof Error) throw answer;
      return answer;
    },
  };
  vm.createContext(context);
  vm.runInContext([fn(usageSrc, "usageBuckets"), fn(usageSrc, "fetchUsage"), "this.fetchUsage = fetchUsage;"].join("\n"), context);
  await context.fetchUsage();
  return context.usageState.envs;
}

test("a machine that can't answer session_costs lists no threads and keeps its rows and status", async () => {
  const row = { model: "claude-opus-5-5", at: NOW, session: "s1" };
  const envs = await fetched({
    old: { token_usage: [row], session_costs: new Error("this machine can't answer that") },
    broken: { token_usage: new Error("disk full"), session_costs: [saved("s2")] },
  });
  assert.deepEqual(plain(envs.get("old")), { rows: [{ ...row, machine: "old" }], error: "", threads: [], loaded: true, loading: false });
  assert.deepEqual(plain(envs.get("broken")), { rows: [], error: "disk full", threads: [{ ...saved("s2"), machine: "broken" }], loaded: true, loading: false });
});
