// Panel.dc.html: "Fix hero spacing on mobile" (site, macbook) with the side panel on its diff of
// src/components/Hero.css and the terminal open under the chat. The live side opens that thread at
// the board's 520px panel with Diff, Browser, PR #12 and Files tabs, the Diff surface on its
// uncommitted changes with the file shown and wrapping off, a review comment drafted on line 14, and
// the 190px drawer with a "dev" project script running in a shell. The fake
// claude keeps no turn checkpoints, so the scope reads Uncommitted, not Latest turn. The script
// prints the board's dev server lines with printf, so its shell shows that command instead of
// `npm run dev`. The task list is not seeded.
const DEV = { id: "dev", name: "dev", icon: "play", command: "printf '  VITE ready in 412 ms\\n  Local:   http://localhost:3000/\\n  hmr update /src/components/Hero.css\\n'" };
const COMMENT = "Use the nav height variable here too, so the two never drift.";

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
  async seed(page, ctx) {
    await page.evaluate(async ({ key, dev }) => {
      const tab = tabByKey(key);
      activate(tab);
      await saveScripts(tab, [dev]);
    }, { key: ctx.threads.hero, dev: DEV });
  },
  async open(page, ctx) {
    await page.evaluate(async ({ key, dev }) => {
      activate(tabByKey(key));
      panelOf(active).width = 520;
      openPanel("diff");
      addSurface(active, newBrowser());
      openSurface("pr");
      openFilesSurface();
      setDiffState(active, { scope: "uncommitted" });
      fetchGitDiff(active, "uncommitted", null, true);
      openSurface("diff");
      setDrawerHeight(190);
      await runScript(active, dev);
    }, { key: ctx.threads.hero, dev: DEV });
    const file = '#panel-body .file[data-path="src/components/Hero.css"]';
    await ctx.waitFor("the Hero.css diff", () => page.isVisible(`${file} > header`), 15000).catch((e) => ctx.warn(e.message));
    if (await page.isVisible(`${file}[data-collapsed] > header`)) await page.click(`${file} > header`);
    if (await page.isVisible('#panel-body [data-diff="wrap"][aria-pressed="true"]')) await page.click('#panel-body [data-diff="wrap"]');
    const line = `${file} .no[data-side="new"][data-line="14"]`;
    await ctx.waitFor("line 14", () => page.isVisible(line), 10000).catch((e) => ctx.warn(e.message));
    await page.hover(line).catch((e) => ctx.warn(e.message));
    await page.click(`${line} [data-comment-line]`).catch((e) => ctx.warn(e.message));
    await page.fill("#panel-body [data-comment-text]", COMMENT).catch((e) => ctx.warn(e.message));
    await page.mouse.move(700, 300);
    await ctx.waitFor("the dev shell's output", () => page.evaluate(() => scriptShells.some(({ term }) => {
      const b = term.buffer.active;
      for (let y = 0; y < b.length; y++) if (/VITE ready/.test(b.getLine(y)?.translateToString(true) ?? "")) return true;
      return false;
    })), 10000).catch((e) => ctx.warn(e.message));
  },
};
