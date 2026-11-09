#!/usr/bin/env node
// Drives the global orchestrator's chat on the real page: the sidebar row, the grouped §8 report
// rows, the Opções quick replies, Esperando você, and read aloud held back on desktop but spoken
// on a phone.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/global-chat.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes global-chat-*.png into scripts/web-sweep/screens/. The stack has one node, so that node
// hosts the orchestrator, and two folders of its own (site, pinned) stand in for the Capitães the
// reports name. The second tab replays the same account under iPhone emulation for read aloud.
//
// The browser is the machine's cached Chrome for Testing (the Playwright browsers cache; CHROME_BIN
// overrides it), driven over the DevTools protocol directly, so the sweep needs no npm install.
// The phone tab is that browser's touch emulation, which alone turns (pointer: coarse) true and
// (any-pointer: fine) false, the two halves of the app's touchComposer test.
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
  // Captured at CSS pixels, so the phone's 3x screen does not triple the committed file.
  async screenshot(file) {
    const { cssLayoutViewport: { clientWidth: width, clientHeight: height } } = await this.send("Page.getLayoutMetrics");
    const shot = await this.send("Page.captureScreenshot", { format: "png", clip: { x: 0, y: 0, width, height, scale: 1 / this.scale } });
    fs.writeFileSync(file, Buffer.from(shot.data, "base64"));
  }
}

async function launchBrowser(errors) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "global-chat-"));
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
    };
  };
  return { proc, profile, tab };
}

