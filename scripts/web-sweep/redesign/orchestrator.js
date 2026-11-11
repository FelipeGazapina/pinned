// Orchestrator.dc.html: the orchestrator's chat on mac-mini with two Captain reports, the Captains
// column on the right, and the sidebar's account menu open. The live side opens the orchestrator's
// thread. Today's app has no account menu at the sidebar's foot (its pages sit in an icon row) and
// no Captains column, so those regions compare against the sidebar and the chat. The board's chat
// (routed #pinned and #site requests, the reports, the #api draft) is not seeded.
module.exports = {
  board: "Orchestrator",
  viewport: { width: 1440, height: 900 },
  regions: [
    { name: "sidebar", x: 0, y: 0, width: 264, height: 900 },
    { name: "menu", x: 8, y: 614, width: 248, height: 228 },
    { name: "header", x: 264, y: 0, width: 1176, height: 49 },
    { name: "chat", x: 264, y: 49, width: 854, height: 851 },
    { name: "captains", x: 1118, y: 49, width: 322, height: 851 },
  ],
  async open(page) {
    await page.evaluate(() => activate(tabs.find(isGlobalOrchestratorTab)));
  },
};
