#!/usr/bin/env node
// Drives the Delegation metrics section of the Way of working page on the real page, with task
// records of several models, projects, delegators and outcomes seeded into the hub's database.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/delegation.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes delegation-*.png into scripts/web-sweep/screens/: desktop at 1440x900 and phone at
// 390x844, dark and light. SWEEP_CHANNEL=chrome drives the installed Google Chrome.
const { execFileSync } = require("child_process");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const PG_PORT = process.env.SWEEP_PG_PORT || 5497;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const SCREENS = path.join(__dirname, "screens");
const MIN = 60_000;
const DAY = 24 * 60 * MIN;

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};
const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};
const psql = (sql) => execFileSync("psql", ["-h", "127.0.0.1", "-p", String(PG_PORT), "-U", "pinned", "-d", "pinned", "-qAtc", sql], { encoding: "utf8" }).trim();

async function signIn(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await page.click("#login .primary");
  }
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  return waitFor("the sweep node", () => page.evaluate(() => [...machines.entries()].find(([, m]) => m.view.online && m.view.name === "sweep-node")?.[0]));
}

/** Viewport shots down the section, from its heading: `<name>-1.png`, `-2.png`, … */
async function shots(page, name) {
  await page.evaluate(() => document.querySelector(".delegation-head").scrollIntoView({ block: "start" }));
  for (let i = 1; i <= 4; i++) {
    await page.waitForTimeout(150);
    await page.screenshot({ path: path.join(SCREENS, `${name}-${i}.png`) });
    const more = await page.evaluate(() => {
      const section = document.querySelector(".taste-section.delegation").getBoundingClientRect();
      if (section.bottom <= window.innerHeight) return false;
      document.querySelector(".delegation-head").closest(".page-body")?.scrollBy(0, window.innerHeight - 160);
      return true;
    });
    if (!more) break;
  }
}

const delegator = (kind, project) => ({ kind, tab: "tab-1", ...(project ? { project: `github.com/example/${project}`, project_name: project } : {}) });

