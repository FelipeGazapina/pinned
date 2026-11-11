#!/usr/bin/env node
// Presses every control of the Needs you page (ui/gui-needs-you.js) on a phone-sized page and checks
// what each did to its thread: Decline and Approve reach the README worker's session, an option and
// a typed answer reach the site thread's question, Approve plan answers a plan, Snooze takes the
// usage limit off the page until it wakes, Continue under Work moves the api thread to that
// account, Deny and Approve decide an agent's request, and Open opens each kind of thread. No
// answer leaves the page, and the open thread's ask sheet shows only on that thread's screen. It
// also shoots the page at 1440x900 and 360x740.
//
//   export SWEEP_DIR=... SWEEP_HUB_PORT=... SWEEP_PG_PORT=... SWEEP_GO_PORT=...
//   export CARGO=true CARGO_TARGET_DIR=<a checkout's target>
//   scripts/web-sweep/stack.sh start
//   PLAYWRIGHT=<dir>/node_modules/playwright node scripts/web-sweep/redesign-needs-you.js
//   scripts/web-sweep/stack.sh stop
//
// The world comes from redesign/world.js and redesign/phone-inbox.js. The README worker and the
// question are put back to waiting at the end; the api thread stays under Work. Writes
// needs-you-<size>.png into $REDESIGN_OUT (default $SWEEP_DIR/redesign).
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawn } = require("child_process");
const playwright = require(process.env.PLAYWRIGHT || "playwright");
const world = require("./redesign/world");
const inbox = require("./redesign/phone-inbox");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const OUT = process.env.REDESIGN_OUT || path.join(DIR, "redesign");
const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname, "../../target"), "debug/pinned-server");
const README_BRIEF = "Tidy the README. Fix the install steps for Windows, drop the two stale badges, and keep every heading that the docs link to.";
const QUESTION_PROMPT = "Pick the hero image for the launch page.";

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};
const waitFor = async (what, fn, timeout = 30000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 150));
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

/** The socket of the stack's own node, whose pid `$DIR/node.pid` holds. */
function nodeSock() {
  const pid = fs.readFileSync(path.join(DIR, "node.pid"), "utf8").trim();
  const dir = fs.readdirSync(os.tmpdir()).filter((d) => d.startsWith(`pinned-${pid}-`)).sort((a, b) => fs.statSync(path.join(os.tmpdir(), b)).mtimeMs - fs.statSync(path.join(os.tmpdir(), a)).mtimeMs)[0];
  return path.join(os.tmpdir(), dir, "pinned.sock");
}

/** One agent `act` call from the session of `tab`, through `pinned-server mcp` as its claude runs it. */
function act(tab, args) {
  return new Promise((resolve, reject) => {
    const { PINNED_TASK, PINNED_ACCOUNT, PINNED_ROLE, CLAUDE_CONFIG_DIR, ...env } = process.env;
    const child = spawn(SERVER, ["mcp"], { env: { ...env, PINNED_SOCK: nodeSock(), PINNED_TAB: tab } });
    const requests = [
      { jsonrpc: "2.0", id: 0, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "sweep", version: "1" } } },
      { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "act", arguments: args } },
    ];
    let buffered = "";
    let sent = 0;
    child.stdout.on("data", (chunk) => {
      buffered += chunk;
      for (let at; (at = buffered.indexOf("\n")) >= 0; ) {
        const message = JSON.parse(buffered.slice(0, at));
        buffered = buffered.slice(at + 1);
        if (message.id === 1) {
          child.stdin.end();
          resolve(JSON.parse(message.result.content[0].text));
        } else child.stdin.write(`${JSON.stringify(requests[sent++])}\n`);
      }
    });
    child.on("error", reject);
    child.stdin.write(`${JSON.stringify(requests[sent++])}\n`);
  });
}

