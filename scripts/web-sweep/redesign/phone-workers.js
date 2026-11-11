// Phone-Workers.dc.html: the Captain's workers in a sheet over its thread, the README worker's
// approval first. The live side opens the Captain's thread and taps its Workers pill.
const { openThread } = require("./phone-thread");

module.exports = {
  board: "Phone-Workers",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "header", x: 0, y: 54, width: 390, height: 52 },
    { name: "behind", x: 0, y: 106, width: 390, height: 262 },
    { name: "sheet", x: 0, y: 368, width: 390, height: 476 },
  ],
  async open(page, ctx) {
    await openThread(page, ctx, ctx.threads.release);
    await page.tap("#workers-pill");
    await ctx.waitFor("the Workers sheet", () => page.evaluate(() => !!document.querySelector(".workers-sheet .ws-card")), 15000);
  },
};
