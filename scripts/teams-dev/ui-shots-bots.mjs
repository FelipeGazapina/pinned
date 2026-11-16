// Screenshots of the Teams "bots" screens (bot page, Create a bot wizard, group page) in the real ui/, dark and light,
// 1360 px wide, with the hub faked: a static server for ui/ and Playwright's routeWebSocket answering the team ops
// with invented data.
//
//   PW_DIR=<folder holding node_modules/playwright-core> OUT=<folder for the pngs> node scripts/teams-dev/ui-shots-bots.mjs
//
// CHROMIUM names the browser binary (default /usr/bin/chromium). Exits 1 when a page throws or a step finds no control.
import { createReadStream, existsSync, mkdirSync, statSync } from "node:fs";
import http from "node:http";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const OUT = process.env.OUT;
if (!OUT) throw new Error("set OUT to the folder for the screenshots");
const { chromium } = createRequire(`${process.env.PW_DIR ?? process.cwd()}/`)("playwright-core");
const UI = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "ui");
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".png": "image/png", ".svg": "image/svg+xml", ".webmanifest": "application/json" };

const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://x");
  const file = path.join(UI, url.pathname === "/" ? "index.html" : decodeURIComponent(url.pathname));
  if (!file.startsWith(UI) || !existsSync(file) || !statSync(file).isFile()) {
    res.writeHead(404, { "content-type": "application/json" });
    return res.end("{}");
  }
  res.writeHead(200, { "content-type": TYPES[path.extname(file)] ?? "application/octet-stream" });
  createReadStream(file).pipe(res);
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const BASE = `http://127.0.0.1:${server.address().port}`;

// ---- Invented data -----------------------------------------------------------------------------

const NOW = () => Math.floor(Date.now() / 1000);
const MEMBERS = [
  { account: "a1", email: "ana.prado@acme.dev", role: "owner", groups: ["publish"] },
  { account: "b2", email: "bruno.lima@acme.dev", role: "member", groups: ["publish"] },
  { account: "c3", email: "carla.souza@acme.dev", role: "member", groups: ["reviewers"] },
  { account: "d4", email: "diego.alves@acme.dev", role: "member", groups: [] },
];
const PROJECTS = [
  { key: "github.com/acme/orders-api", name: "orders-api", repo: "github.com/acme/orders-api", member: "a1" },
  { key: "github.com/acme/storefront", name: "storefront", repo: "github.com/acme/storefront", member: "b2" },
];
const PROMPT = [
  "Take what entered orders-api since the last publish.",
  "Run all the tests.",
  "Stop and call a person before publishing.",
  "Publish to the test environment and check that it came up.",
  "Tell whoever made the changes the result.",
].map((s, i) => `${i + 1}. ${s}`).join("\n");
const workflow = (id, name, prompt, repo) => ({
  id, name, enabled: true, trigger: { kind: "manual" }, repo,
  graph: { nodes: [{ id: "trigger-1", type: "trigger", name: "Trigger", position: [0, 0], watch: { kind: "manual" } }, { id: "session-1", type: "session", name: "Run session", position: [230, 0], prompt, repo }], edges: [] },
});

