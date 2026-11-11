// Phone-More.dc.html: the More tab, pages grouped under Work and Setup. Today's phone has no More
// tab; the live side opens Settings, the page nearest to that list.
module.exports = {
  board: "Phone-More",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "title", x: 0, y: 54, width: 390, height: 55 },
    { name: "list", x: 0, y: 109, width: 390, height: 651 },
    { name: "tabs", x: 0, y: 760, width: 390, height: 84 },
  ],
  async open(page) {
    await page.evaluate(() => {
      toggleSidebar(false);
      openSettings();
    });
  },
};
