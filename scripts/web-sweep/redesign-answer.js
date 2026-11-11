#!/usr/bin/env node
// Answers a waiting worker from its Captain's thread with answerAsk (ui/redesign-model.js), while
// the worker was never opened in the page: its menu is read off a hidden terminal that was never
// fitted. Approve, Decline, and Decline with a reason each reach the worker's session, the worker
// goes on, and the Captain's draft, composer and card stay as they were. The README worker is put
// back to waiting on its push at the end, so the redesign world stays as the boards show it. Then
// the Captain's "Worker waiting." banner (ui/gui-worker-banner.js) is pressed with the mouse: its
// Approve and Decline answer the worker while the prompt keeps the focus and the draft, and its
// Open opens the worker on its own approval card.
//
// Then nothing reaches the worker that the user did not see: a double tap on the banner's Approve
// sends one Enter; a message typed into the README row while it waits sends nothing and leaves its
// push waiting; and, in a page of its own, a menu that changes on the worker's screen between the
// banner's paint and the tap sends no key and the banner repaints with the new command.
//
//   export SWEEP_DIR=... SWEEP_HUB_PORT=... SWEEP_PG_PORT=... SWEEP_GO_PORT=...
//   export CARGO=true CARGO_TARGET_DIR=<a checkout's target>
//   scripts/web-sweep/stack.sh start
//   PLAYWRIGHT=<dir>/node_modules/playwright node scripts/web-sweep/redesign-answer.js
//   scripts/web-sweep/stack.sh stop
//
// The world comes from redesign/world.js: Captain "Release v0.17.21" and its worker "Tidy the
// README", which waits to run `git push origin readme-tidy` under the sweep's fake claude.
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");
const world = require("./redesign/world");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const PUSH = "git push origin readme-tidy";
const README_BRIEF = "Tidy the README. Fix the install steps for Windows, drop the two stale badges, and keep every heading that the docs link to.";
const DRAFT = "Cut the release once the CI is green.";

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};
const waitFor = async (what, fn, timeout = 30000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};

async function login(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", world.EMAIL);
    await page.fill("#login [name=password]", world.PASSWORD);
    await page.click("#login .primary");
  }
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  await page.evaluate(() => setMode("gui"));
}

// Runs in the page: what the open thread's composer and card show, and how the worker's terminal stands.
const snapshot = ({ captain, worker }) => {
  const w = tabByKey(worker);
  return {
    active: active?.key,
    composerTab: composer.dataset.tab,
    prompt: prompt.value,
    draft: drafts[threadKey(tabByKey(captain))] ?? "",
    approvalHidden: approval.hidden,
    approvalKey: approval.dataset.key,
    threadPlace: threadView.dataset.tab,
    workerHidden: w.el.hidden,
    workerSize: `${w.term.cols}x${w.term.rows}`,
    workerScreen: screenLines(w).filter(Boolean).slice(-6),
  };
};

/** Types the worker's brief as its next prompt and waits until its hidden terminal shows the push
 * menu again. The machine titles a thread after its latest prompt, so the prompt leads with the
 * worker's title, as its first one did. */
async function rearm(page, keys) {
  await page.evaluate(({ worker, text }) => {
    const w = tabByKey(worker);
    return callOn(w.machine, { cmd: "prompt", tab: w.view.id, text });
  }, { worker: keys.worker, text: `${world.WORLD.threads.readme.title}\n\n${README_BRIEF}` });
  await waitFor("the worker to ask for the push again", () => page.evaluate((k) => tabByKey(k).menu?.box === "git push origin readme-tidy" && waitingKindOf(tabByKey(k).view) === "needs_input", keys.worker), 30000);
}

async function answer(page, keys, choice, before) {
  const workersBefore = await page.evaluate((k) => workersKey(tabByKey(k)), keys.captain);
  const t0 = Date.now();
  const sent = await page.evaluate(({ worker, choice }) => answerAsk(tabByKey(worker), choice, pendingAsk(tabByKey(worker))), { worker: keys.worker, choice });
  check(sent === true, `answerAsk(${JSON.stringify(choice)}) sent the answer`);
  const after = await page.evaluate(snapshot, keys);
  for (const field of ["active", "composerTab", "prompt", "draft", "approvalHidden", "approvalKey", "threadPlace"]) {
    check(after[field] === before[field], `the Captain's ${field} is unchanged after ${JSON.stringify(choice)} (${JSON.stringify(after[field])})`);
  }
  check(after.workerHidden && after.workerSize === before.workerSize, `the worker's terminal stayed hidden at ${after.workerSize}`);
  return { workersBefore, t0 };
}