/** One finished task, `ago` ms before now, that ran `minutes`. */
function record(n, { model, provider = "claude", account = "claude", by, outcome, minutes, cost, ago = n * 3 * 60 * MIN, limit, review, box, thread, tokens = 400_000 }) {
  const ended = Date.now() - ago;
  return {
    id: `task-seed${n}`,
    title: [`Fix the login redirect`, `Profile the sync loop`, `Port the parser to Rust`, `Sweep the docs for typos`, `Add a --json flag`, `Compare screens with the boards`, `Split the settings page`, `Retry flaky upload test`][n % 8],
    delegator: by,
    provider,
    account,
    model,
    started: ended - minutes * MIN,
    ended,
    usage: { tokens_in: tokens, tokens_out: Math.round(tokens / 40), wall_ms: minutes * MIN, ...(cost != null ? { cost_usd: cost } : {}), ...(box ? { box_usd: box * 0.05 / 60 * 60, box_ms: box * MIN } : {}) },
    outcome,
    ...(outcome === "failed" ? { reason: "tests fail after the change" } : {}),
    ...(outcome === "cancelled" ? { reason: "the parent asked" } : {}),
    ...(limit ? { limit, reason: `pinned stopped it at its ${limit} limit` } : {}),
    ...(review ? { review: { verdict: review, auto: review === "redone", at: ended } } : {}),
    ...(box ? { target: "box" } : {}),
    ...(thread ? { thread } : {}),
  };
}

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  const desktop = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
  const page = await desktop.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  const node = await signIn(page);
  await page.evaluate(() => setMode("gui"));
  const alpha = require("fs").realpathSync(path.join(DIR, "home", "work", "alpha"));
  const tab = await page.evaluate(async ({ node, dir }) => {
    await openRemote(node, dir, false);
    return tabs.find((t) => t.machine === node && t.view.cwd === dir).view.id;
  }, { node, dir: alpha });

  const opus = "claude-opus-5-5", sonnet = "claude-sonnet-5-5", glm = "glm-5.3", grok = "grok-4.7", luna = "gpt-6-luna";
  const seeds = [
    record(0, { model: opus, by: delegator("captain", "alpha"), outcome: "done", minutes: 42, cost: 3.1, review: "accepted", thread: tab }),
    record(1, { model: opus, by: delegator("captain", "alpha"), outcome: "limit", limit: "time", minutes: 120, cost: 7.8 }),
    record(2, { model: opus, by: delegator("global_orchestrator"), outcome: "limit", limit: "time", minutes: 120, cost: 6.4 }),
    record(3, { model: opus, by: delegator("captain", "beta"), outcome: "limit", limit: "budget", minutes: 64, cost: 5.0 }),
    record(4, { model: opus, by: delegator("captain", "beta"), outcome: "done", minutes: 25, cost: 1.9, review: "accepted" }),
    record(5, { model: sonnet, by: delegator("worker", "alpha"), outcome: "done", minutes: 18, cost: 0.62, box: 26 }),
    record(6, { model: sonnet, by: delegator("captain", "gamma"), outcome: "failed", minutes: 9, cost: 0.31, review: "redone" }),
    record(7, { model: glm, account: "personal", by: delegator("captain", "alpha"), outcome: "done", minutes: 14, cost: 0.08 }),
    record(8, { model: glm, account: "personal", by: delegator("captain", "beta"), outcome: "failed", minutes: 6, cost: 0.04 }),
    record(9, { model: glm, account: "personal", by: delegator("captain", "beta"), outcome: "failed", minutes: 7, cost: 0.05, review: "redone" }),
    record(10, { model: grok, provider: "cursor", account: "cursor", by: delegator("session", "gamma"), outcome: "done", minutes: 11, tokens: 120_000 }),
    record(11, { model: grok, provider: "cursor", account: "cursor", by: delegator("captain", "gamma"), outcome: "cancelled", minutes: 4, tokens: 30_000 }),
    record(12, { model: luna, account: "personal", by: delegator("global_orchestrator"), outcome: "done", minutes: 21, cost: 0.44 }),
    record(13, { model: opus, by: delegator("captain", "alpha"), outcome: "done", minutes: 50, cost: 4.2, ago: 12 * DAY }),
    record(14, { model: glm, account: "personal", by: delegator("captain", "beta"), outcome: "done", minutes: 12, cost: 0.07, ago: 20 * DAY }),
  ];
  const account = psql(`select id from accounts where email = '${EMAIL}'`);
  const machine = psql(`select id from machines where account = '${account}' and name = 'sweep-node'`);
  for (const r of seeds) {
    const value = JSON.stringify({ ...r, machine, machine_name: "sweep-node" }).replace(/'/g, "''");
    psql(`insert into delegation_tasks (account, machine, id, ended, record) values ('${account}', '${machine}', '${r.id}', ${r.ended}, '${value}') on conflict (account, machine, id) do update set ended = excluded.ended, record = excluded.record`);
  }
  check(Number(psql(`select count(*) from delegation_tasks where account = '${account}'`)) === seeds.length, `${seeds.length} records seeded`);

  await page.click('#sidebar footer [data-open-page="taste"]');
  await waitFor("the metrics", () => page.isVisible(".delegation-tiles"));
  const tiles = await page.$$eval(".delegation-tiles > div", (els) => els.map((el) => `${el.querySelector("dt").textContent}: ${el.querySelector("dd").textContent}`));
  console.log(tiles.join("\n"));
  check(tiles[0] === "Tasks: 13", "7 days hold the 13 recent tasks, not the two older ones");
  const hints = await page.$$eval(".delegation-hints li", (els) => els.map((el) => el.textContent));
  console.log(hints.join("\n"));
  check(hints.includes("3 of 6 claude-opus-5-5 tasks hit the time limit → Time limit") || hints.some((h) => /claude-opus-5-5 tasks hit the time limit → Time limit/.test(h)), "the Opus row names the time limit it would tune");
  await shots(page, "delegation-desktop-dark");

  await page.click('[data-act="delegation-days"][data-days="30"]');
  await waitFor("30 days", () => page.evaluate(() => document.querySelector(".delegation-tiles dd")?.textContent === "15"));
  check(true, "30 days hold all 15");
  await page.click('[data-act="delegation-days"][data-days="1"]');
  await waitFor("24 hours", () => page.evaluate(() => Number(document.querySelector(".delegation-tiles dd")?.textContent) < 13));
  check(true, "24 hours hold fewer");
  await page.click('[data-act="delegation-days"][data-days="7"]');
  await waitFor("7 days again", () => page.evaluate(() => document.querySelector(".delegation-tiles dd")?.textContent === "13"));

  await page.emulateMedia({ colorScheme: "light" });
  await shots(page, "delegation-desktop-light");

  await page.click(`.delegation-link[data-tab="${tab}"]`);
  await waitFor("the thread", () => page.evaluate(() => document.body.dataset.page !== "taste"), 5000).catch(() => {});
  check(await page.evaluate(() => document.body.dataset.page !== "taste"), "the slowest list's link leaves the page for the thread");

  for (const scheme of ["dark", "light"]) {
    const phone = await browser.newContext({ viewport: { width: 390, height: 844 }, colorScheme: scheme, isMobile: true, hasTouch: true });
    const p = await phone.newPage();
    p.on("pageerror", (e) => errors.push(String(e)));
    await signIn(p);
    await p.evaluate(() => setMode("gui"));
    await p.evaluate(() => openPage("taste"));
    await waitFor("the metrics on the phone", () => p.isVisible(".delegation-tiles"));
    await shots(p, `delegation-phone-${scheme}`);
    const wide = await p.evaluate(() => document.scrollingElement.scrollWidth > window.innerWidth + 1 || [...document.querySelectorAll(".delegation *")].some((el) => el.getBoundingClientRect().right > window.innerWidth + 1));
    check(!wide, `no horizontal overflow on the phone (${scheme})`);
    await phone.close();
  }
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join("; ")}` : ""}`);
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
