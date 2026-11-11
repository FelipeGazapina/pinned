#!/usr/bin/env node
// Presses every control of the phone thread screen (ui/gui-phone-thread.js) with touch, at 390x844
// with an iPhone's safe-area insets: the Captain's Workers pill, banner and Workers sheet (Approve,
// Message all, Stop the running one, a row that opens its worker, back), the README worker's
// approval sheet (Show in terminal, its card, Decline by typing, Approve, Always allow, Decline), a
// question's sheet, the hero thread's actions sheet (each primary action, tile and action), the
// composer chip's pickers, and Send. It checks that each sheet closes when another screen takes its
// place and the ask comes back with its thread, that cancelling the palette that New thread or Add
// project opened goes back to the thread list, and that the owner's password prompt shows and
// unlocks over every phone screen. Then it shoots each sheet at 360x740 into $SWEEP_DIR/phone-360.
// Each answer goes to the worker's real session; the README worker is put back to waiting on its
// push and the CI worker back to running, so the redesign world stays as the boards show it. The
// git action, the script, archive and delete are watched, not run, so the world keeps its tree.
//
//   export SWEEP_DIR=... SWEEP_HUB_PORT=... SWEEP_PG_PORT=... SWEEP_GO_PORT=...
//   export CARGO=true CARGO_TARGET_DIR=<a checkout's target>
//   scripts/web-sweep/stack.sh start
//   PLAYWRIGHT=<dir>/node_modules/playwright node scripts/web-sweep/redesign-phone-thread.js
//   scripts/web-sweep/stack.sh stop
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");
const world = require("./redesign/world");
const { phoneInsets } = require("./redesign/phone-thread");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const T = world.WORLD.threads;
const PUSH = "git push origin readme-tidy";
const README_BRIEF = "Tidy the README. Fix the install steps for Windows, drop the two stale badges, and keep every heading that the docs link to.";
const CI_TASK = "Run the CI on main. Report what fails.";
const QUESTION = { prompt: "Pick the phone hero image.", question: { header: "Hero", question: "Which hero image should the phone page use?", options: [["The product screenshot", "Already in the repo"], ["The team photo", "Needs a crop"]] } };
const QUESTION_TITLE = "Pick the phone hero";

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

const keyOf = (page, title) => waitFor(`the thread ${title}`, () => page.evaluate((title) => tabs.find((t) => threadTitle(t) === title)?.key, title));
async function open(page, key) {
  await page.evaluate((key) => {
    activate(tabByKey(key));
    toggleSidebar(false);
    document.activeElement?.blur();
  }, key);
  await waitFor("the thread", () => page.evaluate((key) => active?.key === key, key));
  await page.waitForTimeout(400);
}
const text = (page, selector) => page.evaluate((s) => document.querySelector(s)?.innerText.replace(/\s+/g, " ").trim() ?? null, selector);
const screenHas = (page, key, words) => waitFor(`"${words}" on ${key}'s screen`, () => page.evaluate(({ key, words }) => screenLines(tabByKey(key)).some((l) => l.includes(words)), { key, words }), 15000);
const say = (page, key, words) => page.evaluate(({ key, words }) => { const tab = tabByKey(key); return callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text: words }); }, { key, words });
const sheetOpen = (page, cls) => waitFor(`the ${cls}`, () => page.evaluate((cls) => !!document.querySelector(`.sheet-panel.${cls}`), cls));
const sheetGone = (page) => waitFor("the sheet to close", () => page.evaluate(() => !document.querySelector(".sheet-panel")));

const readmeAsks = (page, readme) => page.evaluate(({ k, push }) => tabByKey(k).menu?.box === push && waitingKindOf(tabByKey(k).view) === "needs_input", { k: readme, push: PUSH });
/** Puts the README worker back to waiting on its push. The machine titles a thread after its
 * latest prompt, so the prompt leads with the worker's title, as its first one did. */
async function rearm(page, readme) {
  await waitFor("the README worker at its prompt", () => page.evaluate((k) => !tabByKey(k).menu && /^>/.test(screenLines(tabByKey(k)).filter((l) => l.trim()).at(-1) ?? ""), readme), 20000);
  await say(page, readme, `${T.readme.title}\n\n${README_BRIEF}`);
  await waitFor("the README worker to ask for the push again", () => readmeAsks(page, readme), 30000);
}

