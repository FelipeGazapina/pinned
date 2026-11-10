#!/usr/bin/env node
// Drives delegate_task with target.machine on the real page: alpha's Captain on sweep-node starts one
// worker there, one on a second node (sweep-mini) and one on a third node the hub lists as an
// Upstash Box. Each worker nests under the Captain in the sidebar, at 1440x900 and on a 390x844
// phone. Then it connects Upstash against a stub of the Box API, keeps a Claude token per account
// on the Upstash card, and shoots the card.
//
//   UPSTASH_BOX_BASE_URL=http://127.0.0.1:$SWEEP_BOX_PORT SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/remote-targets.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// The third node is a real node of the account; the sweep writes the box row that makes the hub
// list it as a box, since no Upstash box boots here. The script starts and stops sweep-mini, the
// box node and the Box API stub itself. Writes remote-targets*.png into scripts/web-sweep/screens/.
const { execFileSync, spawn } = require("child_process");
const fs = require("fs");
const http = require("http");
const os = require("os");
const path = require("path");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const PG_PORT = process.env.SWEEP_PG_PORT || 5497;
const BOX_PORT = Number(process.env.SWEEP_BOX_PORT || 8799);
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const WORK = path.join(DIR, "home", "work");
const SCREENS = path.join(__dirname, "screens");
const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname, "../../target"), "debug", "pinned-server");
const ALPHA = "https://github.com/example/alpha.git";
const BOX_NAME = "Box · Slow suite";

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
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "remote-targets-"));
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
  if (el.matches(".global-row")) return `orchestrator ${el.querySelector(".count")?.textContent ?? 0}`;
  if (el.matches(".waiting-head")) return `waiting ${el.querySelector(".count")?.textContent ?? 0}`;
  if (el.matches(".waiting-row")) return `asks ${el.dataset.waiting}`;
  if (el.matches(".side-heading")) return `heading ${el.textContent}`;
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

const BOX_ROW = "5eeb0c5000000001";
const BOX_ID = "box_sweep1";

/** The Box API as far as connecting and the hub's sweep need it: listing boxes with a box_ key,
 * which lists the one box the sweep's row names, so the hub keeps it. */
function boxApi() {
  const server = http.createServer((req, res) => {
    const ok = (req.headers["x-box-api-key"] || "").startsWith("box_");
    const label = new URL(req.url, "http://stub").searchParams.get("label");
    res.writeHead(ok ? 200 : 401, { "content-type": "application/json" });
    const listed = [{ id: BOX_ID, labels: [label, `p:${BOX_ROW}`] }];
    res.end(ok && req.method === "GET" && req.url.startsWith("/v2/box") ? JSON.stringify(listed) : JSON.stringify({ error: "Invalid box API key" }));
  });
  return new Promise((resolve) => server.listen(BOX_PORT, "127.0.0.1", () => resolve(server)));
}

/** Another node of the sweep's account, named `name`, with a checkout of alpha in its clones folder. */
function startNode(name, slot) {
  const home = path.join(DIR, `home${slot}`);
  const config = path.join(DIR, `node${slot}`);
  const alpha = path.join(home, "pinned", "alpha");
  fs.mkdirSync(alpha, { recursive: true });
  execFileSync("git", ["-C", alpha, "init", "-q"]);
  execFileSync("git", ["-C", alpha, "remote", "add", "origin", ALPHA]);
  const env = { ...process.env, HOME: home, SHELL: path.join(DIR, "bin", "shell"), PATH: `${path.join(DIR, "bin")}:${process.env.PATH}` };
  for (const key of ["PINNED_SOCK", "PINNED_TASK", "PINNED_ACCOUNT", "PINNED_TAB", "CLAUDE_CONFIG_DIR"]) delete env[key];
  execFileSync(SERVER, ["login", HUB, EMAIL, "--name", name, "--config-dir", config], { env: { ...env, PINNED_PASSWORD: PASSWORD }, stdio: "ignore" });
  const log = fs.openSync(path.join(DIR, `node${slot}.log`), "a");
  const proc = spawn(SERVER, ["node", "--config-dir", config], { env, stdio: ["ignore", log, log] });
  fs.writeFileSync(path.join(DIR, `node${slot}.pid`), String(proc.pid));
  return proc;
}

