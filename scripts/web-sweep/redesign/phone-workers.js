// Phone-Workers.dc.html: the Captain's workers in a sheet over its thread, the README worker's
// approval first. Today's phone has no workers sheet; the live side opens the Captain's thread,
// so the sheet region compares against its chat and composer.
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
    await page.evaluate((key) => {
      activate(tabByKey(key));
      toggleSidebar(false);
    }, ctx.threads.release);
  },
};