/** Presses the Captain's "Worker waiting." banner's `button` with the mouse while the prompt holds
 * the focus and the draft, and checks both stay. */
async function pressBanner(page, keys, button, before) {
  await page.evaluate(() => prompt.focus());
  const banner = await waitFor("the Worker waiting banner", () => page.evaluate(() => !workerBanner.hidden && workerBanner.innerText.replace(/\s+/g, " ").trim()), 15000);
  check(banner.startsWith(`Worker waiting. ${world.WORLD.threads.readme.title} needs your approval to run ${PUSH}`), `the banner reads "${banner}"`);
  check(/Decline/.test(banner) && /Approve/.test(banner) && /Open/.test(banner), "the banner offers Decline, Approve and Open");
  const workersBefore = await page.evaluate((k) => workersKey(tabByKey(k)), keys.captain);
  const t0 = Date.now();
  await page.click(`#worker-banner ${button}`);
  const after = await page.evaluate(({ captain }) => ({ focused: document.activeElement === prompt, prompt: prompt.value, draft: drafts[threadKey(tabByKey(captain))] ?? "", active: active?.key }), keys);
  if (button === "[data-worker-open]") return after;
  check(after.focused, `the prompt kept the focus after ${button}`);
  check(after.prompt === before.prompt && after.draft === before.draft && after.active === before.active, `the Captain's draft and thread stayed after ${button}`);
  return { workersBefore, t0 };
}

async function proveAnswered(page, keys, { workersBefore, t0 }, seen, what) {
  const screen = await waitFor(`${what} on the worker's screen`, async () => {
    const lines = await page.evaluate((k) => screenLines(tabByKey(k)), keys.worker);
    return lines.some((l) => l.includes(seen)) && lines;
  }, 15000);
  console.log(`  ${what}: the worker's screen showed "${seen}" after ${Date.now() - t0} ms`);
  check(screen.some((l) => l.includes(seen)), `the worker's session received ${what}`);
  await waitFor(`the worker to go on after ${what}`, () => page.evaluate((k) => statusOf(tabByKey(k)) === "done", keys.worker), 20000);
  check(true, `the worker went on and finished its turn after ${what}`);
  const state = await page.evaluate(({ captain, worker }) => {
    const row = workersOf(tabByKey(captain)).find((r) => r.tab?.key === worker);
    return { row: row && { state: row.state, ask: row.ask }, key: workersKey(tabByKey(captain)), content: threadView.dataset.content, needs: navCounts().needsYou, listened: window.__modelChanges };
  }, keys);
  check(state.row?.state === "done" && state.row.ask === null, `the Captain's worker row reads done with no ask`);
  check(state.key !== workersBefore && state.content.includes(JSON.stringify(state.key)), "the Captain's thread repainted with the new workers key");
  check(state.listened > 0, `onModelChange listeners ran (${state.listened} times so far)`);
}

/** From now on, records every call the page sends the worker's machine. */
const watchCalls = (page, keys) =>
  page.evaluate(({ worker }) => {
    const transport = machines.get(tabByKey(worker).machine).transport;
    if (!transport.counted) {
      const call = transport.call.bind(transport);
      transport.call = (c) => (window.__calls.push(c), call(c));
      transport.counted = true;
    }
    window.__calls = [];
  }, keys);
/** What the recorded `cmd` calls sent the worker: the keys of a write_input, the text of a prompt. */
const callsTo = (page, keys, cmd) =>
  page.evaluate(({ worker, cmd }) => window.__calls.filter((c) => c.cmd === cmd && c.tab === tabByKey(worker).view.id).map((c) => c.data ?? c.text), { ...keys, cmd });

/** The worker's screen as if claude had moved on to ask about another command. */
const RM_SCREEN = `\x1b[2J\x1b[H${[
  "────────────────────────────────────────",
  " Bash command",
  " Removes the build folder.",
  "╌".repeat(40),
  " rm -rf build",
  "╌".repeat(40),
  " Do you want to proceed?",
  " ❯ 1. Yes",
  "   2. Yes, and don't ask again for rm commands",
  "   3. No",
  " Esc to cancel · Tab to amend",
].join("\r\n")}`;

