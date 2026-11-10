#!/usr/bin/env node
// Opens Settings straight onto Archive and onto Machines, with General as the remembered page, and
// checks the page is still the one asked for 5 s later, at 1440x900 and at 390x844 (issue #115).
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/settings-page.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes settings-page-*.png into $SWEEP_DIR/screens. SWEEP_CHANNEL=chrome drives the installed
// Google Chrome when Playwright's own browser is missing.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const SCREENS = path.join(DIR, "screens");
const PAGES = [["archive", "Archive"], ["machines", "Machines"]];
const VIEWPORTS = [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]];

const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await fn()) return;
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
  await waitFor("the sweep node", () => page.evaluate(() => [...machines.values()].some((m) => m.view.online && m.view.name === "sweep-node")));
}

const shown = (page) =>
  page.evaluate(() => ({
    heading: settingsPage.querySelector("h2")?.textContent ?? null,
    nav: document.querySelector("#settings-nav .active")?.dataset.page ?? null,
  }));

(async () => {
  fs.mkdirSync(SCREENS, { recursive: true });
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  let failed = 0;
  for (const [device, viewport] of VIEWPORTS) {
    const context = await browser.newContext({ viewport, colorScheme: "dark" });
    const page = await context.newPage();
    await signIn(page);
    await page.evaluate(() => setMode("gui"));
    for (const [name, heading] of PAGES) {
      await page.evaluate(async () => {
        if (settingsOpen()) await toggleSettings();
        save("settings-page", "general");
        page = "general";
      });
      await page.evaluate((name) => openSettings(name), name);
      const first = await shown(page);
      await page.screenshot({ path: path.join(SCREENS, `settings-page-${device}-${name}-0s.png`) });
      await page.waitForTimeout(5000);
      const later = await shown(page);
      await page.screenshot({ path: path.join(SCREENS, `settings-page-${device}-${name}-5s.png`) });
      const ok = first.heading === heading && later.heading === heading && later.nav === name;
      if (!ok) failed++;
      console.log(`${ok ? "ok" : "FAIL"}: ${device} openSettings("${name}") shows ${first.heading} at once, ${later.heading} after 5 s, nav on ${later.nav}`);
    }
    await context.close();
  }
  await browser.close();
  process.exit(failed ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
