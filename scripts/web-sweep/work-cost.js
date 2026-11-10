#!/usr/bin/env node
// What a piece of work costs: the Captain card shows its session plus its workers ("$1.26 · 2 workers"),
// the project head and the projects page show the project total, and a box chip whose box_status
// fails says "box cost unavailable". Desktop 1440x900 and phone 390x844, shots in SHOTS.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/work-cost.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// The node does not compute the rollup in this stack (no worker has run): the page is handed the `work`
// the node would send, so this checks what the UI draws from it. The rollup itself is a core test.
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
  const found = fs.readdirSync(root).filter((d) => /^chromium-\d+$/.test(d)).sort().reverse()
    .map((d) => path.join(root, d, "chrome-mac-arm64", "Google Chrome for Testing.app", "Contents", "MacOS", "Google Chrome for Testing"))
    .find((p) => fs.existsSync(p));
  if (!found) throw new Error(`no cached Chromium under ${root}; set CHROME_BIN`);
  return found;
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
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "work-cost-"));
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

const AS = { captain: { active_ms: 72 * 60000, cost_usd: 0.84 }, worker: { active_ms: 4 * 60000, cost_usd: 0.42 }, remote: { active_ms: 9 * 60000, cost_usd: 0.3 } };

/** What the node would send for a Captain with two workers, one of them on another machine. */
const patch = () => {
  const captain = tabs.find((t) => t.view.role === "orchestrator");
  const workers = tabs.filter((t) => t.view.role === "worker").slice(0, 2);
  if (!captain || workers.length < 2) return false;
  captain.view.stats = window.AS.captain;
  captain.view.work = { usd: 1.56, unknown: 0, workers: 2 };
  workers.forEach((w, i) => {
    w.view.spawned_by = captain.view.id;
    w.view.stats = i ? window.AS.remote : window.AS.worker;
    w.view.status = "working";
  });
  sideSettings.grouping = true;
  renderSidebar();
  return true;
};

const openDrawer = (p) => p.evaluate(() => { if (document.body.dataset.sidebar !== "open") document.getElementById("chat-sidebar-toggle")?.click(); });

(async () => {
  const errors = [];
  const browser = await launchBrowser(errors);
  try {
    fs.mkdirSync(SCREENS, { recursive: true });
    const page = await browser.tab(false);
    const node = await login(page);
    fs.mkdirSync(path.join(WORK, "gamma"), { recursive: true });
    const dir = fs.realpathSync(path.join(WORK, "gamma"));
    await page.evaluate(async ({ machine, dir }) => {
      await revealTab(machine, await callOn(machine, { cmd: "open_orchestrator", path: dir }));
      await openRemote(machine, dir, false);
      await openRemote(machine, dir, false);
    }, { machine: node, dir });
    await page.evaluate((as) => { window.AS = as; }, AS);
    await page.evaluate(() => { for (const t of tabs.filter((t) => t.view.role !== "orchestrator").slice(0, 2)) t.view.role = "worker"; });
    for (const [name, phone] of [["1440x900", false], ["390x844", true]]) {
      const p = phone ? await browser.tab(true) : page;
      if (phone) {
        await login(p);
        await p.evaluate(async ({ machine, dir }) => { await openRemote(machine, dir, false); }, { machine: node, dir }).catch(() => {});
        await p.evaluate((as) => { window.AS = as; }, AS);
        await waitFor("the threads on the phone", () => p.evaluate(() => tabs.length >= 3));
        await p.evaluate(() => { for (const t of tabs.filter((t) => t.view.role !== "orchestrator").slice(0, 2)) t.view.role = "worker"; });
      }
      const draw = () => p.evaluate(patch);
      await waitFor("the Captain and two workers", draw);
      if (phone) await openDrawer(p);
      await sleepMs(500);
      await draw();
      const seen = await p.evaluate(() => ({
        captain: document.querySelector(".thread-row [data-stats]")?.textContent,
        stats: [...document.querySelectorAll(".thread-row .stats")].map((e) => e.textContent),
        head: document.querySelector(".project-head .state.cost")?.textContent,
      }));
      console.log(name, JSON.stringify(seen), JSON.stringify(await p.evaluate(() => ({ sidebar: document.body.dataset.sidebar, rows: document.querySelectorAll(".thread-row").length, tabs: tabs.length, roles: tabs.map((t) => t.view.role) }))));
      check(seen.stats.includes("1h 12m · $1.56 · 2 workers"), `${name}: the Captain card says ${JSON.stringify(seen.stats)}`);
      check(seen.head === "$1.56 · 2 workers", `${name}: the project head says ${seen.head}`);
      await p.screenshot(path.join(SCREENS, `real-captain-${name}.png`));

      // The projects page: its row shows the project total.
      await p.evaluate(() => { openGlobalProjects(); });
      await waitFor("the projects page row", () => p.evaluate(() => !!document.querySelector(".gp-row .gp-cost")));
      const row = await p.evaluate(() => document.querySelector(".gp-row .gp-cost").textContent);
      check(row === "$1.56 · 2 workers", `${name}: the projects page row says ${row}`);
      await sleepMs(300);
      await p.screenshot(path.join(SCREENS, `real-projects-${name}.png`));

      // A box tab whose box_status fails keeps its chip and says why. The real refresh runs and fails: this node is no box.
      await p.evaluate((m) => { machines.get(m).view.name = "Box · gamma"; }, node);
      await p.evaluate(() => refreshBoxInfo());
      await p.evaluate(patch);
      await p.evaluate(() => activate(tabs.find((x) => x.view.role === "orchestrator")));
      if (phone) await openDrawer(p);
      await sleepMs(500);
      const chip = await p.evaluate(() => document.querySelector(".thread-row .remote.box.unavailable .head")?.textContent ?? null);
      check(chip === "box cost unavailable", `${name}: the box chip says ${chip}`);
      await p.screenshot(path.join(SCREENS, `real-box-${name}.png`));
      await p.evaluate((m) => { machines.get(m).view.name = "sweep-node"; }, node);
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
