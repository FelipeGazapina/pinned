// Phone-More.dc.html: the More tab, the usage card over the pages grouped under Work and Setup. The
// board's Settings row reads "Notifications on", so the page is allowed to notify first.
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
  async open(page, ctx) {
    await page.context().grantPermissions(["notifications"], { origin: ctx.hub });
    await page.evaluate(() => phoneGo("more"));
    await ctx.waitFor("the More lists", () => page.evaluate(() => document.querySelectorAll("#page-view .more-row").length >= 10));
  },
};
