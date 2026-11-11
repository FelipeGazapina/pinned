// Phone-Actions.dc.html: the thread actions sheet over "Fix hero spacing on mobile": tools to open
// beside the thread, then thread actions. Today's phone has no such sheet; the live side opens the
// hero thread and its header's More menu, the nearest list of the same actions.
module.exports = {
  board: "Phone-Actions",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "header", x: 0, y: 54, width: 390, height: 52 },
    { name: "behind", x: 0, y: 106, width: 390, height: 171 },
    { name: "sheet", x: 0, y: 277, width: 390, height: 567 },
  ],
  async open(page, ctx) {
    await page.evaluate((key) => {
      activate(tabByKey(key));
      toggleSidebar(false);
    }, ctx.threads.hero);
    await page.click("#head-more");
  },
};
