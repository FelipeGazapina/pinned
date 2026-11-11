// Phone-Home.dc.html: the Threads tab, by project, above the tab bar. Today's phone has no tab bar
// and no Threads screen; the live side opens the sidebar drawer, its nearest thing.
module.exports = {
  board: "Phone-Home",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "header", x: 0, y: 54, width: 390, height: 58 },
    { name: "filters", x: 0, y: 112, width: 390, height: 46 },
    { name: "list", x: 0, y: 158, width: 390, height: 602 },
    { name: "tabs", x: 0, y: 760, width: 390, height: 84 },
  ],
  async open(page) {
    await page.evaluate(() => toggleSidebar(true));
  },
};