let world;
function reset(over = {}) {
  const me = over.me ?? "a1";
  world = {
    me,
    role: me === "a1" ? "owner" : "member",
    slack: false,
    keys: true,
    members: structuredClone(MEMBERS),
    groups: [
      { handle: "publish", about: "Whoever can say yes or no when a bot wants to put something live.", members: ["a1", "b2"], fallback: null, slack_group: "" },
      { handle: "reviewers", about: "Who reads pull requests when a bot is unsure.", members: ["c3"], fallback: null, slack_group: "" },
    ],
    projects: PROJECTS,
    bots: [
      { workflow: "c1", name: "Test publisher", owner: "a1", calls: { before_irreversible: { kind: "group", handle: "publish" }, in_doubt: { kind: "group", handle: "publish" }, on_limit: { kind: "owner" }, on_failure: { kind: "person", account: "c3" } }, limits: { usd: 5, minutes: 30 }, on: true, on_by: "a1", once: false },
      { workflow: "c2", name: "Date fixer", owner: "b2", calls: { before_irreversible: { kind: "group", handle: "publish" }, in_doubt: { kind: "owner" }, on_limit: { kind: "owner" }, on_failure: { kind: "owner" } }, limits: { usd: 5, minutes: 30 }, on: false, on_by: null, once: false },
    ],
    workflows: [workflow("c1", "Test publisher", PROMPT, "github.com/acme/orders-api"), workflow("c2", "Date fixer", "1. Find dates written in two formats.\n2. Make them one format.\n3. Call a person if in doubt about which format wins.", "github.com/acme/storefront")],
    runs: [
      { id: "r3", workflow: "c1", state: "done", controller: null, cost_usd: 1.2, active_secs: 600, started_at: NOW() - 86400, ended_at: NOW() - 86000, outcome: "Published. Ana said yes in 4 minutes.", started_by: "b2" },
      { id: "r2", workflow: "c1", state: "done", controller: null, cost_usd: 0.9, active_secs: 500, started_at: NOW() - 3 * 86400, ended_at: NOW() - 3 * 86400 + 900, outcome: "A test failed. Called Carla, who fixed it and ran it again.", started_by: "b2" },
      { id: "r1", workflow: "c1", state: "done", controller: null, cost_usd: 0.1, active_secs: 30, started_at: NOW() - 4 * 86400, ended_at: NOW() - 4 * 86400 + 30, outcome: "Nothing new to publish. Called nobody.", started_by: "a1" },
    ],
    calls: [
      { id: "k2", run: "r3", workflow: "c1", situation: "before_irreversible", asked: [{ kind: "group", handle: "publish" }], question: "Publish to test?", choices: ["Yes", "No"], created_at: NOW() - 86400, escalated: false, answer: { by: "a1", choice: "Yes", text: "", kind: { kind: "choice" }, at: NOW() - 86400 + 240 } },
      { id: "k3", run: "r2", workflow: "c1", situation: "in_doubt", asked: [{ kind: "group", handle: "publish" }], question: "Publish with one failing test?", choices: [], created_at: NOW() - 3 * 86400, escalated: false, answer: { by: "b2", choice: null, text: "A fix is missing.", kind: { kind: "stop" }, at: NOW() - 3 * 86400 + 660 } },
    ],
    ...over,
  };
}

const waiting = () => {
  world.runs.unshift({ id: "r4", workflow: "c1", state: "waiting", controller: null, cost_usd: 0.4, active_secs: 120, started_at: NOW() - 300, ended_at: null, outcome: null, started_by: "b2" });
  world.calls.unshift({ id: "k1", run: "r4", workflow: "c1", situation: "before_irreversible", asked: [{ kind: "group", handle: "publish" }], question: "Publish to test?", choices: ["Yes", "No"], created_at: NOW() - 290, escalated: false, answer: null });
};
const working = () => world.runs.unshift({ id: "r5", workflow: "c1", state: "working", controller: null, cost_usd: 0.1, active_secs: 20, started_at: NOW() - 20, ended_at: null, outcome: null, started_by: world.me });

/** The team as the hub's List answers it for the person looking. */
const teamRow = () => ({ id: "t1", name: "Acme Orders", owner: "a1", role: world.role, credential: { upstash: world.keys, claude: world.keys, github: true, slack: world.slack } });

function teamOp(op) {
  switch (op.op) {
    case "list": return [teamRow()];
    case "members": return world.members;
    case "groups": return world.groups;
    case "projects": return world.projects;
    case "bots": return world.bots;
    case "runs": return world.runs;
    case "calls": return world.calls;
    case "join_group": world.groups.find((g) => g.handle === op.handle).members.push(world.me); return null;
    case "leave_group": { const g = world.groups.find((x) => x.handle === op.handle); g.members = g.members.filter((m) => m !== world.me); return null; }
    case "save_group": Object.assign(world.groups.find((g) => g.handle === op.group.handle), op.group); return null;
    case "turn_bot_on": world.bots.find((b) => b.workflow === op.workflow).on = op.on; return null;
    case "run_now":
      if (world.busy) throw "the hub is busy with this right now; try again in a moment";
      working();
      return world.runs[0];
    case "delete_bot":
      if (world.bots.find((b) => b.workflow === op.workflow)?.on) throw "turn the bot off first";
      world.bots = world.bots.filter((b) => b.workflow !== op.workflow);
      return null;
    // save_bot answers nothing: the new workflow's id is the one `bots` lists afterwards.
    case "save_bot": {
      const id = "c1700";
      world.workflows.push({ ...op.connector, id });
      world.bots.push({ ...op.bot, workflow: id, name: op.connector.name, on: false });
      return null;
    }
    case "hub":
      if (op.call.cmd === "get_settings") return { connectors: world.workflows };
      throw `unexpected hub call ${op.call.cmd}`;
    default: throw `unexpected team op ${op.op}`;
  }
}

