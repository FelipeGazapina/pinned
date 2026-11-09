const path = require("path");
const pw = require("/private/tmp/pinned-playwright/node_modules/playwright");
const DIR = path.join(__dirname, "board");
const OUT = path.join(__dirname, "shots");
(async () => {
  const browser = await pw.chromium.launch({ headless: true, channel: "chrome" });
  for (const [size, vp] of [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]]) {
    const ctx = await browser.newContext({ viewport: vp, deviceScaleFactor: size === "phone" ? 2 : 1, colorScheme: "dark" });
    const page = await ctx.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("file://" + path.join(DIR, "Taste.dc.html"));
    await page.waitForSelector("main h1");
    for (const [tab, name] of [["Orquestrador", "orchestrator"], ["Capitães (todos)", "captains"], ["Capitão pinned", "project"]]) {
      await page.click(`main button:text-is("${tab}")`);
      // Board main is a scrolling column? scroll it into view.
      await page.evaluate((phone) => window.scrollTo(0, phone ? document.querySelector("main").offsetTop : 0), size === "phone");
      await page.screenshot({ path: path.join(OUT, `board-${name}-${size}.png`) });
      await page.screenshot({ path: path.join(OUT, `board-${name}-${size}-full.png`), fullPage: true });
    }
    await page.goto("file://" + path.join(DIR, "Brainstorm.dc.html"));
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(OUT, `board-brainstorm-${size}.png`) });
    console.log(size, "errors:", errors);
    await ctx.close();
  }
  await browser.close();
})();
