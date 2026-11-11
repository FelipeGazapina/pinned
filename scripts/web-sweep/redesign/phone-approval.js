// Phone-Approval.dc.html: the approval sheet for git push origin readme-tidy over the README
// worker's thread. Opening the worker's thread opens the sheet for its ask.
const { openThread } = require("./phone-thread");

module.exports = {
  board: "Phone-Approval",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "header", x: 0, y: 54, width: 390, height: 52 },
    { name: "timeline", x: 0, y: 106, width: 390, height: 326 },
    { name: "sheet", x: 0, y: 432, width: 390, height: 412 },
  ],
  async open(page, ctx) {
    await openThread(page, ctx, ctx.threads.readme);
    await ctx.waitFor("the approval sheet", () => page.evaluate(() => !!document.querySelector(".ask-sheet [data-approval=accept]")), 20000);
  },
};