// ---- The browser ---------------------------------------------------------------------------------

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM ?? "/usr/bin/chromium", args: ["--no-sandbox"] });
const failures = [];
const seen = [];

async function session(theme) {
  const context = await browser.newContext({ viewport: { width: 1360, height: 1180 }, colorScheme: theme, serviceWorkers: "block" });
  const page = await context.newPage();
  page.on("pageerror", (e) => failures.push(`${theme}: page error: ${e}`));
  await page.addInitScript(() => {
    localStorage.setItem("hub-token", "fake");
    // The real entry code (ui/gui-teams.js) owns the current team and the team call; it reads the team from here.
    localStorage.setItem("teams-current", JSON.stringify({ id: "t1", name: "Acme Orders", role: "owner", credential: { upstash: true, claude: true, github: true, slack: false }, me: "a1" }));
  });
  await page.routeWebSocket(/\/ws\/client/, (ws) => {
    ws.onMessage((raw) => {
      const frame = JSON.parse(String(raw));
      if (frame.type === "auth") return ws.send(JSON.stringify({ type: "hello", account: { id: world.me, email: world.members.find((m) => m.account === world.me).email }, machines: [], tabs: {} }));
      if (frame.type !== "hub" && frame.type !== "call") return;
      let reply;
      try {
        if (frame.type === "hub" && frame.call.cmd === "team") reply = { ok: teamOp(frame.call.op) };
        else if (frame.type === "hub" && frame.call.cmd === "get_settings") reply = { ok: { connectors: [] } };
        else reply = { ok: null };
      } catch (e) {
        reply = { err: String(e) };
      }
      ws.send(JSON.stringify({ type: "reply", id: frame.id, ...reply }));
    });
  });
  await page.goto(BASE);
  await page.waitForFunction(() => document.body.dataset.hub === "online", null, { timeout: 15000 });
  return { page, context };
}

// Who is looking: the team's role and keys for them, and the e-mail the app signed in with.
const as = (page) => page.evaluate(([team, email]) => {
  teamsSelect(team);
  account.email = email;
}, [teamRow(), world.members.find((m) => m.account === world.me).email]);
const open = async (page, code, ready) => {
  await as(page);
  await page.evaluate(code);
  await page.waitForSelector(ready, { timeout: 8000 });
  await page.waitForTimeout(150);
};
const shot = async (page, theme, name) => {
  const dir = path.join(OUT, theme);
  mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: path.join(dir, `${name}.png`) });
  seen.push(`${theme}/${name}`);
};
const click = async (page, selector) => {
  const el = page.locator(selector).first();
  if (!(await el.count())) throw new Error(`no control ${selector}`);
  await el.click();
  await page.waitForTimeout(80);
};

async function wizard(page, theme, name, { idea = "publish", own = "", who = {}, project = "orders-api", turnOn = false, trigger = null } = {}) {
  await open(page, "openTeamBotNew()", ".tb-wiz");
  if (name === "wizard-1-empty") await shot(page, theme, name);
  if (own) await page.fill("#tb-desc", own);
  else await click(page, `[data-tb="idea"][data-idea="${idea}"]`);
  if (name === "wizard-1-idea") return shot(page, theme, name);
  await click(page, '[data-tb="next"]');
  if (trigger) {
    await click(page, `[data-tb="trigger"][data-kind="${trigger.kind}"]`);
    await page.fill(trigger.kind === "linear-label" ? "#tb-label-in" : "#tb-channel-in", trigger.value);
  }
  if (name.startsWith("wizard-2")) return shot(page, theme, name);
  await click(page, '[data-tb="next"]');
  for (const [moment, id] of Object.entries(who)) await click(page, `[data-tb="who"][data-moment="${moment}"][data-id="${id}"]`);
  if (name === "wizard-3") return shot(page, theme, name);
  await click(page, '[data-tb="next"]');
  await click(page, `[data-tb="project"][data-key="github.com/acme/${project}"]`);
  if (name === "wizard-4") return shot(page, theme, name);
  await click(page, '[data-tb="next"]');
  await page.waitForSelector('[data-tb="see-bot"]');
  if (turnOn) {
    await click(page, '[data-tb="turn-on"]');
    await page.waitForFunction(() => document.querySelector(".tb-wiz h1")?.textContent.endsWith("is on"));
  }
  await shot(page, theme, name);
}

