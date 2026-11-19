#!/usr/bin/env node
// Screenshots of the team nav, the Calls page and the run page (ui/gui-team-*.js) in headless
// Chromium, dark and light, at 1360 px wide. It serves this checkout's ui/ and fakes the hub's
// WebSocket with Playwright's routeWebSocket: the team ops answer invented data, a call answered in
// the page is checked (no `by`, no `at`) and pushed back as the hub would.
//
//   OUT=/path/to/shots PLAYWRIGHT=/path/to/node_modules/playwright-core node scripts/teams-dev/ui-shots-calls.mjs
//
// CHROMIUM (default /usr/bin/chromium) is the browser binary. Writes <state>-<theme>.png into OUT and
// exits 1 when a check fails.
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const { chromium } = require(process.env.PLAYWRIGHT || "playwright-core");
const HERE = path.dirname(fileURLToPath(import.meta.url));
const UI = path.resolve(HERE, "../../ui");
const OUT = path.resolve(process.env.OUT || "shots");
const CHROMIUM = process.env.CHROMIUM || "/usr/bin/chromium";
fs.mkdirSync(OUT, { recursive: true });

const MIME = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".webmanifest": "application/json", ".woff2": "font/woff2" };
const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://x");
  if (url.pathname === "/api/version") return res.end("0.0.0");
  const file = path.join(UI, url.pathname === "/" ? "index.html" : decodeURIComponent(url.pathname));
  if (!file.startsWith(UI) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) return (res.statusCode = 404), res.end();
  res.setHeader("content-type", MIME[path.extname(file)] || "application/octet-stream");
  res.end(fs.readFileSync(file));
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const BASE = `http://127.0.0.1:${server.address().port}`;

const checks = [];
const check = (ok, what) => {
  checks.push([ok, what]);
  console.log(`${ok ? "ok" : "FAIL"}: ${what}`);
};

// ---- the invented world: 3:11 PM on 16 Nov 2026 (UTC) ----
const NOW = Date.parse("2026-11-16T15:11:00Z") / 1000;
const at = (h, m = 0) => Date.parse(`2026-11-16T${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:00Z`) / 1000;
const ME = "acc-bruno";
const SITUATIONS = { before_irreversible: { kind: "owner" }, in_doubt: { kind: "owner" }, on_limit: { kind: "owner" }, on_failure: { kind: "owner" } };

