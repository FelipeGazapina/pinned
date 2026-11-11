// PhoneTabs.dc.html: the phone tab bar (Threads, Needs you, Orchestrator, More), 390x84. Today's
// phone has no tab bar; the live side opens the sidebar drawer at 390x844 and keeps its bottom 84
// pixels, where the drawer's page icons sit.
module.exports = {
  board: "PhoneTabs",
  phone: true,
  viewport: { width: 390, height: 84 },
  liveViewport: { width: 390, height: 844 },
  clip: { x: 0, y: 760, width: 390, height: 84 },
  regions: [
    { name: "threads", x: 0, y: 0, width: 102, height: 84 },
    { name: "needs-you", x: 102, y: 0, width: 94, height: 84 },
    { name: "orchestrator", x: 196, y: 0, width: 94, height: 84 },
    { name: "more", x: 290, y: 0, width: 100, height: 84 },
  ],
  async open(page) {
    await page.evaluate(() => toggleSidebar(true));
  },
};
