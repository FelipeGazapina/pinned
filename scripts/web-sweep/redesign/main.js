// Main.dc.html: the Captain "Release v0.17.21" open beside the sidebar, its three workers in the
// timeline and the README worker's approval above the composer. The live side opens the Captain's
// thread. The pinned project gets a "dev" script, so the header's scripts control reads Run dev;
// the composer's own pickers set Opus 5.5, High effort and Auto-accept edits, and the header's
// editor menu picks Zed.
const { pickChips } = require("./world");

/** Picks `name` in the header's Open in menu of the open thread, as a click there does. */
async function pickEditor(page, ctx, name) {
  await ctx.waitFor("the Open in control", () => page.isVisible("#open-in .pick"), 15000);
  await page.click("#open-in .pick");
  await page.locator("#panel-menu button[data-i]", { has: page.locator(".label", { hasText: new RegExp(`^${name}$`) }) }).click();
}

module.exports = {
  board: "Main",
  viewport: { width: 1440, height: 900 },
  regions: [
    { name: "sidebar", x: 0, y: 0, width: 264, height: 900 },
    { name: "header", x: 264, y: 0, width: 1176, height: 49 },
    { name: "timeline", x: 264, y: 49, width: 1176, height: 673 },
    { name: "composer", x: 264, y: 722, width: 1176, height: 178 },
  ],
  pickEditor,
  async seed(page, ctx) {
    await page.evaluate((key) => saveScripts(tabByKey(key), [{ id: "dev", name: "dev", icon: "play", command: "npm run dev" }]), ctx.threads.release);
    await pickChips(page, ctx, ctx.threads.release, { effort: "high", access: "accept", model: "Opus" });
  },
  async open(page, ctx) {
    await page.evaluate(({ release, ci }) => {
      threadOf(tabByKey(release)).open.add(`worker:${ci}`);
      activate(tabByKey(release));
    }, ctx.threads);
    await pickEditor(page, ctx, "Zed");
    await ctx.waitFor("the Workers block with the CI worker open", () => page.evaluate(() => !!document.querySelector('#thread .workers .worker-row[data-open] .worker-actions')));
    // The board's thread ends at its last answer with the composer whole, as a thread read to its end shows.
    await page.evaluate(() => {
      threadView.scrollTop = threadView.scrollHeight;
      threadView.dispatchEvent(new Event("scroll"));
    });
    await page.waitForTimeout(300);
  },
};