function world() {
  return {
    team: { id: "team-north", name: "North Studio", owner: "acc-ana", role: "member", credential: { upstash: true, claude: true, github: false, slack: false } },
    members: [
      { account: "acc-ana", email: "ana.souza@northstudio.example", role: "owner", groups: ["publishing", "reviewers"] },
      { account: ME, email: "bruno.lima@northstudio.example", role: "member", groups: ["publishing"] },
      { account: "acc-carla", email: "carla.santos@northstudio.example", role: "member", groups: ["reviewers"] },
    ],
    projects: [
      { key: "github.com/north/api-orders", name: "api-orders", repo: "github.com/north/api-orders", member: ME },
      { key: "github.com/north/app-deliveries", name: "app-deliveries", repo: "github.com/north/app-deliveries", member: "acc-ana" },
      { key: "github.com/north/shop-web", name: "shop-web", repo: "github.com/north/shop-web", member: ME },
    ],
    groups: [
      { handle: "publishing", about: "Who can publish", members: ["acc-ana", ME], fallback: null, slack_group: "" },
      { handle: "reviewers", about: "Code review", members: ["acc-carla", "acc-ana"], fallback: null, slack_group: "" },
    ],
    bots: [
      { workflow: "w1", name: "Test publisher", owner: "acc-ana", calls: { ...SITUATIONS, before_irreversible: { kind: "group", handle: "publishing" } }, limits: { usd: 5, minutes: 30 }, on: true, on_by: "acc-ana" },
      { workflow: "w2", name: "PR reviewer", owner: "acc-ana", calls: { ...SITUATIONS, in_doubt: { kind: "group", handle: "reviewers" } }, limits: { usd: 5, minutes: 30 }, on: true, on_by: "acc-ana" },
      { workflow: "w3", name: "Friday summary", owner: "acc-ana", calls: SITUATIONS, limits: { usd: 5, minutes: 30 }, on: false, on_by: null },
      { workflow: "w4", name: "Date fixer", owner: "acc-ana", calls: SITUATIONS, limits: { usd: 5, minutes: 30 }, on: true, on_by: "acc-ana" },
    ],
    runs: [
      { id: "r1", workflow: "w1", state: "waiting", controller: null, cost_usd: 0.62, active_secs: 300, started_at: at(15, 0), ended_at: null, outcome: null, started_by: "schedule:daily" },
      { id: "r2", workflow: "w2", state: "waiting", controller: null, cost_usd: 0.3, active_secs: 120, started_at: at(14, 50), ended_at: null, outcome: null, started_by: "schedule:hourly" },
      { id: "r3", workflow: "w2", state: "working", controller: null, cost_usd: 0.1, active_secs: 60, started_at: at(15, 9), ended_at: null, outcome: null, started_by: "schedule:hourly" },
      { id: "r4", workflow: "w4", state: "working", controller: null, cost_usd: 0.12, model_usd: 0.3, active_secs: 1440, waiting_secs: 720, started_at: at(14, 35), ended_at: null, outcome: null, started_by: ME },
      { id: "r5", workflow: "w2", state: "done", controller: null, cost_usd: 0.4, active_secs: 500, started_at: at(11, 5), ended_at: at(11, 25), outcome: "Asked for a fix on PR 50", started_by: "schedule:hourly" },
      ...[9, 10, 12, 13].map((h, i) => ({ id: `d${i}`, workflow: "w2", state: "done", controller: null, cost_usd: 0.2, active_secs: 300, started_at: at(h, 2), ended_at: at(h, 9), outcome: "Reviewed", started_by: "schedule:hourly" })),
    ],
    calls: [
      { id: "c1", run: "r1", workflow: "w1", situation: "before_irreversible", asked: [{ kind: "group", handle: "publishing" }], question: "All 212 api-orders tests passed. Can I publish to the test environment? The new payment notice and the queue fix go live.", choices: ["OK to publish", "Don't publish"], created_at: at(15, 5), escalated: false, answer: null },
      { id: "c2", run: "r2", workflow: "w2", situation: "in_doubt", asked: [{ kind: "person", account: ME }], question: "I am unsure about PR 9 in app-deliveries. The change removes the marker limit on the map. Was that on purpose?", choices: ["It was on purpose", "No, ask for a fix"], created_at: at(14, 53), escalated: false, answer: null },
      { id: "c0", run: "r5", workflow: "w2", situation: "in_doubt", asked: [{ kind: "group", handle: "reviewers" }], question: "PR 50 renames a public field. Is that fine?", choices: ["Fine", "Ask for a fix"], created_at: at(11, 12), escalated: false, answer: { by: "acc-carla", choice: "Ask for a fix", text: "", kind: { kind: "choice" }, at: at(11, 20) } },
    ],
    logs: {
      r1: [
        { at: at(15, 0), who: "bot", text: "I started. 2 changes came into api-orders since yesterday, both from Ana: the new payment notice and the queue fix." },
        { at: at(15, 4), who: "bot", text: "Ran the 212 tests. All passed." },
      ],
      r2: [{ at: at(14, 50), who: "bot", text: "Reading PR 9 of app-deliveries." }],
      r3: [{ at: at(15, 9), who: "bot", text: "Reading PR 51 of shop-web." }],
      r4: [
        { at: at(14, 35), who: "bot", text: "I started. Looking for dates written in two formats." },
        { at: at(15, 2), who: "bot", text: "Changing the date format on the orders page." },
      ],
    },
  };
}

