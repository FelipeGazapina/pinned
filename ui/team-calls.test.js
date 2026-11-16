import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

process.env.TZ = "UTC";
const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const hubSrc = read("hub.js");
const sources = ["gui-team-store.js", "gui-team-nav.js", "gui-team-calls.js", "gui-team-run.js"].map(read).join("\n");

const NOW = Date.parse("2026-11-16T15:11:00Z");
const S = NOW / 1000;
const at = (h, m = 0) => Date.parse(`2026-11-16T${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:00Z`) / 1000;
const ME = "acc-bruno";
const flush = () => new Promise((r) => setImmediate(r));
const here = (v) => JSON.parse(JSON.stringify(v));
const rules = (over = {}) => ({ before_irreversible: { kind: "owner" }, in_doubt: { kind: "owner" }, on_limit: { kind: "owner" }, on_failure: { kind: "owner" }, ...over });

const members = [
  { account: "acc-ana", email: "ana.souza@north.example", role: "owner", groups: ["publishing"] },
  { account: ME, email: "bruno.lima@north.example", role: "member", groups: ["publishing"] },
  { account: "acc-carla", email: "carla.santos@north.example", role: "member", groups: ["reviewers"] },
];
const groups = [
  { handle: "publishing", members: ["acc-ana", ME] },
  { handle: "reviewers", members: ["acc-carla", "acc-ana"] },
];
const bots = [
  { workflow: "w1", name: "Test publisher", on: true, calls: rules({ before_irreversible: { kind: "group", handle: "publishing" } }) },
  { workflow: "w2", name: "PR reviewer", on: true, calls: rules({ in_doubt: { kind: "group", handle: "reviewers" } }) },
  { workflow: "w3", name: "Friday summary", on: false, calls: rules() },
];
const call = (over = {}) => ({ id: "c1", run: "r1", workflow: "w1", situation: "before_irreversible", asked: [{ kind: "group", handle: "publishing" }], question: "Can I publish?", choices: ["OK to publish", "Don't publish"], created_at: at(15, 5), escalated: false, answer: null, ...over });
const run = (over = {}) => ({ id: "r1", workflow: "w1", state: "waiting", controller: null, cost_usd: 0, active_secs: 0, started_at: at(15, 0), ended_at: null, outcome: null, started_by: "x", ...over });

