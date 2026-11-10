#!/usr/bin/env node
// Shoots the connection states at 1440x900 and 390x844 on a real hub and two nodes, and checks each:
//   offline       one of two machines stops: its note shows, the other's threads stay
//   all-offline   both stop: both notes, the Orchestrator row kept and disabled, no "No threads yet"
//   reconnecting  the hub stops: the "Reconnecting…" bar, live states dimmed, the timer held; the hub
//                 starts again: the bar clears on its own
//   signin        the hub stops under the sign-in form: a plain sentence and a Retry; the hub starts
//                 again and Retry signs in
//
//   SWEEP_DIR=... CARGO=true CARGO_TARGET_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... CARGO_TARGET_DIR=... node scripts/web-sweep/connection-states.js
//
// Writes connection-states-<state>-real-<desktop|phone>.png into CS_SHOTS (default scripts/web-sweep/screens).
// The script stops and starts the hub and both nodes itself; stack.sh stop kills what is left.
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
const SHOTS = process.env.CS_SHOTS || path.join(__dirname, "screens");
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const SIZES = [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]];
const NODES = [
  { name: "sweep-node", home: path.join(DIR, "home"), config: path.join(DIR, "node"), pid: "node.pid", log: "node.log", project: "alpha" },
  { name: "sweep-node-2", home: path.join(DIR, "home2"), config: path.join(DIR, "node2"), pid: "node-b.pid", log: "node2.log", project: "delta" },
];

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};
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
  const pid = pidOf(file);
  process.kill(pid, "SIGTERM");
  await waitFor(`pid ${pid} to exit`, () => !alive(pid), 15000);
  fs.rmSync(path.join(DIR, file), { force: true });
};

function startHub() {
  const log = fs.openSync(path.join(DIR, "hub.log"), "a");
  const hub = spawn(SERVER, ["hub", "--db", `postgres://pinned@127.0.0.1:${PG_PORT}/pinned`, "--listen", `127.0.0.1:${HUB_PORT}`, "--ui", path.join(REPO, "ui"),
    "--downloads", path.join(DIR, "downloads"), "--key-file", path.join(DIR, "hub.key"), "--public-url", HUB, "--signup", "open"], { stdio: ["ignore", log, log], detached: true });
  hub.unref();
  fs.writeFileSync(path.join(DIR, "hub.pid"), `${hub.pid}\n`);
}

function nodeEnv(n) {
  const bin = path.join(DIR, "bin");
  return { ...process.env, HOME: n.home, SHELL: path.join(bin, "shell"), PATH: `${bin}:${process.env.PATH}`, PINNED_PR_WATCH_SECS: "2" };
}
function pairNode(n) {
  if (fs.existsSync(n.config)) return;
  fs.mkdirSync(path.join(n.home, "work"), { recursive: true });
  const dir = path.join(n.home, "work", n.project);
  fs.mkdirSync(dir, { recursive: true });
  execFileSync("git", ["-C", dir, "init", "-q"]);
  execFileSync("git", ["-C", dir, "remote", "add", "origin", `https://github.com/example/${n.project}.git`]);
  execFileSync(SERVER, ["login", HUB, EMAIL, "--name", n.name, "--config-dir", n.config], { env: { ...process.env, HOME: n.home, PINNED_PASSWORD: PASSWORD }, stdio: "ignore" });
}
function startNode(n) {
  // A node still running from an earlier try is the same machine: a second one would take its socket.
  if (fs.existsSync(path.join(DIR, n.pid)) && alive(pidOf(n.pid))) return;
  const log = fs.openSync(path.join(DIR, n.log), "a");
  const node = spawn(SERVER, ["node", "--config-dir", n.config], { env: nodeEnv(n), stdio: ["ignore", log, log], detached: true });
  node.unref();
  fs.writeFileSync(path.join(DIR, n.pid), `${node.pid}\n`);
}

// Runs in the page: the sidebar as text, in order, and whether the bar shows.
const sidebarText = () => [...document.getElementById("thread-list").children].map((el) => el.textContent.replace(/\s+/g, " ").trim()).filter(Boolean);
const barShows = () => getComputedStyle(document.getElementById("reconnect-bar")).display !== "none";
const machineOnline = (name) => [...machines.values()].find((m) => m.view.name === name)?.view.online === true;

