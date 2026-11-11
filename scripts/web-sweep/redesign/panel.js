// Panel.dc.html: "Fix hero spacing on mobile" (site, macbook) with the side panel on its diff of
// src/components/Hero.css and the terminal open under the chat. The live side opens that thread,
// the Diff surface on its uncommitted changes and the terminal drawer: the fake claude keeps no
// turn checkpoints, so "Latest turn" would be empty. The board's dev server output, task list and
// review comment are not seeded.
module.exports = {
  board: "Panel",
  viewport: { width: 1440, height: 900 },
  regions: [
    { name: "sidebar", x: 0, y: 0, width: 264, height: 900 },
    { name: "header", x: 264, y: 0, width: 655, height: 49 },
    { name: "timeline", x: 264, y: 49, width: 655, height: 567 },
    { name: "composer", x: 264, y: 616, width: 655, height: 93 },
    { name: "terminal", x: 264, y: 709, width: 655, height: 191 },
    { name: "panel", x: 919, y: 0, width: 521, height: 900 },
  ],
  async open(page, ctx) {
    await page.evaluate((key) => {
      activate(tabByKey(key));
      openPanel("diff");
      setDiffState(active, { scope: "uncommitted" });
      fetchGitDiff(active, "uncommitted", null, true);
      renderPanel();
      toggleDrawer();
    }, ctx.threads.hero);
    await ctx.waitFor("the Hero.css diff", () => page.evaluate(() => /Hero\.css/.test(document.getElementById("panel").innerText)), 15000).catch((e) => ctx.warn(e.message));
  },
};