const psql = (sql) => execFileSync("psql", ["-h", "127.0.0.1", "-p", String(PG_PORT), "-U", "pinned", "-d", "pinned", "-tAc", sql]).toString().trim();

const sendText = (page, text) => page.evaluate((t) => {
  prompt.value = t;
  send();
}, text);

const machineIds = (page) => page.evaluate(() => Object.fromEntries([...machines].filter(([, m]) => m.view.online).map(([k, m]) => [m.view.name, k])));

(async () => {
  const errors = [];
  const stub = await boxApi();
  const nodes = [startNode("sweep-mini", 2), startNode(BOX_NAME, 3)];
  const browser = await launchBrowser(errors);
  console.log(`chromium ${chromium}`);
  try {
    const page = await browser.tab(false);
    await page.size(1440, 900);
    await login(page);
    await waitFor("three online machines", async () => Object.keys(await machineIds(page)).length === 3, 60000);
    const ids = await machineIds(page);
    const node = ids["sweep-node"];
    const account = psql(`select id from accounts where email = '${EMAIL}'`);
    // The hub lists a machine with a box row as a box. Its node is real; no Upstash box boots here.
    psql(`insert into boxes (id, account, machine, upstash, connector, subject, spec, size, phase) values ('${BOX_ROW}', '${account}', '${ids[BOX_NAME]}', '${BOX_ID}', 'delegate', '{"issue": "task sweep"}', '{"folder": "", "repo": "${ALPHA}", "args": ["--name", "Slow suite"], "env": []}', 'small', 'running')`);

    const alpha = fs.realpathSync(path.join(WORK, "alpha"));
    const captain = await page.evaluate(async ({ machine, alpha }) => {
      const tab = await revealTab(machine, await callOn(machine, { cmd: "open_orchestrator", path: alpha }));
      activate(tab);
      startTab(tab);
      return { key: tab.key, id: tab.view.id };
    }, { machine: node, alpha });
    await waitFor("the Captain's session", () => page.evaluate((k) => tabByKey(k)?.view.running === true, captain.key));

    const calls = [
      { task: "Write the changelog for v0.17.22.", title: "Changelog for v0.17.22" },
      { task: "Run the slow suite on the mini. [hold]", title: "Slow suite on the mini", target: { machine: "sweep-mini" } },
      { task: "Profile the release build. [hold]", title: "Profile the release build", target: { machine: BOX_NAME } },
    ];
    const children = () => page.evaluate((id) => tabs.filter((t) => t.view.spawned_by === id && (t.view.spawned_on || t.machine === tabs.find((c) => c.view.id === id)?.machine))
      .map((t) => ({ key: t.key, machine: machines.get(t.machine)?.view.name, title: t.view.title, role: t.view.role, spawned_on: t.view.spawned_on ?? null })), captain.id);
    fs.writeFileSync(path.join(DIR, "home", "delegate-calls.jsonl"), calls.map((c) => JSON.stringify(c)).join("\n"));
    await sendText(page, "Ship v0.17.22: changelog here, the slow suite on the mini, a profile in a box.");
    await waitFor("three children", async () => (await children()).length === 3, 60000);
    const kids = await children();
    console.log("children:", JSON.stringify(kids));
    const by = (title) => kids.find((k) => k.title === title);
    check(kids.every((k) => k.role === "worker"), `three worker threads: ${JSON.stringify(kids)}`);
    check(by("Changelog for v0.17.22")?.machine === "sweep-node" && !by("Changelog for v0.17.22").spawned_on, "the local child");
    check(by("Slow suite on the mini")?.machine === "sweep-mini" && by("Slow suite on the mini").spawned_on === node, "the child on sweep-mini, spawned on sweep-node");
    check(by("Profile the release build")?.machine === BOX_NAME && by("Profile the release build").spawned_on === node, "the child in the box, spawned on sweep-node");
    // Renaming the machine to its own name makes the hub push the machine list, which reads the box row.
    await page.evaluate(({ id, name }) => hub.hub({ cmd: "rename_machine", id, name }), { id: ids[BOX_NAME], name: BOX_NAME });
    await waitFor("the box machine listed as a box", () => page.evaluate((name) => [...machines.values()].some((m) => m.view.name === name && m.view.upstash_box), BOX_NAME), 30000);
    await waitFor("the changelog worker to finish", () => page.evaluate((k) => statusOf(tabByKey(k)) === "done", by("Changelog for v0.17.22").key), 30000);
    await page.evaluate((k) => activate(tabByKey(k)), captain.key);
    await waitFor("the delegate_task calls in the chat", () => page.evaluate(() => /Started 3 workers/.test(document.getElementById("thread").innerText)), 30000);

    const nested = (tokens) => {
      const at = tokens.indexOf(captain.key);
      return at >= 0 && JSON.stringify(tokens.slice(at + 1, at + 4).sort()) === JSON.stringify(kids.map((k) => `  ${k.key}`).sort());
    };
    const tokens = await page.evaluate(sidebarTokens);
    console.log("sidebar:", JSON.stringify(tokens, null, 1));
    check(nested(tokens), "the Captain with its three children nested under it, from three machines");
    await waitFor("both remote workers at work", () => page.evaluate((keys) => keys.every((k) => statusOf(tabByKey(k)) === "working"), [by("Slow suite on the mini").key, by("Profile the release build").key]), 30000);
    const busy = await page.evaluate((k) => busyUnder(tabByKey(k)), captain.key);
    check(busy === 2, `the Captain counts its two working children once each, not ${busy}`);
    await page.screenshot(path.join(SCREENS, "remote-targets.png"));

    const small = await browser.tab(true);
    await small.goto(HUB);
    await waitFor("the hub on the phone", () => small.evaluate(() => document.body.dataset.hub === "online"));
    await small.evaluate(() => setMode("gui"));
    await waitFor("the children on the phone", () => small.evaluate((id) => tabs.filter((t) => t.view.spawned_by === id).length >= 3, captain.id), 30000);
    await small.evaluate((k) => activate(tabByKey(k)), captain.key);
    await small.evaluate(() => toggleSidebar(true));
    await waitFor("the phone's sidebar", () => small.evaluate((k) => !!document.querySelector(`.thread-row[data-tab="${k}"]`), captain.key));
    await small.evaluate((k) => document.querySelector(`.thread-row[data-tab="${k}"]`).scrollIntoView({ block: "center" }), captain.key);
    check(nested(await small.evaluate(sidebarTokens)), "the phone nests them too");
    await small.screenshot(path.join(SCREENS, "remote-targets-phone.png"));

    await page.evaluate(async () => {
      await connectorCall("connect_upstash", { key: "box_sweepkey", claude_token: "sk-ant-oat01-sweepdefault" });
      await connectorCall("set_box_claude_token", { name: "claude", token: "sk-ant-oat01-sweepclaude" });
      await connectorCall("set_box_claude_token", { name: "bloom", token: "sk-ant-oat01-sweepbloom" });
    });
    for (const [shot, view, file] of [[page, false, "remote-targets-upstash.png"], [small, true, "remote-targets-upstash-phone.png"]]) {
      await shot.evaluate(() => {
        openSettings();
        showPage("integrations");
      });
      await waitFor("the Upstash card with both tokens", () => shot.evaluate(() => document.querySelectorAll(".box-token").length === 2), 20000);
      await shot.evaluate(() => document.querySelector(".box-tokens").closest(".card").scrollIntoView({ block: view ? "start" : "center" }));
      await shot.screenshot(path.join(SCREENS, file));
    }
    const card = await page.evaluate(() => document.querySelector(".box-tokens").closest(".card").innerText);
    console.log("upstash card:", JSON.stringify(card));
    check(/bloom · token …loom/.test(card.replace(/\s+/g, " ")) && !/sweepbloom/.test(card), `the card shows hints only: ${card}`);

    check(!errors.length, `page errors:\n${errors.slice(0, 5).join("\n")}`);
    console.log("remote targets: every check passed");
  } finally {
    const exited = new Promise((resolve) => browser.proc.once("exit", resolve));
    browser.proc.kill("SIGTERM");
    await exited;
    fs.rmSync(browser.profile, { recursive: true, force: true, maxRetries: 5 });
    for (const proc of nodes) proc.kill("SIGTERM");
    stub.close();
  }
})().catch((error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