// Runs in the page. Tabs on both machines, one of them working with a started time, and a seed applied
// again before every render, since the node's tabs events replace each view.
function seed(spec) {
  const apply = () => {
    for (const tab of tabs) {
      const row = spec.rows.find((r) => r.machine === machines.get(tab.machine)?.view.name && r.index === tabs.filter((t) => t.machine === tab.machine && !isGlobalOrchestratorTab(t)).indexOf(tab));
      if (!row) continue;
      Object.assign(tab.view, { bypass: true, status: row.status ?? "idle", running: row.status === "working", title: row.title, updated: Math.floor(Date.now() / 1000) - row.age });
      if (row.status === "working" && !threadOf(tab).items.some((i) => i.kind === "user")) threadOf(tab).items.push({ kind: "user", text: row.title, at: new Date(Date.now() - 7 * 60000).toISOString() });
    }
  };
  if (!window.__csRender) {
    window.__csRender = renderSidebar;
    renderSidebar = () => {
      apply();
      window.__csRender();
    };
  }
  renderSidebar();
}
const ROWS = [
  { machine: "sweep-node", index: 0, title: "Fix the flaky checkout test", status: "working", age: 120 },
  { machine: "sweep-node", index: 1, title: "Tidy the README", status: "done", age: 86400 * 3 },
  { machine: "sweep-node-2", index: 0, title: "Changelog for v0.17.24", status: "done", age: 7200 },
];

async function signIn(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await page.click("#login .primary");
  }
}