/** The Captain's workers as the boards show them: their titles, the CI worker running its tests
 * and the README worker waiting on its push. */
async function restoreWorkers(page, keys) {
  await page.evaluate((titles) => {
    for (const [key, title] of titles) changeMarks(threadKey(tabByKey(key)), () => (sideMarks.titles[threadKey(tabByKey(key))] = title));
    renderSidebar();
  }, [[keys.ci, T.ci.title], [keys.readme, T.readme.title], [keys.changelog, T.changelog.title]]);
  const ciRuns = () => page.evaluate(({ release, ci }) => workersOf(tabByKey(release)).some((r) => r.key === ci && r.state === "working"), keys);
  if (!(await waitFor("the CI worker's run on this page", ciRuns, 8000).catch(() => false))) {
    await waitFor("the CI worker to finish", () => page.evaluate((k) => statusOf(tabByKey(k)) !== "working", keys.ci), 20000);
    await say(page, keys.ci, `${T.ci.title}\n\n${CI_TASK}`);
    await waitFor("the CI worker running again", ciRuns, 20000);
  }
  const asks = await waitFor("the README worker's ask on this page", () => readmeAsks(page, keys.readme), 8000).catch(() => false);
  if (!asks) await rearm(page, keys.readme);
}

async function workersSheet(page, keys) {
  await restoreWorkers(page, keys);
  await open(page, keys.release);
  await page.evaluate(() => threadView.scrollTo({ top: 0 }));
  const pill = await waitFor("the Workers pill", () => text(page, "#workers-pill"));
  check(/^3 workers 1 working 1 needs you$/.test(pill), `the pill reads "${pill}"`);
  check((await text(page, "#worker-banner")) === "Tidy the README needs approval Review", "the banner reads \"Tidy the README needs approval Review\"");
  const rowTap = await page.evaluate(() => [...threadView.querySelectorAll(".worker-row")].map((r) => r.querySelector(".worker-title").textContent));
  check(rowTap.length === 3, `the chat lists ${rowTap.join(", ")}`);

  await page.tap("#worker-banner [data-worker-review]");
  await sheetOpen(page, "workers-sheet");
  check((await text(page, ".workers-sheet .ws-count")) === "1 of 3 done", "the Workers sheet counts 1 of 3 done");
  const card = await text(page, ".workers-sheet .ws-card");
  check(card.startsWith("Tidy the README Needs your approval · waiting") && card.includes(PUSH) && card.endsWith("Decline Approve"), `Review opens the sheet on the README worker's card: "${card}"`);
  check((await text(page, ".workers-sheet [data-ws=stop]")) === "Stop the running one", "the sheet offers Stop the running one");

  await page.tap(".workers-sheet [data-ws-answer=approve]");
  await screenHas(page, keys.readme, "chose 1");
  check(true, "Approve in the Workers sheet chose Yes on the README worker's menu");
  await waitFor("the sheet to repaint without the card", () => page.evaluate(() => !document.querySelector(".workers-sheet .ws-card") && document.querySelector(".workers-sheet .ws-count")?.textContent === "2 of 3 done"), 20000);
  check(true, "the open sheet repainted: no waiting card, 2 of 3 done");

  await page.tap(".workers-sheet [data-ws=stop]");
  await screenHas(page, keys.ci, "[esc]");
  check(await page.evaluate((k) => screenLines(tabByKey(k)).some((l) => l.includes("[ctrl-x]")), keys.ci), "Stop the running one sent the CI worker the desktop Cancel's keys");

  await page.tap(".workers-sheet [data-ws=message]");
  await page.fill(".workers-sheet .ws-say input", "Status, please.");
  await page.tap(".workers-sheet .ws-say button");
  for (const key of [keys.ci, keys.readme, keys.changelog]) await screenHas(page, key, "Status, please.");
  check(true, "Message all typed the message into each of the three workers");

  await page.tap(".workers-sheet [data-ws-open]");
  const opened = await waitFor("a worker thread", () => page.evaluate(() => active?.view.role === "worker" && active.key));
  check(await page.evaluate(() => !document.querySelector(".workers-sheet")), "a row closed the sheet");
  check((await text(page, "#head-sub")) === `Worker of ${T.release.title}`, "the worker's header names its Captain");
  check((await page.getAttribute("#chat-back", "aria-label")) === `Back to ${T.release.title}`, "its back button goes to the Captain");
  await page.tap("#chat-back");
  await waitFor("the Captain", () => page.evaluate((k) => active?.key === k, keys.release));
  check(opened !== keys.release, "back from the worker opened the Captain");
  await page.tap("#chat-back");
  const back = await waitFor("the threads", () => page.evaluate(() => (typeof phoneGo === "function" ? document.body.dataset.phoneScreen === "threads" : document.body.dataset.sidebar === "open") && "ok"));
  check(back === "ok", "back from a Captain goes to the threads");
  await page.evaluate(() => toggleSidebar(false));
  await restoreWorkers(page, keys);
}