/** The team files in a context with a fake hub: `world.ops` collects every op sent, `world.reply(op)` answers. */
function page(world = {}) {
  const ops = [];
  const listeners = {};
  const pages = world.pages ?? ["team-calls", "team-run", "team-bot", "team-group", "team-bot-new"];
  const context = {
    console,
    Date: class extends Date {
      constructor(...a) {
        super(...(a.length ? a : [NOW]));
      }
      static now() {
        return NOW;
      }
    },
    Intl,
    APP_LOCALE: "en-US",
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    SHELL_PAGES: Object.fromEntries(pages.map((p) => [p, { show() {} }])),
    pageView: { addEventListener() {}, querySelector: () => null },
    shellPage: () => null,
    setPage() {},
    headHtml: (t) => t,
    openPage: (p) => ops.push({ opened: p }),
    // The bots UI's openers, cross-file globals.
    openTeamBot: (id) => ops.push({ bot: id }),
    openTeamGroup: (handle) => ops.push({ group: handle }),
    openTeamBotNew: () => ops.push({ botNew: true }),
    account: { email: world.email ?? "bruno.lima@north.example" },
    document: { addEventListener: (type, fn) => (listeners[type] ??= []).push(fn), body: { dataset: {} }, getElementById: () => ({ before() {} }), createElement: () => ({ remove() {}, setAttribute() {}, dataset: {}, querySelector: () => null, querySelectorAll: () => [] }) },
    teamsCurrent: () => (world.team === null ? null : { id: "team-1", name: "North Studio", role: world.role ?? "member", credential: {} }),
    teamCall: async (op) => {
      ops.push(op);
      const reply = world.reply?.(op);
      if (reply instanceof Error) throw reply.message;
      return reply;
    },
    setInterval: () => 0,
    clearInterval() {},
    CustomEvent: class {},
  };
  vm.createContext(context);
  vm.runInContext(sources, context);
  const eval_ = (code) => vm.runInContext(code, context);
  const set = (key, value) => {
    context.__v = value;
    eval_(`tcStore.${key} = __v`);
  };
  eval_(`tcStore.team = "team-1"; tcStore.loaded = true`);
  set("members", world.members ?? members);
  set("groups", world.groups ?? groups);
  set("bots", world.bots ?? bots);
  set("calls", world.calls ?? []);
  set("runs", world.runs ?? [run()]);
  return { ops, eval: eval_, set, context, listeners, sent: () => ops.filter((o) => o.op) };
}
const text = (html) => html.replace(/<span class="tc-mark[^>]*>[^<]*<\/span>/g, "").replace(/<[^>]+>/g, " ").replace(/&#(\d+);/g, (_, c) => String.fromCharCode(c)).replace(/\s+/g, " ").replace(/ ([,.])/g, "$1").trim();
const buttons = (html) => [...html.matchAll(/<button[^>]*>([\s\S]*?)<\/button>/g)].map((m) => text(m[1]));

test("an answer carries the choice and nothing the hub sets", async () => {
  const p = page({ calls: [call()], reply: (op) => ({ ...call(), answer: { by: ME, choice: "OK to publish", kind: { kind: "choice" }, at: S } }) });
  await p.eval(`tcAnswer(tcStore.calls[0], tcChoiceAnswer("OK to publish"))`);
  assert.deepEqual(here(p.sent()), [{ op: "answer_call", id: "c1", answer: { kind: { kind: "choice" }, choice: "OK to publish" } }]);
  assert.ok(!/"by"|"at"/.test(JSON.stringify(p.sent())), "no by or at");
});

test("a pass and a typed answer are the hub's literal JSON", async () => {
  const p = page({ calls: [call()], reply: () => call() });
  await p.eval(`tcAnswer(tcStore.calls[0], tcPassAnswer("reviewers"))`);
  await p.eval(`tcAnswer(tcStore.calls[0], tcTextAnswer("Only the queue fix."))`);
  assert.deepEqual(here(p.sent()), [
    { op: "answer_call", id: "c1", answer: { kind: { kind: "pass_to", handle: "reviewers" } } },
    { op: "answer_call", id: "c1", answer: { kind: { kind: "choice" }, text: "Only the queue fix." } },
  ]);
});

test("the calls page: open cards, then a green result line once answered", async () => {
  const p = page({ calls: [call(), call({ id: "c2", run: "r2", workflow: "w2", asked: [{ kind: "person", account: ME }], created_at: at(14, 53), question: "Was that on purpose?", choices: ["Yes", "No"] })], runs: [run(), run({ id: "r2", workflow: "w2" })], reply: () => ({ ...call(), answer: { by: ME, choice: "OK to publish", text: "", kind: { kind: "choice" }, at: S } }) });
  let html = p.eval("tcCallsPageHtml()");
  assert.match(text(html), /^2 calls waiting for you/);
  assert.deepEqual(buttons(html).slice(0, 5), ["Test publisher", "@publishing", "OK to publish", "Don't publish", "See what it did"], "bot, chip, the choices, then See what it did");
  assert.ok(html.indexOf("Test publisher") < html.indexOf("PR reviewer"), "newest first");
  assert.match(html, /data-state="waiting"/);
  assert.match(text(html), /Ana Souza was called too\. The first to answer decides\./);
  assert.match(text(html), /@Bruno Lima/);
  await p.eval(`tcAnswer(tcStore.calls[0], tcChoiceAnswer("OK to publish"))`);
  html = p.eval("tcCallsPageHtml()");
  assert.match(text(html), /^1 call waiting for you/);
  assert.match(html, /data-call="c1"[^>]*data-state="quiet"/);
  assert.match(html, /<div class="tc-result" data-state="done" role="status">You answered “OK to publish”\. The bot goes on with it\.<\/div>/);
});

test("a call card shows the whole question, with no line clamp", () => {
  const long = `Approve this step? ${"Run the migration on the orders table, then the backfill. ".repeat(30)}\nThen restart the workers.`;
  const p = page({ calls: [call({ question: long })] });
  const html = p.eval("tcCardHtml(tcStore.calls[0])");
  assert.ok(html.includes(`<p class="tc-question">${long.replace(/&/g, "&#38;")}</p>`), "every character of the question is in the card");
  const css = read("gui-team-calls.css");
  assert.ok(!/line-clamp|max-height|text-overflow/.test(css.match(/\.tc-question[^{]*\{[^}]*\}/)?.[0] ?? "x"), "the question rule clamps nothing");
  assert.ok(!/-webkit-line-clamp/.test(css), "no line clamp anywhere in the calls styles");
  assert.match(css, /\.tc-question \{[^}]*white-space: pre-line/);
});

test("the first choice is the primary button and each other group of the bot is a pass", () => {
  const p = page({ calls: [call({ id: "c2", run: "r2", workflow: "w2", asked: [{ kind: "person", account: ME }], choices: ["Yes", "No"] })] });
  const html = p.eval("tcCardHtml(tcStore.calls[0])");
  assert.match(html, /class="tc-primary" data-tc="choice" data-choice="0">Yes</);
  assert.match(html, /class="tc-secondary" data-tc="choice" data-choice="1">No</);
  assert.deepEqual(buttons(html).filter((b) => /^Pass/.test(b)), ["Pass to @reviewers"]);
  const asked = page({ calls: [call({ id: "c2", workflow: "w2", asked: [{ kind: "group", handle: "reviewers" }, { kind: "person", account: ME }] })] });
  assert.ok(!buttons(asked.eval("tcCardHtml(tcStore.calls[0])")).some((b) => /^Pass/.test(b)), "no pass to a group already asked");
});

test("another person's answer first shows the hub's own words and keeps the card", async () => {
  const p = page({ calls: [call()], reply: (op) => new Error("already answered by Ana Souza") });
  await p.eval(`tcAnswer(tcStore.calls[0], tcChoiceAnswer("OK to publish"))`);
  const html = p.eval("tcCardHtml(tcStore.calls[0])");
  assert.match(html, /<div class="tc-result" data-state="taken" role="status">already answered by Ana Souza<\/div>/);
  assert.ok(!/data-tc="choice"/.test(html), "no buttons to press again");
  assert.match(p.eval("tcCallsPageHtml()"), /^<div class="tc-title"><h1>No calls\. The bots are managing on their own<\/h1>/, "it no longer counts as waiting");
});

test("a call someone else answered while the page was open stays as a quiet card", () => {
  const p = page({ calls: [call()] });
  p.eval("tcCallsPageHtml()");
  p.eval(`tcApply({ team: "team-1", event: { kind: "call_resolved", call: ${JSON.stringify(call({ answer: { by: "acc-ana", choice: "Don't publish", text: "", kind: { kind: "choice" }, at: S } }))} } })`);
  const html = p.eval("tcCallsPageHtml()");
  assert.match(text(html), /Ana Souza already answered “Don't publish”\./);
  assert.match(text(html), /^No calls/);
});

test("a send that failed for another reason says so and keeps the buttons", async () => {
  const p = page({ calls: [call()], reply: () => new Error("the hub is restarting") });
  await p.eval(`tcAnswer(tcStore.calls[0], tcChoiceAnswer("OK to publish"))`);
  const html = p.eval("tcCardHtml(tcStore.calls[0])");
  assert.match(html, /data-state="error" role="status">Could not send: the hub is restarting</);
  assert.match(html, /data-tc="choice"/);
});

test("the headline reads 0, 1 and many", () => {
  const p = page();
  assert.deepEqual([0, 1, 2, 7].map((n) => p.eval(`tcHeadline(${n})`)), ["No calls. The bots are managing on their own", "1 call waiting for you", "2 calls waiting for you", "7 calls waiting for you"]);
});

test("only the calls that ask me count: my groups, me, and the owner's escalations", () => {
  const calls = [
    call({ id: "mine-group" }),
    call({ id: "other-group", asked: [{ kind: "group", handle: "reviewers" }] }),
    call({ id: "someone-else", asked: [{ kind: "person", account: "acc-carla" }] }),
    call({ id: "escalated", asked: [{ kind: "person", account: "acc-carla" }], escalated: true }),
    call({ id: "owner", asked: [{ kind: "owner" }] }),
    call({ id: "answered", answer: { by: "acc-ana", choice: "x", kind: { kind: "choice" }, at: S } }),
  ];
  const ids = (p) => here(p.eval("tcCallsForMe().map((c) => c.id)")).sort();
  assert.deepEqual(ids(page({ calls })), ["mine-group"]);
  const owner = page({ calls, email: "ana.souza@north.example", role: "owner" });
  assert.deepEqual(ids(owner), ["escalated", "mine-group", "other-group", "owner"], "the owner is in both groups here");
});

test("a call of a run that ended no longer asks anyone", () => {
  const p = page({ calls: [call()], runs: [run({ state: "gave_up" })] });
  assert.deepEqual(here(p.eval("tcCallsForMe()")), []);
});

test("a call with no choices asks for a typed answer", () => {
  const html = page({ calls: [call({ choices: [] })] }).eval("tcCardHtml(tcStore.calls[0])");
  assert.match(html, /<form class="tc-typed" data-tc-form><input type="text" name="answer"/);
});

test("who else was called: names, 'were' for several, nothing for only me", () => {
  const p = page({ calls: [call({ asked: [{ kind: "group", handle: "publishing" }, { kind: "group", handle: "reviewers" }] }), call({ id: "c3", asked: [{ kind: "person", account: ME }] })] });
  assert.equal(p.eval("tcCalledToo(tcStore.calls[0])"), "Ana Souza and Carla Santos were called too.");
  assert.equal(p.eval("tcCalledToo(tcStore.calls[1])"), "");
});

test("what people and bots typed is escaped on every screen", () => {
  const evil = `<img src=x onerror=alert(1)>`;
  const p = page({
    bots: [{ workflow: "w1", name: evil, on: true, calls: rules() }],
    members: [{ account: "acc-ana", email: `${evil}@x`, role: "owner", groups: [] }, { account: ME, email: "bruno.lima@north.example", role: "member", groups: [] }],
    groups: [{ handle: evil, members: ["acc-ana", ME] }],
    calls: [call({ question: evil, choices: [evil], asked: [{ kind: "group", handle: evil }], answer: { by: "acc-ana", choice: evil, text: evil, kind: { kind: "choice" }, at: S } }), call({ id: "c2", question: evil, choices: [evil] })],
    runs: [run({ outcome: evil, state: "done" })],
  });
  p.eval(`tcStore.logs.set("r1", [{ at: ${at(15, 1)}, who: ${JSON.stringify(evil)}, text: ${JSON.stringify(evil)} }])`);
  p.eval(`tcRunShown = "r1"`);
  for (const html of [p.eval("tcCallsPageHtml()"), p.eval("tcRunPageHtml()"), p.eval("tcNavHtml(tcCurrent(), 'team-calls')")]) {
    assert.ok(!html.includes("<img"), "no raw tag");
    assert.ok(html.includes("&#60;img"), "the text is there, escaped");
  }
});

test("the calls page lists the runs working and what was settled today", () => {
  const calls = [call({ id: "c0", run: "r5", workflow: "w2", asked: [{ kind: "group", handle: "reviewers" }], answer: { by: "acc-carla", choice: "Ask for a fix", text: "", kind: { kind: "choice" }, at: at(11, 20) } }), call({ id: "old", run: "r6", answer: { by: "acc-ana", choice: "x", kind: { kind: "choice" }, at: at(15, 0) - 86400 } })];
  const runs = [run({ id: "r3", workflow: "w2", state: "working", started_at: at(15, 9) }), run({ id: "r5", workflow: "w2", state: "done", started_at: at(11, 5), ended_at: at(11, 25) }), ...[9, 10, 12].map((h) => run({ id: `d${h}`, workflow: "w2", state: "done", started_at: at(h, 2), ended_at: at(h, 9) }))];
  const p = page({ calls, runs });
  p.eval(`tcStore.logs.set("r3", [{ at: ${at(15, 9)}, who: "bot", text: "Reading PR 51 of shop-web." }])`);
  const html = text(p.eval("tcCallsPageHtml()"));
  assert.match(html, /Working on their own now PR reviewer is working Reading PR 51 of shop-web\. 2 min ago/);
  assert.match(html, /Resolved today Carla Santos answered PR reviewer's call to @reviewers: “Ask for a fix”\. 11:20 AM/);
  assert.match(html, /PR reviewer finished 3 runs without needing anyone\. since 9:02 AM/);
  assert.ok(!/Ana Souza answered/.test(html), "yesterday's answer is not today's");
});

test("live pushes: a call opened, a call resolved and a run changed", () => {
  const p = page({ calls: [] });
  const push = (event, team = "team-1") => p.eval(`tcApply(${JSON.stringify({ type: "team", team, event })})`);
  push({ kind: "call_opened", call: call() });
  assert.equal(p.eval("tcCallsForMe().length"), 1);
  push({ kind: "call_opened", call: call({ id: "c9" }) }, "another-team");
  assert.equal(p.eval("tcStore.calls.length"), 1, "another team's push is ignored");
  push({ kind: "call_resolved", call: call({ answer: { by: "acc-ana", choice: "x", kind: { kind: "choice" }, at: S } }) });
  assert.equal(p.eval("tcCallsForMe().length"), 0);
  assert.equal(p.eval("tcStore.calls.length"), 1, "the resolved call replaced the open one");
  push({ kind: "run_changed", run: run({ state: "taken_over", controller: ME }) });
  assert.equal(p.eval("tcStore.runs[0].state"), "taken_over");
});

test("the nav: team, calls badge, bots with a dot each, groups, no chat, the person", () => {
  const p = page({ calls: [call(), call({ id: "c2", workflow: "w2", run: "r2", asked: [{ kind: "person", account: ME }] })] });
  const html = p.eval("tcNavHtml(tcCurrent(), 'team-calls')");
  assert.match(text(html), /^North Studio Calls 2 Bots Test publisher PR reviewer Friday summary Create a bot Groups @publishing @reviewers Projects No projects in the team yet\. Bruno Lima$/);
  assert.deepEqual(here(p.eval("tcStore.bots.map(tcBotDot)")).map((d) => d.tone), ["call", "call", "off"]);
  assert.ok(!/chat/i.test(html), "no chat item");
  assert.match(html, /data-tc-nav="team-calls"/);
  assert.match(html, /class="nav-row tc-row active"/);
  assert.match(html, /<span class="count" title="2 waiting on you">2<\/span>/);
});

const sitePs = [
  { key: "github.com/north/site", name: "site", repo: "github.com/north/site", member: ME },
  { key: "github.com/north/api", name: "api", repo: "github.com/north/api", member: "acc-ana" },
  { key: "github.com/north/<b>x</b>", name: "<b>x</b>", repo: "", member: "acc-carla" },
];
const projectRows = (html) => [...html.matchAll(/<div class="nav-row tc-row tc-plain tc-project[^"]*"[^>]*data-project="([^"]*)"[\s\S]*?<\/div>/g)].map((m) => ({ key: m[1], html: m[0] }));

test("the nav lists the team's projects, each with Take out of the team for who attached it and for the owner", () => {
  const member = page({ projects: sitePs });
  member.set("projects", sitePs);
  const rows = projectRows(member.eval("tcNavHtml(tcCurrent(), 'team-calls')"));
  assert.equal(rows.length, 3);
  assert.deepEqual(rows.map((r) => /Take out of the team/.test(r.html)), [true, false, false], "a member: only the project they attached");
  const owner = page({ role: "owner", email: "ana.souza@north.example" });
  owner.set("projects", sitePs);
  const all = projectRows(owner.eval("tcNavHtml(tcCurrent(), 'team-calls')"));
  assert.deepEqual(all.map((r) => /Take out of the team/.test(r.html)), [true, true, true], "the owner: every project");
  const html = owner.eval("tcNavHtml(tcCurrent(), 'team-calls')");
  assert.ok(!html.includes("<b>x</b>"), "a name people typed is escaped");
  assert.match(html, /&#60;b&#62;x&#60;\/b&#62;/);
});

test("Take out of the team asks in the row, Keep puts it back, Take it out sends detach_project", async () => {
  const p = page({ role: "owner", email: "ana.souza@north.example", reply: (op) => (op.op === "projects" ? sitePs.slice(1) : []) });
  p.set("projects", sitePs);
  const ask = { dataset: { tcDetach: "ask", key: "github.com/north/site" } };
  p.eval(`tcNavDetach(${JSON.stringify(ask)})`);
  let row = projectRows(p.eval("tcNavHtml(tcCurrent(), 'team-calls')")).find((r) => r.key === "github.com/north/site");
  assert.equal(text(row.html), "Take site out of the team? Bots that work in it go back to draft. Take it out Keep");
  assert.match(row.html, /data-tc-detach="confirm"[^>]*>Take it out</);
  assert.match(row.html, /data-tc-detach="keep"[^>]*>Keep</);
  assert.deepEqual(p.sent(), [], "asking sends nothing");
  p.eval(`tcNavDetach(${JSON.stringify({ dataset: { tcDetach: "keep" } })})`);
  assert.equal(p.eval("tcStore.detach"), null);
  assert.match(projectRows(p.eval("tcNavHtml(tcCurrent(), 'team-calls')")).find((r) => r.key === "github.com/north/site").html, />Take out of the team</);
  assert.deepEqual(p.sent(), [], "Keep sends nothing");
  p.eval(`tcNavDetach(${JSON.stringify(ask)})`);
  await p.eval(`tcNavDetach(${JSON.stringify({ dataset: { tcDetach: "confirm" } })})`);
  await flush();
  assert.deepEqual(here(p.sent()[0]), { op: "detach_project", key: "github.com/north/site" });
  assert.equal(p.eval("tcStore.detach"), null);
  assert.deepEqual(here(p.eval("tcStore.projects.map((x) => x.key)")), ["github.com/north/api", "github.com/north/<b>x</b>"]);
});

test("a project the hub will not take out keeps its question open with the hub's words", async () => {
  const p = page({ role: "owner", email: "ana.souza@north.example", reply: (op) => (op.op === "detach_project" ? new Error("only the member who attached it, or the owner, may take it out") : []) });
  p.set("projects", sitePs);
  p.eval(`tcDetachAsk("github.com/north/api")`);
  await p.eval("tcDetachConfirm()");
  const row = projectRows(p.eval("tcNavHtml(tcCurrent(), 'team-calls')")).find((r) => r.key === "github.com/north/api");
  assert.match(text(row.html), /Could not take it out: only the member who attached it, or the owner, may take it out/);
  assert.equal(p.eval("tcStore.projects.length"), 3, "the project stays in the list");
  assert.match(row.html, /data-tc-detach="confirm"/);
});

test("the team nav has no chat item", () => {
  const p = page({ calls: [call()] });
  p.set("projects", sitePs);
  const html = p.eval("tcNavHtml(tcCurrent(), 'team-calls')");
  assert.ok(!/chat/i.test(html), "no chat row, link or label");
  assert.deepEqual([...html.matchAll(/data-tc-nav="([^"]+)"/g)].map((m) => m[1]).filter((k, i, all) => all.indexOf(k) === i).sort(), ["team-bot", "team-bot-new", "team-calls", "team-group"]);
  assert.ok(!/chat/i.test(read("gui-team-nav.js").replace(/\/\*\*[\s\S]*?\*\//g, "").replace(/\/\/.*$/gm, "")), "and no code for one");
});

test("on a run's page the nav marks the bot that ran it", () => {
  const p = page();
  p.eval(`tcRunShown = "r1"`);
  const html = p.eval("tcNavHtml(tcCurrent(), 'team-run')");
  assert.match(html, /nav-row tc-row active" data-tc-nav="team-bot" data-workflow="w1"/);
  assert.ok(!/active" data-tc-nav="team-calls"/.test(html));
});

test("the nav links only the pages that exist", () => {
  const p = page({ pages: ["team-calls"] });
  const html = p.eval("tcNavHtml(tcCurrent(), null)");
  assert.ok(!/data-tc-nav="team-bot"/.test(html) && !/data-tc-nav="team-group"/.test(html) && !/data-tc-nav="team-bot-new"/.test(html));
  assert.match(text(html), /Create a bot/);
  const full = page().eval("tcNavHtml(tcCurrent(), null)");
  assert.match(full, /data-tc-nav="team-bot-new"/);
  assert.match(full, /data-tc-nav="team-group" data-handle="reviewers"/);
});

test("no team: the nav draws nothing and the sidebar stays personal", () => {
  const p = page({ team: null });
  p.eval("tcNavDraw()");
  assert.equal(p.eval("document.body.dataset.team"), undefined);
  assert.equal(p.eval("tcNavEl"), null);
  assert.match(p.eval("tcCallsPageHtml()"), /Create or join a team/);
});

test("a run reads its state in words", () => {
  const status = (over, calls = [], stopped = false) => {
    const p = page({ calls, runs: [run(over)] });
    if (stopped) p.eval(`tcStore.stopped.add("r1")`);
    return here(p.eval("tcRunStatus(tcStore.runs[0])"));
  };
  assert.equal(status({ state: "waiting" }, [call()]).title, "Waiting for someone in @publishing");
  assert.equal(status({ state: "waiting" }, [call({ asked: [{ kind: "person", account: "acc-carla" }] })]).title, "Waiting for Carla Santos");
  assert.equal(status({ state: "taken_over", controller: ME }).title, "You took over this run");
  assert.equal(status({ state: "taken_over", controller: "acc-ana" }).title, "Ana Souza took over this run");
  assert.equal(status({ state: "working" }).title, "Test publisher is working");
  assert.deepEqual(status({ state: "done", outcome: "Published to test", active_secs: 540, cost_usd: 0.84 }), { title: "Finished", sub: "Published to test · 9 minutes of work, US$ 0.84." });
  assert.equal(status({ state: "stopped" }).title, "Stopped");
  assert.equal(status({ state: "stopped" }, [], true).title, "Stopped by you");
  assert.equal(status({ state: "gave_up" }).title, "Gave up: nobody answered in 2 hours");
  assert.equal(status({ state: "failed" }).title, "Failed");
});

test("the run page: breadcrumb, buttons, the open call and its note", () => {
  const p = page({ calls: [call()], runs: [run()] });
  p.eval(`tcRunShown = "r1"; tcStore.logs.set("r1", [{ at: ${at(15, 0)}, who: "bot", text: "I started." }, { at: ${at(15, 4)}, who: "bot", text: "Ran the tests." }])`);
  const html = p.eval("tcRunPageHtml()");
  const t = text(html);
  assert.match(t, /^Test publisher, run started at 3:00 PM Waiting for someone in @publishing Take over Stop the bot Test publisher, 3:00 PM I started\. 3:04 PM Ran the tests\. 3:05 PM Can I publish\?/);
  assert.match(t, /Asked: Ana Souza and you\. If nobody answers in 30 minutes, I call Ana Souza directly\. Everything the bot and the people do in a run is kept here\.$/);
  assert.deepEqual(buttons(html).filter((b) => /publish$|publish$/.test(b) && !/^[@T]/.test(b)), ["OK to publish", "Don't publish"]);
  assert.ok(!html.includes('data-tc-form'), "no typed field while the call has choices and nobody took over");
});

test("without a run log only the calls and answers are drawn", () => {
  const answered = call({ answer: { by: ME, choice: "OK to publish", text: "", kind: { kind: "choice" }, at: at(15, 8) } });
  const p = page({ calls: [answered], runs: [run({ state: "working" })] });
  p.eval(`tcRunShown = "r1"; tcStore.logs.set("r1", null)`);
  const t = text(p.eval("tcRunPageHtml()"));
  assert.match(t, /Can I publish\? .*You, 3:08 PM OK to publish/);
  assert.match(t, /not available from this hub yet/);
  assert.ok(!/I started/.test(t));
});

test("the timeline keeps time order and does not repeat the question the log already holds", () => {
  const answered = call({ answer: { by: "acc-ana", choice: "Don't publish", text: "", kind: { kind: "choice" }, at: at(15, 8) } });
  const p = page({ calls: [answered], runs: [run({ state: "working" })] });
  p.eval(`tcRunShown = "r1"; tcStore.logs.set("r1", [{ at: ${at(15, 9)}, who: "bot", text: "Stopped without publishing." }, { at: ${at(15, 0)}, who: "bot", text: "Started." }, { at: ${at(15, 5)}, who: "bot", text: "Can I publish?" }])`);
  const t = text(p.eval("tcRunPageHtml()"));
  assert.ok(t.indexOf("Started.") < t.indexOf("Can I publish?") && t.indexOf("Can I publish?") < t.indexOf("Ana Souza, 3:08 PM") && t.indexOf("Ana Souza, 3:08 PM") < t.indexOf("Stopped without publishing."));
  assert.equal(t.split("Can I publish?").length, 2, "the question is drawn once");
});

test("taken over by me: the banner with Give back, a typed answer, and no Take over", () => {
  const p = page({ calls: [call()], runs: [run({ state: "taken_over", controller: ME })] });
  p.eval(`tcRunShown = "r1"; tcStore.logs.set("r1", [])`);
  const html = p.eval("tcRunPageHtml()");
  assert.match(text(html), /You took over this run Stop the bot You are in charge\. The bot stopped where it was and waits\./);
  assert.ok(!/recorded/.test(text(html)), "nothing promises what is recorded");
  assert.deepEqual(buttons(html).filter((b) => /Take over|Give back|Stop/.test(b)), ["Stop the bot", "Give back to the bot"]);
  assert.match(html, /data-tc-run="give_back"/);
  assert.match(html, /<form class="tc-typed" data-tc-form>/);
});

test("taken over by someone else: no give back for a member, one for the owner", () => {
  const member = page({ runs: [run({ state: "taken_over", controller: "acc-carla" })] });
  member.eval(`tcRunShown = "r1"`);
  assert.ok(!/give_back/.test(member.eval("tcRunPageHtml()")));
  assert.match(text(member.eval("tcRunPageHtml()")), /Carla Santos is in charge/);
  const owner = page({ email: "ana.souza@north.example", role: "owner", runs: [run({ state: "taken_over", controller: "acc-carla" })] });
  owner.eval(`tcRunShown = "r1"`);
  assert.match(owner.eval("tcRunPageHtml()"), /data-tc-run="give_back"/);
});

test("an ended run has no buttons and its open call has no controls", () => {
  const p = page({ calls: [call()], runs: [run({ state: "gave_up" })] });
  p.eval(`tcRunShown = "r1"; tcStore.logs.set("r1", [])`);
  const html = p.eval("tcRunPageHtml()");
  assert.ok(!/data-tc-run|data-tc="|data-tc-form/.test(html), "no take over, stop, answer or pass control");
  assert.match(text(html), /Gave up: nobody answered in 2 hours/);
  assert.match(text(html), /Nobody answered\./);
});

test("take over, give back and stop send the run's id and nothing else", async () => {
  const p = page({ runs: [run()], reply: (op) => run({ state: op.op === "stop_run" ? "stopped" : op.op === "take_over" ? "taken_over" : "working" }) });
  await p.eval(`tcRunOp("take_over", "r1")`);
  assert.equal(p.eval("tcStore.runs[0].state"), "taken_over");
  await p.eval(`tcRunOp("give_back", "r1")`);
  await p.eval(`tcRunOp("stop_run", "r1")`);
  assert.deepEqual(here(p.sent()), [{ op: "take_over", run: "r1" }, { op: "give_back", run: "r1" }, { op: "stop_run", run: "r1" }]);
  assert.equal(p.eval("tcStore.stopped.has('r1')"), true);
});

test("a hub that refuses a run op says so on the page", async () => {
  const p = page({ runs: [run()], reply: () => new Error("the run already ended") });
  await p.eval(`tcRunOp("stop_run", "r1")`);
  p.eval(`tcRunShown = "r1"; tcStore.logs.set("r1", [])`);
  assert.match(text(p.eval("tcRunPageHtml()")), /Could not stop the bot: the run already ended/);
});

test("run_log is asked as {op:run_log,run} and its failure leaves the log empty", async () => {
  const good = page({ reply: () => [{ at: S, who: "bot", text: "Hello" }, { bad: 1 }] });
  await good.eval(`tcLoadLog("r1")`);
  assert.deepEqual(here(good.sent()), [{ op: "run_log", run: "r1" }]);
  assert.deepEqual(here(good.eval(`tcStore.logs.get("r1")`)), [{ at: S, who: "bot", text: "Hello" }]);
  const bad = page({ reply: () => new Error("unknown op") });
  await bad.eval(`tcLoadLog("r1")`);
  assert.equal(bad.eval(`tcStore.logs.get("r1")`), null);
});

test("opening a run from another screen remembers it and opens the page", () => {
  const p = page();
  p.eval(`openTeamRun("r7")`);
  assert.equal(p.eval("tcRunShown"), "r7");
  assert.deepEqual(here(p.ops), [{ opened: "team-run" }]);
});

test("a bot, a group, a run and Create a bot open through the one openTeamX style, with no alias", () => {
  const p = page();
  const nav = (key, dataset) => p.eval(`tcNavOpen(${JSON.stringify({ dataset: { tcNav: key, ...dataset } })})`);
  nav("team-bot", { workflow: "w2" });
  nav("team-group", { handle: "reviewers" });
  nav("team-bot-new", {});
  nav("team-calls", {});
  p.eval(`tcOpenTarget(${JSON.stringify({ dataset: { tcOpen: "bot", value: "w1" } })})`);
  p.eval(`tcOpenTarget(${JSON.stringify({ dataset: { tcOpen: "group", value: "publishing" } })})`);
  p.eval(`tcOpenTarget(${JSON.stringify({ dataset: { tcOpen: "run", value: "r1" } })})`);
  assert.deepEqual(here(p.ops), [{ bot: "w2" }, { group: "reviewers" }, { botNew: true }, { opened: "team-calls" }, { bot: "w1" }, { group: "publishing" }, { opened: "team-run" }]);
  for (const name of ["gui-team-store.js", "gui-team-nav.js", "gui-team-calls.js", "gui-team-run.js", "gui-teams.js", "gui-teams-bots.js"]) assert.ok(!/teamRunOpen|teamBotOpen|teamGroupOpen/.test(read(name)), `${name} keeps no old name`);
});

test("me is the account id teamsCurrent gives; the e-mail is the way only when it gives none", () => {
  // The signed-in e-mail names Bruno, but the team says the viewer is Carla: the id wins.
  const p = page({ email: "bruno.lima@north.example" });
  p.context.teamsCurrent = () => ({ id: "team-1", name: "North Studio", role: "member", credential: {}, me: "acc-carla" });
  assert.equal(p.eval("tcMe().account"), "acc-carla");
  assert.equal(p.eval("tcMe().name"), "Carla Santos");
  assert.equal(p.eval("tcPerson('acc-carla')"), "You");
  const owner = page({ email: "nobody@else.example" });
  owner.context.teamsCurrent = () => ({ id: "team-1", name: "North Studio", role: "owner", credential: {}, me: "acc-ana" });
  assert.deepEqual(here(owner.eval("({ account: tcMe().account, owner: tcMe().owner })")), { account: "acc-ana", owner: true });
  // A member: no id on the wire, so the members list and the e-mail give it.
  assert.equal(page({ email: "bruno.lima@north.example" }).eval("tcMe().account"), ME);
});

test("refresh reads the six lists, and one that fails keeps what it had", async () => {
  const p = page({ reply: (op) => (op.op === "calls" ? new Error("down") : op.op === "runs" ? [run({ id: "fresh" })] : []) });
  await p.eval("tcRefresh()");
  assert.deepEqual(here(p.sent().map((o) => o.op)).sort(), ["bots", "calls", "groups", "members", "projects", "runs"]);
  assert.equal(p.eval("tcStore.runs[0].id"), "fresh");
  assert.equal(p.eval("tcStore.members.length"), 0, "an empty list replaces");
});

test("hub.js hands a team push to on.team", () => {
  const seen = [];
  class WebSocket { static OPEN = 1; constructor() { this.readyState = 1; } send() {} close() {} }
  const context = { WebSocket, setTimeout: () => 0, seen };
  vm.createContext(context);
  vm.runInContext(`${hubSrc}\nthis.hub = new Hub("https://hub.example", "login", { team: (f) => seen.push(f) });`, context);
  const frame = { type: "team", team: "team-1", event: { kind: "call_opened", call: call() } };
  context.hub.ws.onmessage({ data: JSON.stringify(frame) });
  assert.deepEqual(here(seen), [frame]);
});

test("a pass names the people of the group, and a call only to me has no 'called too' note", async () => {
  const p = page({ calls: [call({ asked: [{ kind: "person", account: ME }] })], reply: () => call() });
  assert.ok(!/first to answer/i.test(p.eval("tcCardHtml(tcStore.calls[0])")));
  await p.eval(`tcAnswer(tcStore.calls[0], tcPassAnswer("reviewers"))`);
  assert.match(p.eval("tcCardHtml(tcStore.calls[0])"), /Passed to @reviewers\. Carla Santos and Ana Souza are being called\./);
});
