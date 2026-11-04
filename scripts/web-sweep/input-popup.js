#!/usr/bin/env node
// Drives the input popup through the web app: a question opens when its thread is selected, stays
// readable while another thread is visited, and still takes a typed answer from the composer.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/input-popup.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";

const check = (ok, what) => {
  if (!ok) throw new Error(what);
};
const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};

(async () => {
  const browser = await playwright.chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));

  await page.goto(HUB);
  await page.fill("#login [name=email]", EMAIL);
  await page.fill("#login [name=password]", PASSWORD);
  await page.click("#login .primary");
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  const machine = await page.evaluate(() => [...machines.keys()].find((k) => machines.get(k).view.online && machines.get(k).view.name === "sweep-node"));
  check(!!machine, "the sweep node is not online");
  await page.evaluate(() => setMode("gui"));

  const alpha = path.join(DIR, "home/work/alpha");
  const beta = path.join(DIR, "home/work/beta");
  const alphaKey = await page.evaluate(async ([m, p]) => (await openTab(m, p)).key, [machine, alpha]);
  await waitFor("alpha to run", () => page.evaluate((k) => tabByKey(k)?.view.running === true, alphaKey));
  const send = async (text) => {
    await page.fill("#prompt", text);
    // Enter in the `/` menu picks its highlighted command, so close that menu first.
    if (/^\//.test(text)) await waitFor("the / menu", () => page.locator('#composer-menu[data-kind="slash"]').isVisible());
    if (await page.locator('#composer-menu:is([data-kind="slash"], [data-kind="mention"])').isVisible()) await page.press("#prompt", "Escape");
    await page.press("#prompt", "Enter");
  };

  await send("/question");
  await page.waitForSelector("#approval:not([hidden])");
  check((await page.textContent("#approval .q-body .what")).includes("Which color do you prefer?"), "the question text is missing");
  const placement = await page.evaluate(() => ({
    body: document.getElementById("approval").parentElement === document.body,
    position: getComputedStyle(document.getElementById("approval")).position,
    approval: document.getElementById("approval").getBoundingClientRect().bottom,
    composer: document.getElementById("composer").getBoundingClientRect().top,
  }));
  check(placement.body, "the popup is not mounted outside the composer");
  check(placement.position === "fixed", `the popup is ${placement.position}, not fixed`);
  check(placement.approval <= placement.composer - 4, `the popup ends at ${placement.approval}, composer starts at ${placement.composer}`);

  const betaKey = await page.evaluate(async ([m, p]) => (await openTab(m, p)).key, [machine, beta]);
  await waitFor("beta to run", () => page.evaluate((k) => tabByKey(k)?.view.running === true, betaKey));
  check(await page.locator("#approval").isHidden(), "the popup stayed over another thread");

  await page.evaluate((k) => activate(tabByKey(k)), alphaKey);
  await page.waitForSelector("#approval:not([hidden])");
  check((await page.textContent("#approval .q-body .what")).includes("Which color do you prefer?"), "the question did not come back with its thread");
  const labels = await page.locator("#approval .choice .label").allTextContents();
  check(labels.join("|") === "Red|Blue|Type something.|Chat about this", `the options are ${labels.join("|")}`);

  await send("Green, please");
  await waitFor("the typed answer", () => page.evaluate((k) => screenLines(tabByKey(k)).join("\n").includes("answered: Green, please"), alphaKey));
  await waitFor("the popup to close", () => page.locator("#approval").isHidden());

  await send("/ask");
  await page.waitForSelector("#approval:not([hidden])");
  check((await page.textContent("#approval .kind")) === "Command approval", "the permission popup is missing");
  await page.click('#approval [data-approval="accept"]');
  await waitFor("the permission answer", () => page.evaluate((k) => screenLines(tabByKey(k)).join("\n").includes("chose 1"), alphaKey));
  await waitFor("the popup to close", () => page.locator("#approval").isHidden());

  // An unreadable question still opens the popup with the terminal's last screen instead of nothing.
  await page.evaluate((k) => {
    const tab = tabByKey(k);
    tab.view = { ...tab.view, status: "needs-input" };
    tab.menu = null;
    tab.missingInput = true;
    tab.inputFallback = "Which color do you prefer?\n❯ 1. Red\n  2. Blue";
    renderComposer(tab);
  }, alphaKey);
  await page.waitForSelector("#approval:not([hidden])");
  check((await page.textContent("#approval .kind")) === "Input needed", "the fallback popup is missing");
  check((await page.textContent("#approval pre")).includes("Which color do you prefer?"), "the fallback did not carry the terminal screen");
  await page.evaluate((k) => {
    const tab = tabByKey(k);
    tab.view = { ...tab.view, status: null };
    tab.missingInput = false;
    tab.inputFallback = "";
    renderComposer(tab);
  }, alphaKey);
  await waitFor("the fallback to close", () => page.locator("#approval").isHidden());

  await browser.close();
  check(errors.length === 0, `console errors: ${errors.slice(0, 3).join(" | ")}`);
  console.log(JSON.stringify({ popup: placement, options: labels.join(" | "), errors }, null, 2));
})().catch(async (error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