async function approvalSheet(page, keys) {
  await open(page, keys.readme);
  await sheetOpen(page, "ask-sheet");
  const sheet = await page.evaluate(() => ({
    title: document.querySelector(".ask-sheet h2").textContent,
    command: document.querySelector(".ask-sheet pre").textContent,
    about: document.querySelector(".ask-sheet .ask-about")?.textContent,
    answers: [...document.querySelectorAll(".ask-sheet .ask-answers button")].map((b) => b.textContent),
    rows: [...document.querySelectorAll(".ask-sheet .ask-row")].map((b) => b.textContent.trim()),
    approvalHidden: getComputedStyle(approval).display === "none",
  }));
  check(sheet.title === "Allow this command?" && sheet.command === PUSH, `the sheet asks "${sheet.title}" for ${sheet.command}`);
  check(/Pushes one commit/.test(sheet.about ?? ""), "the sheet carries the CLI's description");
  check(JSON.stringify(sheet.answers) === '["Decline","Approve"]', "the sheet answers Decline and Approve");
  check(JSON.stringify(sheet.rows) === JSON.stringify(["Yes, and don't ask again for rm commands", "Decline and say what to do instead", "Show in terminal"]), `the sheet's rows: ${sheet.rows.join(" | ")}`);
  check(sheet.approvalHidden, "the approval card gives way to the sheet");

  await page.tap(".ask-sheet [data-ask=terminal]");
  await sheetGone(page);
  check(await page.evaluate(() => ui.drawer && !document.getElementById("ask-card").hidden), "Show in terminal opened the drawer and left the card");
  await page.evaluate(() => toggleDrawer(false));
  check((await text(page, "#ask-card")) === `Allow ${PUSH}? Review`, "the card says what waits");
  await page.tap("#ask-card [data-ask=review]");
  await sheetOpen(page, "ask-sheet");
  await page.touchscreen.tap(195, 200);
  await sheetGone(page);
  check(await page.evaluate(() => !document.getElementById("ask-card").hidden), "a tap outside closed the sheet to its card");
  await page.tap("#ask-card [data-ask=review]");
  await sheetOpen(page, "ask-sheet");

  await page.tap(".ask-sheet [data-ask=type]");
  await sheetGone(page);
  const typing = await page.evaluate(() => ({ focused: document.activeElement === prompt, placeholder: prompt.placeholder }));
  check(typing.focused && /decline/i.test(typing.placeholder), `Decline and say focused the prompt: "${typing.placeholder}"`);
  await page.fill("#prompt", "Push to the docs branch instead.");
  await page.tap("#send");
  await screenHas(page, keys.readme, "declined: Push to the docs branch instead.");
  check(true, "Send declined the push with the typed words");
  await rearm(page, keys.readme);

  for (const [selector, seen, what] of [[".ask-sheet [data-approval=accept]", "chose 1", "Approve"], [".ask-sheet [data-approval=always]", "chose 2", "Always allow"], [".ask-sheet [data-approval=decline]", "chose 3", "Decline"]]) {
    await sheetOpen(page, "ask-sheet");
    await page.tap(selector);
    await screenHas(page, keys.readme, seen);
    check(true, `${what} in the sheet answered the README worker`);
    await sheetGone(page);
    await rearm(page, keys.readme);
  }
}

