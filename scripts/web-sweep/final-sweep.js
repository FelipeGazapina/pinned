#!/usr/bin/env node
// Shoots every screen of the global orchestrator release at 1440x900 and 390x844, to put beside
// its board in prototype/live: Main (sidebar, Captain card, global chat), Needs you,
// Projects, the orchestrator's settings, and Way of working with Chat to adjust.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/global-chat.js   # the chat, its reports, three waiting threads
//   SWEEP_DIR=... node scripts/web-sweep/taste.js         # alpha's Captain and a saved taste
//   SWEEP_DIR=... node scripts/web-sweep/final-sweep.js
//
// Writes final-<release>-<screen>-<desktop|phone>.png into screens/ (FINAL_TAG, default 0.17.21)
// and fails when the Captain card's "N working" label is clipped.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const WORK = path.join(DIR, "home", "work");
const SCREENS = path.join(__dirname, "screens");
const TAG = process.env.FINAL_TAG || "0.17.21";
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

// Runs in the page: alpha's Captain has ended its turn while its two workers work on, so its card
// reads "2 working" (titled "2 agents working"); a third, done worker sits under it. The node's tabs events replace each
// view, so the seed re-applies itself before every sidebar render.
function seedCaptain(dir) {
  const at = Math.floor(Date.now() / 1000) - 260;
  const apply = () => {
    const alpha = tabs.filter((t) => t.view.cwd === dir && !isGlobalOrchestratorTab(t));
    const captain = alpha.find((t) => t.view.role === "orchestrator") ?? alpha[0];
    if (!captain) return;
    Object.assign(captain.view, { role: "orchestrator", title: "Release v0.17.21", status: "done", running: true, updated: at, bypass: true, stats: { active_ms: 4_320_000, cost_usd: 3.42 } });
    const workers = alpha.filter((t) => t !== captain).slice(0, 3);
    workers.forEach((w, i) => Object.assign(w.view, {
      role: "worker", spawned_by: captain.view.id, running: true, updated: at + 60 * i,
      title: ["Run the CI on main", "Changelog for v0.17.21", "Tidy the README"][i], status: i < 2 ? "working" : "done",
    }));
    const thread = threadOf(captain);
    thread.workspace = { ...(thread.workspace ?? {}), branch: "release/v0.17.21" };
  };
  if (!window.__finalRender) {
    window.__finalRender = renderSidebar;
    renderSidebar = () => {
      apply();
      window.__finalRender();
    };
  }
  renderSidebar();
}