(async () => {
  for (const n of NODES) pairNode(n);
  for (const n of NODES.slice(1)) startNode(n);
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  const errors = [];
  for (const [size, viewport] of SIZES) {
    const phone = size === "phone";
    const context = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
    const page = await context.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    const shot = async (state) => {
      await page.waitForTimeout(500);
      await page.screenshot({ path: path.join(SHOTS, `connection-states-${state}-real-${size}.png`) });
      console.log(`shot ${state}-${size}`);
    };
    // A node's tabs events replace the views the seed set, so the sidebar is drawn again on a short wait.
    const settle = async () => {
      await page.evaluate(() => renderSidebar());
      await page.waitForTimeout(300);
    };

    await signIn(page);
    await page.evaluate(`globalThis.machineOnline = ${machineOnline}`);
    await waitFor("the hub and both nodes", () => page.evaluate(() => document.body.dataset.hub === "online" && machineOnline("sweep-node") && machineOnline("sweep-node-2")));
    await page.evaluate(() => setMode("gui"));
    for (const n of NODES) {
      const dir = fs.realpathSync(path.join(n.home, "work", n.project));
      await page.evaluate(async ([machineName, dir, want]) => {
        const id = [...machines.entries()].find(([, m]) => m.view.name === machineName)[0];
        const have = () => tabs.filter((t) => t.machine === id && t.view.cwd === dir).length;
        while (have() < want) {
          const before = have();
          await callOn(id, { cmd: "open_tab", path: dir, bypass: false });
          const start = Date.now();
          while (have() === before && Date.now() - start < 5000) await new Promise((r) => setTimeout(r, 100));
          if (have() === before) break;
        }
      }, [n.name, dir, n === NODES[0] ? 2 : 1]);
    }
    await page.evaluate(`(${seed})(${JSON.stringify({ rows: ROWS })})`);
    if (phone) await page.evaluate(() => toggleSidebar(true));
    await settle();

    await page.evaluate(() => { window.__ns = []; const o = noteThreadsSeen; noteThreadsSeen = (m, c) => { if (m.startsWith("e031")) window.__ns.push([c, machines.get(m)?.view.online, tabs.length, Date.now() % 100000]); o(m, c); }; });
    // 1. One machine away.
    await stopPid(NODES[1].pid);
    await waitFor("sweep-node-2 to show offline", () => page.evaluate(() => !machineOnline("sweep-node-2")));
    await settle();
    let text = await page.evaluate(sidebarText);
    console.log(`offline:\n  ${text.join("\n  ")}`);
    console.log(JSON.stringify(await page.evaluate(() => window.__ns.slice(-14))));
    console.log(JSON.stringify(await page.evaluate(() => ({ seen: threadsSeen, machines: [...machines.entries()].map(([id, m]) => [id.slice(0, 4), m.view.name, m.view.online, m.view.last_seen, tabs.filter((t) => t.machine === id).length]) }))));
    check(text.some((t) => /^sweep-node-2 offline · seen/.test(t)), `${size}: the offline machine is named with when it was seen`);
    check(!text.some((t) => /^sweep-node offline/.test(t)), `${size}: the online machine is not named`);
    check(await page.evaluate(() => !!document.querySelector(".thread-row")), `${size}: the other machine's threads stay`);
    check(await page.evaluate(() => !document.querySelector(".global-row.disabled")), `${size}: the Orchestrator row is still usable`);
    await shot("offline");

    // 2. Both away.
    await stopPid(NODES[0].pid);
    await waitFor("sweep-node to show offline", () => page.evaluate(() => !machineOnline("sweep-node")));
    await settle();
    text = await page.evaluate(sidebarText);
    console.log(`all-offline:\n  ${text.join("\n  ")}`);
    check(text.filter((t) => / offline · seen/.test(t)).length === 2, `${size}: both machines are named`);
    check(!text.some((t) => /No threads yet/.test(t)), `${size}: it does not say "No threads yet"`);
    check(text.some((t) => /No threads to show while machines are offline/.test(t)), `${size}: it says why nothing shows`);
    check(await page.evaluate(() => document.querySelector(".global-row.disabled")?.getAttribute("aria-disabled") === "true"), `${size}: the Orchestrator row stays, disabled`);
    check(await page.evaluate(() => document.querySelector(".global-row.disabled")?.title ?? "").then((t) => /is offline, so the orchestrator can't open/.test(t)), `${size}: its tooltip says why`);
    await shot("all-offline");
    for (const n of NODES) startNode(n);
    await waitFor("both nodes back", () => page.evaluate(() => machineOnline("sweep-node") && machineOnline("sweep-node-2")));
    await page.waitForTimeout(1500);
    await settle();
    check(await page.evaluate(() => !document.querySelector(".machine-note") && !document.querySelector(".global-row.disabled")), `${size}: the notes and the disabled row clear when the machines are back`);

    // 3. The hub away.
    check(!(await page.evaluate(barShows)), `${size}: no bar while the link is up`);
    await stopPid("hub.pid");
    await waitFor("the app to see the hub gone", () => page.evaluate(() => document.body.dataset.hub === "offline"));
    await settle();
    check(await page.evaluate(barShows), `${size}: the Reconnecting bar shows`);
    check(await page.evaluate(() => document.getElementById("reconnect-bar").textContent.includes("Reconnecting")), `${size}: the bar says Reconnecting`);
    const dimmed = await page.evaluate(() => [...document.querySelectorAll(".thread-row .stamp.status.info, .project-head .state.running")].map((el) => getComputedStyle(el).opacity));
    check(dimmed.length > 0 && dimmed.every((o) => Number(o) < 1), `${size}: working states are dimmed (${dimmed.join(", ")})`);
    const since = () => page.evaluate(() => document.querySelector(".thread-row [data-since]")?.textContent ?? "");
    const held = await since();
    await page.waitForTimeout(2500);
    check(held !== "" && held === (await since()), `${size}: the working timer holds at ${held}`);
    await shot("reconnecting");
    startHub();
    await waitFor("the app to reconnect", () => page.evaluate(() => document.body.dataset.hub === "online"), 90000);
    await waitFor("both nodes back after the hub", () => page.evaluate(() => machineOnline("sweep-node") && machineOnline("sweep-node-2")), 90000);
    await settle();
    check(!(await page.evaluate(barShows)), `${size}: the bar clears when the hub is back`);
    check((await since()) !== held, `${size}: the timer runs again (${held} -> ${await since()})`);
    await shot("reconnected");
    await context.close();

    // 4. Sign-in with the hub away.
    const fresh = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
    const login = await fresh.newPage();
    login.on("pageerror", (e) => errors.push(String(e)));
    await login.goto(HUB);
    await login.waitForSelector("#login [name=email]");
    await login.fill("#login [name=email]", EMAIL);
    await login.fill("#login [name=password]", PASSWORD);
    await stopPid("hub.pid");
    await login.click("#login .primary");
    await waitFor("the sign-in error", () => login.evaluate(() => document.querySelector("#login .error").textContent !== ""));
    const said = await login.evaluate(() => document.querySelector("#login .error").textContent);
    console.log(`signin: ${said}`);
    check(said === `Can't reach the hub at 127.0.0.1:${HUB_PORT}. Check your connection and try again.`, `${size}: sign-in says it can't reach the hub`);
    check(!/TypeError|Failed to fetch/.test(said), `${size}: no raw JavaScript error`);
    check(await login.isVisible("#login .retry"), `${size}: Retry shows`);
    await login.waitForTimeout(400);
    await login.screenshot({ path: path.join(SHOTS, `connection-states-signin-real-${size}.png`) });
    console.log(`shot signin-${size}`);
    startHub();
    await waitFor("the hub", () => { try { execFileSync("curl", ["-fs", HUB], { stdio: "ignore" }); return true; } catch { return false; } });
    await login.click("#login .retry");
    await waitFor("Retry to sign in", () => login.evaluate(() => document.body.dataset.hub === "online"), 30000);
    check(true, `${size}: Retry signed in once the hub was back`);
    await fresh.close();
  }
  await browser.close();
  check(errors.length === 0, `no page errors${errors.length ? `: ${errors.join(" | ")}` : ""}`);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