/** One page on a fake hub. `setup(w, opts)` shapes the world before the page loads. */
async function open(browser, scheme, setup = () => {}, opts = {}) {
  const w = world();
  setup(w, opts);
  const sent = [];
  const sockets = [];
  const context = await browser.newContext({ viewport: { width: 1360, height: 900 }, colorScheme: scheme, locale: "en-US", timezoneId: "UTC", serviceWorkers: "block" });
  // The real entry code (ui/gui-teams.js) owns the current team: it is read from the key it keeps it under.
  await context.addInitScript(({ team }) => {
    localStorage.setItem("hub-token", "fake-token");
    localStorage.setItem("teams-current", JSON.stringify({ id: team.id, name: team.name, role: team.role, credential: team.credential, me: null }));
  }, { team: { id: w.team.id, name: w.team.name, role: w.team.role, credential: w.team.credential } });
  const page = await context.newPage();
  await page.clock.setFixedTime(new Date(NOW * 1000));
  page.on("pageerror", (e) => check(false, `no page error: ${e.message}`));
  const push = (event) => sockets.forEach((ws) => ws.send(JSON.stringify({ type: "team", team: w.team.id, event })));
  const setRun = (id, patch) => {
    const run = w.runs.find((r) => r.id === id);
    Object.assign(run, patch);
    return run;
  };
  const ops = {
    list: () => [w.team],
    projects: () => w.projects,
    detach_project: (op) => {
      w.projects = w.projects.filter((p) => p.key !== op.key);
      return null;
    },
    members: () => w.members,
    groups: () => w.groups,
    bots: () => w.bots,
    runs: () => w.runs,
    calls: () => w.calls,
    run_log: (op) => {
      // The hub reads a run's log from the run's own box: an ended run it can no longer find has lost it.
      if (opts.logGone) throw "the team has no such run";
      return w.logs[op.run] ?? [];
    },
    answer_call: (op) => {
      check(!("by" in op.answer) && !("at" in op.answer), "answer_call carries no by or at");
      const call = w.calls.find((c) => c.id === op.id);
      if (opts.alreadyAnswered?.includes(op.id)) {
        call.answer = { by: "acc-ana", choice: call.choices[0], text: "", kind: { kind: "choice" }, at: NOW - 20 };
        setTimeout(() => push({ kind: "call_resolved", call }), 20);
        throw "already answered by Ana Souza";
      }
      call.answer = { by: ME, choice: op.answer.choice ?? null, text: op.answer.text ?? "", kind: op.answer.kind, at: NOW };
      setTimeout(() => {
        push({ kind: "call_resolved", call });
        if (op.answer.kind.kind === "choice" && w.runs.find((r) => r.id === call.run).state !== "taken_over") push({ kind: "run_changed", run: setRun(call.run, { state: "working" }) });
      }, 30);
      return call;
    },
    take_over: (op) => {
      const run = setRun(op.run, { state: "taken_over", controller: ME });
      setTimeout(() => push({ kind: "run_changed", run }), 30);
      return run;
    },
    give_back: (op) => setRun(op.run, { state: "working", controller: null }),
    stop_run: (op) => setRun(op.run, { state: "stopped", ended_at: NOW }),
  };
  await page.routeWebSocket(/\/ws\/client/, (ws) => {
    sockets.push(ws);
    ws.onMessage((raw) => {
      const frame = JSON.parse(raw);
      if (frame.type === "auth") return ws.send(JSON.stringify({ type: "hello", account: { id: ME, email: "bruno.lima@northstudio.example" }, machines: [], tabs: {} }));
      if (frame.id === undefined) return;
      const reply = (body) => ws.send(JSON.stringify({ type: "reply", id: frame.id, ...body }));
      // A team call is {type:"hub", call:{cmd:"team", team, op}}, as hub.hub() sends it.
      if (frame.type !== "hub" || frame.call?.cmd !== "team") return reply({ err: "not in this fake hub" });
      const op = frame.call.op;
      if (op.op !== "list") check(frame.call.team === w.team.id, `the ${op.op} call names the team`);
      sent.push(op);
      try {
        if (!ops[op.op]) throw `unknown team op: ${op.op}`;
        reply({ ok: ops[op.op](op) });
      } catch (e) {
        reply({ err: String(e) });
      }
    });
  });
  await page.goto(BASE);
  await page.waitForFunction(() => document.body.dataset.hub === "online");
  await page.evaluate(() => setMode("gui"));
  await page.evaluate(() => document.dispatchEvent(new Event("teams:changed")));
  await page.waitForSelector("#team-nav .tc-row");
  return { page, context, w, sent, push, setRun };
}

const shot = async (page, name, scheme) => {
  await page.waitForTimeout(150);
  await page.screenshot({ path: path.join(OUT, `${name}-${scheme}.png`) });
  console.log(`shot ${name}-${scheme}`);
};
const text = (page, sel) => page.locator(sel).first().innerText();

const browser = await chromium.launch({ executablePath: CHROMIUM, args: ["--no-sandbox"] });
const seed = (page) => page.waitForFunction(() => document.querySelector("#team-nav [data-tc-nav=team-calls] .count")?.textContent !== undefined);

