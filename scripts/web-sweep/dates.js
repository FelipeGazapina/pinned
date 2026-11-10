#!/usr/bin/env node
// Opens the page in a Portuguese browser (Playwright locale pt-BR) and checks that dates and times
// still read in the app's language: the invite card in Settings › Account says "Created Nov 10",
// not "10 de nov.", and a thread's Snooze menu shows "9:00 AM", not "09:00".
//
//   SWEEP_SIGNUP=invite SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... PLAYWRIGHT=... node scripts/web-sweep/dates.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// CHROME_BIN runs that Chrome instead of Playwright's own. Writes dates-<LABEL>-*.png (LABEL
// defaults to "after") at 1440x900 and 390x844 into SCREENS, scripts/web-sweep/screens by default,
// and exits 1 when a date or time is not in English or a page error shows.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const SCREENS = process.env.SCREENS || path.join(__dirname, "screens");
const LABEL = process.env.LABEL || "after";
const SIZES = { desktop: { width: 1440, height: 900 }, phone: { width: 390, height: 844 } };

const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`timed out waiting for ${what}`);
};

async function run(browser, name, size) {
  const phone = size.width < 768;
  const context = await browser.newContext({ viewport: size, isMobile: phone, hasTouch: phone, colorScheme: "dark", locale: "pt-BR" });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e.stack ?? e).split("\n").slice(0, 3).join(" | ")));
  const problems = [];
  const expect = (ok, what) => ok || problems.push(what);

  await page.goto(HUB);
  expect((await page.evaluate(() => navigator.language)) === "pt-BR", "the browser speaks pt-BR");
  await page.fill("#login [name=email]", "sweep@example.test");
  await page.fill("#login [name=password]", "sweep password 1");
  await page.click("#login .primary");
  const node = await waitFor("the sweep node", () => page.evaluate(() => document.body.dataset.hub === "online" && [...machines.keys()].find((k) => machines.get(k).view.online && machines.get(k).view.name === "sweep-node")));
  await page.evaluate(() => setMode("gui"));

  // The invite card: the first invite asks for the account's password.
  await page.evaluate(() => {
    openSettings();
    showPage("account");
  });
  await page.waitForSelector(".card.invites", { timeout: 15000 });
  await page.click('[data-action="create-invite"]');
  await page.waitForSelector("dialog[open] form [name=password]", { timeout: 20000 });
  await page.fill("dialog[open] form [name=password]", "sweep password 1");
  await page.click("dialog[open] form button.primary");
  await page.waitForSelector(".card.invites .row.command code", { timeout: 20000 });
  await page.waitForSelector(".card.invites .who .hint");
  const created = await page.textContent(".card.invites .who .hint");
  expect(/^Created [A-Z][a-z]{2} \d{1,2}$/.test(created.trim()), `invite date reads in English, got "${created.trim()}"`);
  await page.locator(".card.invites").scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(SCREENS, `dates-${LABEL}-${name}-invite.png`) });

  // A thread's time in the sidebar: its Snooze menu lists when it would come back.
  const alpha = fs.realpathSync(path.join(DIR, "home", "work", "alpha"));
  await page.click("#settings-back");
  await page.evaluate(async ({ node, alpha }) => {
    if (!tabs.some((t) => t.machine === node && t.view.cwd === alpha)) await openRemote(node, alpha, false);
  }, { node, alpha });
  if (phone) await page.locator("#chat-sidebar-toggle:visible, #sidebar-toggle:visible").first().click();
  const row = page.locator(".thread-row:visible").first();
  await row.waitFor({ timeout: 8000 }).catch(async (e) => {
    await page.screenshot({ path: path.join(SCREENS, `dates-debug-${name}.png`) });
    throw new Error(`${name}: no visible thread row (${e.message.split("\n")[0]})`);
  });
  // On a phone nothing hovers, so the thread's context menu leads to the same Snooze list, with each time in its label.
  if (phone) {
    await row.dispatchEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 60, clientY: 300 });
    await page.locator("#side-menu button", { hasText: "Snooze" }).first().click();
  } else {
    await row.hover();
    await row.locator('[data-act="snooze"]').click();
  }
  const hints = await waitFor("the snooze menu", () => page.evaluate(() => {
    const items = [...document.querySelectorAll("#side-menu button")].map((b) => (b.querySelector(".hint") ?? b.querySelector(".label"))?.textContent.trim()).filter((t) => /\d:\d\d/.test(t ?? ""));
    return items.length ? items.map((t) => t.replace(/^.*\(([^)]*)\)$/, "$1")) : null;
  }));
  expect(hints.every((h) => /\d{1,2}:\d{2} (AM|PM)$/.test(h)), `snooze times read in English, got ${JSON.stringify(hints)}`);
  await page.screenshot({ path: path.join(SCREENS, `dates-${LABEL}-${name}-sidebar.png`) });

  await context.close();
  return { name, created: created.trim(), hints, problems, errors };
}

(async () => {
  fs.mkdirSync(SCREENS, { recursive: true });
  const browser = await playwright.chromium.launch({
    headless: true,
    ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}),
    args: ["--no-sandbox"],
  });
  const results = [];
  for (const [name, size] of Object.entries(SIZES)) results.push(await run(browser, name, size));
  await browser.close();
  for (const r of results) console.log(JSON.stringify(r));
  process.exit(results.some((r) => r.problems.length || r.errors.length) ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