// Runs in the page: each card as `kind title: buttons`.
const cardsText = () => [...document.querySelectorAll("#page-view .ny-card")].map((c) => `${c.dataset.kind} ${c.querySelector(".ny-name").textContent}: ${[...c.querySelectorAll("button")].map((b) => b.textContent.trim()).join(" | ")}`);
const card = (page, key) => page.locator(`#page-view .ny-card[data-key="${key}"]`);
const screenHas = (page, key, text) => page.evaluate(({ key, text }) => screenLines(tabByKey(key)).some((l) => l.includes(text)), { key, text });
const onPage = (page) => page.evaluate(() => document.body.dataset.page === "needs-you");
const kindOf = (page, key) => page.evaluate((key) => document.querySelector(`#page-view .ny-card[data-key="${key}"]`)?.dataset.kind ?? null, key);

async function press(page, key, label, what) {
  await card(page, key).getByRole("button", { name: label, exact: true }).click();
  check(await onPage(page), `${what}: the page stayed open`);
}

/** Types `text` as the thread's next prompt and waits for `ready` on it. */
async function prompt(page, key, text, ready, what) {
  await page.evaluate(({ key, text }) => {
    const tab = tabByKey(key);
    return callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text });
  }, { key, text });
  await waitFor(what, () => page.evaluate(ready, key), 30000);
}
const asking = (key) => !!tabByKey(key).menu && waitingKindOf(tabByKey(key).view) === "needs_input";
const settled = (key) => statusOf(tabByKey(key)) === "done";

async function reopen(page) {
  await page.evaluate(() => openPage("needs-you"));
  await waitFor("the page", () => onPage(page));
}

const unlockIfAsked = (page) => waitFor("the password prompt", () => inbox.unlock(page), 5000).catch(() => null);