async function questionSheet(page, ctx, keys) {
  const machine = ctx.machines[T.hero.machine];
  const file = path.join(world.homeOf(ctx, T.hero.machine), "fake-turns.json");
  const turns = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : [];
  if (!turns.some((t) => t.prompt === QUESTION.prompt)) fs.writeFileSync(file, JSON.stringify([QUESTION, ...turns], null, 1));
  let key = await page.evaluate((title) => tabs.find((t) => threadTitle(t) === title)?.key ?? null, QUESTION_TITLE);
  if (!key) {
    const cwd = fs.realpathSync(path.join(world.homeOf(ctx, T.hero.machine), "work", "site"));
    key = await page.evaluate(async ({ machine, cwd, title }) => {
      const tab = await revealTab(machine, await callOn(machine, { cmd: "open_tab", path: cwd, bypass: false }));
      startTab(tab);
      changeMarks(threadKey(tab), () => (sideMarks.titles[threadKey(tab)] = title));
      return tab.key;
    }, { machine, cwd, title: QUESTION_TITLE });
    await waitFor("the question thread to start", () => page.evaluate((k) => tabByKey(k)?.view.running === true, key));
  }
  if (!(await page.evaluate((k) => !!tabByKey(k).menu, key))) await say(page, key, QUESTION.prompt);
  await waitFor("the question on its screen", () => page.evaluate((k) => !!tabByKey(k).menu, key), 30000);
  await open(page, key);
  await sheetOpen(page, "ask-sheet");
  const sheet = await page.evaluate(() => ({
    title: document.querySelector(".ask-sheet h2").textContent,
    question: document.querySelector(".ask-sheet .ask-question")?.textContent,
    options: [...document.querySelectorAll(".ask-sheet .ask-option .label")].map((b) => b.textContent),
    rows: [...document.querySelectorAll(".ask-sheet .ask-row")].map((b) => b.textContent.trim()),
  }));
  check(sheet.title === "Hero" && sheet.question === QUESTION.question.question, `the question's sheet asks "${sheet.question}" under "${sheet.title}"`);
  check(JSON.stringify(sheet.options) === JSON.stringify([...QUESTION.question.options.map((o) => o[0]), "Chat about this"]), `its options, claude's own last: ${sheet.options.join(" | ")}`);
  check(sheet.rows.includes("Type your own answer") && sheet.rows.includes("Dismiss without answering") && sheet.rows.includes("Show in terminal"), `its rows: ${sheet.rows.join(" | ")}`);
  await page.tap(".ask-sheet .ask-option[data-option='1']");
  await waitFor("the question answered", () => page.evaluate((k) => !tabByKey(k).menu, key), 15000);
  await sheetGone(page);
  check(true, "an option in the question's sheet answered it");
}