// Runs in the page: the Captain card's label against its slot. Each part of the label (icon, count,
// time) is either whole inside the slot or wholly out of it; a part cut at an edge is clipped.
const captainCard = () => {
  const row = [...document.querySelectorAll("#thread-list .thread-row")].find((r) => r.querySelector(".role")?.textContent === "Captain" && /^\d+ (agents? )?working/.test(r.querySelector(".stamp.status")?.textContent.trim() ?? ""));
  if (!row) return null;
  const slot = row.querySelector(".slot").getBoundingClientRect();
  const stamp = row.querySelector(".stamp.status");
  const rectOf = (node) => {
    if (node.nodeType === 1) return node.getBoundingClientRect();
    const range = document.createRange();
    range.selectNodeContents(node);
    return range.getBoundingClientRect();
  };
  const parts = [...stamp.childNodes].filter((n) => n.nodeType === 1 || n.textContent.trim()).map((n) => ({ text: n.textContent.trim() || n.nodeName, r: rectOf(n) }));
  const seen = (r) => r.width > 0 && r.top < slot.bottom && r.bottom > slot.top && r.left < slot.right && r.right > slot.left;
  const whole = (r) => r.left >= slot.left - 0.5 && r.right <= slot.right + 0.5 && r.top >= slot.top - 0.5 && r.bottom <= slot.bottom + 0.5;
  const count = parts.find((p) => /^\d+ /.test(p.text));
  return {
    shown: parts.filter((p) => seen(p.r) && whole(p.r)).map((p) => p.text).join(" | "),
    title: stamp.title,
    clipped: !count || !whole(count.r) || parts.some((p) => seen(p.r) && !whole(p.r)),
    slot: Math.round(slot.width),
  };
};

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  const errors = [];
  const cards = [];
  const alpha = fs.realpathSync(path.join(WORK, "alpha"));
  for (const [size, viewport] of SIZES) {
    const phone = size === "phone";
    const context = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
    const page = await context.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    const shot = async (name) => {
      await page.waitForTimeout(400);
      await page.screenshot({ path: path.join(SCREENS, `final-${TAG}-${name}-${size}.png`) });
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
    await page.evaluate((src) => (0, eval)(src), `${seedCaptain}; seedCaptain(${JSON.stringify(alpha)});`);

    // Main: the global orchestrator's chat with its reports open, beside the sidebar.
    await page.evaluate(() => activate(tabs.find(isGlobalOrchestratorTab)));
    await waitFor("the report fold", () => page.evaluate(() => !!document.querySelector(".report-fold")));
    if (!(await page.evaluate(() => document.querySelector(".report-row")))) await page.click(".report-fold");
    await page.evaluate(() => document.querySelector(".report-fold").scrollIntoView({ block: "start" }));
    if (phone) {
      await shot("main-chat");
      await page.evaluate(() => toggleSidebar(true));
    }
    await page.evaluate(() => renderSidebar());
    await waitFor("the Captain card", () => page.evaluate(captainCard));
    const card = await page.evaluate(captainCard);
    console.log("captain card:", JSON.stringify(card));
    await shot("main");
    if (phone) await page.evaluate(() => document.querySelector(".thread-row.child")?.scrollIntoView({ block: "center" }));
    if (phone) await shot("main-captain");
    cards.push([size, card]);

    // Needs you, and a waiting thread opened from its card.
    await page.evaluate(() => openPage("needs-you"));
    await waitFor("the waiting cards", () => page.evaluate(() => document.querySelectorAll("#page-view .ny-card").length > 0));
    await shot("waiting");
    await page.click('#page-view .ny-card [data-ny="open"]');
    await waitFor("the card's thread", () => page.evaluate(() => !shellPage() && !!active));
    await shot("waiting-thread");

    // Projects.
    await page.evaluate(() => openGlobalProjects());
    await waitFor("the project rows", () => page.evaluate(() => document.querySelectorAll(".gp-row").length > 0));
    await shot("projects");

    // The orchestrator's settings.
    await page.evaluate(() => openSettings());
    await waitFor("the Global orchestrator section", () => page.evaluate(() => !!document.getElementById("global-orchestrator-settings")));
    await page.evaluate(() => document.getElementById("global-orchestrator-settings").scrollIntoView({ block: "start" }));
    await shot("settings");

    // Way of working, its Captains scope, and Chat to adjust.
    await page.evaluate(() => openPage("taste"));
    await waitFor("the page", () => page.isVisible(".taste-split"));
    await page.click('.taste-scopes button:text-is("Captains (all)")');
    await page.waitForTimeout(1200);
    await shot("taste");
    await page.click(".page-head .taste-chat");
    await waitFor("the chat's thread", () => page.evaluate(() => !shellPage() && !!active));
    await page.waitForTimeout(1500);
    await shot("taste-chat");
    // Close the chat's thread, so the next run's sidebar starts where this one did.
    await page.evaluate(() => closeTab(active));
    await context.close();
  }
  await browser.close();
  for (const [size, card] of cards) check(!card.clipped, `the Captain card shows "${card.shown}" with nothing cut at ${size}`);
  check(!errors.length, `no page errors${errors.length ? `:\n${errors.slice(0, 5).join("\n")}` : ""}`);
})().catch((e) => {
  console.error(e.stack || String(e));
  process.exit(1);
});
