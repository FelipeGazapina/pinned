#!/usr/bin/env node
// Drives the Delegation limits section of the Way of working page on the real page: the Captains'
// limits saved to the hub, a project's override, the Capitão's launch prompt naming them, and each
// view beside its boards, prototype/live/Taste.dc.html (workers at once) and Maquinas.dc.html
// (budget per worker, box budget), at 1440x900 and 390x844.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   BOARD_SUPPORT=<board-support.js> SWEEP_DIR=... node scripts/web-sweep/limits.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// The boards load a support.js runtime that is not in the repo; BOARD_SUPPORT stands in for it.
// Writes limits-*.png into scripts/web-sweep/screens/.
const fs = require("fs");
const os = require("os");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const BOARD_DIR = process.env.BOARD_DIR || path.join(__dirname, "../../.claude/research/global-orchestrator/prototype/live");
const BOARD_SUPPORT = process.env.BOARD_SUPPORT || path.join(__dirname, "board-support.js");
const WORK = path.join(DIR, "home", "work");
const LAUNCHES = path.join(DIR, "home", "claude-launch.log");
const SCREENS = path.join(__dirname, "screens");
const ALPHA = "github.com/example/alpha";
const SIZES = [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]];

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

async function signIn(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", "sweep@example.test");
    await page.fill("#login [name=password]", "sweep password 1");
    await page.click("#login .primary");
  }
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  return waitFor("the sweep node", () => page.evaluate(() => [...machines.entries()].find(([, m]) => m.view.online && m.view.name === "sweep-node")?.[0]));
}

const scope = (page, label) => page.click(`.taste-scopes button:text-is("${label}")`);
const toLimits = (page) => page.evaluate(() => [...document.querySelectorAll(".taste-section h2")].find((el) => el.textContent === "Delegation limits").scrollIntoView({ block: "start" }));
const output = (page, label) => page.evaluate((label) => [...document.querySelectorAll(".taste-limits .taste-field")].find((f) => f.querySelector(".taste-label span")?.textContent === label)?.querySelector("output")?.textContent, label);

async function appShots(browser, out, node) {
  const shots = {};
  for (const [size, viewport] of SIZES) {
    const phone = size === "phone";
    const context = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
    const page = await context.newPage();
    await signIn(page);
    await page.evaluate(() => {
      setMode("gui");
      openPage("taste");
    });
    await waitFor("the page", () => page.isVisible(".taste-split"));
    await waitFor("alpha's tab", () => page.isVisible('.taste-scopes button:text-is("Captain alpha")'));
    for (const [name, tab] of [["captains", "Captains (all)"], ["project", "Captain alpha"], ["orchestrator", "Orchestrator"]]) {
      await scope(page, tab);
      await toLimits(page);
      await page.waitForTimeout(400);
      shots[`${name}-${size}`] = path.join(SCREENS, `limits-${name}-${size}.png`);
      await page.screenshot({ path: shots[`${name}-${size}`] });
    }
    if (phone) {
      const overflow = await page.evaluate(() => document.querySelector("#page-view .page-body").scrollWidth - document.querySelector("#page-view .page-body").clientWidth);
      check(overflow <= 0, `nothing overflows sideways on the phone (${overflow}px)`);
    }
    await context.close();
  }
  return shots;
}

/** The board scrolled so the text `anchor` sits at the top, at each size. */
async function boardShots(browser, out, file, anchor, click) {
  const board = fs.mkdtempSync(path.join(out, "board-"));
  fs.copyFileSync(path.join(BOARD_DIR, file), path.join(board, file));
  fs.copyFileSync(BOARD_SUPPORT, path.join(board, "support.js"));
  const shots = {};
  for (const [size, viewport] of SIZES) {
    const context = await browser.newContext({ viewport, deviceScaleFactor: size === "phone" ? 2 : 1, colorScheme: "dark" });
    const page = await context.newPage();
    await page.goto(`file://${path.join(board, file)}`);
    await waitFor(`${file} to draw`, () => page.evaluate((anchor) => [...document.querySelectorAll("span, h2, button")].some((el) => el.textContent.trim() === anchor), anchor));
    if (click) await page.click(`button:text-is("${click}")`);
    await page.evaluate((anchor) => [...document.querySelectorAll("span, h2, button")].find((el) => el.textContent.trim() === anchor).scrollIntoView({ block: "start" }), anchor);
    await page.waitForTimeout(300);
    shots[size] = path.join(out, `${path.basename(file, ".dc.html")}-${size}.png`);
    await page.screenshot({ path: shots[size] });
    await context.close();
  }
  return shots;
}

