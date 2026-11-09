const path = require("path");
const pw = require("/private/tmp/pinned-playwright/node_modules/playwright");
const HUB = "http://127.0.0.1:8871";
const OUT = path.join(__dirname, "shots");
const PREFIX = process.argv[2] || "app";
const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) { const v = await fn(); if (v) return v; await new Promise((r) => setTimeout(r, 100)); }
  throw new Error(`timed out waiting for ${what}`);
};
async function signIn(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", "sweep@example.test");
    await page.fill("#login [name=password]", "sweep password 1");
    await page.click("#login .primary");
  }
  await waitFor("hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  return waitFor("node", () => page.evaluate(() => [...machines.entries()].find(([, m]) => m.view.online && m.view.name === "sweep-node")?.[0]));
}
const fs = require("fs");
const WORK = path.join(__dirname, "sweep", "home", "work");
(async () => {
  const browser = await pw.chromium.launch({ headless: true, channel: "chrome" });
  for (const [size, vp] of [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]]) {
    const ctx = await browser.newContext({ viewport: vp, deviceScaleFactor: size === "phone" ? 2 : 1, colorScheme: "dark", isMobile: size === "phone", hasTouch: size === "phone" });
    const page = await ctx.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    const node = await signIn(page);
    await page.evaluate(() => setMode("gui"));
    const dirs = ["alpha", "beta"].map((n) => fs.realpathSync(path.join(WORK, n)));
    await page.evaluate(async ({ node, dirs }) => {
      for (const d of dirs) if (!tabs.some((t) => t.machine === node && t.view.cwd === d)) await openRemote(node, d, false);
    }, { node, dirs });
    await page.evaluate(() => openPage("taste"));
    await waitFor("page", () => page.isVisible(".taste-split"));
    await waitFor("project tab", () => page.isVisible('.taste-scopes button:text-is("Captain alpha")'));
    for (const [tab, name] of [["Orchestrator", "orchestrator"], ["Captains (all)", "captains"], ["Captain alpha", "project"]]) {
      await page.click(`.taste-scopes button:text-is("${tab}")`);
      if (name === "project" && !(await page.isVisible(".taste-tag.changed"))) {
        await page.click('button[aria-label="More parallel workers"]');
        await page.click('button[aria-label="More parallel workers"]');
        await page.click('.taste-foot [data-act="save"]');
        await waitFor("saved", async () => !(await page.isVisible(".taste-foot")));
      }
      await page.evaluate(() => document.querySelector("#page-view .page-body")?.scrollTo(0, 0));
      await page.waitForTimeout(1600);
      await page.screenshot({ path: path.join(OUT, `${PREFIX}-${name}-${size}.png`) });
    }
    console.log(size, "errors:", errors);
    await ctx.close();
  }
  await browser.close();
})().catch((e) => { console.error(e); process.exit(1); });