async function shoot(browser, ctx, size, viewport) {
  const phone = viewport.width < 768;
  const context = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
  const page = await context.newPage();
  await login(page);
  await world.ensure(page, ctx);
  await reopen(page);
  await waitFor("the cards", () => page.evaluate(() => document.querySelectorAll("#page-view .ny-card").length >= 4));
  await page.waitForTimeout(600);
  const file = path.join(OUT, `needs-you-${size}.png`);
  await page.screenshot({ path: file });
  const over = await page.evaluate(() => [...document.querySelectorAll("#page-view .ny-card, #page-view .ny-card *")].filter((el) => el.getBoundingClientRect().right > innerWidth + 0.5).map((el) => el.className || el.tagName));
  check(!over.length, `${size}: nothing runs past the right edge${over.length ? `: ${over.join(", ")}` : ""}`);
  const short = await page.evaluate(() => [...document.querySelectorAll("#page-view .ny-card button, #page-view .ny-card input")].filter((el) => el.offsetParent && el.getBoundingClientRect().height < 44).map((el) => el.textContent || el.placeholder));
  check(!phone || !short.length, `${size}: every control is at least 44px tall${short.length ? `: ${short.join(", ")}` : ""}`);
  console.log(`shot ${file}`);
  await context.close();
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
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
    await inbox.seed(page, ctx);
    const { readme, question, rate, hero, two } = ctx.threads;
    const k = (key) => `thread:${key}`;
    await page.evaluate(() => {
      window.__nyListeners = () => modelListeners.size;
    });
    await page.evaluate((key) => activate(tabByKey(key)), question);
    await reopen(page);
    await waitFor("the four cards", () => page.evaluate(() => document.querySelectorAll("#page-view .ny-card").length >= 4));
    await page.waitForTimeout(300);
    check(await page.evaluate((key) => active?.key === key && !!tabByKey(key).menu && !document.querySelector(".sheet-panel"), question), "the open thread's question sheet stays off the Needs you page");
    // An open thread counts as seen, and the question's card must turn Finished once answered.
    await page.evaluate((key) => (activate(tabByKey(key)), openPage("needs-you")), ctx.threads.updater);
    console.log(`  cards:\n    ${(await page.evaluate(cardsText)).join("\n    ")}`);
    check((await page.evaluate(() => document.querySelector(".ny-title .ny-count").textContent)) === "4 threads on 3 machines", "the summary reads 4 threads on 3 machines");

    await press(page, k(readme), "Decline", "Decline");
    await waitFor("the decline on the worker's screen", () => screenHas(page, readme, "chose 3"), 15000);
    await waitFor("the approval card to leave", async () => !(await card(page, k(readme)).count()), 20000);
    check(true, "Decline reached the worker, which went on, and its card left");
    await prompt(page, readme, `${world.WORLD.threads.readme.title}\n\n${README_BRIEF}`, asking, "the worker to ask again");
    await waitFor("the approval card back", async () => (await card(page, k(readme)).count()) > 0);
    await press(page, k(readme), "Approve", "Approve");
    await waitFor("the approval on the worker's screen", () => screenHas(page, readme, "chose 1"), 15000);
    await waitFor("the approval card to leave", async () => !(await card(page, k(readme)).count()), 20000);
    check(true, "Approve reached the worker, which ran the push, and its card left");
    await prompt(page, readme, `${world.WORLD.threads.readme.title}\n\n${README_BRIEF}`, asking, "the worker to ask again");

    await waitFor("the question card's options", async () => (await card(page, k(question)).locator(".ny-option").count()) > 0);
    await press(page, k(question), "The team photo Needs a crop for mobile", "an option");
    await waitFor("the option on the question's screen", () => screenHas(page, question, "answered: The team photo"), 15000);
    await waitFor("the question card to turn into a finished one", async () => (await kindOf(page, k(question))) === "review", 20000);
    check(true, "an option answered the question; its card now reads Finished");
    await prompt(page, question, QUESTION_PROMPT, asking, "the question again");
    await waitFor("the question's field", async () => (await card(page, k(question)).locator("input").count()) > 0);
    await card(page, k(question)).locator("input").fill("Use the product shot, cropped");
    await card(page, k(question)).getByRole("button", { name: "Send" }).click();
    check(await onPage(page), "a typed answer: the page stayed open");
    await waitFor("the typed answer on the question's screen", () => screenHas(page, question, "answered: Use the product shot, cropped"), 20000);
    await waitFor("the question card to turn into a finished one", async () => (await kindOf(page, k(question))) === "review", 20000);
    check(true, "the typed answer reached the question; its card now reads Finished");

    await prompt(page, question, "/plan-ready", (key) => tabByKey(key).menu && menuKind(tabByKey(key).menu) === "plan", "a plan on the site thread");
    await waitFor("the plan card", async () => (await kindOf(page, k(question))) === "plan");
    await press(page, k(question), "Approve plan", "Approve plan");
    await waitFor("the plan approved on its screen", () => screenHas(page, question, "answered: Yes, and use auto mode"), 15000);
    await waitFor("the plan card to turn into a finished one", async () => (await kindOf(page, k(question))) === "review", 20000);
    check(true, "Approve plan answered the plan");
    await prompt(page, question, QUESTION_PROMPT, asking, "the question again");

    // The set's row of tabs stays in the scrollback, so it asks in a thread of its own.
    await prompt(page, two, "/questions", asking, "a question set in the notes thread");
    await waitFor("the question set's card", async () => (await kindOf(page, k(two))) === "question");
    const setButtons = await card(page, k(two)).locator("button").allTextContents();
    check(JSON.stringify(setButtons.map((b) => b.trim())) === '["Open"]', `a question set offers only Open (${setButtons.join(", ")})`);
    await page.evaluate((key) => writeTo(tabByKey(key), "\x1b"), two);
    await waitFor("the set dismissed", () => page.evaluate(settled, two), 15000);
    await page.evaluate((key) => {
      seenAt[threadKey(tabByKey(key))] = lastActivity(tabByKey(key)) + 1;
      storeMarks("gui-seen");
    }, two);

    await press(page, k(rate), "Snooze", "Snooze");
    await waitFor("the limit card to leave", async () => !(await card(page, k(rate)).count()), 10000);
    const snoozed = await page.evaluate((key) => sideMarks.snoozed[threadKey(tabByKey(key))]?.until === tabByKey(key).view.resume_at, rate);
    check(snoozed, "Snooze snoozed the thread until its limit resets, and its card left");
    await page.evaluate((key) => wake(tabByKey(key)), rate);
    await waitFor("the limit card back once the thread wakes", async () => (await card(page, k(rate)).count()) > 0, 10000);
    // A rerun finds the thread under Work already, so it offers the default account back.
    const switchTo = card(page, k(rate)).locator('[data-ny="switch"]');
    const [label, account] = [(await switchTo.textContent()).trim(), await switchTo.getAttribute("data-account")];
    await press(page, k(rate), label, label);
    await waitFor(`the thread under ${account}`, () => page.evaluate(({ key, account }) => tabByKey(key).view.account === account && !tabByKey(key).view.resume_at, { key: rate, account }), 20000);
    await waitFor("the limit card to leave", async () => !(await card(page, k(rate)).count()), 10000);
    check(true, `${label} moved the thread to ${account}, off its limit, and its card left`);

    const [deny, allow] = [await act(ctx.threads.release.split(":")[1], { action: "pair_code", request_id: `needs-you-deny-${Date.now()}` }), await act(ctx.threads.release.split(":")[1], { action: "pair_code", request_id: `needs-you-allow-${Date.now()}` })];
    check(deny.outcome === "pending" && allow.outcome === "pending", "an agent asked twice to pair a machine");
    await waitFor("the agent's cards", async () => (await page.locator("#page-view .ny-card[data-kind=agent]").count()) === 2);
    check((await page.evaluate(() => document.querySelector(".ny-title .ny-count").textContent)).endsWith("· 2 agent requests"), "the summary counts the agent's requests");
    await press(page, `agent:${deny.id}`, "Deny", "Deny");
    await unlockIfAsked(page);
    await waitFor("the denied request to leave", async () => !(await card(page, `agent:${deny.id}`).count()), 15000);
    await press(page, `agent:${allow.id}`, "Approve", "Approve an agent");
    await unlockIfAsked(page);
    await waitFor("the approved request to leave", async () => !(await card(page, `agent:${allow.id}`).count()), 15000);
    await waitFor("the pairing command shown once", async () => (await card(page, "agent-shown").count()) > 0, 10000);
    const command = (await card(page, "agent-shown").locator("code").textContent()).trim();
    check(command.length > 0, `the approved pairing hands its command over on the page (${command.slice(0, 40)}…)`);
    await press(page, "agent-shown", "Done", "Done");
    await waitFor("the shown command to go", async () => !(await card(page, "agent-shown").count()), 10000);
    check(true, "Done put the pairing command away");
    const decided = await page.evaluate(() => hub.hub({ cmd: "agent_audit", limit: 10 }));
    check(decided.some((r) => r.action === "pair_code" && r.outcome === "denied") && decided.some((r) => r.action === "pair_code" && /approved|ok|done/.test(r.outcome)), `Deny and Approve decided the agent's requests (${decided.slice(0, 2).map((r) => r.outcome).join(", ")})`);

    const listening = await page.evaluate(() => window.__nyListeners());
    await card(page, k(hero)).getByRole("button", { name: "Open" }).click();
    await waitFor("the hero thread open", () => page.evaluate((key) => active?.key === key && !document.body.dataset.page, hero));
    check(true, "Open on a finished thread opened it");
    await page.evaluate(() => renderSidebar());
    check((await page.evaluate(() => window.__nyListeners())) === listening - 1, "the closed page stopped listening to model changes");
    await reopen(page);
    check(!(await card(page, k(hero)).count()), "the finished thread, now seen, left the page");
    await page.evaluate((key) => markUnread(tabByKey(key)), hero);
    await card(page, k(readme)).getByRole("button", { name: "Open" }).click();
    await waitFor("the worker open", () => page.evaluate((key) => active?.key === key && !document.body.dataset.page, readme));
    check(true, "Open on an approval opened the worker's thread");
    await waitFor("the worker's approval sheet", () => page.evaluate(() => !!document.querySelector(".sheet-panel.ask-sheet")));
    check(true, "the worker's approval sheet rose on its thread's screen");
    await reopen(page);
    await page.waitForTimeout(300);
    check(await page.evaluate(() => !document.querySelector(".sheet-panel")), "back on Needs you, the thread's sheet closed");
    await context.close();

    await shoot(browser, ctx, "1440x900", { width: 1440, height: 900 });
    await shoot(browser, ctx, "360x740", { width: 360, height: 740 });
    check(!errors.length, `no page errors${errors.length ? `: ${errors.join("; ")}` : ""}`);
    if (ctx.warnings.length) console.log(`warnings: ${ctx.warnings.join("; ")}`);
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
