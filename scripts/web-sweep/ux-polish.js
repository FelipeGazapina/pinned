#!/usr/bin/env node
// UX polish sweep (#163 part 3): before and after shots of each item on a real hub and two nodes, at
// 1440x900 and 390x844, plus checks on what the page does.
//
//   SWEEP_DIR=... SWEEP_PG_PORT=... SWEEP_HUB_PORT=... CARGO=true CARGO_TARGET_DIR=... scripts/web-sweep/stack.sh start
//   node scripts/web-sweep/ux-polish.js seed
//   node scripts/web-sweep/ux-polish.js shoot before <ui dir of the base>
//   node scripts/web-sweep/ux-polish.js shoot after  <ui dir of this branch>
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// `seed` pairs a second node, sweep-node-2, opens alpha, beta and gamma on both nodes, starts a Captain
// on alpha with two workers (one held working), and leaves a question, a plan and a failed thread. `shoot`
// restarts the hub on the given ui folder, so both labels run against the same threads, and writes
// ux-polish-<item>-<label>-<desktop|phone>.png into UX_SHOTS (default scripts/web-sweep/screens/ux-polish).
// SWEEP_CHANNEL=chrome drives the installed Chrome; PLAYWRIGHT is the path of a playwright package.
const fs = require("fs");
const path = require("path");
const { execFileSync, spawn } = require("child_process");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const REPO = path.join(__dirname, "..", "..");
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const PG_PORT = process.env.SWEEP_PG_PORT || 5497;
const HUB_PORT = process.env.SWEEP_HUB_PORT || 8797;
const HUB = `http://127.0.0.1:${HUB_PORT}`;
const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(REPO, "target"), "debug", "pinned-server");
const SHOTS = process.env.UX_SHOTS || path.join(__dirname, "screens", "ux-polish");
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const SIZES = [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]];
const NODE2 = { name: "sweep-node-2", home: path.join(DIR, "home2"), config: path.join(DIR, "node2"), pid: "node-b.pid", log: "node2.log" };

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const waitFor = async (what, fn, timeout = 60000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await sleep(150);
  }
  throw new Error(`timed out waiting for ${what}`);
};
const pidOf = (file) => Number(fs.readFileSync(path.join(DIR, file), "utf8").trim());
const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};
const stopPid = async (file) => {
  if (!fs.existsSync(path.join(DIR, file))) return;
  const pid = pidOf(file);
  if (alive(pid)) process.kill(pid, "SIGTERM");
  await waitFor(`pid ${pid} to exit`, () => !alive(pid), 15000);
  fs.rmSync(path.join(DIR, file), { force: true });
};
const hubUp = () => {
  try {
    execFileSync("curl", ["-fs", HUB], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
};
const cleanEnv = (extra) => {
  const env = { ...process.env, ...extra };
  for (const key of ["PINNED_SOCK", "PINNED_TASK", "PINNED_ACCOUNT", "PINNED_TAB", "CLAUDE_CONFIG_DIR"]) delete env[key];
  return env;
};

async function startHub(ui) {
  await stopPid("hub.pid");
  await waitFor("the hub to go", () => !hubUp(), 15000);
  const log = fs.openSync(path.join(DIR, "hub.log"), "a");
  const hub = spawn(SERVER, ["hub", "--db", `postgres://pinned@127.0.0.1:${PG_PORT}/pinned`, "--listen", `127.0.0.1:${HUB_PORT}`, "--ui", ui,
    "--downloads", path.join(DIR, "downloads"), "--key-file", path.join(DIR, "hub.key"), "--public-url", HUB, "--signup", "open"], { env: cleanEnv({}), stdio: ["ignore", log, log], detached: true });
  hub.unref();
  fs.writeFileSync(path.join(DIR, "hub.pid"), `${hub.pid}\n`);
  await waitFor("the hub", hubUp);
}

function nodeEnv(home) {
  const bin = path.join(DIR, "bin");
  return cleanEnv({ HOME: home, SHELL: path.join(bin, "shell"), PATH: `${bin}:${process.env.PATH}`, PINNED_PR_WATCH_SECS: "2" });
}
function pairNode2() {
  if (fs.existsSync(NODE2.config)) return;
  const work = path.join(NODE2.home, "work");
  for (const p of ["alpha", "beta", "gamma"]) {
    fs.mkdirSync(path.join(work, p), { recursive: true });
    execFileSync("git", ["-C", path.join(work, p), "init", "-q"]);
    execFileSync("git", ["-C", path.join(work, p), "remote", "add", "origin", `https://github.com/example/${p}.git`]);
  }
  fs.writeFileSync(path.join(work, "gamma", ".untrusted"), "");
  execFileSync(SERVER, ["login", HUB, EMAIL, "--name", NODE2.name, "--config-dir", NODE2.config], { env: cleanEnv({ HOME: NODE2.home, PINNED_PASSWORD: PASSWORD }), stdio: "ignore" });
}
function startNode2() {
  if (fs.existsSync(path.join(DIR, NODE2.pid)) && alive(pidOf(NODE2.pid))) return;
  const log = fs.openSync(path.join(DIR, NODE2.log), "a");
  const node = spawn(SERVER, ["node", "--config-dir", NODE2.config], { env: nodeEnv(NODE2.home), stdio: ["ignore", log, log], detached: true });
  node.unref();
  fs.writeFileSync(path.join(DIR, NODE2.pid), `${node.pid}\n`);
}

const machineOnline = (name) => [...machines.values()].find((m) => m.view.name === name)?.view.online === true;
async function signIn(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await page.click("#login .primary");
  }
  await page.evaluate(`globalThis.machineOnline = ${machineOnline}`);
}
const bothOnline = (page) => waitFor("both nodes", () => page.evaluate(() => document.body.dataset.hub === "online" && machineOnline("sweep-node") && machineOnline("sweep-node-2")));

async function seed() {
  if (!hubUp()) await startHub(path.join(REPO, "ui"));
  pairNode2();
  startNode2();
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  const context = await browser.newContext({ viewport: SIZES[0][1], colorScheme: "dark" });
  const page = await context.newPage();
  await signIn(page);
  await bothOnline(page);
  await page.evaluate(() => setMode("gui"));
  const ids = await page.evaluate(() => ({ n1: [...machines].find(([, m]) => m.view.name === "sweep-node")[0], n2: [...machines].find(([, m]) => m.view.name === "sweep-node-2")[0] }));
  const w1 = path.join(DIR, "home", "work");
  const w2 = path.join(NODE2.home, "work");
  const real = (p) => fs.realpathSync(p);
  const send = (text) => page.evaluate((t) => { prompt.value = t; send(); }, text);
  fs.writeFileSync(path.join(DIR, "home", "delegate-calls.jsonl"), [
    { task: "Run the CI on main and report what fails. [hold]", title: "Run the CI on main", target: { providerInstanceId: "claude", model: "claude-sonnet-5-5", options: { effort: "high" } } },
    { task: "Write the changelog for v0.17.24.", title: "Changelog for v0.17.24", target: { providerInstanceId: "claude", model: "claude-haiku-5-5" } },
  ].map((x) => JSON.stringify(x)).join("\n"));
  const saved = await page.evaluate((m) => callOn(m, { cmd: "set_global_orchestrator_settings", settings: { host: true } }), ids.n1);
  await page.evaluate((d) => document.dispatchEvent(new CustomEvent("global-orchestrator-settings", { detail: d })), saved);
  await page.evaluate(async ({ list }) => {
    for (const [id, dirs] of list) for (const d of dirs) if (!tabs.some((t) => t.machine === id && t.view.cwd === d)) await openRemote(id, d, false);
  }, { list: [[ids.n1, ["alpha", "beta", "gamma"].map((n) => real(path.join(w1, n)))], [ids.n2, ["alpha", "beta", "gamma"].map((n) => real(path.join(w2, n)))]] });
  const start = async (machine, dir) => {
    const key = await page.evaluate(async ({ m, d }) => { const t = tabs.find((t) => t.machine === m && t.view.cwd === d); activate(t); startTab(t); return t.key; }, { m: machine, d: dir });
    await waitFor("a run", () => page.evaluate((k) => tabByKey(k)?.view.running === true, key));
    await sleep(1000);
  };
  const cap = await page.evaluate(async ({ m, d }) => {
    const tab = await revealTab(m, await callOn(m, { cmd: "open_orchestrator", path: d })); activate(tab); startTab(tab); return { key: tab.key, id: tab.view.id };
  }, { m: ids.n1, d: real(path.join(w1, "alpha")) });
  await waitFor("the Captain", () => page.evaluate((k) => tabByKey(k)?.view.running === true, cap.key));
  await sleep(1500);
  await send("Release v0.17.24 once the CI passes, with its changelog.");
  await waitFor("two workers", () => page.evaluate((id) => tabs.filter((t) => t.view.spawned_by === id).length >= 2, cap.id), 40000);
  await start(ids.n1, real(path.join(w1, "beta")));
  await send("asks");
  await start(ids.n2, real(path.join(w2, "gamma")));
  await send("plan");
  await start(ids.n2, real(path.join(w2, "alpha")));
  await send("fail");
  await sleep(3000);
  console.log(JSON.stringify(await page.evaluate(() => tabs.map((t) => `${machines.get(t.machine)?.view.name} ${t.view.cwd.split("/").pop()} ${t.view.title} ${statusOf(t)} ${t.view.role ?? ""}`)), null, 1));
  await browser.close();
}

// Runs in the page: what the sidebar toggle and the header show.
const headState = () => {
  const toggle = document.getElementById("chat-sidebar-toggle");
  const box = (el) => (el ? { w: Math.round(el.getBoundingClientRect().width), h: Math.round(el.getBoundingClientRect().height) } : null);
  return {
    sidebar: document.body.dataset.sidebar,
    toggle: toggle.getAttribute("aria-label") ?? toggle.title,
    badge: toggle.querySelector(".count")?.textContent ?? null,
    dot: !!toggle.querySelector(".running"),
    stats: [...document.querySelectorAll("#chat-head .stats")].filter((e) => e.getBoundingClientRect().width > 0).map((e) => e.textContent),
    crumb: box(document.querySelector("#chat-head .crumb")),
  };
};

async function shoot(label, ui) {
  fs.mkdirSync(SHOTS, { recursive: true });
  await startHub(ui);
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  const out = {};
  for (const [size, viewport] of SIZES) {
    const phone = size === "phone";
    const note = (key, value) => { out[`${key}-${size}`] = value; };
    const newPage = async () => {
      const context = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
      const page = await context.newPage();
      page.on("pageerror", (e) => note("pageerror", String(e)));
      return page;
    };
    const shot = async (page, item) => {
      await page.waitForTimeout(500);
      await page.screenshot({ path: path.join(SHOTS, `ux-polish-${item}-${label}-${size}.png`) });
    };
    const ready = async (page) => {
      await signIn(page);
      await bothOnline(page);
      await waitFor("threads", () => page.evaluate(() => tabs.length >= 6));
      await page.evaluate(() => setMode("gui"));
      await page.waitForTimeout(1500);
    };

    // UX-4: the first screen on a phone, then a thread with the drawer closed.
    let page = await newPage();
    await ready(page);
    note("ux4-first", await page.evaluate(headState));
    await shot(page, "ux4-first-load");
    await page.evaluate(() => { toggleSidebar(false); });
    await page.waitForTimeout(400);
    note("ux4-closed", await page.evaluate(headState));
    await shot(page, "ux4-drawer-closed");
    await page.evaluate(() => { const t = tabs.find((t) => t.view.role === "orchestrator") ?? tabs[0]; activate(t); });
    await page.waitForTimeout(800);
    note("ux4-thread", await page.evaluate(headState));
    await shot(page, "ux4-thread-header");

    // UX-6 and the sidebar: the list with the drawer open.
    await page.evaluate(() => { ui.waitingOpen = true; toggleSidebar(true); renderSidebar(); });
    await page.waitForTimeout(500);
    note("ux6", await page.evaluate(() => ({
      heads: [...document.querySelectorAll(".project-head .where")].map((e) => e.textContent),
      remotes: [...document.querySelectorAll(".thread-row .remote")].map((e) => e.textContent.trim() || e.title),
    })));
    await shot(page, "ux6-sidebar");

    // UX-9, UX-10: the Machines page, with a failed update on one row.
    await page.evaluate(() => { openSettings("machines"); });
    await page.waitForTimeout(800);
    note("ux9", await page.evaluate(() => [...document.querySelectorAll("#settings-page .machine-row")].map((r) => r.innerText.replace(/\s+/g, " ").trim())));
    await shot(page, "ux9-machines");
    await page.evaluate(() => {
      const view = [...machines.values()].find((m) => m.view.name === "sweep-node-2").view;
      view.update = { state: "failed", to: "0.17.24", reason: "checksum mismatch" };
      showMachinesPage();
    });
    await page.waitForTimeout(300);
    await shot(page, "ux10-update-failed");

    // UX-12: Settings › General on a phone, down to the last toggle.
    await page.evaluate(() => { openSettings("general"); });
    await page.waitForTimeout(800);
    await page.evaluate(() => { const s = document.getElementById("settings-page"); s.scrollTop = s.scrollHeight; });
    await page.waitForTimeout(300);
    note("ux12", await page.evaluate(() => {
      const v = document.getElementById("version").getBoundingClientRect();
      const last = [...document.querySelectorAll("#settings-page .side-setting")].at(-1)?.getBoundingClientRect();
      const name = document.querySelector("#global-orchestrator-settings [data-setting=name]")?.getBoundingClientRect();
      return { version: { top: Math.round(v.top), bottom: Math.round(v.bottom), left: Math.round(v.left) }, lastSetting: last && { top: Math.round(last.top), bottom: Math.round(last.bottom) }, nameRow: name && Math.round(name.height), versionShown: getComputedStyle(document.getElementById("version")).display };
    }));
    await shot(page, "ux12-general-bottom");
    await page.evaluate(() => { document.querySelector("#global-orchestrator-settings [data-setting=name]")?.scrollIntoView({ block: "center" }); });
    await page.waitForTimeout(300);
    await shot(page, "ux12-name-row");

    // The sidebar follow-ups: a machine away, then the hub link down.
    await page.evaluate(() => settingsOpen() && toggleSettings());
    await page.evaluate(() => { toggleSidebar(true); renderSidebar(); });
    await stopPid(NODE2.pid);
    await waitFor("node 2 offline", () => page.evaluate(() => !machineOnline("sweep-node-2")));
    await page.evaluate(() => { renderSidebar(); document.getElementById("thread-list").scrollTop = 0; });
    await page.waitForTimeout(500);
    note("machine-note", await page.evaluate(() => {
      const note = document.querySelector(".machine-note");
      const list = document.getElementById("thread-list").getBoundingClientRect();
      const r = note?.getBoundingClientRect();
      return note && { top: Math.round(r.top), listTop: Math.round(list.top), height: Math.round(r.height), scrollTop: document.getElementById("thread-list").scrollTop, clipped: r.top < list.top };
    }));
    await shot(page, "sidebar-machine-note");
    await page.evaluate(() => openSettings("machines"));
    await page.waitForTimeout(800);
    note("ux9-offline", await page.evaluate(() => [...document.querySelectorAll("#settings-page .machine-row")].map((r) => ({ text: r.innerText.replace(/\s+/g, " ").trim(), updateDisabled: r.querySelector("[data-action=update-machine]")?.disabled ?? null }))));
    await shot(page, "ux9-machines-offline");
    startNode2();
    await page.evaluate(() => { settingsOpen() && toggleSettings(); toggleSidebar(true); });
    await waitFor("node 2 back", () => page.evaluate(() => machineOnline("sweep-node-2")), 90000);
    await waitFor("its threads", () => page.evaluate(() => tabs.length >= 6), 60000);
    await page.context().setOffline(true);
    await page.evaluate(() => hub.ws.close());
    await waitFor("the hub link to drop", () => page.evaluate(() => document.body.dataset.hub === "offline"), 30000);
    await page.waitForTimeout(800);
    note("reconnecting", await page.evaluate(() => [...document.querySelectorAll(".thread-row .stamp.status.info")].map((e) => ({ text: e.textContent.trim(), opacity: getComputedStyle(e).opacity }))));
    await shot(page, "sidebar-reconnecting");
    await page.context().close();

    // UX-13: the sign-in form with the hub out of reach.
    page = await newPage();
    await page.route("**/api/login", (route) => route.abort());
    await page.goto(HUB);
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await page.click("#login .primary");
    await page.waitForTimeout(800);
    note("ux13", await page.evaluate(() => document.querySelector("#login .error").textContent));
    await shot(page, "ux13-signin");
    await page.context().close();
  }
  await browser.close();
  fs.writeFileSync(path.join(SHOTS, `ux-polish-${label}.json`), JSON.stringify(out, null, 2));
  console.log(JSON.stringify(out, null, 2));
}

const [cmd, label, ui] = process.argv.slice(2);
(cmd === "seed" ? seed() : cmd === "shoot" ? shoot(label, path.resolve(ui)) : Promise.reject(new Error("usage: ux-polish.js seed | shoot <label> <ui dir>")))
  .then(() => process.exit(0))
  .catch((e) => { console.error(e); process.exit(1); });
