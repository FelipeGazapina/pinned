// Phone-Thread.dc.html: the Captain "Release v0.17.21" on a phone, with the workers card in the
// chat and the approval banner above the composer. The live side opens the Captain's thread.
module.exports = {
  board: "Phone-Thread",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "header", x: 0, y: 54, width: 390, height: 52 },
    { name: "banner", x: 0, y: 106, width: 390, height: 50 },
    { name: "timeline", x: 0, y: 156, width: 390, height: 503 },
    { name: "composer", x: 0, y: 659, width: 390, height: 185 },
  ],
  async open(page, ctx) {
    await page.evaluate((key) => {
      activate(tabByKey(key));
      toggleSidebar(false);
    }, ctx.threads.release);
  },
};
