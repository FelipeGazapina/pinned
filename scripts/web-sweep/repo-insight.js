#!/usr/bin/env node
// Settings > Account's "Orchestrator repo access" switch: off by default, then on, saved on the hub.
// Also screenshots the static prototype ($PROTOTYPE, built from the app's own CSS served by the hub).
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... CHROME_BIN=... PROTOTYPE=.../prototype.html node scripts/web-sweep/repo-insight.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes repo-insight-*.png into scripts/web-sweep/screens/.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const SCREENS = path.join(__dirname, "screens");
const SIZES = { desktop: { width: 1440, height: 900 }, phone: { width: 390, height: 844 } };
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

async function accountPage(page) {
  await page.evaluate(async () => {
    await toggleSettings();
    showPage("account");
  });
  await page.waitForSelector("#repo-insight", { state: "attached", timeout: 15000 });
  await page.evaluate(() => document.querySelector(".card.repo-insight").scrollIntoView({ block: "center" }));
}

(async () => {
  fs.mkdirSync(SCREENS, { recursive: true });
  const browser = await playwright.chromium.launch({ headless: true, executablePath: process.env.CHROME_BIN || undefined });
  const errors = [];
  for (const [size, viewport] of Object.entries(SIZES)) {
    if (process.env.PROTOTYPE) {
      const proto = await (await browser.newContext({ viewport, colorScheme: "dark" })).newPage();
      await proto.goto(`file://${process.env.PROTOTYPE}`);
      await proto.waitForTimeout(500);
      await proto.screenshot({ path: path.join(SCREENS, `repo-insight-prototype-${size}.png`) });
    }
  }
  const context = await browser.newContext({ viewport: SIZES.desktop, colorScheme: "dark" });
  const page = await context.newPage();
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(HUB);
  await page.fill("#login [name=email]", "sweep@example.test");
  await page.fill("#login [name=password]", "sweep password 1");
  await page.click("#login .primary");
  await waitFor("online", () => page.evaluate(() => document.body.dataset.hub === "online"));
  await accountPage(page);
  check(!(await page.isChecked("#repo-insight")), "repo access starts off");
  await page.screenshot({ path: path.join(SCREENS, "repo-insight-off-desktop.png") });
  await page.click(".card.repo-insight .switch");
  if (await page.waitForSelector("dialog[open] form [name=password]", { timeout: 3000 }).catch(() => null)) {
    await page.fill("dialog[open] form [name=password]", "sweep password 1");
    await page.click("dialog[open] form button.primary");
  }
  await waitFor("the hub to keep it on", () => page.evaluate(async () => (await hub.hub({ cmd: "get_settings" })).repo_insight?.on === true));
  await accountPage(page);
  check(await page.isChecked("#repo-insight"), "the switch shows on after a redraw");
  await page.screenshot({ path: path.join(SCREENS, "repo-insight-on-desktop.png") });
  await page.setViewportSize(SIZES.phone);
  await accountPage(page);
  await page.screenshot({ path: path.join(SCREENS, "repo-insight-on-phone.png") });
  await page.click(".card.repo-insight .switch");
  await waitFor("the hub to keep it off", () => page.evaluate(async () => !(await hub.hub({ cmd: "get_settings" })).repo_insight));
  await accountPage(page);
  await page.screenshot({ path: path.join(SCREENS, "repo-insight-off-phone.png") });
  check(errors.length === 0, `page errors: ${errors.join("; ")}`);
  await browser.close();
  console.log("repo-insight: ok");
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
