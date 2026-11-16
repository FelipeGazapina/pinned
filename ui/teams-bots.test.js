import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui-teams-bots.js", import.meta.url), "utf8");
const flush = async () => {
  for (let i = 0; i < 6; i++) await new Promise((resolve) => setImmediate(resolve));
};
const here = (value) => JSON.parse(JSON.stringify(value));
const text = (html) => html.replace(/<[^>]+>/g, " ").replace(/&#(\d+);/g, (_, c) => String.fromCharCode(c)).replace(/\s+/g, " ").trim();
const NOW = Math.floor(Date.now() / 1000);

const ANA = { account: "a1", email: "ana.prado@acme.dev", role: "owner", groups: ["publish"] };
const BRUNO = { account: "b2", email: "bruno@acme.dev", role: "member", groups: ["publish"] };
const CARLA = { account: "c3", email: "carla@acme.dev", role: "member", groups: [] };
const MEMBERS = [ANA, BRUNO, CARLA];
const PUBLISH = { handle: "publish", about: "Whoever can say yes or no when a bot wants to put something live.", members: ["a1", "b2"], fallback: null, slack_group: "" };
const REVIEWERS = { handle: "reviewers", about: "", members: ["c3"], fallback: null, slack_group: "" };
const PROJECTS = [
  { key: "github.com/acme/orders-api", name: "orders-api", repo: "github.com/acme/orders-api", member: "a1" },
  { key: "github.com/acme/site", name: "site", repo: "github.com/acme/site", member: "b2" },
];
const OWNER_TEAM = { id: "t1", name: "Acme", role: "owner", credential: { upstash: true, claude: true, github: true, slack: false } };
const MEMBER_TEAM = { ...OWNER_TEAM, role: "member" };

const calls = (over = {}) => ({ before_irreversible: { kind: "group", handle: "publish" }, in_doubt: { kind: "group", handle: "publish" }, on_limit: { kind: "owner" }, on_failure: { kind: "person", account: "c3" }, ...over });
const bot = (over = {}) => ({ workflow: "c1", name: "Test publisher", owner: "a1", calls: calls(), limits: { usd: 5, minutes: 30 }, on: true, on_by: "a1", once: false, ...over });
const PROMPT = "1. Take what entered orders-api since the last publish.\n2. Run all the tests.\n3. Stop and call a person before publishing.\n4. Publish to the test environment and tell whoever made the changes the result.";
const workflow = (over = {}) => ({
  id: "c1",
  name: "Test publisher",
  trigger: { kind: "linear-label", label: "pinned-run-now-only" },
  repo: "github.com/acme/orders-api",
  graph: { nodes: [{ id: "trigger-1", type: "trigger" }, { id: "session-1", type: "session", prompt: PROMPT, repo: "github.com/acme/orders-api" }], edges: [] },
  ...over,
});
const run = (over = {}) => ({ id: "r1", workflow: "c1", state: "done", controller: null, cost_usd: 1, active_secs: 60, started_at: NOW - 100, ended_at: NOW - 50, outcome: null, started_by: "a1", ...over });

/** gui-teams-bots.js in a page whose hub answers each team op from `answers` (a value, or a function of the call). */
function app({ answers = {}, team = OWNER_TEAM, email = "bruno@acme.dev", pages = ["team-run", "team-calls"] } = {}) {
  const frames = [];
  const listeners = {};
  const docListeners = {};
  const shown = [];
  const context = {
    console,
    Date,
    document: { body: { dataset: { page: "" } }, addEventListener: (type, fn) => (docListeners[type] = fn) },
    SHELL_PAGES: Object.fromEntries(pages.map((p) => [p, {}])),
    pageView: { addEventListener: (type, fn) => (listeners[type] = fn), querySelector: () => null },
    shellPage: () => context.document.body.dataset.page || null,
    setPage: (html) => shown.push(html),
    headHtml: (title) => `<header class="page-head"><h1>${title}</h1></header>`,
    esc: (v) => String(v ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    account: { email },
    opened: [],
    openPage: (name) => {
      context.opened.push(name);
      context.document.body.dataset.page = name;
      context.SHELL_PAGES[name]?.show?.();
    },
    closePage: () => context.opened.push("closed"),
    // The calls UI's run opener, a cross-file global.
    runsOpened: [],
    openTeamRun: (id) => {
      context.runsOpened.push(id);
      context.opened.push("team-run");
    },
    teamsCurrent: () => team,
    teamCall: async (op) => {
      frames.push(here(op));
      const answer = answers[op.op === "hub" ? `hub:${op.call.cmd}` : op.op];
      const out = typeof answer === "function" ? answer(op) : answer;
      if (out instanceof Error) throw out.message;
      return here(out ?? null);
    },
  };
  vm.createContext(context);
  vm.runInContext(src, context);
  const page = () => shown[shown.length - 1] ?? "";
  const open = async (name) => {
    context.document.body.dataset.page = name;
    context.SHELL_PAGES[name].show();
    await flush();
  };
  /** Whether the page has a control `data-tb=name`, and whether it is enabled. */
  const control = (name, attrs = "") => {
    const m = new RegExp(`<button[^>]*data-tb="${name}"${attrs}[^>]*>`).exec(page());
    return m ? { disabled: /\sdisabled(\s|>|=)/.test(m[0]) } : null;
  };
  const click = async (tb, data = {}) => {
    const el = { dataset: { tb, ...data }, disabled: false };
    listeners.click({ target: { closest: (sel) => (sel === "[data-tb]" ? el : null) } });
    await flush();
  };
  const input = async (field, value, data = {}) => {
    listeners.input({ target: { dataset: { tbInput: field, ...data }, value } });
    await flush();
  };
  return { context, frames, shown, page, open, click, input, control, docListeners, listeners, read: (code) => here(vm.runInContext(code, context)) };
}

const ASKS_LEAD = "It asks the team before the steps on this list:";
const ASKS_ITEMS = ["Delete outside the worktree", "Force-push", "Push to main", "Release", "Deploy", "Production", "Secrets", "Payment", "Write outside the worktree", "Change its own permissions", "Network install", "Machine-wide change"];
const ASKS_KEYS = "The bot can do whatever the team's keys allow, so connect keys with the least access the bot needs.";
/** The page says what the bot asks the team before, as a visible list of twelve, then the advice on keys, in this order. */
function assertAsksList(html, where) {
  const t = text(html);
  const list = html.match(/<ul class="tb-asks">([\s\S]*?)<\/ul>/)?.[1] ?? "";
  assert.deepEqual([...list.matchAll(/<li>([^<]*)<\/li>/g)].map((m) => m[1]), ASKS_ITEMS, `${where}: the twelve items`);
  assert.ok(t.indexOf(ASKS_LEAD) >= 0 && t.indexOf(ASKS_LEAD) < t.indexOf("Delete outside the worktree"), `${where}: the lead comes first`);
  assert.ok(t.indexOf("Machine-wide change") < t.indexOf(ASKS_KEYS), `${where}: the advice on keys follows the list`);
  assert.ok(!t.includes("It never deploys"), `${where}: the old sentence is gone`);
  assert.ok(!/\b(cannot|blocked|prevents?|never|won't)\b/i.test(t), `${where}: none of the words the guard may not be described with`);
}

/** The ops a page asks to draw itself. */
const wizardAnswers = (over = {}) => ({ members: MEMBERS, groups: [PUBLISH, REVIEWERS], projects: PROJECTS, ...over });

// ---- Helpers ------------------------------------------------------------------------------------

test("steps are read from the numbered prompt, and a step that calls a person says which rule", () => {
  const a = app();
  assert.deepEqual(a.read(`tbStepsOf(${JSON.stringify(PROMPT)})`), [
    "Take what entered orders-api since the last publish.",
    "Run all the tests.",
    "Stop and call a person before publishing.",
    "Publish to the test environment and tell whoever made the changes the result.",
  ]);
  assert.deepEqual(a.read(`["Stop and call a person before publishing.", "Call a person if in doubt about what the change is for.", "Calls @publish when a test fails.", "Find the open pull requests nobody has reviewed.", "Tell whoever made the changes the result."].map(tbCallStep)`), ["before_irreversible", "in_doubt", "on_limit", null, null]);
  assert.equal(a.read(`tbPromptOf(["a ", "b"])`), "1. a\n2. b");
});

test("a time reads as today, yesterday, a weekday, then a date", () => {
  const a = app();
  const noon = Date.UTC(2026, 10, 18, 12, 0, 0);
  const at = (days) => Math.floor((noon - days * 86400000) / 1000);
  assert.deepEqual(a.read(`[0, 1, 3, 9].map((d) => tbWhen(${at(0)} - d * 86400 + (d ? 0 : 0), ${noon}))`), ["today", "yesterday", "Sunday", "Nov 9"]);
});

test("the viewer is the member teamsCurrent().me names, and the e-mail finds one when it names none", () => {
  const named = app({ team: { ...MEMBER_TEAM, me: "c3" }, email: "bruno@acme.dev" });
  assert.equal(named.read(`tbMe(${JSON.stringify(MEMBERS)}).account`), "c3");
  assert.equal(app({ team: MEMBER_TEAM, email: "bruno@acme.dev" }).read(`tbMe(${JSON.stringify(MEMBERS)}).account`), "b2");
  assert.equal(app({ team: MEMBER_TEAM, email: "x@y.z" }).read(`tbMe(${JSON.stringify(MEMBERS)})`), null);
});

test("a person is shown by the name in their e-mail", () => {
  const a = app();
  assert.equal(a.read(`tbPerson("ana.prado@acme.dev")`), "Ana Prado");
  assert.equal(a.read(`tbPerson("bruno@acme.dev")`), "Bruno");
  assert.equal(a.read(`tbInitials("Ana Prado")`), "AP");
});

test("runs read in words: what it came to, and who a waiting run waits for", () => {
  const a = app();
  const line = (r, extra = {}) => a.read(`tbRunLine(${JSON.stringify(r)}, ${JSON.stringify({ members: MEMBERS, ...extra })})`);
  assert.deepEqual(line(run({ state: "done", outcome: "Published. Ana said yes in 4 minutes." })), { text: "Published. Ana said yes in 4 minutes.", tone: "" });
  assert.deepEqual(line(run({ state: "done" })), { text: "Done.", tone: "" });
  assert.deepEqual(line(run({ state: "waiting" }), { open: { asked: [{ kind: "group", handle: "publish" }] } }), { text: "Waiting for @publish", tone: "wait" });
  assert.deepEqual(line(run({ state: "waiting" })), { text: "Waiting for a person", tone: "wait" });
  assert.deepEqual(line(run({ state: "taken_over", controller: "c3" })), { text: "Carla took it over", tone: "live" });
  assert.deepEqual(line(run({ state: "gave_up" })), { text: "Nobody answered in time, so it stopped without doing what it asked.", tone: "bad" });
  assert.deepEqual(line(run({ state: "failed", outcome: "A test failed." })), { text: "Failed. A test failed.", tone: "bad" });
  assert.deepEqual(line(run({ state: "working" })), { text: "Working now", tone: "live" });
});

// ---- Create a bot -------------------------------------------------------------------------------

test("the wizard reads the team's people, groups and projects when it opens", async () => {
  const a = app({ answers: wizardAnswers() });
  await a.open("team-bot-new");
  assert.deepEqual(a.frames, [{ op: "members" }, { op: "groups" }, { op: "projects" }]);
  const t = text(a.page());
  assert.match(t, /Step 1 of 4 What should the bot do\?/);
  for (const idea of ["Review every new PR", "Publish to the test environment", "Weekly summary", "Or describe it in your own words"]) assert.ok(t.includes(idea), idea);
  assert.deepEqual(a.control("next"), { disabled: true }, "nothing chosen yet");
});

test("an idea fills its fixed steps, the plan says nothing was understood, and Not available yet marks what the hub cannot do", async () => {
  const a = app({ answers: wizardAnswers() });
  await a.open("team-bot-new");
  await a.click("idea", { idea: "publish" });
  assert.deepEqual(a.control("next"), { disabled: false });
  await a.click("next");
  const t = text(a.page());
  assert.match(t, /Step 2 of 4 Here is the plan\. Is it right\?/);
  assert.ok(t.includes("Nothing was worked out from your words"));
  const steps = [...a.page().matchAll(/data-tb-input="step" data-i="(\d)" value="([^"]*)"/g)].map((m) => m[2]);
  assert.deepEqual(steps, ["Take what entered the project since the last publish.", "Run all the tests.", "Stop and call a person before publishing.", "Publish to the test environment and tell whoever made the changes the result."]);
  assert.match(t, /When it runs Only when someone runs it Every weekday at 3pm Not available yet When a PR is merged Not available yet/);
  assert.equal([...a.page().matchAll(/class="tb-pill" disabled/g)].length, 2, "the schedule and event choices are disabled");
  assert.match(a.page(), /<button type="button" class="tb-pill" aria-pressed="true">Only when someone runs it<\/button>/);
  assert.ok(t.includes("Run once, then discard"));
});

test("a description of the person's own becomes one editable step, not a plan", async () => {
  const a = app({ answers: wizardAnswers() });
  await a.open("team-bot-new");
  await a.input("desc", "whenever a PR is open, check that it has tests");
  assert.deepEqual(a.control("next"), { disabled: true }, "the page was not redrawn while typing, so the button is read from the state");
  await a.click("next");
  const t = text(a.page());
  assert.ok(t.includes("Your description is the only step for now"), t);
  assert.ok(!t.includes("Nothing was worked out"), "no claim of having understood anything");
  assert.deepEqual([...a.page().matchAll(/data-tb-input="step" data-i="(\d)" value="([^"]*)"/g)].map((m) => m[2]), ["whenever a PR is open, check that it has tests"]);
  assert.ok(a.page().includes('data-tb-input="name" value="whenever a PR is open, check"'));
  assert.match(text(a.page()), /On a schedule Not available yet When something happens on GitHub Not available yet/);
  await a.click("add-step");
  await a.input("step", "Say so in the PR.", { i: "1" });
  await a.click("remove-step", { i: "0" });
  assert.deepEqual([...a.page().matchAll(/data-tb-input="step" data-i="(\d)" value="([^"]*)"/g)].map((m) => m[2]), ["Say so in the PR."]);
});

test("creating a bot saves the workflow in the team's store through hub, then the bot, as a draft", async () => {
  const a = app({ answers: wizardAnswers({ "hub:save_connector": { id: "c1700", name: "Test publisher" }, save_bot: null }) });
  await a.open("team-bot-new");
  await a.click("idea", { idea: "publish" });
  await a.click("next");
  await a.click("once");
  await a.click("next");
  // Step 3: who stays nearby.
  assert.match(text(a.page()), /Step 3 of 4 Who stays nearby\? .*Before something that can't be undone.*When in doubt.*At a limit or on failure/);
  await a.click("who", { moment: "before", kind: "group", id: "publish" });
  await a.click("who", { moment: "doubt", kind: "group", id: "reviewers" });
  await a.click("who", { moment: "limit", kind: "person", id: "b2" });
  assert.ok(a.page().includes('data-moment="before" data-kind="group" data-id="publish" aria-pressed="true"'));
  await a.click("next");
  // Step 4: how far.
  let t = text(a.page());
  assert.match(t, /Step 4 of 4 How far can it go on its own\?/);
  assert.ok(t.includes("up to US$ 5") && t.includes("up to 30 minutes"));
  assertAsksList(a.page(), "step 4");
  assert.ok(t.includes("The bot starts as a draft. It runs on its own only after someone in @publish or @reviewers (or the owner) turns it on."), t);
  assert.deepEqual(a.control("next"), { disabled: true }, "two projects and none picked");
  await a.click("project", { key: "github.com/acme/orders-api" });
  a.frames.length = 0;
  await a.click("next");
  assert.deepEqual(a.frames, [
    {
      op: "hub",
      call: {
        cmd: "save_connector",
        connector: {
          id: "",
          name: "Test publisher",
          enabled: false,
          trigger: { kind: "linear-label", label: "pinned-run-now-only" },
          folder: "",
          repo: "github.com/acme/orders-api",
          model: "opus",
          effort: "high",
          permissionMode: "auto",
          worktree: false,
          proof: false,
          instructions: "",
          graph: {
            nodes: [
              { id: "trigger-1", name: "Trigger", position: [0, 0], type: "trigger", watch: { kind: "linear-label", label: "pinned-run-now-only" } },
              {
                id: "session-1",
                name: "Run session",
                position: [230, 0],
                type: "session",
                prompt: "1. Take what entered the project since the last publish.\n2. Run all the tests.\n3. Stop and call a person before publishing.\n4. Publish to the test environment and tell whoever made the changes the result.",
                folder: "",
                repo: "github.com/acme/orders-api",
                model: "",
                effort: "",
                permission: "",
                account: "",
                worktree: false,
                instructions: "",
              },
            ],
            edges: [{ from: "trigger-1", port: "out", to: "session-1" }],
          },
        },
      },
    },
    {
      op: "save_bot",
      bot: {
        workflow: "c1700",
        name: "Test publisher",
        owner: null,
        calls: { before_irreversible: { kind: "group", handle: "publish" }, in_doubt: { kind: "group", handle: "reviewers" }, on_limit: { kind: "person", account: "b2" }, on_failure: { kind: "person", account: "b2" } },
        limits: { usd: 5, minutes: 30 },
        on: false,
        on_by: null,
        once: true,
      },
    },
  ]);
  t = text(a.page());
  assert.ok(t.includes("Test publisher is ready, as a draft"), t);
});

test("a failed save of the bot keeps the workflow, so trying again updates it instead of making another", async () => {
  let fail = true;
  const a = app({ answers: wizardAnswers({ "hub:save_connector": (op) => ({ id: op.call.connector.id || "c9", name: "x" }), save_bot: () => (fail ? new Error("this team has no group @publish") : null) }) });
  await a.open("team-bot-new");
  await a.click("idea", { idea: "weekly" });
  for (let i = 0; i < 3; i++) await a.click("next");
  await a.click("project", { key: "github.com/acme/site" });
  await a.click("next");
  assert.ok(text(a.page()).includes("Could not create the bot: this team has no group @publish"));
  fail = false;
  a.frames.length = 0;
  await a.click("next");
  assert.equal(a.frames[0].call.connector.id, "c9");
  assert.equal(a.frames[1].op, "save_bot");
  assert.ok(text(a.page()).includes("Weekly summary is ready, as a draft"));
});

test("the done screen turns the bot on only when the person may", async () => {
  const done = async (team, who, email) => {
    const a = app({ team, email, answers: wizardAnswers({ "hub:save_connector": { id: "c1" }, save_bot: null, turn_bot_on: null }) });
    await a.open("team-bot-new");
    await a.click("idea", { idea: "review" });
    await a.click("next");
    await a.click("next");
    for (const [moment, w] of Object.entries(who)) await a.click("who", { moment, ...w });
    await a.click("next");
    await a.click("project", { key: "github.com/acme/site" });
    await a.click("next");
    return a;
  };
  // The owner may.
  let a = await done(OWNER_TEAM, {}, "ana.prado@acme.dev");
  assert.match(text(a.page()), /PR reviewer is ready, as a draft It is not running yet\. You are the team's owner, so you can turn it on now\./);
  assert.deepEqual(a.control("turn-on"), { disabled: false });
  a.frames.length = 0;
  await a.click("turn-on");
  assert.deepEqual(a.frames, [{ op: "turn_bot_on", workflow: "c1", on: true }]);
  assert.match(text(a.page()), /PR reviewer is on It runs when someone runs it, from its page\. When it needs a person, it calls Ana Prado \(owner\)\./);
  assert.equal(a.control("turn-on"), null);
  assert.ok(a.control("see-bot"));
  // A member of a group the bot calls may.
  a = await done(MEMBER_TEAM, { before: { kind: "group", id: "publish" } }, "bruno@acme.dev");
  assert.match(text(a.page()), /You are in @publish, so you can turn it on now\./);
  assert.deepEqual(a.control("turn-on"), { disabled: false });
  // A member whose group the bot does not call may not, and is told whom to ask.
  a = await done(MEMBER_TEAM, { before: { kind: "group", id: "reviewers" } }, "bruno@acme.dev");
  assert.ok(text(a.page()).includes("It is not running yet. Ask someone in @reviewers to turn it on."), text(a.page()));
  assert.equal(a.control("turn-on"), null);
  // A member named by a rule may.
  a = await done(MEMBER_TEAM, { limit: { kind: "person", id: "b2" } }, "bruno@acme.dev");
  assert.match(text(a.page()), /A rule names you, so you can turn it on now\./);
  // A member nobody names asks the owner.
  a = await done(MEMBER_TEAM, {}, "bruno@acme.dev");
  assert.ok(text(a.page()).includes("Ask the owner to turn it on."));
});

test("seeing the bot's page opens it by the workflow's id", async () => {
  const a = app({ answers: { ...wizardAnswers({ "hub:save_connector": { id: "c1" }, save_bot: null }), bots: [bot()], runs: [], calls: [], "hub:get_settings": { connectors: [workflow()] } } });
  await a.open("team-bot-new");
  await a.click("idea", { idea: "review" });
  await a.click("next");
  await a.click("next");
  await a.click("next");
  await a.click("project", { key: "github.com/acme/site" });
  await a.click("next");
  await a.click("see-bot");
  assert.equal(a.context.opened.at(-1), "team-bot");
  assert.ok(text(a.page()).includes("Test publisher"));
});

test("back goes to the team's calls page", async () => {
  const a = app({ answers: wizardAnswers() });
  await a.open("team-bot-new");
  await a.click("back-team");
  assert.deepEqual(a.context.opened, ["team-calls"]);
});

test("text people typed stays text in the wizard", async () => {
  const bad = `<img src=x onerror="window.pwned=1">`;
  const a = app({ answers: wizardAnswers({ groups: [{ ...PUBLISH, handle: bad }], projects: [{ ...PROJECTS[0], name: bad }] }) });
  await a.open("team-bot-new");
  await a.input("desc", bad);
  await a.click("next");
  assert.ok(!a.page().includes("<img"), "no tag from typed text");
  await a.click("next");
  assert.ok(!a.page().includes("<img"));
  await a.click("next");
  assert.ok(!a.page().includes("<img"));
});

// ---- The bot page -------------------------------------------------------------------------------

const botAnswers = (over = {}) => ({
  bots: [bot()],
  members: MEMBERS,
  groups: [PUBLISH, REVIEWERS],
  projects: PROJECTS,
  runs: [
    run({ id: "r4", started_at: NOW - 100, state: "waiting", ended_at: null }),
    run({ id: "r3", started_at: NOW - 86400, outcome: "Published. Ana said yes in 4 minutes." }),
    run({ id: "r2", started_at: NOW - 2 * 86400, state: "failed", outcome: "A test failed." }),
    run({ id: "other", workflow: "c2", started_at: NOW - 10 }),
  ],
  calls: [{ id: "k1", run: "r4", workflow: "c1", situation: "before_irreversible", asked: [{ kind: "group", handle: "publish" }], question: "Publish?", choices: [], created_at: NOW - 90, escalated: false, answer: null }],
  "hub:get_settings": { connectors: [workflow()] },
  ...over,
});
const openBot = async (a, id = "c1") => {
  a.context.document.body.dataset.page = "team-bot";
  a.context.openTeamBot(id);
  await flush();
};

test("the bot page says what the bot asks the team before, with the twelve steps listed", async () => {
  const a = app({ answers: botAnswers({ runs: [run()], calls: [] }) });
  await openBot(a);
  assertAsksList(a.page(), "the bot page");
});

test("the bot page reads the bot, people, groups, projects, runs and calls of the team", async () => {
  const a = app({ answers: botAnswers() });
  await openBot(a);
  assert.deepEqual(
    a.frames.map((f) => (f.op === "hub" ? f.call.cmd : f.op)).sort(),
    ["bots", "calls", "get_settings", "groups", "members", "projects", "runs"],
  );
});

test("an off bot is a draft: Run now waits, and the toggle turns it on", async () => {
  const a = app({ answers: botAnswers({ bots: [bot({ on: false, on_by: null })], runs: [run()], turn_bot_on: null }) });
  await openBot(a);
  const t = text(a.page());
  assert.match(t, /Test publisher Draft Bot of orders-api\. Owner: Ana Prado\./);
  assert.deepEqual(a.control("run-now"), { disabled: true });
  assert.ok(a.page().includes("Turn the bot on first: a draft does not run."));
  assert.deepEqual(a.control("toggle", ' aria-pressed="false"'), { disabled: false });
  assert.ok(t.includes("Off, turn it on"));
  a.frames.length = 0;
  await a.click("toggle");
  assert.deepEqual(a.frames[0], { op: "turn_bot_on", workflow: "c1", on: true });
});

test("an on bot runs now, then shows Running, follow it, which opens the run", async () => {
  const runs = botAnswers().runs.filter((r) => r.id !== "r4");
  let started = null;
  const a = app({
    answers: botAnswers({
      runs: () => (started ? [started, ...runs] : runs),
      calls: [],
      run_now: () => {
        started = run({ id: "r5", state: "working", ended_at: null, started_at: NOW });
        return started;
      },
    }),
  });
  await openBot(a);
  assert.ok(!text(a.page()).includes("Draft"));
  assert.ok(text(a.page()).includes("Turn the bot off"));
  assert.deepEqual(a.control("run-now"), { disabled: false });
  a.frames.length = 0;
  await a.click("run-now");
  assert.deepEqual(a.frames[0], { op: "run_now", workflow: "c1" });
  assert.equal(a.control("run-now"), null);
  assert.match(text(a.page()), /Running, follow it/);
  assert.match(a.page(), /data-tb="open-run" data-run="r5"/);
  await a.click("open-run", { run: "r5" });
  assert.deepEqual(a.context.runsOpened, ["r5"], "the run opener of the calls UI is called with the run");
  assert.equal(a.context.opened.at(-1), "team-run");
});

test("a refused Run now says why, on the page", async () => {
  const a = app({ answers: botAnswers({ runs: [run()], calls: [], run_now: new Error("the team needs its Upstash key and Claude token before a bot can run") }) });
  await openBot(a);
  await a.click("run-now");
  assert.ok(text(a.page()).includes("the team needs its Upstash key and Claude token before a bot can run"));
  assert.deepEqual(a.control("run-now"), { disabled: false });
});

test("turning an on bot off sends on false", async () => {
  const a = app({ answers: botAnswers({ turn_bot_on: null }) });
  await openBot(a);
  a.frames.length = 0;
  await a.click("toggle");
  assert.deepEqual(a.frames[0], { op: "turn_bot_on", workflow: "c1", on: false });
});

test("the step where the bot calls a person is highlighted with the group's chip", async () => {
  const a = app({ answers: botAnswers() });
  await openBot(a);
  const html = a.page();
  assert.match(text(html), /What it does, only when someone runs it 1 Take what entered orders-api since the last publish\. 2 Run all the tests\. 3 Stop and call a person before publishing\. @publish 4 Publish/);
  const calling = [...html.matchAll(/<li class="tb-step call">(.*?)<\/li>/g)];
  assert.equal(calling.length, 1);
  assert.match(calling[0][1], /data-tb="open-group" data-handle="publish">@publish</);
});

test("Who it calls has a row for each moment, a person's name, the owner's, and the limits", async () => {
  const a = app({ answers: botAnswers() });
  await openBot(a);
  const rows = [...a.page().matchAll(/<div class="tb-row" data-moment="(\w+)">(.*?)<\/div>/g)].map((m) => [m[1], text(m[2])]);
  assert.deepEqual(rows, [
    ["before_irreversible", "Before something that can't be undone @publish"],
    ["in_doubt", "When in doubt @publish"],
    ["on_limit", "At a limit (up to US$ 5 or 30 minutes per run) Ana Prado (owner)"],
    ["on_failure", "On failure Carla"],
  ]);
  assert.ok(text(a.page()).includes("Outside those moments it works alone, and anyone on the team can follow or take over."));
});

test("a limit and a failure that go to the same person are one row", async () => {
  const a = app({ answers: botAnswers({ bots: [bot({ calls: calls({ on_failure: { kind: "owner" } }) })] }) });
  await openBot(a);
  const rows = [...a.page().matchAll(/<div class="tb-row" data-moment="(\w+)">/g)].map((m) => m[1]);
  assert.deepEqual(rows, ["before_irreversible", "in_doubt", "on_limit"]);
  assert.ok(text(a.page()).includes("At a limit or on failure (up to US$ 5 or 30 minutes per run)"));
});

test("Latest runs are this bot's own, newest first, each outcome in words", async () => {
  const a = app({ answers: botAnswers() });
  await openBot(a);
  const rows = [...a.page().matchAll(/<span class="tb-when">([^<]*)<\/span><span class="tb-line" data-tone="(\w*)">([^<]*)<\/span>/g)].map((m) => [m[1], m[2], m[3]]);
  assert.deepEqual(rows, [
    ["today", "wait", "Waiting for @publish"],
    ["yesterday", "", "Published. Ana said yes in 4 minutes."],
    [rows[2][0], "bad", "Failed. A test failed."],
  ]);
  assert.equal(a.read("1"), 1);
});

test("the owner, a member of a group it calls and a person it names may turn a draft on; others are told whom to ask", async () => {
  const draft = bot({ on: false, on_by: null });
  const may = async (team, email, over = {}) => {
    const a = app({ team, email, answers: botAnswers({ bots: [draft], ...over }) });
    await openBot(a);
    return a;
  };
  let a = await may(OWNER_TEAM, "ana.prado@acme.dev");
  assert.deepEqual(a.control("toggle"), { disabled: false });
  a = await may(MEMBER_TEAM, "bruno@acme.dev");
  assert.deepEqual(a.control("toggle"), { disabled: false }, "Bruno is in @publish");
  a = await may(MEMBER_TEAM, "carla@acme.dev", { bots: [{ ...draft, calls: calls({ before_irreversible: { kind: "owner" }, in_doubt: { kind: "owner" } }) }] });
  assert.deepEqual(a.control("toggle"), { disabled: false }, "a rule names Carla");
  a = await may(MEMBER_TEAM, "carla@acme.dev");
  assert.deepEqual(a.control("toggle"), { disabled: false }, "Carla is named for failures");
  a = await may(MEMBER_TEAM, "carla@acme.dev", { bots: [{ ...draft, calls: calls({ on_failure: { kind: "owner" } }) }] });
  assert.deepEqual(a.control("toggle"), { disabled: true });
  assert.ok(text(a.page()).includes("Ask someone in @publish to turn it on."));
});

test("a bot made to run once says so, and a team with no keys is told", async () => {
  const a = app({ team: { ...OWNER_TEAM, credential: { upstash: false, claude: false, github: false, slack: false } }, answers: botAnswers({ bots: [bot({ on: false, once: true })] }) });
  await openBot(a);
  const t = text(a.page());
  assert.ok(t.includes("Runs once, then is discarded"));
  assert.ok(t.includes("The team needs its Upstash key and Claude token before a bot can run."));
});

test("the bot page asks for a team when none is chosen, and for a bot the team does not have", async () => {
  let a = app({ team: null, answers: botAnswers() });
  await openBot(a);
  assert.ok(text(a.page()).includes("Choose a team in the account menu"));
  assert.deepEqual(a.frames, []);
  a = app({ answers: botAnswers({ bots: [] }) });
  await openBot(a, "nope");
  assert.ok(text(a.page()).includes("This team has no such bot."));
});

test("a bot's name, steps and a run's outcome stay text", async () => {
  const bad = `<img src=x onerror="window.pwned=1">`;
  const a = app({ answers: botAnswers({ bots: [bot({ name: bad })], runs: [run({ outcome: bad })], "hub:get_settings": { connectors: [workflow({ graph: { nodes: [{ type: "session", prompt: `1. ${bad}` }] } })] } }) });
  await openBot(a);
  assert.ok(!a.page().includes("<img"));
  assert.ok(a.page().includes("&#60;img") || a.page().includes("&lt;img") || a.page().includes("&#"), "escaped");
});

test("a push from the hub reads the bot page again", async () => {
  const a = app({ answers: botAnswers() });
  await openBot(a);
  a.frames.length = 0;
  a.docListeners["teams:event"]();
  await flush();
  assert.ok(a.frames.some((f) => f.op === "runs"));
});

// ---- The group page -----------------------------------------------------------------------------

const groupAnswers = (over = {}) => ({
  groups: [PUBLISH, REVIEWERS],
  members: MEMBERS,
  bots: [bot(), bot({ workflow: "c2", name: "Date fixer", calls: calls({ before_irreversible: { kind: "group", handle: "publish" }, in_doubt: { kind: "owner" } }) }), bot({ workflow: "c3", name: "Reviewer", calls: calls({ before_irreversible: { kind: "owner" }, in_doubt: { kind: "owner" } }) })],
  calls: [
    { id: "k1", run: "r4", workflow: "c1", situation: "before_irreversible", asked: [{ kind: "group", handle: "publish" }], question: "?", choices: [], created_at: NOW - 600, escalated: false, answer: null },
    { id: "k2", run: "r3", workflow: "c1", situation: "in_doubt", asked: [{ kind: "group", handle: "publish" }], question: "?", choices: ["ok"], created_at: NOW - 86400, escalated: false, answer: { by: "a1", choice: "Ship it", text: "", kind: { kind: "choice" }, at: NOW - 86400 + 240 } },
    { id: "k3", run: "r2", workflow: "c1", situation: "in_doubt", asked: [{ kind: "group", handle: "publish" }], question: "?", choices: [], created_at: NOW - 3 * 86400, escalated: false, answer: { by: "b2", choice: null, text: "", kind: { kind: "stop" }, at: NOW - 3 * 86400 + 660 } },
    { id: "k4", run: "r9", workflow: "c1", situation: "in_doubt", asked: [{ kind: "group", handle: "publish" }], question: "?", choices: [], created_at: NOW - 20 * 86400, escalated: false, answer: null },
    { id: "k5", run: "r8", workflow: "c1", situation: "in_doubt", asked: [{ kind: "group", handle: "reviewers" }], question: "?", choices: [], created_at: NOW - 100, escalated: false, answer: null },
  ],
  ...over,
});
const openGroup = async (a, handle = "publish") => {
  a.context.document.body.dataset.page = "team-group";
  a.context.openTeamGroup(handle);
  await flush();
};

test("the group page reads the groups, people, bots and calls of the team", async () => {
  const a = app({ answers: groupAnswers() });
  await openGroup(a, "@publish");
  assert.deepEqual(a.frames.map((f) => f.op).sort(), ["bots", "calls", "groups", "members"]);
  assert.match(text(a.page()), /^@publish /);
});

test("the group page: its handle, about, who answers with the fallback marked, and how a call works", async () => {
  const a = app({ answers: groupAnswers(), email: "bruno@acme.dev" });
  await openGroup(a);
  const t = text(a.page());
  assert.ok(t.includes("@publish Whoever can say yes or no when a bot wants to put something live."));
  assert.ok(t.includes("2 people answer"));
  assert.match(t, /Ana Prado answers if nobody else does/);
  assert.match(t, /You in the app(?! and in Slack)/);
  const steps = [...a.page().matchAll(/<span class="tb-step-text">([^<]*)<\/span>/g)].map((m) => m[1]);
  assert.deepEqual(steps, [
    "The bot stops and writes @publish. Everyone in the group is told, in the app.",
    "The first person to answer settles it. The others see who answered and what they said.",
    "If nobody answers in 30 minutes, the bot calls Ana Prado directly.",
    "With no answer in 2 hours, the bot ends without doing what it asked and leaves the notice here.",
  ]);
});

test("and in Slack is said only when the team's Slack is connected", async () => {
  const slack = { ...MEMBER_TEAM, credential: { ...MEMBER_TEAM.credential, slack: true } };
  const a = app({ team: slack, answers: groupAnswers(), email: "bruno@acme.dev" });
  await openGroup(a);
  const t = text(a.page());
  assert.ok(t.includes("Everyone in the group is told, in the app and in Slack."));
  assert.ok(t.includes("You in the app and in Slack"));
});

test("a member sees Leave and sends leave_group; a person outside the group sees Join and sends join_group", async () => {
  let a = app({ answers: groupAnswers({ leave_group: null }), team: MEMBER_TEAM, email: "bruno@acme.dev" });
  await openGroup(a);
  assert.ok(text(a.page()).includes("Leave the group"));
  assert.deepEqual(a.control("membership"), { disabled: false });
  a.frames.length = 0;
  await a.click("membership");
  assert.deepEqual(a.frames[0], { op: "leave_group", handle: "publish" });
  a = app({ answers: groupAnswers({ join_group: null }), team: MEMBER_TEAM, email: "carla@acme.dev" });
  await openGroup(a);
  assert.ok(text(a.page()).includes("Join the group"));
  assert.ok(text(a.page()).includes("2 people answer"));
  assert.ok(!text(a.page()).includes("You in the app"), "not listed as a member");
  a.frames.length = 0;
  await a.click("membership");
  assert.deepEqual(a.frames[0], { op: "join_group", handle: "publish" });
});

test("only the owner sees Add someone, and adds through save_group with the whole group", async () => {
  let a = app({ answers: groupAnswers(), team: MEMBER_TEAM, email: "bruno@acme.dev" });
  await openGroup(a);
  assert.equal(a.control("show-add"), null);
  assert.equal(a.control("remove-member"), null);
  a = app({ answers: groupAnswers({ save_group: null }), team: OWNER_TEAM, email: "ana.prado@acme.dev" });
  await openGroup(a);
  assert.ok(a.control("show-add"));
  await a.click("show-add");
  assert.ok(text(a.page()).includes("Who joins the group? Carla"), text(a.page()));
  a.frames.length = 0;
  await a.click("add-member", { account: "c3" });
  assert.deepEqual(a.frames[0], { op: "save_group", group: { ...PUBLISH, members: ["a1", "b2", "c3"] } });
  a.frames.length = 0;
  await a.click("remove-member", { account: "b2" });
  assert.deepEqual(a.frames[0], { op: "save_group", group: { ...PUBLISH, members: ["a1"] } });
});

test("Bots that call this group name each bot and the moments", async () => {
  const a = app({ answers: groupAnswers() });
  await openGroup(a);
  const rows = [...a.page().matchAll(/data-tb="open-bot" data-workflow="(\w+)">.*?<span class="tb-line">([^<]*)</g)].map((m) => [m[1], text(m[2])]);
  assert.deepEqual(rows, [
    ["c1", "Test publisher, before something that can't be undone and when in doubt"],
    ["c2", "Date fixer, before something that can't be undone"],
  ]);
  await a.click("open-bot", { workflow: "c2" });
  assert.equal(a.context.opened.at(-1), "team-bot");
});

test("Calls this week: this group's calls of the last seven days, in words", async () => {
  const a = app({ answers: groupAnswers(), email: "bruno@acme.dev" });
  await openGroup(a);
  const rows = [...a.page().matchAll(/<span class="tb-when">([^<]*)<\/span><span class="tb-line" data-tone="(\w*)">([^<]*)<\/span>/g)].map((m) => [m[1], m[2], m[3]]);
  assert.equal(rows.length, 3, "the call of 20 days ago and the other group's call are not here");
  assert.deepEqual(rows[0], ["today", "wait", "Test publisher is waiting"]);
  assert.deepEqual(rows[1], ["yesterday", "", "Ana Prado answered in 4 minutes and chose “Ship it”."]);
  assert.equal(rows[2][2], "You answered in 11 minutes and stopped the run.");
});

test("a group the team does not have, and no team chosen", async () => {
  let a = app({ answers: groupAnswers() });
  await openGroup(a, "nope");
  assert.ok(text(a.page()).includes("This team has no such group."));
  a = app({ team: null, answers: groupAnswers() });
  await openGroup(a);
  assert.ok(text(a.page()).includes("Choose a team in the account menu"));
});

test("the three pages register themselves, and a team change reads them again", async () => {
  const a = app({ answers: groupAnswers() });
  assert.deepEqual(Object.keys(a.context.SHELL_PAGES).filter((k) => k.startsWith("team-b") || k === "team-group").sort(), ["team-bot", "team-bot-new", "team-group"]);
  await openGroup(a);
  a.frames.length = 0;
  a.docListeners["teams:changed"]();
  await flush();
  assert.ok(a.frames.length > 0);
});

test("a hub that fails shows the error and Try again", async () => {
  const a = app({ answers: groupAnswers({ groups: new Error("The hub did not answer.") }) });
  await openGroup(a);
  assert.ok(text(a.page()).includes("Could not load: The hub did not answer."));
  assert.ok(a.control("reload"));
});
