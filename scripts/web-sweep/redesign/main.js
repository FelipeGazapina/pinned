// Main.dc.html: the Captain "Release v0.17.21" open beside the sidebar, its three workers in the
// timeline and the README worker's approval above the composer. The live side opens the Captain's
// thread; today's app shows its workers as tool calls and the approval only on the worker.
module.exports = {
  board: "Main",
  viewport: { width: 1440, height: 900 },
  regions: [
    { name: "sidebar", x: 0, y: 0, width: 264, height: 900 },
    { name: "header", x: 264, y: 0, width: 1176, height: 49 },
    { name: "timeline", x: 264, y: 49, width: 1176, height: 673 },
    { name: "composer", x: 264, y: 722, width: 1176, height: 178 },
  ],
  async open(page, ctx) {
    await page.evaluate((key) => activate(tabByKey(key)), ctx.threads.release);
  },
};
