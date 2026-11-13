// The real ui/ in headless Chromium against a fake hub: the WebSocket is routed, and `stuck_boxes`
// and `box_usage` answer fixed fake data (ids like box_fake_1). No real hub, no Upstash. Fake ids and text only.
const http = require("http");
const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright-core");

const UI = process.argv[2];
const OUT = process.argv[3];
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".webmanifest": "application/json", ".png": "image/png", ".woff2": "font/woff2" };

const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://x");
  if (url.pathname === "/api/version") return res.end("0.18.2-fake");
  const file = path.join(UI, url.pathname === "/" ? "index.html" : decodeURIComponent(url.pathname));
  if (!file.startsWith(UI) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) return (res.statusCode = 404), res.end();
  res.setHeader("content-type", TYPES[path.extname(file)] ?? "application/octet-stream");
  res.end(fs.readFileSync(file));
});

const cut = (text) => `${text.slice(0, 299)}…`;
const LONG = cut("Upstash answered 503: fake-upstream-detail ".repeat(20));
const known = (n, why, extra = {}) => ({ id: `fake-row-${n}`, why, upstash: `box_fake_${n}`, ...extra });
const unconfirmed = (n, why) => ({ id: `fake-row-u${n}`, why, unconfirmed: true, label: `p:fake-row-u${n}` });
const SCENARIOS = {
  "known-one": [known(1, "Upstash answered 503")],
  "unconfirmed-one": [unconfirmed(1, "Upstash answered 502")],
  "known-several": [
    known(1, "Upstash answered 503"),
    known(2, "Upstash answered 503"),
    known(3, "Upstash Box refused the key: Invalid box API key"),
    known(4, "Upstash answered 503"),
    known(5, "Upstash answered 503"),
  ],
  "unconfirmed-several": [unconfirmed(1, "Upstash is not connected"), unconfirmed(2, "Upstash is not connected"), unconfirmed(3, "Upstash answered 502")],
  "both-kinds": [known(1, "Upstash answered 503"), known(2, "Upstash answered 503"), unconfirmed(1, "Upstash is not connected"), unconfirmed(2, "Upstash answered 502")],
  "long-reason": [known(1, LONG)],
  // What the real hub answers with no Upstash key (boxes.rs: BoxUsage { stuck, ..default }): connected false, so no "From Upstash" card.
  "not-connected": { connected: false, stuck: [known(1, "Upstash is not connected"), known(2, "Upstash is not connected"), unconfirmed(1, "Upstash is not connected")] },
};

const usage = (stuck, connected = true) => (connected
  ? { connected, error: null, boxes: [], upstash: { cpu_hours: 0.0, compute_usd: 0.0, boxes: 0 }, pinned: { box_hours: 0.0, tasks: 0, since: 0 }, stuck }
  : { connected, boxes: [], upstash: { cpu_hours: 0.0, compute_usd: 0.0, boxes: 0 }, pinned: { box_hours: 0.0, tasks: 0, since: 0 }, stuck });

(async () => {
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ executablePath: "/usr/bin/chromium", args: ["--no-sandbox"] });
  for (const [name, scenario] of Object.entries(SCENARIOS)) {
    const { stuck, connected } = Array.isArray(scenario) ? { stuck: scenario, connected: true } : scenario;
    const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    await context.addInitScript(() => localStorage.setItem("hub-token", "fake-token-0000"));
    const page = await context.newPage();
    const asked = [];
    page.on("console", (m) => m.type() === "error" && console.log(`[${name}] console error: ${m.text()}`));
    await page.routeWebSocket(/\/ws\/client$/, (ws) => {
      ws.onMessage((raw) => {
        const frame = JSON.parse(raw);
        if (frame.type === "auth") return ws.send(JSON.stringify({ type: "hello", account: { id: "acct-fake-0001", email: "fake-account" }, machines: [], tabs: {} }));
        if (frame.type !== "hub") return;
        asked.push(frame.call.cmd);
        const answer = { stuck_boxes: stuck, box_usage: usage(stuck, connected), get_settings: { connectors: [], upstash: null } }[frame.call.cmd];
        ws.send(JSON.stringify(answer === undefined ? { type: "reply", id: frame.id, err: "fake hub: not answered" } : { type: "reply", id: frame.id, ok: answer }));
      });
    });
    await page.goto(base);
    await page.waitForSelector('[data-nav="usage"] .stuck-box:not([hidden])', { timeout: 15000 });
    await page.click('[data-nav="usage"]');
    await page.waitForSelector(".box-usage .limit-notices", { timeout: 15000 });
    if (!connected) await page.waitForSelector(".box-usage .limit-notices ~ .muted.note, .box-usage .muted.note", { timeout: 15000 });
    await page.waitForFunction(() => document.querySelector(".box-usage")?.getAttribute("aria-busy") === "false");
    await page.screenshot({ path: path.join(OUT, `${name}.png`) });
    const text = await page.locator(".box-usage .limit-notices").allInnerTexts();
    console.log(`${name}: asked ${[...new Set(asked)]}; mark title: ${await page.getAttribute('[data-nav="usage"]', "title")}`);
    for (const t of text) console.log(`  notice: ${t.replace(/\s+/g, " ")}`);
    await context.close();
  }
  await browser.close();
  server.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
