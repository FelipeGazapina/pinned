#!/usr/bin/env node
//   SWEEP_DIR=... SWEEP_REAL_CLAUDE=~/.local/bin/claude scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/go-thread.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// GO_THREAD_VARIANTS picks variants by name (comma-separated); every one runs by default. Each
// variant saves what the app showed once its checks ended as $SWEEP_DIR/go-thread-<variant>.png.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const MOCK = `http://127.0.0.1:${process.env.SWEEP_GO_PORT || 8798}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const HOME = path.join(DIR, "home");
const KEY = "oc_sk_sweep";

const check = (ok, what) => {
  if (!ok) throw new Error(what);
};
const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};

function transcriptTimes(session, t0) {
  const projects = path.join(DIR, "node/accounts/go/projects");
  const file = fs.existsSync(projects) && fs.readdirSync(projects).map((p) => path.join(projects, p, `${session}.jsonl`)).find((f) => fs.existsSync(f));
  if (!file) return {};
  const seen = {};
  for (const line of fs.readFileSync(file, "utf8").split("\n").filter(Boolean)) {
    const entry = JSON.parse(line);
    const blocks = entry.type === "assistant" ? entry.message.content : [];
    const at = +((Date.parse(entry.timestamp) - t0) / 1000).toFixed(2);
    for (const b of blocks) {
      const name = b.type === "thinking" ? "disk.thinking" : b.type === "tool_use" ? (b.name === "Bash" ? "disk.bash" : b.name === "AskUserQuestion" ? "disk.ask" : null) : null;
      if (name && !(name in seen)) seen[name] = at;
    }
  }
  return seen;
}

(async () => {
  const browser = await playwright.chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.exposeFunction("goThreadCapture", (variant) => page.screenshot({ path: path.join(DIR, `go-thread-${variant}.png`) }));

  await page.goto(HUB);
  await page.fill("#login [name=email]", EMAIL);
  await page.fill("#login [name=password]", PASSWORD);
  await page.click("#login .primary");
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  await waitFor("the sweep node", () => page.evaluate(() => [...machines.values()].some((m) => m.view.online && m.view.name === "sweep-node")));
  const machine = await page.evaluate(() => [...machines.keys()].find((k) => machines.get(k).view.name === "sweep-node"));

  await page.evaluate(() => setMode("gui"));
  await page.click("#gui-settings");
  await page.click('#settings-nav [data-page="accounts"]');
  await page.waitForSelector("#account-list .card", { timeout: 15000 });
  await page.selectOption(".add-account [name=provider]", "opencode-go");
  await page.fill(".add-account [name=name]", "Go");
  await page.fill(".add-account [name=key]", KEY);
  await page.click(".add-account .primary");
  await page.waitForSelector('#account-list .card[data-account="go"]', { timeout: 15000 });
  check(/Signed in/.test(await page.textContent('#account-list .card[data-account="go"]')), "the Go account did not sign in");
  await page.evaluate(() => toggleSettings());

  await page.addScriptTag({ path: path.join(__dirname, "go-thread-page.js") });
  const wanted = (process.env.GO_THREAD_VARIANTS || "").split(",").filter(Boolean);
  const variants = (await page.evaluate(() => GO_THREAD_VARIANTS)).filter((v) => !wanted.length || wanted.includes(v.variant));
  const results = [];
  for (const v of variants) {
    const result = await page.evaluate((o) => goThread(o), { ...v, machine, account: "go", folder: path.join(HOME, "work/alpha"), mock: MOCK });
    if (result.session) Object.assign(result.seen, transcriptTimes(result.session, result.t0));
    results.push(result);
    console.log(`${result.ok ? "PASS" : "FAIL"}  ${result.variant}  ${JSON.stringify(result)}`);
  }
  await browser.close();
  if (errors.length) console.log(`page errors: ${errors.slice(0, 3).join(" | ")}`);
  process.exit(results.every((r) => r.ok) ? 0 : 1);
})().catch((error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
