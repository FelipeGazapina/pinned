// Phone-Inbox.dc.html: the Needs you page with four cards, an approval, a question, a usage limit
// and a finished thread. The shared world has the README worker's approval on pinned and the
// unread hero fix on site; seed adds the rest through real sessions: a thread on site whose fake
// claude asks the board's question, a signed-in "Work" account on vps, and a usage limit typed
// into api's working thread. The other finished threads are marked read, as opening them would.
const fs = require("fs");
const path = require("path");
const world = require("./world");

const T = world.WORLD.threads;
const QUESTION_TITLE = "Pick the launch page hero";
const QUESTION_TURN = {
  prompt: "Pick the hero image for the launch page.",
  question: { header: "Hero", question: "Which hero image should the launch page use?", options: [["The product screenshot", "Already in the repo, 180 KB"], ["The team photo", "Needs a crop for mobile"]] },
};

/** Adds `turn` to the fake claude's script on `machine`, once. */
function scriptTurn(ctx, machine, turn) {
  const file = path.join(world.homeOf(ctx, machine), "fake-turns.json");
  const turns = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : [];
  if (turns.some((t) => t.prompt === turn.prompt)) return;
  fs.writeFileSync(file, JSON.stringify([turn, ...turns], null, 1));
}

async function seedQuestion(page, ctx) {
  const machine = ctx.machines[T.hero.machine];
  scriptTurn(ctx, T.hero.machine, QUESTION_TURN);
  const cwd = fs.realpathSync(path.join(world.homeOf(ctx, T.hero.machine), "work", "site"));
  let key = await page.evaluate((title) => tabs.find((t) => threadTitle(t) === title)?.key ?? null, QUESTION_TITLE);
  if (!key) {
    key = await page.evaluate(async ({ machine, cwd }) => {
      const tab = await revealTab(machine, await callOn(machine, { cmd: "open_tab", path: cwd, bypass: false }));
      startTab(tab);
      return tab.key;
    }, { machine, cwd });
    await ctx.waitFor("the question thread to start", () => page.evaluate((k) => tabByKey(k)?.view.running === true, key));
  }
  // A thread whose node restarted since has answered "Continue from where you left off" instead.
  await page.evaluate(({ key, text, title }) => {
    const tab = tabByKey(key);
    changeMarks(threadKey(tab), () => (sideMarks.titles[threadKey(tab)] = title));
    if (!tab.menu) callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text });
  }, { key, text: QUESTION_TURN.prompt, title: QUESTION_TITLE });
  await ctx.waitFor("the question on its screen", () => page.evaluate((k) => tabByKey(k).menu && waitingKindOf(tabByKey(k).view) === "needs_input", key), 30000);
  return key;
}

/** Types the account's password into the prompt the hub's LOCKED answer opens, when one is up. The
 * prompt sits in the sidebar, which a phone keeps closed, so it is filled in the page. */
const unlock = (page) =>
  page.evaluate((password) => {
    const field = document.querySelector("dialog[open] [name=password]");
    if (!field) return false;
    field.value = password;
    field.form.requestSubmit();
    return true;
  }, world.PASSWORD);

async function seedLimit(page, ctx) {
  const vps = T.rate.machine;
  const machine = ctx.machines[vps];
  const accounts = await page.evaluate((m) => callOn(m, { cmd: "list_accounts" }), machine);
  if (!accounts.some((a) => a.name === "Work" && a.signed_in)) {
    let work = accounts.find((a) => a.name === "Work");
    if (!work) {
      const added = page.evaluate((m) => callOn(m, { cmd: "add_account", provider: "claude", name: "Work" }), machine);
      await ctx.waitFor("the account added", () => unlock(page).then(() => Promise.race([added, new Promise((r) => setTimeout(r, 300))])), 20000);
      work = await added;
    }
    const home = path.join(ctx.dir, `node-${vps}`, "accounts", work.id);
    fs.mkdirSync(home, { recursive: true });
    fs.writeFileSync(path.join(home, "signed-in"), "");
  }
  await ctx.waitFor("Work signed in on vps", () => page.evaluate(async (m) => setAccounts(m, await callOn(m, { cmd: "list_accounts" })).some((a) => a.name === "Work" && a.signed_in), machine), 20000);
  const key = ctx.threads.rate;
  // redesign-needs-you.js leaves the thread under Work; the board limits it under the default account.
  if (await page.evaluate((k) => tabByKey(k).view.account !== DEFAULT_ACCOUNT, key)) {
    await page.evaluate((k) => switchAccount(tabByKey(k), DEFAULT_ACCOUNT), key);
    await ctx.waitFor("api's thread back under the default account", () => page.evaluate((k) => tabByKey(k).view.account === DEFAULT_ACCOUNT && tabByKey(k).view.running, key), 20000);
  }
  await page.evaluate(({ key, title }) => {
    const tab = tabByKey(key);
    changeMarks(threadKey(tab), () => (sideMarks.titles[threadKey(tab)] = title));
  }, { key, title: T.rate.title });
  // The fake claude takes keys once its prompt shows, and a resumed one may hold keys typed before.
  const state = await ctx.waitFor("api's prompt", () => page.evaluate((k) => (tabByKey(k).view.resume_at ? "stopped" : /^>/.test(screenLines(tabByKey(k)).filter((l) => l.trim()).at(-1) ?? "") ? "ready" : null), key), 20000);
  if (state === "ready") {
    await page.evaluate(async (key) => {
      const tab = tabByKey(key);
      await callOn(tab.machine, { cmd: "write_input", tab: tab.view.id, data: "\x7f".repeat(16) });
      await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text: "limited" });
    }, key);
  }
  await ctx.waitFor("api's thread stopped by a usage limit", () => page.evaluate((k) => !!tabByKey(k).view.resume_at && phaseOf(tabByKey(k)) === "failed", key), 30000);
}

module.exports = {
  board: "Phone-Inbox",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "title", x: 0, y: 54, width: 390, height: 55 },
    { name: "cards", x: 0, y: 109, width: 390, height: 651 },
    { name: "tabs", x: 0, y: 760, width: 390, height: 84 },
  ],
  QUESTION_TITLE,
  unlock,
  async seed(page, ctx) {
    ctx.threads.question = await seedQuestion(page, ctx);
    await seedLimit(page, ctx);
    await page.evaluate(({ hero }) => {
      for (const tab of tabs) if (tab.key !== hero && unseenDone(tab)) seenAt[threadKey(tab)] = lastActivity(tab) + 1;
      storeMarks("gui-seen");
      renderSidebar();
    }, ctx.threads);
    await unlock(page);
  },
  async open(page, ctx) {
    await page.evaluate(() => phoneGo("needs-you"));
    await ctx.waitFor("the four cards", () => page.evaluate(() => document.querySelectorAll("#page-view .ny-card").length >= 4), 15000);
  },
};
