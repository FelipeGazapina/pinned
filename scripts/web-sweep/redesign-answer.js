#!/usr/bin/env node
// Answers a waiting worker from its Captain's thread with answerAsk (ui/redesign-model.js), while
// the worker was never opened in the page: its menu is read off a hidden terminal that was never
// fitted. Approve, Decline, and Decline with a reason each reach the worker's session, the worker
// goes on, and the Captain's draft, composer and card stay as they were. The README worker is put
// back to waiting on its push at the end, so the redesign world stays as the boards show it.
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
  const sent = await page.evaluate(({ worker, choice }) => answerAsk(tabByKey(worker), choice), { worker: keys.worker, choice });
  check(sent === true, `answerAsk(${JSON.stringify(choice)}) sent the answer`);
  const after = await page.evaluate(snapshot, keys);
  for (const field of ["active", "composerTab", "prompt", "draft", "approvalHidden", "approvalKey", "threadPlace"]) {
    check(after[field] === before[field], `the Captain's ${field} is unchanged after ${JSON.stringify(choice)} (${JSON.stringify(after[field])})`);
  }
  check(after.workerHidden && after.workerSize === before.workerSize, `the worker's terminal stayed hidden at ${after.workerSize}`);
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
