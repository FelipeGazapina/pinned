// Phone-Thread.dc.html: the Captain "Release v0.17.21" on a phone, with the Workers pill under the
// header, the workers in the chat and the README worker's banner above the composer. The board
// draws an iPhone's status bar and home indicator, so the page gets the same safe-area insets.

/** Gives the page an iPhone's safe-area insets, which the app pads its shell with. */
async function phoneInsets(page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setSafeAreaInsetsOverride", { insets: { top: 54, bottom: 34 } });
}

/** Opens `key`'s thread as a phone shows it, with nothing focused. */
async function openThread(page, ctx, key) {
  await phoneInsets(page);
  await page.evaluate((key) => {
    activate(tabByKey(key));
    toggleSidebar(false);
    document.activeElement?.blur();
  }, key);
  await ctx.waitFor("the thread on screen", () => page.evaluate((key) => active?.key === key && !document.getElementById("chat-head").hidden, key));
}

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
  phoneInsets,
  openThread,
  async open(page, ctx) {
    await openThread(page, ctx, ctx.threads.release);
    await ctx.waitFor("the Workers pill", () => page.evaluate(() => !document.getElementById("workers-pill").hidden), 15000);
  },
};