for (const theme of ["dark", "light"]) {
  reset();
  const { page, context } = await session(theme);

  // Create a bot
  reset();
  await wizard(page, theme, "wizard-1-empty");
  reset();
  await wizard(page, theme, "wizard-1-idea");
  reset();
  await wizard(page, theme, "wizard-2-idea");
  reset();
  await wizard(page, theme, "wizard-2-own", { own: "whenever a PR is open, check that it has tests" });
  reset();
  await wizard(page, theme, "wizard-2-linear", { trigger: { kind: "linear-label", value: "bot-review" } });
  reset();
  await wizard(page, theme, "wizard-2-slack", { trigger: { kind: "slack-mention", value: "#releases" } });
  reset();
  await wizard(page, theme, "wizard-3", { who: { before: "publish", doubt: "reviewers", limit: "b2" } });
  reset();
  await wizard(page, theme, "wizard-4", { who: { before: "publish", doubt: "reviewers", limit: "b2" } });
  reset();
  await wizard(page, theme, "wizard-done-draft-owner", { who: { before: "publish" } });
  reset({ me: "b2" });
  await wizard(page, theme, "wizard-done-draft-in-group", { who: { before: "publish" } });
  reset({ me: "d4" });
  await wizard(page, theme, "wizard-done-draft-cannot", { who: { before: "publish" } });
  reset();
  await wizard(page, theme, "wizard-done-on", { who: { before: "publish" }, turnOn: true });

  // The bot page
  const bot = async (name, { id = "c1", setup = () => {}, steps = async () => {} } = {}) => {
    setup();
    await open(page, `openTeamBot("${id}")`, ".tb-section");
    await steps();
    await shot(page, theme, name);
  };
  reset();
  world.bots[0].on = false;
  await bot("bot-draft-owner");
  reset({ me: "b2" });
  world.bots[0].on = false;
  await bot("bot-draft-member-in-group");
  reset({ me: "d4" });
  world.bots[0].on = false;
  await bot("bot-draft-cannot");
  reset();
  await bot("bot-on-idle");
  reset();
  await bot("bot-on-running", { steps: async () => { await click(page, '[data-tb="run-now"]'); await page.waitForSelector(".tb-btn.follow"); } });
  reset();
  waiting();
  await bot("bot-on-waiting");
  reset();
  world.bots[0].on = false;
  world.bots[0].once = true;
  world.keys = false;
  await bot("bot-draft-once-no-keys");

  // Skipped events per reason, an unknown reason among them, and the off note.
  reset();
  world.bots[0].skipped = [{ reason: "off", count: 3 }, { reason: "queue_full", count: 2 }, { reason: "start_failed", count: 1 }, { reason: "never_started", count: 4 }, { reason: "burnt_toast", count: 7 }];
  await bot("bot-skipped");
  reset();
  world.bots[0].on = false;
  world.bots[0].off_note = "the team's Claude token stopped working, so the bot was turned off";
  world.bots[0].skipped = [{ reason: "off", count: 5 }];
  await bot("bot-off-note");
  // Delete a draft: asked in place, then the hub's refusal when the bot is on.
  reset();
  world.bots[0].on = false;
  await bot("bot-delete-ask", { steps: async () => { await click(page, '[data-tb="delete-ask"]'); await page.waitForSelector("[data-delete]"); } });
  reset();
  world.bots[0].on = false;
  await bot("bot-delete-refused", { steps: async () => {
    await click(page, '[data-tb="delete-ask"]');
    world.bots[0].on = true;
    await click(page, '[data-tb="delete"]');
    await page.waitForSelector(".tb-error");
  } });
  // Run now, busy.
  reset();
  world.busy = true;
  await bot("bot-run-now-busy", { steps: async () => { await click(page, '[data-tb="run-now"]'); await page.waitForSelector(".tb-error"); } });

  // The group page
  const group = async (name, handle = "publish", steps = async () => {}) => {
    await open(page, `openTeamGroup("${handle}")`, ".tb-section");
    await steps();
    await shot(page, theme, name);
  };
  reset({ me: "b2" });
  waiting();
  await group("group-member");
  reset({ me: "d4" });
  waiting();
  await group("group-not-member");
  reset({ me: "a1" });
  waiting();
  await group("group-owner-add", "publish", async () => { await click(page, '[data-tb="show-add"]'); });
  reset({ me: "b2" });
  world.slack = true;
  waiting();
  await group("group-slack");

  await context.close();
}

await browser.close();
server.close();
console.log(`${seen.length} screenshots in ${OUT}`);
if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}
