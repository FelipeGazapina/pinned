#!/usr/bin/env node
// A long Captain report in the global orchestrator's chat folds to a few lines under "Show more",
// shows its whole text on a click, and a short report is drawn as it was. Desktop 1440x900 and
// phone 390x844, shots in SHOTS (default scripts/web-sweep/screens/).
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/captain-report-clamp.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Same harness as global-chat.js: Chromium over the DevTools protocol, one node that hosts the
// orchestrator, fake-claude echoing the pasted report.
const { spawn } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const WORK = path.join(DIR, "home", "work");
const SCREENS = process.env.SHOTS || path.join(__dirname, "screens");

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  return true;
};
const sleepMs = (ms) => new Promise((r) => setTimeout(r, ms));
const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};

const chromium = process.env.CHROME_BIN || (() => {
  const root = process.env.PLAYWRIGHT_BROWSERS_PATH || path.join(process.env.HOME || "", "Library", "Caches", "ms-playwright");
  const found = (fs.existsSync(root) ? fs.readdirSync(root) : []).filter((d) => /^chromium-\d+$/.test(d)).sort().reverse()
    .map((d) => path.join(root, d, "chrome-mac-arm64", "Google Chrome for Testing.app", "Contents", "MacOS", "Google Chrome for Testing"))
    .find((p) => fs.existsSync(p));
  return found || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
})();

class Cdp {
  constructor(socket) {
    this.socket = socket;
    this.id = 0;
    this.pending = new Map();
    this.listeners = [];
    socket.addEventListener("message", (ev) => {
      const m = JSON.parse(ev.data);
      if (m.id && this.pending.has(m.id)) {
        const { resolve, reject } = this.pending.get(m.id);
        this.pending.delete(m.id);
        m.error ? reject(new Error(m.error.message)) : resolve(m.result);
      } else this.listeners.forEach((l) => l(m));
    });
  }
  send(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = ++this.id;
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  on(listener) { this.listeners.push(listener); }
  async evaluate(fn, arg) {
    const r = await this.send("Runtime.evaluate", { expression: `(${fn})(${JSON.stringify(arg ?? null)})`, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result.value;
  }
  async goto(url) {
    const loaded = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`timed out loading ${url}`)), 20000);
      this.listeners.push((m) => m.method === "Page.loadEventFired" && (clearTimeout(timer), resolve()));
    });
    await this.send("Page.navigate", { url });
    await loaded;
  }
  async shotOf(selector, file) {
    const box = await this.evaluate((s) => {
      const r = document.querySelector(s).getBoundingClientRect();
      return { x: r.left, y: r.top, width: r.width, height: r.height };
    }, selector);
    const shot = await this.send("Page.captureScreenshot", { format: "png", clip: { ...box, scale: 1 } });
    fs.writeFileSync(file, Buffer.from(shot.data, "base64"));
  }
  // Captured at CSS pixels, so the phone's 3x screen does not triple the committed file.
  async screenshot(file) {
    const { cssLayoutViewport: { clientWidth: width, clientHeight: height } } = await this.send("Page.getLayoutMetrics");
    const shot = await this.send("Page.captureScreenshot", { format: "png", clip: { x: 0, y: 0, width, height, scale: 1 / this.scale } });
    fs.writeFileSync(file, Buffer.from(shot.data, "base64"));
  }
}

async function launchBrowser(errors) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "captain-report-clamp-"));
  const proc = spawn(chromium, [
    "--headless=new", "--remote-debugging-port=0", "--remote-allow-origins=*",
    "--no-first-run", "--no-default-browser-check", "--disable-gpu", "--hide-scrollbars",
    `--user-data-dir=${profile}`, "about:blank",
  ], { stdio: ["ignore", "pipe", "pipe"] });
  const ws = await new Promise((resolve, reject) => {
    const fail = setTimeout(() => reject(new Error("Chromium never said its DevTools port")), 15000);
    let buf = "";
    const on = (chunk) => { buf += chunk; const m = buf.match(/DevTools listening on (ws:\/\/\S+)/); if (m) { clearTimeout(fail); resolve(m[1]); } };
    proc.stderr.on("data", on);
    proc.stdout.on("data", on);
    proc.on("exit", () => reject(new Error(`Chromium exited early: ${buf.slice(-300)}`)));
  });
  const base = `http://${ws.match(/^ws:\/\/(\S+?)\//)[1]}`;
  const tab = async (phone) => {
    const target = await (await fetch(`${base}/json/new?about:blank`, { method: "PUT" })).json();
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = () => reject(new Error("the DevTools socket did not open")); });
    const cdp = new Cdp(socket);
    cdp.scale = phone ? 3 : 1;
    await cdp.send("Runtime.enable");
    await cdp.send("Page.enable");
    cdp.on((m) => m.method === "Runtime.exceptionThrown" && errors.push(m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text));
    await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
    if (phone) {
      await cdp.send("Emulation.setDeviceMetricsOverride", { width: 390, height: 844, deviceScaleFactor: cdp.scale, mobile: true });
      await cdp.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 5 });
      await cdp.send("Emulation.setUserAgentOverride", { userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1" });
    } else {
      await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
    }
    return {
      cdp,
      evaluate: (fn, arg) => cdp.evaluate(fn, arg),
      click: (selector) => cdp.evaluate((s) => document.querySelector(s).click(), selector),
      goto: (url) => cdp.goto(url),
      screenshot: (file) => cdp.screenshot(file),
      shotOf: (selector, file) => cdp.shotOf(selector, file),
      size: (width, height) => cdp.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: cdp.scale, mobile: !!phone }),
    };
  };
  return { proc, profile, tab };
}