async function actionsSheet(page, keys) {
  await page.evaluate((k) => {
    const tab = tabByKey(k);
    if (settled(tab)) unsettle(tab);
    if (isPinned(tab)) pin(tab);
    if (snoozedNow(tab)) wake(tab);
  }, keys.hero);
  await open(page, keys.hero);
  await page.evaluate(() => {
    window.__calls = [];
    for (const name of ["runGitAction", "runScript", "archiveThread", "deleteThreads", "copyAndTell"]) window[name] = (...args) => window.__calls.push([name, args[1]?.name ?? args[1] ?? null]);
  });
  const calls = () => page.evaluate(() => window.__calls.splice(0));
  const sheet = async () => {
    await page.tap("#head-more");
    await sheetOpen(page, "actions-sheet");
  };
  const tap = async (selector) => {
    await sheet();
    await page.tap(selector);
  };
  const surface = (kind) => waitFor(`the ${kind} surface`, () => page.evaluate((kind) => panelOf(active).open && shownSurface(active)?.kind === kind, kind));
  const closePanelNow = () => page.evaluate(() => closePanel());

  await sheet();
  const primary = await page.evaluate(() => [...document.querySelectorAll(".actions-sheet .act-primary")].map((b) => b.textContent));
  check(primary.length === 3 && primary[1] === "Run dev" && primary[2] === "PR #12", `the primary row: ${primary.join(" | ")}`);
  const actions = await page.evaluate(() => [...document.querySelectorAll(".actions-sheet .act-main .act-row")].map((b) => b.querySelector(".label").textContent));
  const movable = await page.evaluate(() => switchItems(active).length > 0);
  const listed = ["Pin thread", "Settle", "Snooze", "Rename, mark unread, copy", ...(movable ? ["Move to account"] : []), "Archive or delete"];
  check(JSON.stringify(actions) === JSON.stringify(listed), `the actions: ${actions.join(" | ")}`);
  await page.tap(".actions-sheet .act-primary[data-act=git]");
  check(JSON.stringify(await calls()) === '[["runGitAction","commit_push"]]', "the git action runs runGitAction as the header's does");
  await tap(".actions-sheet .act-primary[data-act=script]");
  check(JSON.stringify(await calls()) === '[["runScript","dev"]]', "Run dev runs the project's dev script");
  await tap(".actions-sheet .act-primary[data-act=pr]");
  await surface("pr");
  check(true, "PR #12 opens the pull request beside the thread");
  await closePanelNow();

  for (const [id, kind] of [["diff", "diff"], ["browser", "browser"], ["files", "files"], ["pr", "pr"], ["memory", "memory"], ["device", "device"]]) {
    await tap(`.actions-sheet .act-tile[data-act=${id}]`);
    await surface(kind);
    check(true, `the ${id} tile opens the ${kind} surface`);
    // A machine's first device opens its setup, which Cancel leaves.
    if (await page.isVisible("#device-setup [data-setup-back]")) await page.tap("#device-setup [data-setup-back]");
    await closePanelNow();
  }
  await tap(".actions-sheet .act-tile[data-act=terminal]");
  check(await page.evaluate(() => ui.drawer), "the Terminal tile shows the terminal");
  await tap(".actions-sheet .act-tile[data-act=terminal]");
  check(await page.evaluate(() => !ui.drawer), "the Terminal tile hides it again");
  await tap(".actions-sheet .act-tile[data-act=details]");
  check(await page.evaluate(() => !detailsCard.hidden), "the Details tile opens Thread details");
  await page.evaluate(() => toggleDetails(false));

  await tap(".actions-sheet .act-row[data-act=pin]");
  check(await page.evaluate(() => isPinned(active)), "Pin thread pins it");
  await tap(".actions-sheet .act-row[data-act=pin]");
  check(await page.evaluate(() => !isPinned(active)), "Unpin thread unpins it");
  // Settling or snoozing the open thread moves on to another, as the row menu's do.
  const hero = (test) => page.evaluate(({ k, test }) => ({ settled: settled(tabByKey(k)), snoozed: snoozedNow(tabByKey(k)) })[test], { k: keys.hero, test });
  await tap(".actions-sheet .act-row[data-act=settle]");
  check(await hero("settled"), "Settle settles it");
  await open(page, keys.hero);
  await tap(".actions-sheet .act-row[data-act=settle]");
  check(!(await hero("settled")), "Un-settle brings it back");
  await open(page, keys.hero);
  await tap(".actions-sheet .act-row[data-act=snooze]");
  await page.tap(".actions-sheet .act-sub [data-item='0']");
  check(await hero("snoozed"), "Snooze's first choice snoozes it");
  await open(page, keys.hero);
  await tap(".actions-sheet .act-row[data-act=snooze]");
  check(!(await hero("snoozed")), "Wake thread wakes it");
  await open(page, keys.hero);
  await tap(".actions-sheet .act-row[data-act=more]");
  const more = await page.evaluate(() => [...document.querySelectorAll(".actions-sheet .act-sub .act-row .label")].map((b) => b.textContent));
  check(more.includes("Rename thread") && more.includes("Mark unread") && more.includes("Copy"), `More lists ${more.join(" | ")}`);
  await page.tap(".actions-sheet .act-sub [data-back]");
  check(await page.evaluate(() => !document.querySelector(".actions-sheet .act-main").hidden), "Back returns to the actions");
  await page.tap(".actions-sheet .act-row[data-act=more]");
  await page.evaluate(() => document.querySelector(".actions-sheet .act-sub").querySelectorAll(".act-row").forEach((b) => b.querySelector(".label").textContent === "Rename thread" && b.setAttribute("data-pick", "")));
  await page.tap(".actions-sheet .act-sub [data-pick]");
  check(await page.evaluate(() => !!document.querySelector("#chat-head input.rename")), "Rename thread edits the title in the header");
  await page.keyboard.press("Escape");
  await tap(".actions-sheet .act-row[data-act=more]");
  await page.evaluate(() => document.querySelectorAll(".actions-sheet .act-sub .act-row").forEach((b) => b.querySelector(".label").textContent === "Mark unread" && b.setAttribute("data-pick", "")));
  await page.tap(".actions-sheet .act-sub [data-pick]");
  check(await page.evaluate(() => (seenAt[threadKey(active)] ?? 0) === 0), "Mark unread marks it unread");
  await tap(".actions-sheet .act-row[data-act=more]");
  await page.evaluate(() => document.querySelectorAll(".actions-sheet .act-sub .act-row").forEach((b) => b.querySelector(".label").textContent === "Copy" && b.setAttribute("data-pick", "")));
  await page.tap(".actions-sheet .act-sub [data-pick]");
  await page.tap(".actions-sheet .act-sub [data-item='0']");
  check(JSON.stringify(await calls()) === JSON.stringify([["copyAndTell", "path"]]), "Copy › Path copies the thread's path");
  await tap(".actions-sheet .act-row[data-act=archive]");
  const archive = await page.evaluate(() => [...document.querySelectorAll(".actions-sheet .act-sub .act-row .label")].map((b) => b.textContent));
  check(JSON.stringify(archive) === '["Archive thread","Delete"]', "Archive or delete lists Archive thread and Delete");
  await page.tap(".actions-sheet .act-sub [data-item='0']");
  await tap(".actions-sheet .act-row[data-act=archive]");
  await page.tap(".actions-sheet .act-sub [data-item='1']");
  const gone = (await calls()).map((c) => c[0]);
  check(JSON.stringify(gone) === '["archiveThread","deleteThreads"]', "Archive thread and Delete run the row menu's own functions");
  await page.evaluate(() => (seenAt[threadKey(active)] = lastActivity(active) + 1, storeMarks("gui-seen")));
}