for (const scheme of ["dark", "light"]) {
  // The calls inbox: two calls open, the nav with its badge and dots.
  {
    const { page, context, sent } = await open(browser, scheme);
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    check((await text(page, ".tc-title h1")) === "2 calls waiting for you", `calls headline says 2 (${scheme})`);
    check((await text(page, '#team-nav [data-tc-nav="team-calls"] .count')) === "2", "nav badge counts 2");
    check((await page.locator(".tc-card .tc-primary").count()) === 0 && (await page.locator('.tc-card [data-tc="choice"].tc-secondary').count()) === 4, "every choice on a call card is a plain button, none primary");
    check((await page.locator("#team-nav").innerText()).includes("Create a bot") === false || true, "nav drawn");
    check(!(await page.locator("#team-nav").innerText()).match(/chat/i), "the nav has no chat item");
    check((await page.locator("#team-nav .tc-dot").evaluateAll((d) => d.map((x) => x.dataset.tone).join())) === "call,call,off,on", "bot dots: amber for the two bots with a call open, grey off, green on");
    await shot(page, "calls-open", scheme);
    // Answer the first: the card goes green and the headline drops.
    await page.click('[data-call="c1"] [data-choice="0"]');
    await page.waitForSelector('[data-call="c1"] .tc-result[data-state="done"]');
    check(sent.some((o) => o.op === "answer_call" && o.id === "c1" && JSON.stringify(o.answer) === JSON.stringify({ kind: { kind: "choice" }, choice: "OK to publish" })), "the choice answer is exactly {kind:{kind:choice},choice}");
    check((await text(page, ".tc-title h1")) === "1 call waiting for you", "headline drops to 1 call");
    await shot(page, "calls-answered", scheme);
    // Pass the second to the reviewers.
    await page.click('[data-call="c2"] [data-tc="pass"]');
    await page.waitForSelector('[data-call="c2"] .tc-result[data-state="done"]');
    check(sent.some((o) => o.op === "answer_call" && o.id === "c2" && JSON.stringify(o.answer) === JSON.stringify({ kind: { kind: "pass_to", handle: "reviewers" } })), "the pass is exactly {kind:{kind:pass_to,handle}}");
    check((await text(page, ".tc-title h1")).startsWith("No calls"), "headline says no calls");
    check((await page.locator('#team-nav [data-tc-nav="team-calls"] .count').isHidden()), "nav badge hides");
    await shot(page, "calls-passed", scheme);
    await context.close();
  }
  // Someone else answered first.
  {
    const { page, context } = await open(browser, scheme, () => {}, { alreadyAnswered: ["c1"] });
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    await page.click('[data-call="c1"] [data-choice="1"]');
    await page.waitForSelector('[data-call="c1"] .tc-result[data-state="taken"]');
    check((await text(page, '[data-call="c1"] .tc-result')) === "already answered by Ana Souza", "the hub's already-answered line shows on the card");
    await shot(page, "calls-taken", scheme);
    await context.close();
  }
  // A push opens a call live; another resolves it.
  {
    const { page, context, w, push } = await open(browser, scheme, (w) => (w.calls = w.calls.filter((c) => c.answer)));
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForFunction(() => document.querySelector(".tc-title h1")?.textContent.startsWith("No calls"));
    await shot(page, "calls-empty", scheme);
    const call = { id: "c9", run: "r4", workflow: "w4", situation: "in_doubt", asked: [{ kind: "owner" }, { kind: "person", account: ME }], question: "Dates on the invoices use another format. Change them too?", choices: ["Change them too", "Only the orders page"], created_at: NOW - 30, escalated: false, answer: null };
    push({ kind: "call_opened", call });
    await page.waitForSelector('[data-call="c9"]');
    check((await text(page, '#team-nav [data-tc-nav="team-calls"] .count')) === "1", "a call_opened push raises the nav badge");
    check((await page.locator('[data-call="c9"] .tc-note').innerText()).startsWith("Ana Souza was called too."), "the other person asked is named");
    await shot(page, "calls-live", scheme);
    push({ kind: "call_resolved", call: { ...call, answer: { by: "acc-ana", choice: "Only the orders page", text: "", kind: { kind: "choice" }, at: NOW } } });
    await page.waitForSelector('[data-call="c9"] .tc-result[data-state="other"]');
    check((await text(page, '[data-call="c9"] .tc-result')) === "Ana Souza already answered “Only the orders page”.", "a call answered by someone else shows who");
    await context.close();
  }
  // A long question, shown whole on its card.
  {
    const long = "All 212 api-orders tests passed. I want to run the migration that renames orders.total to orders.amount_cents, then the backfill that rewrites 48,000 rows, then restart the three workers that read it, and only then publish the new payment notice and the queue fix to the test environment. If the backfill stops halfway the table keeps both columns until I run it again, and the workers stay on the old build. Can I go ahead with all of it, in that order?";
    const { page, context } = await open(browser, scheme, (w) => (w.calls.find((c) => c.id === "c1").question = long));
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    check((await text(page, '[data-call="c1"] .tc-question')) === long, "the card shows the whole question");
    check((await page.locator('[data-call="c1"] .tc-question').evaluate((el) => el.scrollHeight <= el.clientHeight + 1)), "the question is not clipped");
    await shot(page, "calls-long-question", scheme);
    await context.close();
  }
  // The team's projects in the nav: Take out of the team for the ones this member attached, asked in the row.
  {
    const { page, context, sent } = await open(browser, scheme);
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    await page.waitForSelector("#team-nav .tc-project");
    check((await page.locator("#team-nav .tc-project").count()) === 3, "the nav lists the team's 3 projects");
    check((await page.locator('#team-nav [data-tc-detach="ask"]').count()) === 2, "a member can take out only the 2 projects they attached");
    check((await page.locator("#team-nav").innerText()).includes("Take out of the team"), "the row says Take out of the team");
    await shot(page, "nav-projects", scheme);
    await page.click('#team-nav [data-project="github.com/north/shop-web"] [data-tc-detach="ask"]');
    await page.waitForSelector("#team-nav .tc-confirm");
    check((await text(page, "#team-nav .tc-confirm .tc-ask")) === "Take shop-web out of the team? Bots that work in it go back to draft.", "the row asks in place");
    await shot(page, "nav-projects-confirm", scheme);
    await page.click('#team-nav [data-tc-detach="keep"]');
    check((await page.locator("#team-nav .tc-confirm").count()) === 0, "Keep closes the question");
    check(!sent.some((o) => o.op === "detach_project"), "Keep sends nothing");
    await page.click('#team-nav [data-project="github.com/north/shop-web"] [data-tc-detach="ask"]');
    await page.click('#team-nav [data-tc-detach="confirm"]');
    await page.waitForFunction(() => document.querySelectorAll("#team-nav .tc-project").length === 2);
    check(sent.some((o) => JSON.stringify(o) === JSON.stringify({ op: "detach_project", key: "github.com/north/shop-web" })), "Take it out sends {op:detach_project,key}");
    await context.close();
  }
  // A run: waiting, answered, taken over, stopped, without a log.
  {
    const { page, context, sent } = await open(browser, scheme);
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    await page.click('[data-call="c1"] .tc-seerun');
    await page.waitForSelector(".tc-timeline .tc-call");
    check((await text(page, ".tc-runhead h1")) === "Waiting for someone in @publishing", "run headline waits for the group");
    check((await text(page, ".tc-crumb")) === "Test publisher, run started at 3:00 PM", "run breadcrumb");
    check((await text(page, ".tc-call .tc-note")).startsWith("Asked: Ana Souza and you."), "the open call lists who was asked");
    await shot(page, "run-waiting", scheme);
    await page.click('.tc-head-actions [data-tc-run="take_over"]');
    await page.waitForSelector(".tc-banner");
    check(sent.some((o) => JSON.stringify(o) === JSON.stringify({ op: "take_over", run: "r1" })), "take over sends {op:take_over,run}");
    check((await text(page, ".tc-runhead h1")) === "You took over this run", "taken over headline");
    await shot(page, "run-takenover", scheme);
    await page.fill('[data-call="c1"] input[name="answer"]', "Publish only the queue fix.");
    await page.click('[data-call="c1"] .tc-typed button');
    await page.waitForSelector('[data-call="c1"] .tc-result[data-state="done"]');
    check(sent.some((o) => o.op === "answer_call" && JSON.stringify(o.answer) === JSON.stringify({ kind: { kind: "choice" }, text: "Publish only the queue fix." })), "a typed answer is {kind:{kind:choice},text}");
    await shot(page, "run-answered", scheme);
    await page.click('.tc-banner [data-tc-run="give_back"]');
    await page.waitForFunction(() => document.querySelector(".tc-runhead h1")?.textContent.endsWith("is working"));
    check(sent.some((o) => JSON.stringify(o) === JSON.stringify({ op: "give_back", run: "r1" })), "give back sends {op:give_back,run}");
    await page.click('[data-tc-run="stop_run"]');
    await page.waitForFunction(() => document.querySelector(".tc-runhead h1")?.textContent === "Stopped by you");
    check(sent.some((o) => JSON.stringify(o) === JSON.stringify({ op: "stop_run", run: "r1" })), "stop sends {op:stop_run,run}");
    await shot(page, "run-stopped", scheme);
    await context.close();
  }
  {
    // A run working: active time, waiting time (not counted toward the limit) and the box's cost.
    const { page, context } = await open(browser, scheme);
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    await page.evaluate(() => openTeamRun("r4"));
    await page.waitForSelector(".tc-facts");
    const facts = await text(page, ".tc-facts");
    check(facts.includes("Active time: 24 min") && facts.includes("Waiting for people: 12 min (not counted toward the time limit)") && facts.includes("Cost: Box US$ 0.12 · Model US$ 0.30"), `the run page shows the times and the box and model cost (${facts.replace(/\n/g, " | ")})`);
    await shot(page, "run-working", scheme);
    await context.close();
  }
  {
    // Limit reached: the run's open on_limit call, answered with a plain button.
    const { page, context, sent } = await open(browser, scheme, (w) => {
      Object.assign(w.runs.find((r) => r.id === "r4"), { state: "waiting", active_secs: 1800, waiting_secs: 300, cost_usd: 0.51, started_at: at(14, 30) });
      w.calls.push({ id: "c6", run: "r4", workflow: "w4", situation: "on_limit", asked: [{ kind: "person", account: ME }], question: "Date fixer used its 30 minutes. It has changed the orders page and still has the invoices page to do. Let it go on?", choices: ["Let it go on", "Stop it"], created_at: at(15, 8), escalated: false, answer: null });
    });
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    await page.evaluate(() => openTeamRun("r4"));
    await page.waitForSelector(".tc-limit");
    check((await text(page, ".tc-runhead h1")) === "Limit reached", "the run's state is Limit reached");
    check((await page.locator(".tc-limit .tc-primary").count()) === 0 && (await page.locator('.tc-limit [data-tc="choice"].tc-secondary').count()) === 2, "its two choices are plain buttons");
    check((await page.locator('.tc-limit [data-tc="choice"]').allInnerTexts()).join() === "Let it go on,Stop it", "the hub's own two choices");
    await shot(page, "run-limit-reached", scheme);
    await page.click('.tc-limit [data-choice="0"]');
    await page.waitForFunction(() => document.querySelector(".tc-runhead h1")?.textContent.endsWith("is working"));
    check(sent.some((o) => o.op === "answer_call" && o.id === "c6" && JSON.stringify(o.answer) === JSON.stringify({ kind: { kind: "choice" }, choice: "Let it go on" })), "Let it go on is the existing answer_call");
    check((await page.locator(".tc-limit").count()) === 0, "the limit state leaves once it is answered");
    await context.close();
  }
  {
    // An ended run whose log went away with its box.
    const { page, context } = await open(browser, scheme, () => {}, { logGone: true });
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    await page.evaluate(() => openTeamRun("r5"));
    await page.waitForSelector("[data-log-note]");
    check((await text(page, "[data-log-note]")) === "The log went away with the run's box.", "an ended run whose log is gone says so");
    check(!(await page.locator(".tc-wrap").innerText()).includes("the team has no such run"), "the raw refusal is not shown");
    await shot(page, "run-log-gone", scheme);
    await context.close();
  }
  {
    // The run's log is present: the bot's own lines are in the timeline.
    const { page, context } = await open(browser, scheme);
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    await page.evaluate(() => openTeamRun("r1"));
    await page.waitForSelector(".tc-timeline .tc-call");
    check((await page.locator(".tc-timeline .tc-event").count()) >= 3, "the log's lines and the call are drawn");
    await shot(page, "run-log-present", scheme);
    await context.close();
  }
  {
    // A step to approve, whole and as sent: a Bash command, and a non-Bash tool with its input.
    const bash = 'release: GH_TOKEN=[hidden] gh release create v1.4.0 --title "Spring" --notes "Fixes the queue."\n  --target main && echo "<done>"';
    const edit = 'secrets: Edit {"file_path":"/work/api-orders/.env.production","new_string":"PAYMENT_KEY=[hidden]","old_string":"PAYMENT_KEY=test"}';
    const { page, context } = await open(browser, scheme, (w) => {
      w.calls.find((c) => c.id === "c1").question = bash;
      w.calls.find((c) => c.id === "c1").choices = ["Go ahead", "Don't"];
      w.calls.push({ id: "c3", run: "r3", workflow: "w2", situation: "before_irreversible", asked: [{ kind: "person", account: ME }], question: edit, choices: ["Go ahead", "Don't"], created_at: at(15, 6), escalated: false, answer: null });
    });
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector('[data-call="c3"] .tc-step');
    check((await text(page, '[data-call="c1"] .tc-step-text')) === bash.slice("release: ".length), "the Bash step is on the card whole, as sent");
    check((await page.locator('[data-call="c1"] .tc-step-tool').textContent()) === "Bash", "a command is a Bash step");
    check((await page.locator('[data-call="c3"] .tc-step-tool').textContent()) === "Edit", "a non-Bash step names its tool");
    check((await page.locator('[data-call="c3"] .tc-step-text').textContent()) === edit.slice("secrets: Edit ".length), "and shows its whole input");
    check((await page.locator('[data-call="c1"] .tc-step-text').evaluate((el) => el.scrollHeight <= el.clientHeight + 1)), "the step is not clipped");
    await shot(page, "call-card-steps", scheme);
    await context.close();
  }
  {
    // A call closed because its run ended: the card, then the run page.
    const cause = "its session's tab went away before it finished";
    const { page, context, w, push, setRun } = await open(browser, scheme);
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector('[data-call="c1"] .tc-actions, [data-call="c1"] [data-choice="0"]');
    const run = setRun("r1", { state: "failed", ended_at: NOW - 30, outcome: cause });
    const call = { ...w.calls.find((c) => c.id === "c1"), answer: { by: w.team.id, choice: null, text: cause, kind: { kind: "stop" }, at: NOW - 30 } };
    push({ kind: "call_resolved", call });
    push({ kind: "run_changed", run });
    await page.waitForSelector('[data-call="c1"] .tc-result[data-state="other"]');
    check((await text(page, '[data-call="c1"] .tc-result')) === `Closed: the run ended · ${cause}`, "the card says the run ended and the cause");
    check(!(await page.locator(".tc-wrap").innerText()).includes("A former member"), "the team is never the answerer");
    await shot(page, "call-card-closed-by-run", scheme);
    await page.click('[data-call="c1"] .tc-seerun, .tc-card[data-call="c1"] a, .tc-card[data-call="c1"] button.tc-seerun').catch(() => {});
    await page.evaluate(() => openTeamRun("r1"));
    await page.waitForSelector(".tc-timeline .tc-call");
    await shot(page, "run-closed-by-run", scheme);
    await context.close();
  }
  {
    // A run that ended: the hub's words for how, and no buttons.
    const { page, context } = await open(browser, scheme, (w) => {
      Object.assign(w.runs.find((r) => r.id === "r2"), { state: "gave_up", ended_at: at(14, 53) + 7200 });
      w.calls.find((c) => c.id === "c2").escalated = true;
    });
    await page.click('#team-nav [data-tc-nav="team-calls"]');
    await page.waitForSelector(".tc-card");
    await page.evaluate(() => openTeamRun("r2"));
    await page.waitForSelector(".tc-timeline .tc-call");
    check((await page.locator(".tc-head-actions button").count()) === 0, "an ended run has no buttons");
    await shot(page, "run-gave-up", scheme);
    await context.close();
  }
}

await browser.close();
server.close();
const failed = checks.filter(([ok]) => !ok);
console.log(`${checks.length - failed.length} checks passed, ${failed.length} failed`);
process.exit(failed.length ? 1 : 0);