async function doubleTap(page, keys) {
  await page.evaluate(() => prompt.focus());
  await waitFor("the banner's Approve", () => page.isVisible('#worker-banner [data-worker-answer="approve"]:not([disabled])'), 15000);
  await watchCalls(page, keys);
  const workersBefore = await page.evaluate((k) => workersKey(tabByKey(k)), keys.captain);
  const t0 = Date.now();
  await page.dblclick('#worker-banner [data-worker-answer="approve"]');
  await page.waitForTimeout(250);
  const third = await page.click('#worker-banner [data-worker-answer="approve"]:not([disabled])', { timeout: 500 }).then(() => "clicked", () => "no enabled Approve left");
  console.log(`  a third tap 250 ms later: ${third}`);
  await proveAnswered(page, keys, { workersBefore, t0 }, "chose 1", "a double tap on the banner's Approve");
  const writes = await callsTo(page, keys, "write_input");
  check(writes.length === 1 && writes[0] === "\r", `one Enter reached the worker for three taps (${JSON.stringify(writes)})`);
}

async function messageWaiting(page, keys) {
  await watchCalls(page, keys);
  await page.evaluate(({ captain, worker }) => {
    const t = threadOf(tabByKey(captain));
    t.open.add(`worker:${worker}`);
    t.open.add(`say:${worker}`);
    rerender();
  }, keys);
  const row = (k) => `.worker-row[data-worker="${k}"]`;
  const line = await waitFor("the README row's message line", () => page.evaluate((sel) => threadView.querySelector(`${sel} .worker-say-waiting`)?.innerText.replace(/\s+/g, " ").trim(), row(keys.worker)), 10000);
  check(/^Tidy the README is waiting on you\. A message would answer its ask, so answer it in its thread\. ?Open$/.test(line), `the row's message line says it waits and offers Open: "${line}"`);
  check(!(await page.isVisible(`${row(keys.worker)} input.worker-say`)), "the row has no field to message it");
  const said = await page.evaluate(({ worker }) => sayToWorker(worker, "Push to a fork instead"), keys);
  check(said === false, "a message sent to it anyway is refused");
  await page.waitForTimeout(2000);
  const after = await page.evaluate((k) => ({ box: tabByKey(k).menu?.box, waiting: waitingKindOf(tabByKey(k).view), toast: document.querySelector("#toast, .toast")?.innerText ?? "" }), keys.worker);
  check(after.box === PUSH && after.waiting === "needs_input", `its push still waits (${JSON.stringify(after)})`);
  check(!(await callsTo(page, keys, "prompt")).length && !(await callsTo(page, keys, "write_input")).length, "no prompt and no key went to the waiting worker");
  await page.evaluate(({ captain, worker }) => {
    const t = threadOf(tabByKey(captain));
    t.open.delete(`worker:${worker}`);
    t.open.delete(`say:${worker}`);
    rerender();
  }, keys);
}

