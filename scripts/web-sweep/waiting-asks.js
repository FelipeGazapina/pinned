#!/usr/bin/env node
// Shoots "Waiting on you" with the threads that decide it, at 1440x900 and 390x844: an approval
// pending mid-turn, a failure, and four finished or stopped threads whose last reply asked something
// (a loose thread, a Captain, a worker, a stopped thread). Before the change every one of them is a
// Question; after it only the pending approval and the failure wait, and the loose thread and the
// Captain sit greyed under "Asked in its last reply". The worker (its result went up) and the
// stopped thread drop out.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... WAITING_TAG=before node scripts/web-sweep/waiting-asks.js
//
// Writes waiting-asks-<WAITING_TAG>-<screen>-<desktop|phone>.png into scripts/web-sweep/screens/ and
// prints the sidebar's rows as text. WAITING_EXPECT=before|after fails the run when the rows differ
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

// Runs in the page: what the sidebar's waiting area shows, as text, in order.
const waitingArea = () => {
  const list = document.getElementById("thread-list");
  const out = [];
  for (const el of list.children) {
    if (el.matches(".thread-row, .project-head, .side-heading, .empty")) break;
    if (el.matches(".waiting-head, .offers-head")) out.push(`HEAD ${el.textContent.replace(/\s+/g, " ").trim()}`);
    else if (el.matches(".waiting-row")) out.push(`${el.classList.contains("offer") ? "OFFER" : "ROW"} ${el.textContent.replace(/\s+/g, " ").trim()}`);
  }
  return out;
};

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  const errors = [];
  const alpha = fs.realpathSync(path.join(WORK, "alpha"));
  const seen = {};
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
    if (phone) await page.evaluate(() => toggleSidebar(true));
    await page.evaluate(() => { ui.waitingOpen = true; ui.offersOpen = true; renderSidebar(); });
    await waitFor("the waiting rows", () => page.evaluate(() => document.querySelectorAll(".waiting-row").length > 0));
    await page.evaluate(() => document.getElementById("thread-list").scrollTo(0, 0));
    seen[size] = await page.evaluate(waitingArea);
    console.log(`${size}:\n  ${seen[size].join("\n  ")}`);
    await shot("open");
    if (TAG === "after") {
      await page.evaluate(() => { ui.offersOpen = false; renderSidebar(); });
      await shot("collapsed");
    }
    await context.close();
  }
  await browser.close();
  const expect = process.env.WAITING_EXPECT;
  for (const [size, lines] of Object.entries(seen)) {
    const rows = lines.filter((l) => l.startsWith("ROW ")).length;
    const offers = lines.filter((l) => l.startsWith("OFFER ")).length;
    if (expect === "before") check(rows === 6 && offers === 0, `${size}: before, six threads wait on the user (${rows} rows)`);
    if (expect === "after") {
      check(rows === 2 && offers === 2, `${size}: after, two threads wait and two offers sit below (${rows} rows, ${offers} offers)`);
      check(lines.some((l) => /Needs input/.test(l)) && lines.some((l) => /Failed/.test(l)) && lines.some((l) => /Asked in its last reply/.test(l)), `${size}: the kinds read as text`);
    }
  }
  check(!errors.length, `no page errors${errors.length ? `:\n${errors.slice(0, 5).join("\n")}` : ""}`);
})().catch((e) => {
  console.error(e.stack || String(e));
  process.exit(1);
});