// Runs in the page: everything the assertions read.
const state = () => ({
  row: document.querySelector(".global-row")?.innerText.replace(/\s+/g, " ").trim() ?? null,
  projectsLink: !!document.querySelector("[data-global-projects]"),
  waitingHead: document.querySelector(".waiting-head")?.innerText.replace(/\s+/g, " ").trim() ?? null,
  waitingRows: [...document.querySelectorAll(".waiting-row")].map((r) => r.innerText.replace(/\s+/g, " ").trim()),
  threadRows: [...document.querySelectorAll("#thread-list .thread-row")].map((r) => r.dataset.tab),
  orquestradorProject: projectGroups().some((g) => /orquestrador/i.test(g.name)),
  fold: document.querySelector(".report-fold")?.innerText.replace(/\s+/g, " ").trim() ?? null,
  reportRows: [...document.querySelectorAll(".report-row")].map((r) => r.innerText.replace(/\s+/g, " ").trim()),
  quickReplies: [...document.querySelectorAll(".quick-reply")].map((b) => b.dataset.reply),
  thread: document.getElementById("thread").innerText,
  active: active?.key ?? null,
  activeGlobal: active ? isGlobalOrchestratorTab(active) : false,
  activeRunning: active?.view.running ?? null,
  touch: touchComposer(),
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
    const node = await login(page);

    // The node claims the host seat, as the settings page's save would make it.
    const saved = await page.evaluate((machine) => callOn(machine, { cmd: "set_global_orchestrator_settings", settings: { host: true } }), node);
    check(saved?.host === true && saved?.read_aloud === false, `saved settings: ${JSON.stringify(saved)}`);
    await page.evaluate((detail) => document.dispatchEvent(new CustomEvent("global-orchestrator-settings", { detail })), saved);
    await waitFor("the orchestrator's row", () => page.evaluate(() => !!document.querySelector(".global-row")));
    let s = await page.evaluate(state);
    console.log("the row:", JSON.stringify(s.row));
    check(s.row === "Orquestrador sweep-node", `row: ${s.row}`);
    check(!s.projectsLink, "the Projects link shows without the settings page");
    check(!s.orquestradorProject, "orquestrador is a project in the tree");

    await page.click(".global-row");
    await waitFor("the global tab", () => page.evaluate(() => active ? isGlobalOrchestratorTab(active) : false));
    s = await page.evaluate(state);
    const cwd = await page.evaluate(() => active.view.cwd);
    console.log("opened:", JSON.stringify({ active: s.active, running: s.activeRunning, cwd }));
    check(s.activeRunning === false, "the row click started the tab");
    check(/\.pinned\/orquestrador$/.test(cwd), `cwd: ${cwd}`);
    const globalKey = s.active;
    check(!s.threadRows.includes(globalKey), "the global tab is a loose thread in the list");
    await page.screenshot(path.join(SCREENS, "global-chat-open.png"));

    // The reports name real tabs, so their Captains open real threads.
    const keys = {};
    for (const name of ["site", "pinned"]) {
      fs.mkdirSync(path.join(WORK, name), { recursive: true });
      keys[name] = await page.evaluate(async ({ machine, dir }) => {
        const open = tabs.find((t) => t.machine === machine && t.view.cwd === dir);
        return (open ?? (await openRemote(machine, dir, false))).key;
      }, { machine: node, dir: fs.realpathSync(path.join(WORK, name)) });
    }
    const ids = await page.evaluate((k) => ({ site: tabByKey(k.site).view.id, pinned: tabByKey(k.pinned).view.id }), keys);
    const reports = `Relatórios de agora:
[pinned] Capitão site (sweep-node) [${node} ${ids.site}]: Publiquei a v2 e escrevi o changelog.
[pinned] Capitão pinned (sweep-node) [${node} ${ids.pinned}]: Fechei o PR #12 e apaguei a branch.`;

    await page.evaluate(() => activate(tabs.find(isGlobalOrchestratorTab)));
    await page.evaluate(() => startTab(tabs.find(isGlobalOrchestratorTab)));
    await waitFor("the global tab to start", () => page.evaluate(() => tabs.find(isGlobalOrchestratorTab)?.view.running === true));
    await sendText(page, reports);
    await waitFor("the report fold", () => page.evaluate(() => !!document.querySelector(".report-fold")));
    await waitFor("the turn to end", () => page.evaluate(() => document.getElementById("thread").innerText.includes("echo:")));
    s = await page.evaluate(state);
    console.log("reports:", JSON.stringify({ fold: s.fold, rows: s.reportRows }));
    check(s.fold === "2 atualizações · Capitão site, Capitão pinned", `fold: ${s.fold}`);
    check(s.reportRows.length === 0, "the reports show expanded while folded");
    await page.screenshot(path.join(SCREENS, "global-chat-reports-folded.png"));

    await page.click(".report-fold");
    await waitFor("the reports to expand", () => page.evaluate(() => document.querySelectorAll(".report-row").length === 2));
    s = await page.evaluate(state);
    check(s.reportRows[0].startsWith("Capitão site · sweep-node") && s.reportRows[0].includes("Publiquei a v2"), `site row: ${s.reportRows[0]}`);
    check(s.reportRows[1].startsWith("Capitão pinned · sweep-node") && s.reportRows[1].includes("Fechei o PR #12"), `pinned row: ${s.reportRows[1]}`);
    await page.screenshot(path.join(SCREENS, "global-chat-reports-open.png"));

    await page.click(".report-row .who");
    await waitFor("the site thread to open", () => page.evaluate((k) => active?.key === k, keys.site));
    await page.evaluate(() => activate(tabs.find(isGlobalOrchestratorTab)));

    await sendText(page, "options");
    await waitFor("the quick replies", () => page.evaluate(() => document.querySelectorAll(".quick-reply").length === 2));
    s = await page.evaluate(state);
    console.log("quick replies:", JSON.stringify(s.quickReplies));
    check(JSON.stringify(s.quickReplies) === JSON.stringify(["Sim", "Ainda não"]), `options: ${JSON.stringify(s.quickReplies)}`);
    check(s.thread.includes("Li os relatórios dos dois Capitães."), "the answer is missing");
    check(!s.thread.includes("Opções:"), "the Opções line shows as text");
    await page.screenshot(path.join(SCREENS, "global-chat-quick-replies.png"));

    await page.click(".quick-reply");
    await waitFor("Sim to send", () => page.evaluate(() => document.getElementById("thread").innerText.includes("echo: Sim")));
    s = await page.evaluate(state);
    console.log("after Sim:", JSON.stringify({ replies: s.quickReplies, active: s.active }));
    check(s.quickReplies.length === 0, "the buttons stayed after the click");
    check(!s.thread.includes("Opções:"), "the earlier answer shows its Opções line");
    check(/\nSim\n/.test(s.thread), "Sim did not become the user's message");
    await page.screenshot(path.join(SCREENS, "global-chat-quick-reply-sent.png"));

    // Claude Code's idle notice after a finished turn is needs-input too, but nobody waits on it.
    await page.evaluate((k) => activate(tabByKey(k)), keys.pinned);
    await sendText(page, "idle");
    await waitFor("the idle notice", () => page.evaluate((k) => {
      const view = tabByKey(k).view;
      return view.status === "needs-input" && !view.interrupted;
    }, keys.pinned));
    s = await page.evaluate(state);
    console.log("idle notice:", JSON.stringify({ head: s.waitingHead, rows: s.waitingRows }));
    check(s.waitingHead === null && s.waitingRows.length === 0, `the idle notice waits on Felipe: ${JSON.stringify(s.waitingRows)}`);

    await page.evaluate((k) => activate(tabByKey(k)), keys.site);
    await sendText(page, "/question");
    await waitFor("Esperando você", () => page.evaluate(() => !!document.querySelector(".waiting-row")));
    s = await page.evaluate(state);
    console.log("waiting:", JSON.stringify({ head: s.waitingHead, rows: s.waitingRows }));
    check(s.waitingHead === "Esperando você 1", `head: ${s.waitingHead}`);
    check(s.waitingRows.length === 1 && s.waitingRows[0].startsWith("pergunta") && s.waitingRows[0].endsWith("site · sweep-node"), `rows: ${JSON.stringify(s.waitingRows)}`);
    check(!s.threadRows.includes(globalKey), "the global tab joined the thread list");
    check(await page.evaluate((k) => tabByKey(k).view.status === "needs-input" && !tabByKey(k).view.interrupted, keys.pinned), "the pinned Captain left its idle notice");
    await page.screenshot(path.join(SCREENS, "global-chat-waiting.png"));

    await page.click(".waiting-row");
    await waitFor("the question's thread to open", () => page.evaluate((k) => active?.key === k, keys.site));

    // Read aloud stays silent on a desktop even with the setting on.
    await page.evaluate(() => {
      window.__spoken = [];
      if (typeof speechSynthesis !== "undefined" && speechSynthesis) {
        speechSynthesis.speak = (u) => window.__spoken.push({ text: u.text, lang: u.lang });
      }
    });
    const aloud = await page.evaluate((machine) => callOn(machine, { cmd: "set_global_orchestrator_settings", settings: { host: true, read_aloud: true } }), node);
    await page.evaluate((detail) => document.dispatchEvent(new CustomEvent("global-orchestrator-settings", { detail })), aloud);
    await page.evaluate(() => activate(tabs.find(isGlobalOrchestratorTab)));
    await sendText(page, "options");
    await waitFor("the second answer", () => page.evaluate(() => document.getElementById("thread").innerText.split("Li os relatórios").length > 2));
    s = await page.evaluate(state);
    const spoken = await page.evaluate(() => window.__spoken);
    console.log("desktop:", JSON.stringify({ touch: s.touch, spoken }));
    check(s.touch === false, "the desktop counts as a phone");
    check(spoken.length === 0, `the desktop spoke: ${JSON.stringify(spoken)}`);

    // The same account on a phone speaks the new answer, pt-BR, without the options.
    const phone = await browser.tab(true);
    await phone.goto(HUB);
    await waitFor("the hub on the phone", () => phone.evaluate(() => document.body.dataset.hub === "online"));
    await waitFor("the orchestrator's row on the phone", () => phone.evaluate(() => !!document.querySelector(".global-row")));
    check(await phone.evaluate(() => touchComposer() === true), "the phone emulation did not report a coarse pointer");
    await phone.evaluate(() => toggleSidebar(true));
    await phone.screenshot(path.join(SCREENS, "global-chat-phone-sidebar.png"));
    await phone.click(".global-row");
    await waitFor("the global tab on the phone", () => phone.evaluate(() => active ? isGlobalOrchestratorTab(active) : false));
    await phone.evaluate(() => {
      window.__spoken = [];
      if (typeof speechSynthesis !== "undefined" && speechSynthesis) {
        speechSynthesis.speak = (u) => window.__spoken.push({ text: u.text, lang: u.lang });
      }
    });
    await sendText(phone, "options");
    await waitFor("the phone to speak", () => phone.evaluate(() => window.__spoken.length >= 1));
    const phoneSpoken = await phone.evaluate(() => window.__spoken);
    console.log("phone:", JSON.stringify(phoneSpoken));
    check(JSON.stringify(phoneSpoken) === JSON.stringify([{ text: "Li os relatórios dos dois Capitães.", lang: "pt-BR" }]), `spoken: ${JSON.stringify(phoneSpoken)}`);
    await waitFor("the phone's quick replies", () => phone.evaluate(() => document.querySelectorAll(".quick-reply").length === 2));
    await phone.screenshot(path.join(SCREENS, "global-chat-phone.png"));

    check(!errors.length, `page errors:\n${errors.slice(0, 5).join("\n")}`);
    console.log("global chat: every check passed");
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
