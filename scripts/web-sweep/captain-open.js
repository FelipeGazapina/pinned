#!/usr/bin/env node
// Opens a Captain and the orchestrator from the sidebar while the Captain's screen ends with an
// answer that says "Settings › Archive", as real Captains' answers did on 2026-11-10. In 0.17.22 that
// line made every render throw: a click on the Captain did nothing and a click on the orchestrator
// toasted "Couldn't open the orchestrator: TypeError: Cannot read properties of undefined (reading
// 'length')". alpha's Captain runs two held workers, so its project reads "2 running".
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... PLAYWRIGHT=... node scripts/web-sweep/captain-open.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// CHROME_BIN runs that Chrome instead of Playwright's own. Writes captain-open-<LABEL>-*.png
// (LABEL defaults to "after") at 1440x900 and 390x844 into SCREENS, scripts/web-sweep/screens by
// default, and exits 1 when a click does not open its thread or a toast or page error shows.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const SCREENS = process.env.SCREENS || path.join(__dirname, "screens");
const LABEL = process.env.LABEL || "after";
const ANSWER = "Both are in Settings › Archive and can be brought back.";

const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`timed out waiting for ${what}`);
};

async function login(page, errors) {
  await page.goto(HUB);
  await page.fill("#login [name=email]", "sweep@example.test");
  await page.fill("#login [name=password]", "sweep password 1");
  await page.click("#login .primary");
  await waitFor("the sweep node", () => page.evaluate(() => document.body.dataset.hub === "online" && [...machines.values()].some((m) => m.view.online && m.view.name === "sweep-node")));
  await page.evaluate(() => setMode("gui")).catch((e) => errors.push(e.message.split("\n")[0]));
  return page.evaluate(() => [...machines.keys()].find((k) => machines.get(k).view.name === "sweep-node"));
}

/** alpha's Captain with two held workers, kept across runs: its key and its first worker's. */
async function captainWithWorkers(page, node) {
  const alpha = fs.realpathSync(path.join(DIR, "home", "work", "alpha"));
  const key = await page.evaluate(async ({ node, alpha }) => {
    const tab = await revealTab(node, await callOn(node, { cmd: "open_orchestrator", path: alpha }));
    if (!tab.view.running) await startTab(tab);
    return tab.key;
  }, { node, alpha });
  await waitFor("the Captain's session", () => page.evaluate((k) => tabByKey(k)?.view.running === true, key));
  await waitFor("the Captain's prompt", () => page.evaluate((k) => screenLines(tabByKey(k)).some((l) => l.startsWith("fake claude in")), key));
  const workers = () => page.evaluate((k) => tabs.filter((t) => t.view.spawned_by === tabByKey(k).view.id).length, key);
  if ((await workers()) < 2) {
    const calls = ["Check the staging merge of task one. [hold]", "Check the staging merge of task two. [hold]"]
      .map((task, i) => JSON.stringify({ task, title: `Staging merge ${i + 1}`, target: { providerInstanceId: "claude", model: "claude-sonnet-5-5" } }));
    fs.writeFileSync(path.join(DIR, "home", "delegate-calls.jsonl"), calls.join("\n"));
    await page.evaluate((k) => callOn(tabByKey(k).machine, { cmd: "write_input", tab: tabByKey(k).view.id, data: "Check both staging merges.\r" }), key);
    await waitFor("two workers", async () => (await workers()) >= 2, 30000);
  }
  await waitFor("the Captain's turn to end", () => page.evaluate((k) => ["done", "delegating"].includes(phaseOf(tabByKey(k))), key), 30000);
  const worker = await page.evaluate((k) => tabs.find((t) => t.view.spawned_by === tabByKey(k).view.id).key, key);
  return { key, worker };
}

async function shoot(browser, size, setUp) {
  const phone = size.width < 768;
  const context = await browser.newContext({ viewport: size, isMobile: phone, hasTouch: phone, colorScheme: "dark" });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e.stack ?? e).split("\n").slice(0, 3).join(" | ")));
  const node = await login(page, errors);
  const { key: captain, worker, first } = await setUp(page, node);
  // Start from a worker's thread. On a build with the bug this activate throws once the answer is up.
  await page.evaluate((k) => activate(tabByKey(k)), worker).catch((e) => errors.push(e.message.split("\n")[0]));
  await page.waitForTimeout(500);
  if (first) await page.evaluate(({ k, answer }) => callOn(tabByKey(k).machine, { cmd: "write_input", tab: tabByKey(k).view.id, data: `${answer}\r` }), { k: captain, answer: ANSWER });
  await waitFor("the answer on the Captain's screen", () => page.evaluate(({ k, answer }) => screenLines(tabByKey(k)).some((l) => l.includes(answer)), { k: captain, answer: ANSWER }), 20000);
  const name = `${size.width}x${size.height}`;
  const toast = () => page.evaluate(() => (document.getElementById("side-notice").hidden ? "" : document.getElementById("side-notice").textContent));
  const openSidebar = () => page.evaluate(() => narrow(768) && toggleSidebar(true));
  const result = { size: name, errors };

  await openSidebar();
  await page.waitForTimeout(400);
  await page.click(`.thread-row[data-tab="${captain}"]`, { timeout: 5000 }).catch((e) => errors.push(`no Captain row: ${e.message.split("\n")[0]}`));
  await page.waitForTimeout(1200);
  result.captain = { opened: await page.evaluate((k) => active?.key === k && document.getElementById("thread").innerText.includes("Started 2 workers"), captain), toast: await toast() };
  await page.screenshot({ path: path.join(SCREENS, `captain-open-${LABEL}-captain-${name}.png`) });

  // A phone opens the orchestrator from its tab bar, which shows on the Threads screen.
  await openSidebar();
  await page.waitForTimeout(400);
  await page.click(phone ? '#phone-tabs [data-phone-tab="orchestrator"]' : "[data-global-open]", { timeout: 5000 }).catch((e) => errors.push(`no orchestrator row: ${e.message.split("\n")[0]}`));
  await page.waitForTimeout(1500);
  // The toast sits in the sidebar, which the click closed on a phone.
  if (phone && (await toast())) await openSidebar();
  result.orchestrator = { opened: await page.evaluate(() => isGlobalOrchestratorTab(active)), toast: await toast() };
  await page.waitForTimeout(300);
  await page.screenshot({ path: path.join(SCREENS, `captain-open-${LABEL}-orchestrator-${name}.png`) });
  await context.close();
  return result;
}

(async () => {
  fs.mkdirSync(SCREENS, { recursive: true });
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  let failed = false;
  try {
    let captain = null;
    const setUp = async (page, node) => (captain ? { ...captain, first: false } : { ...(captain = await captainWithWorkers(page, node)), first: true });
    for (const size of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
      const result = await shoot(browser, size, setUp);
      console.log(JSON.stringify(result));
      const ok = result.captain.opened && !result.captain.toast && result.orchestrator.opened && !result.orchestrator.toast && !result.errors.length;
      if (!ok) failed = true;
    }
  } finally {
    await browser.close();
  }
  console.log(failed ? "FAIL: a click did not open its thread" : "ok: the Captain and the orchestrator open at both sizes");
  process.exit(failed ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
