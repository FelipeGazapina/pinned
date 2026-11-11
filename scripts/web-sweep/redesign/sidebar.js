// Sidebar.dc.html: the sidebar alone, by project, with the Captain active. The board is 264x900;
// the live side runs at 1440x900 with the Captain open and keeps its left 264 pixels.
module.exports = {
  board: "Sidebar",
  viewport: { width: 264, height: 900 },
  liveViewport: { width: 1440, height: 900 },
  clip: { x: 0, y: 0, width: 264, height: 900 },
  regions: [
    { name: "top", x: 0, y: 0, width: 264, height: 84 },
    { name: "nav", x: 0, y: 84, width: 264, height: 131 },
    { name: "threads", x: 0, y: 215, width: 264, height: 634 },
    { name: "footer", x: 0, y: 849, width: 264, height: 51 },
  ],
  async open(page, ctx) {
    await page.evaluate((key) => activate(tabByKey(key)), ctx.threads.release);
  },
};
