import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// The team sidebar (gui-team-nav.js) over the store it draws, with the bot wizard and the bot page that change the bots.
const files = ["gui-team-store.js", "gui-team-nav.js", "gui-teams-bots.js"].map((name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8"));
const flush = async () => {
  for (let i = 0; i < 6; i++) await new Promise((resolve) => setImmediate(resolve));
};
const NOW = Math.floor(Date.now() / 1000);

const ANA = { account: "a1", email: "ana.prado@acme.dev", role: "owner", groups: ["publish"] };
const BRUNO = { account: "b2", email: "bruno@acme.dev", role: "member", groups: ["publish"] };
const MEMBERS = [ANA, BRUNO];
const PUBLISH = { handle: "publish", about: "Whoever can say yes or no when a bot wants to put something live.", members: ["a1", "b2"], fallback: null, slack_group: "" };
const PROJECTS = [
  { key: "github.com/acme/orders-api", name: "orders-api", repo: "github.com/acme/orders-api", member: "a1" },
  { key: "github.com/acme/site", name: "site", repo: "github.com/acme/site", member: "b2" },
];
const TEAM = { id: "t1", name: "Acme", role: "owner", credential: { upstash: true, claude: true, github: true, slack: false } };
const CALLS = { before_irreversible: { kind: "group", handle: "publish" }, in_doubt: { kind: "group", handle: "publish" }, on_limit: { kind: "owner" }, on_failure: { kind: "person", account: "a1" } };

/** The sidebar and the wizard or bot page over one fake hub. `answers` maps an op to its answer, or to a function of the call. */
function sidebar({ answers }) {
  const navs = [];
  const pageListeners = {};
  const context = {
    console,
    Date,
    crypto: globalThis.crypto,
    document: {
      body: { dataset: { page: "" } },
      addEventListener() {},
      getElementById: () => ({ before() {} }),
      createElement: () => {
        const nav = { dataset: {}, innerHTML: "", setAttribute() {}, remove() {} };
        navs.push(nav);
        return nav;
      },
    },
    SHELL_PAGES: {},
    pageView: { addEventListener: (type, fn) => (pageListeners[type] = fn), querySelector: () => null },
    shellPage: () => context.document.body.dataset.page || null,
    setPage() {},
    headHtml: (title) => `<header><h1>${title}</h1></header>`,
    esc: (v) => String(v ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    account: { email: "ana.prado@acme.dev" },
    openPage: (name) => {
      context.document.body.dataset.page = name;
      context.SHELL_PAGES[name]?.show?.();
    },
    closePage() {},
    teamsCurrent: () => TEAM,
    teamCall: async (op) => {
      const answer = answers[op.op === "hub" ? `hub:${op.call.cmd}` : op.op];
      const out = typeof answer === "function" ? answer(op) : answer;
      if (out instanceof Error) throw out.message;
      return JSON.parse(JSON.stringify(out ?? null));
    },
  };
  vm.createContext(context);
  for (const src of files) vm.runInContext(src, context);
  const nav = () => navs.at(-1)?.innerHTML ?? "";
  const draw = async () => {
    vm.runInContext("tcNavDraw()", context);
    await flush();
  };
  const open = async (name) => {
    context.document.body.dataset.page = name;
    context.SHELL_PAGES[name].show();
    await flush();
  };
  const click = async (tb, data = {}) => {
    const el = { dataset: { tb, ...data }, disabled: false };
    pageListeners.click({ target: { closest: (sel) => (sel === "[data-tb]" ? el : null) } });
    await flush();
  };
  const openBot = async (id) => {
    context.document.body.dataset.page = "team-bot";
    context.openTeamBot(id);
    await flush();
  };
  return { nav, draw, open, click, openBot };
}

/** A hub that keeps its bots: a saved bot is listed, and a turn on or off changes its `on`. */
function hub(over = {}) {
  const state = { bots: [], saves: [], turns: [], reads: 0 };
  const answers = {
    members: MEMBERS,
    groups: [PUBLISH],
    projects: PROJECTS,
    calls: [],
    runs: [],
    bots: () => {
      state.reads += 1;
      return state.bots;
    },
    save_bot: (op) => {
      state.saves.push(op);
      state.bots = [{ workflow: op.bot.workflow, name: op.bot.name, owner: op.bot.owner, calls: op.bot.calls, limits: op.bot.limits, on: op.bot.on, on_by: op.bot.on_by, once: op.bot.once }];
      return { workflow: op.bot.workflow };
    },
    turn_bot_on: (op) => {
      state.turns.push(op);
      state.bots = state.bots.map((b) => (b.workflow === op.workflow ? { ...b, on: op.on, on_by: op.on ? "a1" : null } : b));
      return null;
    },
    "hub:get_settings": { connectors: [] },
    ...over,
  };
  return { answers, state };
}

/** The wizard from its start to the step that saves the bot: the idea "weekly", its three steps, and the project. */
async function createBot(s) {
  await s.open("team-bot-new");
  await s.click("idea", { idea: "weekly" });
  for (let i = 0; i < 3; i++) await s.click("next");
  await s.click("project", { key: "github.com/acme/site" });
}

test("a bot the wizard saves is in the sidebar at once, before Create a bot, with no reload", async () => {
  const h = hub();
  const s = sidebar({ answers: h.answers });
  await s.draw();
  assert.ok(s.nav().includes("Create a bot"));
  assert.ok(!s.nav().includes("Test publisher"));
  await createBot(s);
  await s.click("next");
  assert.equal(h.state.saves.length, 1);
  const name = h.state.saves[0].bot.name;
  const html = s.nav();
  assert.ok(html.includes(`aria-label="${name}, off"`), html);
  assert.ok(html.indexOf(name) < html.indexOf("Create a bot"), "the new bot is listed in the Bots section, above Create a bot");
});

test("a save the hub refuses leaves the sidebar as it was and reads the bots no more", async () => {
  const h = hub({ save_bot: () => new Error("this team has no group @publish") });
  const s = sidebar({ answers: h.answers });
  await s.draw();
  await createBot(s);
  const before = s.nav();
  const reads = h.state.reads;
  await s.click("next");
  assert.match(before, /Create a bot/);
  assert.equal(s.nav(), before, "the sidebar is the same");
  assert.equal(h.state.reads, reads, "no list is read after a refused save");
});

test("turning a bot on from its page changes its dot in the sidebar at once", async () => {
  const h = hub();
  h.state.bots = [{ workflow: "c1", name: "Test publisher", owner: "a1", calls: CALLS, limits: { usd: 5, minutes: 30 }, on: false, on_by: null, once: false }];
  const s = sidebar({ answers: h.answers });
  await s.draw();
  assert.ok(s.nav().includes('aria-label="Test publisher, off"'), s.nav());
  await s.openBot("c1");
  await s.click("toggle");
  assert.equal(h.state.turns.at(-1).on, true);
  assert.ok(s.nav().includes('aria-label="Test publisher, on"'), s.nav());
});

test("turning a new bot on from the done screen changes its dot in the sidebar at once", async () => {
  const h = hub();
  const s = sidebar({ answers: h.answers });
  await s.draw();
  await createBot(s);
  await s.click("next");
  const name = h.state.saves[0].bot.name;
  assert.ok(s.nav().includes(`aria-label="${name}, off"`), s.nav());
  await s.click("turn-on");
  assert.equal(h.state.turns.at(-1).on, true);
  assert.ok(s.nav().includes(`aria-label="${name}, on"`), s.nav());
});
