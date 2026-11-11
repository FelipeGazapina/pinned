#!/usr/bin/env node
// Drives delegate_task end to end on the real page: alpha's Captain calls the tool twice through
// pinned's MCP server, as Claude Code does, and each child is a worker thread nested under it in
// the sidebar, one still working and one done, at 1440x900 and on a 390x844 phone.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/child-threads.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes child-threads*.png into scripts/web-sweep/screens/. The browser is the one global-chat.js
// picks, driven over the DevTools protocol as global-chat.js drives it.
const { spawn } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const WORK = path.join(DIR, "home", "work");
const SCREENS = path.join(__dirname, "screens");

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
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "child-threads-"));
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
      await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });
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

// Runs in the page: the sidebar from the top, each element as a token.
const sidebarTokens = () => [...document.querySelectorAll("#thread-list > *")].map((el) => {
  if (el.matches(".project-head")) return `project ${[...el.querySelectorAll(".name, .state, .where")].map((e) => e.textContent).join(" ")}`;
  if (el.matches(".thread-row")) return `${el.classList.contains("child") ? `  `.repeat(Number(el.style.getPropertyValue("--depth"))) : ""}${el.dataset.tab}`;
  return el.className;
});

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

(async () => {
  const errors = [];
  const browser = await launchBrowser(errors);
  console.log(`chromium ${chromium}`);
  try {
    const page = await browser.tab(false);
    await page.size(1440, 900);
    const node = await login(page);
    const alpha = fs.realpathSync(path.join(WORK, "alpha"));
    const captain = await page.evaluate(async ({ machine, alpha }) => {
      const tab = await revealTab(machine, await callOn(machine, { cmd: "open_orchestrator", path: alpha }));
      activate(tab);
      startTab(tab);
      return { key: tab.key, id: tab.view.id };
    }, { machine: node, alpha });
    await waitFor("the Captain's session", () => page.evaluate((k) => tabByKey(k)?.view.running === true, captain.key));
    await waitFor("the Captain's prompt", () => page.evaluate((k) => tabByKey(k)?.view.session != null || tabByKey(k)?.view.running, captain.key));

    // The calls a Captain's Claude Code makes for one request: the account, model and effort go in target.
    const calls = [
      { task: "Run the CI on main and report what fails. [hold]", title: "Run the CI on main", target: { providerInstanceId: "claude", model: "claude-sonnet-5-5", options: { effort: "high" } } },
      { task: "Write the changelog for v0.17.21.", title: "Changelog for v0.17.21", target: { providerInstanceId: "claude", model: "claude-haiku-5-5" } },
    ];
    const count = () => page.evaluate((id) => tabs.filter((t) => t.view.spawned_by === id).length, captain.id);
    const before = await count();
    fs.writeFileSync(path.join(DIR, "home", "delegate-calls.jsonl"), calls.map((c) => JSON.stringify(c)).join("\n"));
    await sendText(page, "Release v0.17.21 once the CI passes, with its changelog.");
    await waitFor("both children", async () => (await count()) === before + 2, 30000);
    const children = await page.evaluate((id) => tabs.filter((t) => t.view.spawned_by === id).sort((a, b) => a.view.opened - b.view.opened).slice(-2)
      .map((t) => ({ key: t.key, title: t.view.title, role: t.view.role, running: t.view.running })), captain.id);
    console.log("children:", JSON.stringify(children));
    check(children.length === 2 && children.every((c) => c.role === "worker" && c.running), `two running worker threads: ${JSON.stringify(children)}`);
    check(JSON.stringify(children.map((c) => c.title)) === JSON.stringify(calls.map((c) => c.title)), `their titles: ${JSON.stringify(children)}`);
    await waitFor("the changelog worker to finish", () => page.evaluate((k) => statusOf(tabByKey(k)) === "done", children[1].key), 30000);
    await waitFor("the CI worker to work", () => page.evaluate((k) => statusOf(tabByKey(k)) === "working", children[0].key), 30000);
    await page.evaluate((k) => activate(tabByKey(k)), captain.key);
    await waitFor("the Captain's turn to end", () => page.evaluate((k) => ["done", "delegating"].includes(phaseOf(tabByKey(k))), captain.key), 30000);
    await waitFor("the delegate_task calls in the chat", () => page.evaluate(() => /Started 2 workers/.test(document.getElementById("thread").innerText)), 30000);

    const tokens = await page.evaluate(sidebarTokens);
    console.log("sidebar:", JSON.stringify(tokens, null, 1));
    const at = tokens.indexOf(captain.key);
    check(at >= 0 && JSON.stringify(tokens.slice(at + 1, at + 3)) === JSON.stringify(children.map((c) => `  ${c.key}`)),
      `the Captain with both children nested under it: ${JSON.stringify(tokens.slice(at, at + 3))}`);
    // The Captain's row names its role on its second line and sums up its working child in its slot.
    const slot = await page.evaluate((k) => {
      const row = document.querySelector(`.thread-row[data-tab="${k}"]`);
      return row && `${row.querySelector(".role")?.textContent} | ${row.querySelector(".stamp.status")?.textContent.trim()}`;
    }, captain.key);
    console.log("captain card:", JSON.stringify(slot));
    check(/^Captain \| 1 working/.test(slot ?? ""), `the Captain's card sums up its working child: ${slot}`);
    await page.screenshot(path.join(SCREENS, "child-threads.png"));

    await page.click(`.thread-row[data-tab="${children[0].key}"]`);
    await waitFor("the CI worker's thread", () => page.evaluate((k) => active?.key === k && document.getElementById("thread").innerText.includes("Run the CI on main"), children[0].key), 30000);
    await page.screenshot(path.join(SCREENS, "child-thread-open.png"));

    const small = await browser.tab(true);
    await small.goto(HUB);
    await waitFor("the hub on the phone", () => small.evaluate(() => document.body.dataset.hub === "online"));
    await small.evaluate(() => setMode("gui"));
    await waitFor("the children on the phone", () => small.evaluate((id) => tabs.filter((t) => t.view.spawned_by === id).length === 2, captain.id));
    await small.evaluate((k) => activate(tabByKey(k)), captain.key);
    await small.evaluate(() => toggleSidebar(true));
    await waitFor("the phone's sidebar", () => small.evaluate((k) => !!document.querySelector(`.thread-row[data-tab="${k}"]`), captain.key));
    await small.evaluate((k) => document.querySelector(`.thread-row[data-tab="${k}"]`).scrollIntoView({ block: "center" }), captain.key);
    const phoneTokens = await small.evaluate(sidebarTokens);
    const phoneAt = phoneTokens.indexOf(captain.key);
    check(JSON.stringify(phoneTokens.slice(phoneAt + 1, phoneAt + 3)) === JSON.stringify(children.map((c) => `  ${c.key}`)), `phone sidebar: ${JSON.stringify(phoneTokens)}`);
    await small.screenshot(path.join(SCREENS, "child-threads-phone.png"));

    check(!errors.length, `page errors:\n${errors.slice(0, 5).join("\n")}`);
    console.log("child threads: every check passed");
  } finally {
    const exited = new Promise((resolve) => browser.proc.once("exit", resolve));
    browser.proc.kill("SIGTERM");
    await exited;
    fs.rmSync(browser.profile, { recursive: true, force: true, maxRetries: 5 });
  }
})().catch((error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
