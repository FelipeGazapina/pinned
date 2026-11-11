// Phone-Approval.dc.html: the approval sheet for git push origin readme-tidy over the README
// worker's thread. Today's phone has no sheet; the live side opens the worker's thread, whose
// approval sits in place of the composer.
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
    await page.evaluate((key) => {
      activate(tabByKey(key));
      toggleSidebar(false);
    }, ctx.threads.readme);
  },
};
