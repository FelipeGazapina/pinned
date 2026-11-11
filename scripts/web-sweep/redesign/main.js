// Main.dc.html: the Captain "Release v0.17.21" open beside the sidebar, its three workers in the
// timeline and the README worker's approval above the composer. The live side opens the Captain's
// thread; today's app shows its workers as tool calls and the approval only on the worker. The
// pinned project gets a "dev" script, so the header's scripts control reads Run dev.
module.exports = {
  board: "Main",
  viewport: { width: 1440, height: 900 },
  regions: [
    { name: "sidebar", x: 0, y: 0, width: 264, height: 900 },
    { name: "header", x: 264, y: 0, width: 1176, height: 49 },
    { name: "timeline", x: 264, y: 49, width: 1176, height: 673 },
    { name: "composer", x: 264, y: 722, width: 1176, height: 178 },
  ],
  async seed(page, ctx) {
    await page.evaluate((key) => saveScripts(tabByKey(key), [{ id: "dev", name: "dev", icon: "play", command: "npm run dev" }]), ctx.threads.release);
  },
  async open(page, ctx) {
    await page.evaluate(({ release, ci }) => {
      threadOf(tabByKey(release)).open.add(`worker:${ci}`);
      activate(tabByKey(release));
    }, ctx.threads);
    await ctx.waitFor("the Workers block with the CI worker open", () => page.evaluate(() => !!document.querySelector('#thread .workers .worker-row[data-open] .worker-actions')));
    // The board starts at the Captain's first prompt; the live thread has one more turn below.
    await page.evaluate(() => {
      threadView.scrollTop = 0;
      threadView.dispatchEvent(new Event("scroll"));
    });
    await page.waitForTimeout(300);
  },
};
