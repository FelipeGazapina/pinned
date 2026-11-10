#!/usr/bin/env node
// Settings › Archive's Auto-archive section (issue #158): it shows the machine's defaults (worker
// threads after 30 minutes, threads you open never), a change reaches the node and survives a
// reload, and a value outside the choices is listed as it is. Shoots 1440x900 and 390x844.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/auto-archive.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes auto-archive-*.png into $SWEEP_DIR/screens. SWEEP_CHANNEL=chrome drives the installed
// Google Chrome when Playwright's own browser is missing.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const SCREENS = path.join(DIR, "screens");
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

const values = (page) => page.evaluate(() => ({ workers: document.getElementById("aa-workers")?.value, sessions: document.getElementById("aa-sessions")?.value }));
const node = (page) => page.evaluate(async () => {
  const id = [...machines.entries()].find(([, m]) => m.view.name === "sweep-node")[0];
  return callOn(id, { cmd: "auto_archive_settings" });
});

(async () => {
  fs.mkdirSync(SCREENS, { recursive: true });
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  let failed = 0;
  const check = (ok, what) => {
    if (!ok) failed++;
    console.log(`${ok ? "ok" : "FAIL"}: ${what}`);
  };
  for (const [device, viewport] of VIEWPORTS) {
    const context = await browser.newContext({ viewport, colorScheme: "dark" });
    const page = await context.newPage();
    await signIn(page);
    await page.evaluate(() => setMode("gui"));
    await page.evaluate(() => openSettings("archive"));
    await waitFor("the section", () => page.isVisible("#auto-archive-settings"));
    await page.waitForTimeout(800);
    check(JSON.stringify(await values(page)) === JSON.stringify({ workers: "30", sessions: "never" }), `${device}: the defaults are workers 30 minutes, threads you open never`);
    await page.screenshot({ path: path.join(SCREENS, `auto-archive-${device}-default.png`) });

    await page.selectOption("#aa-workers", "120");
    await page.selectOption("#aa-sessions", "1440");
    await waitFor("the node to hold it", async () => JSON.stringify(await node(page)) === JSON.stringify({ workers: 120, sessions: 1440 }));
    check(true, `${device}: a change reaches the node: ${JSON.stringify(await node(page))}`);
    check((await page.textContent("#aa-error")) === "", `${device}: no error under the section`);
    await page.screenshot({ path: path.join(SCREENS, `auto-archive-${device}-changed.png`) });

    await page.reload();
    await waitFor("the hub again", () => page.evaluate(() => document.body.dataset.hub === "online"));
    await waitFor("the sweep node again", () => page.evaluate(() => [...machines.values()].some((m) => m.view.online && m.view.name === "sweep-node")));
    await page.evaluate(() => setMode("gui"));
    await page.evaluate(() => openSettings("archive"));
    await waitFor("the saved values", async () => JSON.stringify(await values(page)) === JSON.stringify({ workers: "120", sessions: "1440" }));
    check(true, `${device}: after a reload the page shows what the node holds`);

    await page.selectOption("#aa-workers", "never");
    await page.selectOption("#aa-sessions", "never");
    await waitFor("never on the node", async () => JSON.stringify(await node(page)) === JSON.stringify({ workers: null, sessions: null }));
    check(true, `${device}: never is saved as null`);
    await page.screenshot({ path: path.join(SCREENS, `auto-archive-${device}-never.png`) });
    await page.selectOption("#aa-workers", "30");
    await page.selectOption("#aa-sessions", "never");
    await waitFor("the defaults back", async () => JSON.stringify(await node(page)) === JSON.stringify({ workers: 30, sessions: null }));
    await context.close();
  }
  await browser.close();
  process.exit(failed ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
