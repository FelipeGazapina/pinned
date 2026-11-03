#!/usr/bin/env node
// Drives one OpenCode Go account through the web app: add it with an API key, replace the key,
// pick a Go model, read the usage meter and the monthly limit, then remove it. The stack's
// mock-opencode-go answers validation and usage; no real key leaves the machine.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/go-account.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const HOME = path.join(DIR, "home");
const KEY = "oc_sk_sweep";

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
  await page.click("#gui-settings");
  await page.click('#settings-nav [data-page="accounts"]');
  await page.waitForSelector("#account-list .card", { timeout: 15000 });

  // The CLI login path still reads its own card and sign-in action.
  await page.fill(".add-account [name=name]", "Spare");
  await page.click(".add-account .primary");
  await page.waitForSelector('#account-list .card[data-account="spare"]', { timeout: 15000 });
  const spare = await page.textContent('#account-list .card[data-account="spare"]');
  check(/Not signed in/.test(spare), `the login card reads ${spare}`);
  check(await page.isVisible('[data-account="spare"] [data-action="sign-in-account"]'), "a login account has no sign-in action");
  await page.click('[data-account="spare"] [data-action="remove-account"]');
  await page.click('[data-account="spare"] [data-action="confirm-remove-account"]');
  await page.waitForSelector('#account-list .card[data-account="spare"]', { state: "detached", timeout: 15000 });

  await page.selectOption(".add-account [name=provider]", "opencode-go");
  await page.fill(".add-account [name=name]", "Go");
  await page.fill(".add-account [name=key]", KEY);
  await page.click(".add-account .primary");
  await page.waitForSelector('#account-list .card[data-account="go"]', { timeout: 15000 });
  const card = await page.textContent('#account-list .card[data-account="go"]');
  check(/OpenCode Go/.test(card), `the Go card reads ${card}`);
  check(/Signed in/.test(card), `the Go card is not signed in: ${card}`);
  check(await page.isHidden('[data-account="go"] [data-action="sign-in-account"]'), "a Go account offers the CLI sign-in");

  await page.click('[data-account="go"] [data-action="replace-account-key"]');
  await page.fill('[data-account="go"] [name=key]', KEY);
  await page.click('[data-account="go"] [data-action="save-account-key"]');
  await page.waitForSelector('[data-account="go"] [name=key]', { state: "detached", timeout: 15000 });

  const folder = path.join(HOME, "work/alpha");
  const tabKey = await page.evaluate(async ([m, p]) => (await openTab(m, p)).key, [machine, folder]);
  await waitFor("the Claude tab to run", () => page.evaluate((k) => tabByKey(k)?.view.running === true, tabKey));
  await page.evaluate(() => setMode("gui"));
  await page.click("#model-button");
  await page.waitForSelector('[data-provider="claude"]', { timeout: 10000 });
  check((await page.locator('[data-provider="go"]').count()) === 0, "a Claude account shows the Go rail");
  await page.keyboard.press("Escape");
  await page.evaluate(async ([m, p]) => {
    await callOn(m, { cmd: "set_project_account", path: p, account: "go" });
    await switchAccount(tabByKey(active.key), "go");
  }, [machine, folder]);
  await waitFor("the Go tab to run", () => page.evaluate((k) => tabByKey(k)?.view.account === "go" && tabByKey(k)?.view.running === true, tabKey));
  await page.evaluate(() => {
    if (settingsOpen()) toggleSettings();
    setMode("gui");
  });
  await page.click("#model-button");
  await page.waitForSelector('[data-provider="go"]', { timeout: 10000 });
  await page.click('[data-provider="go"]');
  await page.waitForSelector('[data-row="kimi-k3"]', { timeout: 10000 });
  await page.click('[data-row="kimi-k3"] [data-fav="kimi-k3"]');
  await page.click('[data-provider="favorites"]');
  check((await page.locator('[data-row="kimi-k3"]').count()) === 1, "the Go favorite is missing");
  check((await page.locator('[data-row="claude-opus-5-5"]').count()) === 0, "the Go favorites list shows a Claude model");
  await page.click('[data-row="kimi-k3"]');
  const launch = path.join(HOME, "claude-launch.log");
  await waitFor("the chosen Go model", () => fs.existsSync(launch) && fs.readFileSync(launch, "utf8").includes("--model kimi-k3"));
  await waitFor("the Go usage meter", async () => /5h 42%/.test(await page.textContent("#usage")), 20000);
  const meter = await page.textContent("#usage");
  check(/7d 18%/.test(meter), `the Go meter reads ${meter}`);

  await page.evaluate(() => openPage("usage"));
  await waitFor("the usage page", () => page.evaluate(() => document.body.dataset.page === "usage"));
  await waitFor("the monthly allowance", async () => /Monthly/.test(await page.textContent("#page-view")), 15000);
  const usage = await page.textContent("#page-view");
  check(/Subscriptions/.test(usage), "the Limits heading does not name subscriptions");
  check(/Session/.test(usage) && /Weekly/.test(usage), "the Go windows are missing");
  check(/Go\s*58%/.test(usage), "the Go limits column is missing");

  await page.evaluate((k) => closeTab(tabByKey(k)), tabKey);
  await page.evaluate(async () => {
    delete document.body.dataset.page;
    await toggleSettings();
  });
  await page.click('#settings-nav [data-page="accounts"]');
  await page.waitForSelector('#account-list .card[data-account="go"]', { timeout: 15000 });
  await page.click('[data-account="go"] [data-action="remove-account"]');
  await page.click('[data-account="go"] [data-action="confirm-remove-account"]');
  await page.waitForSelector('#account-list .card[data-account="go"]', { state: "detached", timeout: 15000 });
  check(!fs.existsSync(path.join(DIR, "node/accounts/go/opencode-go.key")), "the Go key stayed on disk");

  await browser.close();
  check(errors.length === 0, `console errors: ${errors.slice(0, 3).join(" | ")}`);
  console.log(JSON.stringify({ card: card.replace(/\s+/g, " ").trim(), meter: meter.replace(/\s+/g, " ").trim(), limits: usage.match(/Session[\s\S]{0,120}/)?.[0].replace(/\s+/g, " ").trim() }, null, 2));
})().catch(async (error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
