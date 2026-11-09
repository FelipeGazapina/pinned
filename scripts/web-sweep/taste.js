#!/usr/bin/env node
// Drives the Way of working page on the real page: every scope, a project's override saved to the
// hub, a reload, and the Capitão's launch prompt carrying the new brief.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/taste.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes taste-*.png into scripts/web-sweep/screens/, desktop at 1440x900 and phone at 390x844.
// SWEEP_CHANNEL=chrome drives the installed Google Chrome when Playwright's own browser is missing.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const WORK = path.join(DIR, "home", "work");
const LAUNCHES = path.join(DIR, "home", "claude-launch.log");
const SCREENS = path.join(__dirname, "screens");
const ALPHA = "github.com/example/alpha";

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
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await page.click("#login .primary");
  }
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  return waitFor("the sweep node", () => page.evaluate(() => [...machines.entries()].find(([, m]) => m.view.online && m.view.name === "sweep-node")?.[0]));
}

const brief = (page) => page.evaluate(() => [...document.querySelectorAll(".taste-line")].map((l) => l.textContent).join("\n"));
const scope = (page, label) => page.click(`.taste-scopes button:text-is("${label}")`);
const shot = (page, name) => page.screenshot({ path: path.join(SCREENS, `${name}.png`) });

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  const desktop = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
  const page = await desktop.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  const node = await signIn(page);
  await page.evaluate(() => setMode("gui"));
  const folders = ["alpha", "beta"].map((n) => fs.realpathSync(path.join(WORK, n)));
  await page.evaluate(async ({ node, dirs }) => {
    for (const d of dirs) if (!tabs.some((t) => t.machine === node && t.view.cwd === d)) await openRemote(node, d, false);
  }, { node, dirs: folders });

  await page.evaluate(() => toggleSettings());
  await waitFor("the General page", () => page.isVisible("#orchestration-settings"));
  check((await page.textContent("#orchestration-settings h2")) === "Captain and workers" && !(await page.isVisible("#orchestration-style")), "settings hold no cost style any more");
  await page.locator("#orchestration-settings").screenshot({ path: path.join(SCREENS, "taste-settings-entry.png") });
  await page.click("#orchestration-taste");
  await waitFor("the page from settings", () => page.isVisible(".taste-split"));
  await page.evaluate(() => closePage());
  await page.hover('#sidebar footer [data-open-page="taste"]');
  await waitFor("the button's tooltip", () => page.evaluate(() => [...document.querySelectorAll("body *")].some((el) => el.textContent === "Way of working" && el.offsetParent && !el.closest("#sidebar"))));
  await page.screenshot({ path: path.join(SCREENS, "taste-sidebar-entry.png"), clip: { x: 0, y: 740, width: 420, height: 160 } });
  await page.click('#sidebar footer [data-open-page="taste"]');
  await waitFor("the page", () => page.isVisible(".taste-split"));
  await waitFor("the project tabs", () => page.isVisible('.taste-scopes button:text-is("Captain alpha")'));
  check((await page.textContent("#page-view .page-head h1")) === "Way of working", "the footer button opens Way of working");
  const defaults = await brief(page);
  check(defaults.startsWith("Run up to 3 workers at once.\nStart each worker on glm-5.3"), "Captains (all) previews the balanced brief");
  await shot(page, "taste-captains");

  await scope(page, "Orchestrator");
  check((await page.textContent(".taste-preview h2")) === "What the orchestrator receives", "the Orchestrator tab previews the orchestrator's brief");
  await shot(page, "taste-orchestrator");

  await scope(page, "Captain alpha");
  check((await page.textContent(".taste-scopes .taste-quiet")) === "Inherits from Captains (all).", "a project tab says it inherits");
  check((await page.locator(".taste-tag:text-is('inherited')").count()) >= 7, "every field of a fresh project is inherited");
  await page.click('button[aria-label="More workers at once"]');
  await page.click('.taste-switch[aria-label="Second opinion on risky changes"]');
  await page.fill("[data-note-input]", "Deploys go through the VPS.");
  await page.click(".taste-note-form .primary");
  await page.selectOption("[data-add-model]", "kimi-k3.1-pro");
  await waitFor("the unsaved footer", () => page.isVisible(".taste-unsaved"));
  check((await page.locator(".taste-tag.changed").count()) === 4, "four fields of alpha read changed here");
  check((await page.textContent(".taste-card.custom.active b")) === "Custom", "an edit makes the profile Custom");
  const edited = await brief(page);
  await shot(page, "taste-project-edited");

  await page.click('.taste-foot [data-act="save"]');
  await waitFor("the saved toast", () => page.isVisible(".taste-toast"));
  check(!(await page.isVisible(".taste-foot")), "the footer goes once saved");
  await shot(page, "taste-project-saved");
  const stored = await page.evaluate(() => hub.hub({ cmd: "get_tastes" }));
  const expected = { parallel: 4, second_opinion: true, ladder: [{ model: "glm-5.3" }, { model: "claude-sonnet-5-5" }, { model: "claude-opus-5-5" }, { model: "kimi-k3.1-pro" }], context: ["Deploys go through the VPS."] };
  check(JSON.stringify(stored.projects[ALPHA], Object.keys(expected).sort()) === JSON.stringify(expected, Object.keys(expected).sort()), `the hub stores alpha's patch: ${JSON.stringify(stored.projects[ALPHA])}`);

  await page.reload();
  await signIn(page);
  await page.evaluate(() => openPage("taste"));
  await waitFor("the page after reload", () => page.isVisible(".taste-split"));
  await waitFor("alpha's tab after reload", () => page.isVisible('.taste-scopes button:text-is("Captain alpha")'));
  await scope(page, "Captain alpha");
  const reloaded = await brief(page);
  check(reloaded === edited, "after a reload alpha previews the saved brief");
  check(reloaded !== defaults && reloaded.includes("Run up to 4 workers at once.") && reloaded.includes("then kimi-k3.1-pro.") && reloaded.endsWith("Deploys go through the VPS."), "the saved brief differs from the Captains'");
  await shot(page, "taste-after-reload");

  fs.rmSync(LAUNCHES, { force: true });
  const alphaDir = folders[0];
  await page.evaluate(async ({ node, dir }) => {
    const tab = await callOn(node, { cmd: "open_orchestrator", path: dir });
    await callOn(node, { cmd: "start_claude", tab, cols: 80, rows: 24 });
  }, { node, dir: alphaDir });
  const launched = await waitFor("the Capitão's launch", () => fs.existsSync(LAUNCHES) && fs.readFileSync(LAUNCHES, "utf8").includes("--append-system-prompt") && fs.readFileSync(LAUNCHES, "utf8"));
  check(launched.includes(reloaded), "alpha's Capitão starts with exactly the brief the page shows");
  fs.writeFileSync(path.join(DIR, "captain-prompt.txt"), launched);

  const phone = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true, colorScheme: "dark" });
  const small = await phone.newPage();
  small.on("pageerror", (e) => errors.push(String(e)));
  await signIn(small);
  await small.evaluate(() => {
    setMode("gui");
    openPage("taste");
  });
  await waitFor("the phone page", () => small.isVisible(".taste-split"));
  await waitFor("the phone's project tabs", () => small.isVisible('.taste-scopes button:text-is("Captain alpha")'));
  const overflow = await small.evaluate(() => document.querySelector("#page-view .page-body").scrollWidth - document.querySelector("#page-view .page-body").clientWidth);
  check(overflow <= 0, `nothing overflows sideways on the phone (${overflow}px)`);
  await shot(small, "taste-phone");
  await small.click('.taste-scopes button:text-is("Captain alpha")');
  await small.click('button[aria-label="Less workers at once"]');
  await waitFor("the phone's unsaved footer", () => small.isVisible(".taste-unsaved"));
  await small.evaluate(() => document.querySelector("#page-view .page-body").scrollTo(0, 1e6));
  await shot(small, "taste-phone-edited");
  await small.click('.taste-foot [data-act="discard"]');

  check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join("; ")}` : ""}`);
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
