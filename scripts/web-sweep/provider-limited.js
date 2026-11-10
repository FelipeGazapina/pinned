#!/usr/bin/env node
// An OpenCode Go account whose weekly window is full shows as Limited on the Accounts page, with
// the window as its reason and the reset as the time it resumes. No session ever ran on it: the
// machine asks the Console itself. The stack's mock-opencode-go answers validation and usage, so
// start it with the weekly window full:
//
//   GO_MOCK_WEEKLY=100 GO_MOCK_WEEKLY_SECS=$((5*86400+11*3600)) SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... SWEEP_OUT=<folder> node scripts/web-sweep/provider-limited.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// It saves accounts-1440x900.png and accounts-390x844.png in SWEEP_OUT.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const OUT = process.env.SWEEP_OUT || path.join(process.env.TMPDIR || "/tmp", "provider-limited");
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
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await playwright.chromium.launch(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {});
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    const context = await browser.newContext({ viewport: { width, height }, isMobile: width < 500, hasTouch: width < 500 });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto(HUB);
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await page.click("#login .primary");
    await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
    await waitFor("the node", () => page.evaluate(() => [...machines.values()].some((m) => m.view.online)));
    // Adding an account asks for the password (0.17.22); unlock the session the way a person does.
    await page.evaluate((password) => hub.hub({ cmd: "unlock", password }), PASSWORD);
    await page.evaluate(() => setMode("gui"));
    await page.evaluate(() => openSettings("accounts"));
    await page.waitForSelector("#account-list .card", { timeout: 15000 });
    if ((await page.locator('#account-list .card[data-account="personal"]').count()) === 0) {
      await page.selectOption(".add-account [name=provider]", "opencode-go");
      await page.fill(".add-account [name=name]", "personal");
      await page.fill(".add-account [name=key]", "oc_sk_sweep");
      await page.click(".add-account .primary");
      await page.waitForSelector('#account-list .card[data-account="personal"]', { timeout: 15000 }).catch(async (e) => {
        throw new Error(`the Go account was not added: ${await page.textContent("#add-account-error")} (${e.message.split("\n")[0]})`);
      });
    }
    await page.evaluate(() => openSettings("accounts"));
    await waitFor("the limit on the Go card", async () => /Limited/.test((await page.textContent('#account-list .card[data-account="personal"]')) ?? ""), 20000);
    const card = await page.textContent('#account-list .card[data-account="personal"]');
    check(/Limited: weekly usage at 100%\. Resumes/.test(card), `the Go card reads ${card}`);
    check((await page.locator('#account-list .card[data-account="claude"] .limit-note').count()) === 0, "an account with no limit shows one");
    await page.locator('#account-list .card[data-account="personal"]').scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(OUT, `accounts-${width}x${height}.png`) });
    check(errors.length === 0, `page errors: ${errors.join("; ")}`);
    await context.close();
  }
  await browser.close();
  console.log(`saved the screenshots in ${OUT}`);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
