// Phone-Actions.dc.html: the thread actions sheet over "Fix hero spacing on mobile": its next
// steps, the tools that open beside it, then the thread's actions. Seed gives the site project the
// board's "dev" script through the script editor's save; the live side opens the hero thread, taps
// ⋯ and waits for the tiles' hints to be read.
const { openThread } = require("./phone-thread");

const DEV = { id: "dev", name: "dev", command: "npm run dev", icon: "play", runOnWorktreeCreate: false, runOnSettle: false };

module.exports = {
  board: "Phone-Actions",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "header", x: 0, y: 54, width: 390, height: 52 },
    { name: "behind", x: 0, y: 106, width: 390, height: 171 },
    { name: "sheet", x: 0, y: 277, width: 390, height: 567 },
  ],
  async seed(page, ctx) {
    const key = ctx.threads.hero;
    await ctx.waitFor("the site project's scripts", () => page.evaluate((k) => {
      const tab = tabByKey(k);
      return !!threadOf(tab).workspace && !projectScripts(tab).loading;
    }, key));
    await page.evaluate(async ({ key, dev }) => {
      const tab = tabByKey(key);
      const scripts = projectScripts(tab).scripts;
      if (!scripts.some((s) => s.id === dev.id)) await saveScripts(tab, [...scripts, dev]);
    }, { key, dev: DEV });
  },
  async open(page, ctx) {
    await openThread(page, ctx, ctx.threads.hero);
    await page.tap("#head-more");
    await ctx.waitFor("the actions sheet", () => page.evaluate(() => !!document.querySelector(".actions-sheet .act-grid")), 15000);
    await ctx.waitFor("the tiles' hints", () => page.evaluate(() => /\+\d/.test(document.querySelector('.actions-sheet .act-tile[data-act="diff"] .hint')?.textContent ?? "")), 8000).catch((e) => ctx.warn(e.message));
  },
};
