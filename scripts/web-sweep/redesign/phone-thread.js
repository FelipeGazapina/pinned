// Phone-Thread.dc.html: the Captain "Release v0.17.21" on a phone, with the Workers pill under the
// header, the workers in the chat and the README worker's banner above the composer. The board
// draws an iPhone's status bar and home indicator, so the page gets the same safe-area insets.

/** Gives the page an iPhone's safe-area insets, which the app pads its shell with. */
async function phoneInsets(page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets: { top: 54, bottom: 34 } });
}

/** Opens `key`'s thread as a phone shows it, with nothing focused, and scrolled to its first turn
 * when `top`, where the boards draw the Captain's. */
async function openThread(page, ctx, key, top = false) {
  await phoneInsets(page);
  await page.evaluate((key) => {
    activate(tabByKey(key));
    toggleSidebar(false);
    document.activeElement?.blur();
  }, key);
  await ctx.waitFor("the thread on screen", () => page.evaluate((key) => active?.key === key && !document.getElementById("chat-head").hidden, key));
  if (top) {
    await page.waitForTimeout(300);
    await page.evaluate(() => threadView.scrollTo({ top: 0 }));
  }
}

module.exports = {
  board: "Phone-Thread",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "header", x: 0, y: 54, width: 390, height: 52 },
    { name: "banner", x: 0, y: 106, width: 390, height: 50 },
    { name: "timeline", x: 0, y: 156, width: 390, height: 503 },
    { name: "composer", x: 0, y: 659, width: 390, height: 185 },
  ],
  phoneInsets,
  openThread,
  // The board's chip reads "Opus 5.5 · High · Auto". The fake claude draws claude 2.1's effort marks
  // and mode line once told /traits and /modes; then the composer's own pickers set each.
  async seed(page, ctx) {
    const key = ctx.threads.release;
    const looks = () => page.evaluate((key) => {
      const tab = tabByKey(key);
      return { effort: traitsOn(screenLines(tab))?.effort ?? null, mode: terminalMode(tab), model: modelName(tab) };
    }, key);
    const shows = async (want) => {
      const now = await looks();
      return Object.entries(want).every(([k, v]) => (v === true ? !!now[k] : now[k] === v));
    };
    const looked = await looks();
    const tell = (text) => page.evaluate(({ key, text }) => { const tab = tabByKey(key); return callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text }); }, { key, text });
    if (!looked.effort) {
      await tell("/traits");
      await ctx.waitFor("the Captain's effort marks", () => shows({ effort: true }));
    }
    if (!looked.mode) {
      await tell("/modes");
      await ctx.waitFor("the Captain's mode line", () => shows({ mode: true }));
    }
    await page.evaluate((key) => {
      const tab = tabByKey(key);
      tab.traits = traitsOn(screenLines(tab));
      tab.mode = terminalMode(tab);
      pickEffort(tab, "high");
    }, key);
    await ctx.waitFor("high effort", () => shows({ effort: "high" }), 20000);
    await page.evaluate((key) => {
      const tab = tabByKey(key);
      tab.traits = traitsOn(screenLines(tab));
      return pickAccess(tab, "auto");
    }, key);
    await ctx.waitFor("auto mode", () => shows({ mode: "auto" }), 20000);
    await page.evaluate((key) => {
      const tab = tabByKey(key);
      pickModel(tab, modelsFor(tab).find((m) => m.menu === "Opus"), accountOf(tab.machine, tab.view.account ?? DEFAULT_ACCOUNT));
    }, key);
    await ctx.waitFor("Opus 5.5", () => shows({ model: "Opus 5.5" }), 20000);
  },
  async open(page, ctx) {
    await openThread(page, ctx, ctx.threads.release, true);
    await ctx.waitFor("the Workers pill", () => page.evaluate(() => !document.getElementById("workers-pill").hidden), 15000);
  },
};
