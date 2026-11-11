// Phone-Inbox.dc.html: the Needs you tab, four cards (an approval, a question, a usage limit and a
// finished thread). Today's phone has no Needs you tab; the live side opens the sidebar drawer with
// Waiting on you expanded at the top. The board's question on site and usage limit on api are not
// seeded: in the shared world site's thread is finished and api's is working.
module.exports = {
  board: "Phone-Inbox",
  phone: true,
  viewport: { width: 390, height: 844 },
  regions: [
    { name: "status", x: 0, y: 0, width: 390, height: 54 },
    { name: "title", x: 0, y: 54, width: 390, height: 55 },
    { name: "cards", x: 0, y: 109, width: 390, height: 651 },
    { name: "tabs", x: 0, y: 760, width: 390, height: 84 },
  ],
  async open(page) {
    await page.evaluate(() => {
      toggleSidebar(true);
      ui.waitingOpen = true;
      renderSidebar();
      document.getElementById("thread-list").scrollTo(0, 0);
    });
  },
};
