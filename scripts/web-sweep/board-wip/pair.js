// node pair.js <board.png> <app.png> <out.png>: board left, app right, one labelled image.
const fs = require("fs");
const path = require("path");
const pw = require("/private/tmp/pinned-playwright/node_modules/playwright");
(async () => {
  const pairs = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
  const browser = await pw.chromium.launch({ headless: true, channel: "chrome" });
  const page = await browser.newPage({ deviceScaleFactor: 1 });
  for (const [board, app, out, width] of pairs) {
    const img = (p) => `data:image/png;base64,${fs.readFileSync(p).toString("base64")}`;
    const w = width || 1440;
    await page.setViewportSize({ width: w * 2 + 48, height: 400 });
    await page.setContent(`<body style="margin:0;background:#222;font:600 20px system-ui;color:#eee">
      <div style="display:flex;gap:16px;padding:16px;align-items:flex-start">
      <figure style="margin:0;width:${w}px"><figcaption style="padding:0 0 8px">Board (prototype/live/Taste.dc.html)</figcaption><img style="width:${w}px;display:block" src="${img(board)}"></figure>
      <figure style="margin:0;width:${w}px"><figcaption style="padding:0 0 8px">App (web sweep)</figcaption><img style="width:${w}px;display:block" src="${img(app)}"></figure></div></body>`);
    await page.waitForFunction(() => [...document.images].every((i) => i.complete));
    await page.screenshot({ path: out, fullPage: true });
    console.log("wrote", out);
  }
  await browser.close();
})();