async function chip(page, keys) {
  await open(page, keys.release);
  const chipText = await text(page, "#composer-chip");
  check(chipText === "Opus 5.5 · High · Auto", `the chip reads "${chipText}"`);
  const menu = async () => {
    await page.tap("#composer-chip");
    await waitFor("the chip's menu", () => page.evaluate(() => !composerMenu.hidden && composerMenu.dataset.kind === "chip"));
    return page.evaluate(() => [...composerMenu.querySelectorAll("button .label")].map((b) => b.textContent));
  };
  const pick = async (label, kind) => {
    const rows = await menu();
    await page.evaluate((label) => [...composerMenu.querySelectorAll("button")].find((b) => b.querySelector(".label").textContent === label).setAttribute("data-pick", ""), label);
    await page.tap("#composer-menu [data-pick]");
    if (kind) await waitFor(`the ${kind} picker`, () => page.evaluate((kind) => !composerMenu.hidden && composerMenu.dataset.kind === kind, kind));
    return rows;
  };
  const rows = await pick("Model", "models");
  check(JSON.stringify(rows) === JSON.stringify(["Model", "Reasoning", "Access", "Plan mode", "Dictation language"]), `the chip's menu: ${rows.join(" | ")}`);
  check(true, "Model opens the model picker");
  await page.evaluate(() => (composerMenu.hidden = true));
  await pick("Reasoning", "traits");
  check(await page.evaluate(() => [...composerMenu.querySelectorAll("button.checked .label")].some((b) => b.textContent === "High")), "Reasoning opens the effort picker on High");
  await page.evaluate(() => (composerMenu.hidden = true));
  await pick("Access", "access");
  check(await page.evaluate(() => [...composerMenu.querySelectorAll("button.checked .label")].some((b) => b.textContent === "Auto")), "Access opens the access picker on Auto");
  await page.evaluate(() => (composerMenu.hidden = true));
  await pick("Plan mode");
  await waitFor("plan mode", () => page.evaluate(() => terminalMode(active) === "plan"), 20000);
  check(await waitFor("the chip in Plan", async () => (await text(page, "#composer-chip")) === "Opus 5.5 · High · Plan"), "Plan mode switches claude to plan, and the chip says so");
  await pick("Plan mode");
  await waitFor("auto mode", () => page.evaluate(() => terminalMode(active) === "auto"), 20000);
  check(true, "Plan mode again goes back to Auto");
  const lang = await page.evaluate(() => document.getElementById("composer-dictate-lang").textContent);
  await pick("Dictation language");
  const switched = await page.evaluate(() => document.getElementById("composer-dictate-lang").textContent);
  check(switched !== lang, `Dictation language switched ${lang} to ${switched}`);
  await page.evaluate(() => document.getElementById("composer-dictate-lang").click());
}

