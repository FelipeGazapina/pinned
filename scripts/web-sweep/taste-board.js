#!/usr/bin/env node
// Puts the Way of working page beside its board, prototype/live/Taste.dc.html: each scope tab on
// the board and on the real page, at 1440x900 and 390x844, one board-left, app-right image a pair.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/taste.js        # leaves alpha with a saved patch
//   BOARD_DIR=<folder holding Taste.dc.html> SWEEP_DIR=... node scripts/web-sweep/taste-board.js
//
// The boards load a support.js runtime that is not in the repo; board-support.js stands in for it.
// Writes taste-vs-board-{orchestrator,captains,captains-lower,project}-{desktop,phone}.png into
// screens/; captains-lower starts at the Pace section.
const fs = require("fs");
const os = require("os");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const BOARD_DIR = process.env.BOARD_DIR || path.join(__dirname, "../../.claude/research/global-orchestrator/prototype/live");
const SCREENS = path.join(__dirname, "screens");
const SIZES = [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]];
const TABS = [["orchestrator", "Orquestrador", "Orchestrator"], ["captains", "Capitães (todos)", "Captains (all)"], ["project", "Capitão pinned", "Captain alpha"]];

const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};

async function boardShots(browser, out) {
  const board = fs.mkdtempSync(path.join(out, "board-"));
  fs.copyFileSync(path.join(BOARD_DIR, "Taste.dc.html"), path.join(board, "Taste.dc.html"));
  fs.copyFileSync(path.join(__dirname, "board-support.js"), path.join(board, "support.js"));
  const shots = {};
  for (const [size, viewport] of SIZES) {
    const context = await browser.newContext({ viewport, deviceScaleFactor: size === "phone" ? 2 : 1, colorScheme: "dark" });
    const page = await context.newPage();
    await page.goto(`file://${path.join(board, "Taste.dc.html")}`);
    await page.waitForSelector("main h1");
    for (const [name, tab] of TABS) {
      await page.click(`main button:text-is("${tab}")`);
      // At phone width the board stacks its own sidebar above main, so the shot starts at main.
      await page.evaluate((phone) => window.scrollTo(0, phone ? document.querySelector("main").offsetTop : 0), size === "phone");
      shots[`${name}-${size}`] = path.join(out, `board-${name}-${size}.png`);
      await page.screenshot({ path: shots[`${name}-${size}`] });
      if (name !== "captains") continue;
      await page.evaluate(() => [...document.querySelectorAll("main span")].find((el) => el.textContent === "Ritmo").scrollIntoView({ block: "start" }));
      shots[`captains-lower-${size}`] = path.join(out, `board-captains-lower-${size}.png`);
      await page.screenshot({ path: shots[`captains-lower-${size}`] });
    }
    await context.close();
  }
  return shots;
}

async function appShots(browser, out) {
  const shots = {};
  for (const [size, viewport] of SIZES) {
    const phone = size === "phone";
    const context = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
    const page = await context.newPage();
    await page.goto(HUB);
    if (await page.isVisible("#login [name=email]")) {
      await page.fill("#login [name=email]", "sweep@example.test");
      await page.fill("#login [name=password]", "sweep password 1");
      await page.click("#login .primary");
    }
    await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
    await page.evaluate(() => {
      setMode("gui");
      openPage("taste");
    });
    await waitFor("the page", () => page.isVisible(".taste-split"));
    await waitFor("alpha's tab", () => page.isVisible('.taste-scopes button:text-is("Captain alpha")'));
    for (const [name, , tab] of TABS) {
      await page.click(`.taste-scopes button:text-is("${tab}")`);
      await page.evaluate(() => document.querySelector("#page-view .page-body")?.scrollTo(0, 0));
      await page.waitForTimeout(1600);
      shots[`${name}-${size}`] = path.join(out, `app-${name}-${size}.png`);
      await page.screenshot({ path: shots[`${name}-${size}`] });
      if (name !== "captains") continue;
      await page.evaluate(() => [...document.querySelectorAll(".taste-section h2")].find((el) => el.textContent === "Pace").scrollIntoView({ block: "start" }));
      await page.waitForTimeout(300);
      shots[`captains-lower-${size}`] = path.join(out, `app-captains-lower-${size}.png`);
      await page.screenshot({ path: shots[`captains-lower-${size}`] });
    }
    await context.close();
  }
  return shots;
}

async function pair(browser, board, app, out, width) {
  const page = await browser.newPage({ deviceScaleFactor: 1, viewport: { width: width * 2 + 48, height: 400 } });
  const img = (file) => `data:image/png;base64,${fs.readFileSync(file).toString("base64")}`;
  const figure = (caption, file) => `<figure style="margin:0;width:${width}px"><figcaption style="padding:0 0 8px">${caption}</figcaption><img style="width:${width}px;display:block" src="${img(file)}"></figure>`;
  await page.setContent(`<body style="margin:0;background:#222;font:600 20px system-ui;color:#eee">
    <div style="display:flex;gap:16px;padding:16px;align-items:flex-start">${figure("Board (prototype/live/Taste.dc.html)", board)}${figure("App (web sweep)", app)}</div></body>`);
  await page.waitForFunction(() => [...document.images].every((i) => i.complete));
  await page.screenshot({ path: out, fullPage: true });
  await page.close();
}

(async () => {
  const out = fs.mkdtempSync(path.join(os.tmpdir(), "taste-board-"));
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  const board = await boardShots(browser, out);
  const app = await appShots(browser, out);
  for (const key of Object.keys(board)) {
    const file = path.join(SCREENS, `taste-vs-board-${key}.png`);
    await pair(browser, board[key], app[key], file, key.endsWith("phone") ? 390 : 1440);
    console.log(`wrote ${path.relative(process.cwd(), file)}`);
  }
  await browser.close();
  fs.rmSync(out, { recursive: true, force: true });
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
