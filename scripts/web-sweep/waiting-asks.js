#!/usr/bin/env node
// Shoots the Needs you page with the threads that decide it, at 1440x900 and 390x844: an approval
// pending mid-turn, a failure, and four finished or stopped threads whose last reply asked something
// (a loose thread, a Captain, a worker, a stopped thread). Before the change every one of them is a
// Question; after it only the pending approval and the failure wait, counted on the Needs you row,
// and the loose thread and the Captain sit below them as quiet cards that block nobody. The worker
// (its result went up) and the stopped thread drop out.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... WAITING_TAG=before node scripts/web-sweep/waiting-asks.js
//
// Writes waiting-asks-<WAITING_TAG>-<screen>-<desktop|phone>.png into scripts/web-sweep/screens/ and
// prints the page's cards as text. WAITING_EXPECT=before|after fails the run when the rows differ
// from what that side should show. The node's tabs events replace each view, so the seed applies
// itself again before every sidebar render.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const WORK = path.join(DIR, "home", "work");
const SCREENS = process.env.WAITING_SHOTS || path.join(__dirname, "screens");
const TAG = process.env.WAITING_TAG || "after";
const SIZES = [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]];

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

// Runs in the page. Six threads of alpha, each a row of the table below, in the order they are listed.
function seedThreads(dir) {
  const now = Math.floor(Date.now() / 1000);
  const rows = [
    { title: "Apply the orders migration", status: "needs-input", running: true, interrupted: true, message: "Claude needs your permission to use Bash", updated: now - 90 },
    { title: "Fix the flaky checkout test", status: "failed", running: true, message: "API Error: 500", updated: now - 600 },
    { title: "Release v0.17.24", role: "orchestrator", status: "done", running: true, question: "Want me to cut the release?", updated: now - 1500 },
    { title: "Why did the CI fail?", status: "done", running: true, question: "Want me to open the PR?", updated: now - 7200 },
    { title: "Changelog for v0.17.24", role: "worker", status: "done", running: true, question: "Should I also update the README?", updated: now - 14400 },
    { title: "Tidy the README", status: "done", running: false, question: "Do you want the badges back?", updated: now - 86400 * 3 },
  ];
  const apply = () => {
    const alpha = tabs.filter((t) => t.view.cwd === dir && !isGlobalOrchestratorTab(t)).sort((a, b) => a.view.opened - b.view.opened);
    const captain = rows.find((r) => r.role === "orchestrator");
    rows.forEach((row, i) => {
      const tab = alpha[i];
      if (!tab) return;
      Object.assign(tab.view, { bypass: true, interrupted: false, question: undefined, message: null, role: undefined, ...row });
      if (row.role === "worker") tab.view.spawned_by = alpha[rows.indexOf(captain)]?.view.id;
    });
  };
  if (!window.__waitingRender) {
    window.__waitingRender = renderSidebar;
    renderSidebar = () => {
      apply();
      window.__waitingRender();
    };
  }
  renderSidebar();
}