async function stalePaint(browser, keys) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
  try {
    const page = await context.newPage();
    await login(page);
    await page.evaluate((k) => activate(tabByKey(k)), keys.captain);
    await waitFor("the banner on the push", () => page.evaluate((cmd) => !workerBanner.hidden && workerBanner.querySelector(".cmd")?.textContent === cmd && !!workerBanner.querySelector('[data-worker-answer="approve"]'), PUSH), 30000);
    await watchCalls(page, keys);
    const tapped = await page.evaluate(({ worker, screen }) => new Promise((resolve) => {
      const w = tabByKey(worker);
      w.term.write(screen, () => {
        const drawn = workerBanner.querySelector(".cmd").textContent;
        workerBanner.querySelector('[data-worker-answer="approve"]').click();
        resolve({ drawn, onScreen: terminalMenu(w)?.box });
      });
    }), { ...keys, screen: RM_SCREEN });
    check(tapped.drawn === PUSH && tapped.onScreen === "rm -rf build", `Approve was tapped on the push while the screen already asked for ${tapped.onScreen}`);
    await page.waitForTimeout(1500);
    check(!(await callsTo(page, keys, "write_input")).length, "no key went to the worker");
    const banner = await page.evaluate(() => workerBanner.innerText.replace(/\s+/g, " ").trim());
    check(banner.includes("rm -rf build"), `the banner repainted with the command now asked: "${banner}"`);
  } finally {
    await context.close();
  }
}

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  const ctx = { hub: HUB, dir: DIR, waitFor, warnings: [] };
  ctx.warn = (m) => ctx.warnings.push(m);
  try {
    // Seeding opens every thread it creates, so the proof runs in a page load of its own.
    const seeding = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const seedPage = await seeding.newPage();
    await login(seedPage);
    await world.ensure(seedPage, ctx);
    await seeding.close();

    const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await login(page);
    await world.ensure(page, ctx);
    const keys = { captain: ctx.threads.release, worker: ctx.threads.readme };
    await page.evaluate(({ captain, draft }) => {
      window.__modelChanges = 0;
      onModelChange(() => window.__modelChanges++);
      const tab = tabByKey(captain);
      activate(tab);
      prompt.value = draft;
      saveDraft(tab, draft);
    }, { ...keys, draft: DRAFT });
    await waitFor("the worker's menu on its hidden terminal", () => page.evaluate((k) => tabByKey(k).menu?.box, keys.worker), 20000).catch(() => null);
    const before = await page.evaluate(snapshot, keys);
    check(before.active === keys.captain && before.prompt === DRAFT, "the Captain is open with a draft");
    check(before.workerHidden && before.workerSize === "120x32", `the worker was never shown in this page: hidden, unfitted at ${before.workerSize}`);
    const parsed = await page.evaluate(({ captain, worker }) => {
      const w = tabByKey(worker);
      return { menu: w.menu, ask: pendingAsk(w), row: workersOf(tabByKey(captain)).find((r) => r.tab?.key === worker), needs: needsYouItems().find((i) => i.tab.key === worker)?.kind };
    }, keys);
    console.log(`  hidden terminal read: box ${JSON.stringify(parsed.menu?.box)}, options ${JSON.stringify(parsed.menu?.options.map((o) => o.head))}`);
    check(parsed.menu?.box === PUSH, "the worker's push menu parses off its hidden terminal");
    check(parsed.ask?.kind === "approval" && parsed.ask.command === PUSH, `pendingAsk reads an approval for ${PUSH}`);
    check(JSON.stringify(parsed.ask.choices.map((c) => c.choice)) === JSON.stringify(["approve", "decline", "always"]) && parsed.ask.typed, "the ask offers Approve, Decline, Always allow and a typed decline");
    check(parsed.row?.state === "needs" && parsed.row.ask?.command === PUSH, "the Captain's worker row needs you with that ask");
    check(parsed.needs === "approval", "Needs you lists the worker as an approval");

    await proveAnswered(page, keys, await answer(page, keys, "approve", before), "chose 1", "Approve");
    await rearm(page, keys);
    await proveAnswered(page, keys, await answer(page, keys, "decline", before), "chose 3", "Decline");
    await rearm(page, keys);
    const reason = "Push to a fork instead";
    await proveAnswered(page, keys, await answer(page, keys, { text: reason }, before), `declined: ${reason}`, "Decline with a reason");
    await rearm(page, keys);

    await proveAnswered(page, keys, await pressBanner(page, keys, '[data-worker-answer="approve"]', before), "chose 1", "the banner's Approve");
    await waitFor("the banner to go once the worker went on", () => page.evaluate(() => workerBanner.hidden), 10000);
    check(true, "the banner went away with nothing left to answer");
    await rearm(page, keys);
    await proveAnswered(page, keys, await pressBanner(page, keys, '[data-worker-answer="decline"]', before), "chose 3", "the banner's Decline");
    await rearm(page, keys);
    const opened = await pressBanner(page, keys, "[data-worker-open]", before);
    check(opened.active === keys.worker, `the banner's Open opened the worker (${opened.active})`);
    check(await page.evaluate(() => approval.dataset.kind === "approval" && !approval.hidden && approval.querySelector("pre")?.textContent === "git push origin readme-tidy"), "the worker's own card shows its push");
    await page.evaluate((k) => activate(tabByKey(k)), keys.captain);

    await doubleTap(page, keys);
    await rearm(page, keys);
    await messageWaiting(page, keys);
    await stalePaint(browser, keys);

    const title = await page.evaluate((k) => threadTitle(tabByKey(k)), keys.worker);
    check(title === world.WORLD.threads.readme.title, `the README worker waits on its push again under its title "${title}", as the boards show it`);
    check(!errors.length, `no page errors${errors.length ? `: ${errors.join("; ")}` : ""}`);
    if (ctx.warnings.length) console.log(`warnings: ${ctx.warnings.join("; ")}`);
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
