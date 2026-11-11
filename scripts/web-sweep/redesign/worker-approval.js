// Worker-Approval.dc.html: the worker "Tidy the README" open, waiting for approval to run
// git push origin readme-tidy. The live side opens that worker's thread, where today's app shows
// the approval in place of the composer. The board's two changed files (README.md, docs/install.md)
// are not seeded: the fake worker changes no file.
module.exports = {
  board: "Worker-Approval",
  viewport: { width: 1440, height: 900 },
  regions: [
    { name: "sidebar", x: 0, y: 0, width: 264, height: 900 },
    { name: "header", x: 264, y: 0, width: 1176, height: 49 },
    { name: "timeline", x: 264, y: 49, width: 1176, height: 589 },
    { name: "approval", x: 264, y: 638, width: 1176, height: 262 },
  ],
  async open(page, ctx) {
    await page.evaluate((key) => activate(tabByKey(key)), ctx.threads.readme);
  },
};