const login = async (page) => {
  await page.goto(HUB);
  await page.evaluate(({ email, password }) => {
    const form = document.querySelector("#login");
    if (form) {
      form.querySelector("[name=email]").value = email;
      form.querySelector("[name=password]").value = password;
      form.querySelector(".primary").click();
    }
  }, { email: EMAIL, password: PASSWORD });
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  await waitFor("the sweep node", () => page.evaluate(() => [...machines.values()].some((m) => m.view.online && m.view.name === "sweep-node")));
  await page.evaluate(() => setMode("gui"));
  return page.evaluate(() => [...machines.keys()].find((k) => machines.get(k).view.name === "sweep-node"));
};

const sendText = (page, text) => page.evaluate((t) => {
  prompt.value = t;
  send();
}, text);


const long = Array.from({ length: 60 }, (_, i) => `- PR #${600 + i}: merged, CI green, ${i * 3} rows verified on shard ${i}.`).join("\n");
const rowState = () => {
  const row = document.querySelector(".report-row");
  const body = row?.querySelector(".body");
  const button = row?.querySelector(".show-full");
  return { rows: document.querySelectorAll(".report-row").length, height: Math.round(row?.getBoundingClientRect().height ?? 0), clamped: !!body?.classList.contains("clamped"), button: button?.textContent ?? null, text: body?.innerText ?? "" };
};

(async () => {
  const errors = [];
  const browser = await launchBrowser(errors);
  try {
    fs.mkdirSync(SCREENS, { recursive: true });
    const page = await browser.tab(false);
    const node = await login(page);
    const saved = await page.evaluate((machine) => callOn(machine, { cmd: "set_global_orchestrator_settings", settings: { host: true } }), node);
    check(saved?.host === true, `saved settings: ${JSON.stringify(saved)}`);
    await page.evaluate((detail) => document.dispatchEvent(new CustomEvent("global-orchestrator-settings", { detail })), saved);
    await waitFor("the orchestrator's row", () => page.evaluate(() => !!document.querySelector(".global-row")));
    await page.click(".global-row");
    await waitFor("the global tab", () => page.evaluate(() => active ? isGlobalOrchestratorTab(active) : false));
    await page.evaluate(() => startTab(tabs.find(isGlobalOrchestratorTab)));
    await waitFor("the global tab to start", () => page.evaluate(() => tabs.find(isGlobalOrchestratorTab)?.view.running === true));
    fs.mkdirSync(path.join(WORK, "alpha"), { recursive: true });
    const key = await page.evaluate(async ({ machine, dir }) => (await openRemote(machine, dir, false)).key, { machine: node, dir: fs.realpathSync(path.join(WORK, "alpha")) });
    const id = await page.evaluate((k) => tabByKey(k).view.id, key);
    await page.evaluate(() => activate(tabs.find(isGlobalOrchestratorTab)));
    const report = `[pinned] Captain alpha (sweep-node) [${node} ${id}]: Release v0.17.24 status, every PR in one list:\n${long}\nNext: I merge the base once CI is green.`;
    await sendText(page, report);
    await waitFor("the report row", () => page.evaluate(() => !!document.querySelector(".report-row")));
    await waitFor("the turn to end", () => page.evaluate(() => document.getElementById("thread").innerText.includes("echo:")));

    for (const [name, tab, w] of [["1440x900", page, "desktop"], ["390x844", null, "phone"]]) {
      let p = tab;
      if (!p) {
        p = await browser.tab(true);
        await login(p);
        await p.evaluate(() => activate(tabs.find(isGlobalOrchestratorTab)));
        await waitFor("the report row on the phone", () => p.evaluate(() => !!document.querySelector(".report-row")));
        await p.evaluate(() => { const bar = document.getElementById("chat-sidebar-toggle"); if (document.body.dataset.sidebar === "open") bar?.click(); });
      }
      await sleepMs(600);
      let s = await p.evaluate(rowState);
      console.log(w, "folded:", JSON.stringify({ ...s, text: s.text.slice(0, 40) }));
      check(s.clamped && s.button === "Show more", `${w}: the long report is not folded: ${JSON.stringify({ ...s, text: "" })}`);
      check(s.height < 330, `${w}: a folded report is ${s.height}px tall`);
      await p.evaluate(() => document.querySelector(".report-row").scrollIntoView({ block: "start" }));
      await sleepMs(300);
      await p.screenshot(path.join(SCREENS, `real-folded-${name}.png`));
      await p.click(".report-row .show-full");
      await waitFor("the report to open", () => p.evaluate(() => !document.querySelector(".report-row .body.clamped")));
      s = await p.evaluate(rowState);
      console.log(w, "open:", JSON.stringify({ ...s, text: s.text.slice(-60) }));
      check(s.button === "Show less" && s.text.includes("PR #659") && s.text.includes("Next: I merge the base"), `${w}: the whole report is not shown`);
      check(s.height > 900, `${w}: open height ${s.height}`);
      await p.evaluate(() => document.querySelector(".report-row").scrollIntoView({ block: "start" }));
      await sleepMs(300);
      await p.screenshot(path.join(SCREENS, `real-open-${name}.png`));
      await p.click(".report-row .show-full");
      await waitFor("the report to fold again", () => p.evaluate(() => !!document.querySelector(".report-row .body.clamped")));
    }
    check(errors.length === 0, `page errors: ${errors.join(" | ")}`);
    console.log("OK");
  } finally {
    browser.proc.kill();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