// Runs in the page: the Needs you page's cards as text, in order. A card that waits on the user is a
// ROW; a finished thread's or one whose last reply asked something is an OFFER, since it blocks nobody.
const waitingArea = () => {
  const out = [`HEAD Needs you ${document.querySelector("#page-view .ny-title .ny-count")?.textContent ?? ""}`.trim()];
  for (const el of document.querySelectorAll("#page-view .ny-card")) {
    const quiet = ["offer", "review"].includes(el.dataset.kind);
    out.push(`${quiet ? "OFFER" : "ROW"} ${el.getAttribute("aria-label")} ${el.textContent.replace(/\s+/g, " ").trim()}`);
  }
  return out;
};
// Runs in the page: the count on the Needs you row (the phone's tab), which counts only what blocks.
const needsYouCount = () => (document.querySelector('#side-nav [data-nav="needs-you"] .count:not([hidden])') ?? document.querySelector('#phone-tabs [data-phone-tab="needs-you"] .badge:not([hidden])'))?.textContent ?? "0";

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  const errors = [];
  const alpha = fs.realpathSync(path.join(WORK, "alpha"));
  const seen = {};
  const counts = {};
  for (const [size, viewport] of SIZES) {
    const phone = size === "phone";
    const context = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
    const page = await context.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    const shot = async (name) => {
      await page.waitForTimeout(400);
      await page.screenshot({ path: path.join(SCREENS, `waiting-asks-${TAG}-${name}-${size}.png`) });
      console.log(`shot ${name}-${size}`);
    };
    await page.goto(HUB);
    if (await page.isVisible("#login [name=email]")) {
      await page.fill("#login [name=email]", "sweep@example.test");
      await page.fill("#login [name=password]", "sweep password 1");
      await page.click("#login .primary");
    }
    await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
    await waitFor("the orchestrator's row", () => page.evaluate(() => !!document.querySelector(".global-row")));
    await page.evaluate(() => setMode("gui"));
    await page.evaluate(async (dir) => {
      const machine = [...machines.keys()][0];
      const have = () => tabs.filter((t) => t.view.cwd === dir && !isGlobalOrchestratorTab(t)).length;
      while (have() < 6) {
        const before = have();
        await callOn(machine, { cmd: "open_tab", path: dir, bypass: false });
        const start = Date.now();
        while (have() === before && Date.now() - start < 5000) await new Promise((r) => setTimeout(r, 100));
        if (have() === before) break;
      }
    }, alpha);
    await page.evaluate((src) => (0, eval)(src), `${seedThreads}; seedThreads(${JSON.stringify(alpha)});`);
    // Needs you also lists a finished thread the user has not read; the threads here were read, so what
    // their last reply asked is what puts them on the page.
    await page.evaluate((dir) => tabs.filter((t) => t.view.cwd === dir).forEach(markThreadSeen), alpha);
    await page.click(phone ? '#phone-tabs [data-phone-tab="needs-you"]' : '#side-nav [data-nav="needs-you"]');
    await waitFor("the Needs you page", () => page.evaluate(() => shellPage() === "needs-you"));
    await waitFor("the cards", () => page.evaluate(() => document.querySelectorAll("#page-view .ny-card").length > 0));
    await page.waitForTimeout(400);
    seen[size] = await page.evaluate(waitingArea);
    counts[size] = await page.evaluate(needsYouCount);
    console.log(`${size} (Needs you ${counts[size]}):\n  ${seen[size].join("\n  ")}`);
    await shot("open");
    await context.close();
  }
  await browser.close();
  const expect = process.env.WAITING_EXPECT;
  for (const [size, lines] of Object.entries(seen)) {
    const rows = lines.filter((l) => l.startsWith("ROW ")).length;
    const offers = lines.filter((l) => l.startsWith("OFFER ")).length;
    if (expect === "before") check(rows === 6 && offers === 0, `${size}: before, six threads wait on the user (${rows} rows)`);
    if (expect === "after") {
      check(rows === 2 && offers === 2, `${size}: after, two threads wait and two quiet cards sit below (${rows} rows, ${offers} offers)`);
      check(counts[size] === "2", `${size}: the Needs you count reads ${counts[size]}, the two that wait`);
      check(lines.some((l) => /^ROW Approval: Apply the orders migration/.test(l)) && lines.some((l) => /^ROW Failed: Fix the flaky checkout test/.test(l)), `${size}: the kinds read as text`);
      check(lines.some((l) => /^OFFER .*Release v0\.17\.24/.test(l)) && lines.some((l) => /^OFFER .*Why did the CI fail\?/.test(l)), `${size}: the Captain and the loose thread sit below`);
      check(!lines.some((l) => /Changelog for v0\.17\.24|Tidy the README/.test(l)), `${size}: the worker or the stopped thread shows`);
    }
  }
  check(!errors.length, `no page errors${errors.length ? `:\n${errors.slice(0, 5).join("\n")}` : ""}`);
})().catch((e) => {
  console.error(e.stack || String(e));
  process.exit(1);
});