async function pair(browser, board, caption, app, out, width) {
  const page = await browser.newPage({ deviceScaleFactor: 1, viewport: { width: width * 2 + 48, height: 400 } });
  const img = (file) => `data:image/png;base64,${fs.readFileSync(file).toString("base64")}`;
  const figure = (title, file) => `<figure style="margin:0;width:${width}px"><figcaption style="padding:0 0 8px">${title}</figcaption><img style="width:${width}px;display:block" src="${img(file)}"></figure>`;
  await page.setContent(`<body style="margin:0;background:#222;font:600 20px system-ui;color:#eee">
    <div style="display:flex;gap:16px;padding:16px;align-items:flex-start">${figure(caption, board)}${figure("App (web sweep)", app)}</div></body>`);
  await page.waitForFunction(() => [...document.images].every((i) => i.complete));
  await page.screenshot({ path: out, fullPage: true });
  await page.close();
}

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  const page = await (await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" })).newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  const node = await signIn(page);
  await page.evaluate(() => setMode("gui"));
  const alpha = fs.realpathSync(path.join(WORK, "alpha"));
  await page.evaluate(async ({ node, dir }) => {
    if (!tabs.some((t) => t.machine === node && t.view.cwd === dir)) await openRemote(node, dir, false);
  }, { node, dir: alpha });
  await page.evaluate(() => openPage("taste"));
  await waitFor("the page", () => page.isVisible(".taste-split"));
  await waitFor("alpha's tab", () => page.isVisible('.taste-scopes button:text-is("Captain alpha")'));

  await toLimits(page);
  check((await output(page, "Time limit per worker")) === "8 h" && (await output(page, "Budget per worker")) === "No cap", "the Captains start with 8 h and no budget cap");
  await page.click('.taste-limits button[data-field="tier"][data-value="mid"]');
  await page.click('button[aria-label="Less time per worker"]');
  await page.click('button[aria-label="More budget per worker"]');
  await page.click('button[aria-label="More budget per worker"]');
  await page.click('button[aria-label="More budget per worker"]');
  await page.click('button[aria-label="More budget per box"]');
  await page.selectOption('[data-effort="default_effort"]', "medium");
  check((await output(page, "Time limit per worker")) === "6 h" && (await output(page, "Budget per worker")) === "US$ 1.50", "the steppers show 6 h and US$ 1.50");
  await page.click('.taste-foot [data-act="save"]');
  await waitFor("the saved toast", () => page.isVisible(".taste-toast"));
  let stored = await page.evaluate(() => hub.hub({ cmd: "get_tastes" }));
  const { tier, default_effort, max_effort, budget_cents, time_limit_min, box_budget_cents } = stored.captains;
  check(JSON.stringify({ tier, default_effort, max_effort, budget_cents, time_limit_min, box_budget_cents }) === JSON.stringify({ tier: "mid", default_effort: "medium", max_effort: "max", budget_cents: 150, time_limit_min: 360, box_budget_cents: 50 }), `the hub stores the Captains' limits: ${JSON.stringify(stored.captains)}`);

  await scope(page, "Captain alpha");
  await toLimits(page);
  await page.click('.taste-limits button[data-field="tier"][data-value="low"]');
  await page.click('.taste-foot [data-act="save"]');
  await waitFor("the saved toast", () => page.isVisible(".taste-toast"));
  stored = await page.evaluate(() => hub.hub({ cmd: "get_tastes" }));
  check(stored.projects[ALPHA]?.tier === "low", `the hub stores alpha's own tier: ${JSON.stringify(stored.projects[ALPHA])}`);
  check((await page.locator(".taste-limits .taste-tag.changed").count()) === 1, "only alpha's tier reads changed here");

  fs.rmSync(LAUNCHES, { force: true });
  await page.evaluate(async ({ node, dir }) => {
    const tab = await callOn(node, { cmd: "open_orchestrator", path: dir });
    await callOn(node, { cmd: "start_claude", tab, cols: 80, rows: 24 });
  }, { node, dir: alpha });
  const launched = await waitFor("the Capitão's launch", () => fs.existsSync(LAUNCHES) && fs.readFileSync(LAUNCHES, "utf8").includes("delegation limits") && fs.readFileSync(LAUNCHES, "utf8"));
  check(launched.includes("models up to the low tier, effort up to max (medium when a call names none), 360 minutes per worker, US$ 1.50 per worker, US$ 0.50 per box."), "alpha's Capitão is told alpha's tier over the Captains' other limits");

  const out = fs.mkdtempSync(path.join(os.tmpdir(), "limits-board-"));
  const app = await appShots(browser, out, node);
  const taste = await boardShots(browser, out, "Taste.dc.html", "Ritmo", "Capitães (todos)");
  const maquinas = await boardShots(browser, out, "Maquinas.dc.html", "Upstash Box");
  for (const [size] of SIZES) {
    const width = size === "phone" ? 390 : 1440;
    await pair(browser, taste[size], "Board (Taste.dc.html, Ritmo)", app[`captains-${size}`], path.join(SCREENS, `limits-vs-taste-${size}.png`), width);
    await pair(browser, maquinas[size], "Board (Maquinas.dc.html, Upstash Box)", app[`captains-${size}`], path.join(SCREENS, `limits-vs-maquinas-${size}.png`), width);
  }
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join("; ")}` : ""}`);
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