async function send(page, keys) {
  await open(page, keys.updater);
  await page.tap("#prompt");
  await page.fill("#prompt", "Phone send check");
  await page.tap("#send");
  await screenHas(page, keys.updater, "Phone send check");
  check(await page.evaluate(() => prompt.value === ""), "Send sent the prompt and emptied the box");
  // The machine titles a thread after its latest prompt; the world finds this one by its title.
  await page.evaluate(({ k, title }) => changeMarks(threadKey(tabByKey(k)), () => (sideMarks.titles[threadKey(tabByKey(k))] = title)), { k: keys.updater, title: T.updater.title });
}

const screenIs = (page, screen) => waitFor(`the ${screen} screen`, () => page.evaluate((screen) => document.body.dataset.phoneScreen === screen, screen));

/** Each sheet belongs to its thread's screen: another screen closes it, and the thread's ask comes
 * back with its screen. */
async function sheetsStayOnTheirScreen(page, keys) {
  await restoreWorkers(page, keys);
  await open(page, keys.readme);
  await sheetOpen(page, "ask-sheet");
  for (const screen of ["needs-you", "more", "threads", "orchestrator"]) {
    await page.evaluate((screen) => phoneGo(screen), screen);
    await screenIs(page, screen);
    await sheetGone(page);
    check(true, `the approval sheet closed on the ${screen} screen`);
    await open(page, keys.readme);
    await sheetOpen(page, "ask-sheet");
    check(true, `and rose again back on the worker's screen from ${screen}`);
  }
  await page.evaluate(() => openedSheet.close());

  await open(page, keys.release);
  await page.tap("#workers-pill");
  await sheetOpen(page, "workers-sheet");
  await page.evaluate(() => phoneGo("needs-you"));
  await sheetGone(page);
  check(true, "the Workers sheet closed when Needs you took the screen");

  await open(page, keys.hero);
  await page.tap("#head-more");
  await sheetOpen(page, "actions-sheet");
  await page.evaluate(() => phoneGo("threads"));
  await sheetGone(page);
  check(true, "the actions sheet closed when the thread list took the screen");
}

/** New thread and Add project open the palette from the thread list; cancelling it goes back there,
 * with the tab bar and nothing focused, whatever thread was open before. */
async function paletteCancel(page, keys) {
  await open(page, keys.release);
  const cancels = [
    ["New thread", "#phone-new", "a tap beside it", () => page.touchscreen.tap(195, 820)],
    ["New thread", "#phone-new", "Escape", () => page.keyboard.press("Escape")],
    ["Add project", "#phone-chips [data-add-project]", "a tap beside it", () => page.touchscreen.tap(195, 820)],
  ];
  for (const [what, opener, how, cancel] of cancels) {
    await page.evaluate(() => phoneGo("threads"));
    await screenIs(page, "threads");
    await page.tap(opener);
    await waitFor("the palette", () => page.evaluate(() => paletteOpen()));
    await cancel();
    await waitFor("the palette to close", () => page.evaluate(() => !paletteOpen()));
    const back = await waitFor("the thread list", () => page.evaluate(() => document.body.dataset.phoneScreen === "threads" && document.body.hasAttribute("data-phone-tabs") && document.activeElement === document.body && "ok"));
    check(back === "ok", `${what}, cancelled by ${how}, went back to the thread list with its tab bar`);
  }
}

/** The owner's password prompt that an owner-only call opens shows over each phone screen and
 * unlocks from there. Each screen gets a new page, which starts locked. */
async function ownerPrompt(browser, keys) {
  const opens = { threads: () => phoneGo("threads"), "needs-you": () => phoneGo("needs-you"), more: () => phoneGo("more"), orchestrator: () => phoneGo("orchestrator"), thread: (key) => (activate(tabByKey(key)), toggleSidebar(false)) };
  for (const [screen, go] of Object.entries(opens)) {
    const context = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, colorScheme: "dark", isMobile: true, hasTouch: true });
    const page = await context.newPage();
    try {
      await login(page);
      await phoneInsets(page);
      await waitFor("the threads", () => page.evaluate((key) => !!tabByKey(key), keys.updater));
      await page.evaluate(`(${go})(${JSON.stringify(keys.updater)})`);
      await screenIs(page, screen);
      await page.evaluate(() => {
        window.__owner = hub.hub({ cmd: "revoke_invite", id: "no-such-invite" }).then(() => "ok", (e) => String(e));
      });
      await waitFor("the password prompt", () => page.evaluate(() => document.querySelector("#side-dialog[open] [name=password]")));
      const shown = await page.evaluate(() => {
        const dialog = document.getElementById("side-dialog");
        const r = dialog.getBoundingClientRect();
        const hit = document.elementFromPoint(r.left + r.width / 2, r.top + 20);
        return r.width > 0 && r.left >= 0 && r.right <= innerWidth && r.top >= 0 && r.bottom <= innerHeight && dialog.contains(hit);
      });
      check(shown, `the password prompt shows whole and on top over the ${screen} screen`);
      await page.tap("#side-dialog [name=password]");
      await page.keyboard.type(world.PASSWORD);
      await page.tap("#side-dialog button.primary");
      const answer = await waitFor("the owner-only call's answer", () => page.evaluate(() => window.__owner && Promise.race([window.__owner, new Promise((r) => setTimeout(() => r(null), 200))])));
      check(!/password/.test(answer) && (await page.evaluate(() => !document.querySelector("#side-dialog[open]"))), `it unlocked from the ${screen} screen, and the hub answered the call: ${answer}`);
    } finally {
      await context.close();
    }
  }
}

async function shoot360(browser, ctx, keys) {
  const out = path.join(DIR, "phone-360");
  fs.mkdirSync(out, { recursive: true });
  const context = await browser.newContext({ viewport: { width: 360, height: 740 }, deviceScaleFactor: 2, colorScheme: "dark", isMobile: true, hasTouch: true });
  const page = await context.newPage();
  try {
    await login(page);
    await phoneInsets(page);
    const overflow = () => page.evaluate(() => document.documentElement.scrollWidth > innerWidth || [...document.querySelectorAll(".sheet-panel *, #chat-head *, #composer *, #workers-pill *")].some((el) => el.offsetParent && el.getBoundingClientRect().right > innerWidth + 0.5));
    await open(page, keys.release);
    await page.evaluate(() => threadView.scrollTo({ top: 0 }));
    await page.screenshot({ path: path.join(out, "thread.png") });
    check(!(await overflow()), "nothing runs past 360px on the thread");
    await page.tap("#workers-pill");
    await sheetOpen(page, "workers-sheet");
    await page.screenshot({ path: path.join(out, "workers.png") });
    check(!(await overflow()), "nothing runs past 360px in the Workers sheet");
    await page.evaluate(() => openedSheet.close());
    await open(page, keys.readme);
    await sheetOpen(page, "ask-sheet");
    await page.screenshot({ path: path.join(out, "approval.png") });
    check(!(await overflow()), "nothing runs past 360px in the approval sheet");
    await page.evaluate(() => openedSheet.close());
    await open(page, keys.hero);
    await page.tap("#head-more");
    await sheetOpen(page, "actions-sheet");
    await page.waitForTimeout(1500);
    await page.screenshot({ path: path.join(out, "actions.png") });
    check(!(await overflow()), "nothing runs past 360px in the actions sheet");
    console.log(`  360x740 shots in ${out}`);
  } finally {
    await context.close();
  }
}

(async () => {
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: process.env.SWEEP_CHANNEL || "chrome" }) });
  const ctx = { hub: HUB, dir: DIR, waitFor, warnings: [] };
  ctx.warn = (m) => ctx.warnings.push(m);
  const errors = [];
  try {
    const context = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, colorScheme: "dark", isMobile: true, hasTouch: true });
    const page = await context.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    await login(page);
    await world.ensure(page, ctx);
    await require("./redesign/phone-thread").seed(page, ctx);
    await phoneInsets(page);
    const keys = {};
    for (const id of ["release", "ci", "readme", "changelog", "hero", "updater"]) keys[id] = await keyOf(page, T[id].title);
    await workersSheet(page, keys);
    await approvalSheet(page, keys);
    await questionSheet(page, ctx, keys);
    await actionsSheet(page, keys);
    await chip(page, keys);
    await send(page, keys);
    await sheetsStayOnTheirScreen(page, keys);
    await paletteCancel(page, keys);
    await context.close();
    await ownerPrompt(browser, keys);
    await shoot360(browser, ctx, keys);
    check(!errors.length, `no page errors${errors.length ? `: ${errors.join("; ")}` : ""}`);
  } catch (e) {
    console.error(e.stack || String(e));
    if (errors.length) console.error("page errors:", errors);
    process.exitCode = 1;
  } finally {
    await browser.close();
  }
})();
