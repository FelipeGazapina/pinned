#!/usr/bin/env node
// Settings › Machines with load and accounts (issue #151): a second node joins the stack with no
// claude program, and the page must show each machine's workers, CPU, memory, disk and accounts, with
// claude ready on the stack's node and not ready on the second. Screenshots at
// 1440x900 and 390x844 go to $SWEEP_DIR/screens/machine-load-*.png.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/machine-load.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// The script starts sweep-mini and stops it again. SWEEP_CHANNEL=chrome drives the installed Chrome.
const fs = require("fs");
const path = require("path");
const { execFileSync, spawn } = require("child_process");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const SERVER = `${process.env.CARGO_TARGET_DIR || path.join(__dirname, "../../target")}/debug/pinned-server`;
const SCREENS = path.join(DIR, "screens");
const VIEWPORTS = [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]];

const waitFor = async (what, fn, timeout = 60000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`timed out waiting for ${what}`);
};

function startMini() {
  const home = path.join(DIR, "home-mini");
  const config = path.join(DIR, "node-mini");
  fs.mkdirSync(home, { recursive: true });
  // No fake claude on its PATH, so this machine has the account but can't run a worker with it.
  const env = { ...process.env, HOME: home, SHELL: "/bin/sh", PATH: "/usr/bin:/bin:/usr/sbin:/sbin" };
  for (const key of ["PINNED_SOCK", "PINNED_TASK", "PINNED_ACCOUNT", "PINNED_TAB", "CLAUDE_CONFIG_DIR"]) delete env[key];
  execFileSync(SERVER, ["login", HUB, "sweep@example.test", "--name", "sweep-mini", "--config-dir", config], { env: { ...env, PINNED_PASSWORD: "sweep password 1" }, stdio: "ignore" });
  const log = fs.openSync(path.join(DIR, "node-mini.log"), "a");
  const proc = spawn(SERVER, ["node", "--config-dir", config], { env, stdio: ["ignore", log, log] });
  fs.writeFileSync(path.join(DIR, "node-mini.pid"), String(proc.pid));
  return proc;
}

async function signIn(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", "sweep@example.test");
    await page.fill("#login [name=password]", "sweep password 1");
    await page.click("#login .primary");
  }
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
}

const rows = (page) =>
  page.evaluate(() =>
    [...document.querySelectorAll("#settings-page .machine-row")].map((row) => ({
      name: row.querySelector(".who b")?.textContent,
      stats: [...row.querySelectorAll(".stat")].map((s) => s.textContent.replace(/\s+/g, " ").trim()),
      accounts: [...row.querySelectorAll(".acct")].map((a) => a.textContent),
    })),
  );

(async () => {
  fs.mkdirSync(SCREENS, { recursive: true });
  const mini = startMini();
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined });
  let failed = 0;
  const check = (ok, what) => {
    if (!ok) failed++;
    console.log(`${ok ? "ok" : "FAIL"}: ${what}`);
  };
  try {
    for (const [device, viewport] of VIEWPORTS) {
      const context = await browser.newContext({ viewport, colorScheme: "dark" });
      const page = await context.newPage();
      await signIn(page);
      await page.evaluate(() => setMode("gui"));
      console.log(`${device}: signed in`);
      await waitFor("two online machines", () => page.evaluate(() => [...machines.values()].filter((m) => m.view.online).length >= 2));
      console.log(`${device}: machines online`);
      await page.evaluate(() => openSettings("machines"));
      await waitFor("both machines' state and accounts", async () => {
        await page.evaluate(() => hub.hub({ cmd: "machines" }).then((list) => showMachines(list, null)));
        const shown = await rows(page);
        return shown.filter((r) => r.stats.length >= 3 && r.accounts.length).length >= 2;
      });
      const shown = await rows(page);
      console.log(JSON.stringify(shown));
      const claudeOff = shown.filter((r) => r.accounts.includes("claude · not ready")).map((r) => r.name);
      const claudeReady = shown.filter((r) => r.accounts.includes("claude")).map((r) => r.name);
      check(claudeOff.join() === "sweep-mini" && claudeReady.join() === "sweep-node", `${device}: claude is ready on ${claudeReady} and not ready on ${claudeOff}`);
      check(shown.every((r) => /^Workers \d+ of \d+/.test(r.stats[0]) && r.stats.some((s) => s.startsWith("CPU ")) && r.stats.some((s) => s.startsWith("Disk "))), `${device}: each machine shows workers, CPU and disk`);
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
      check(!overflow, `${device}: the page does not scroll sideways`);
      await page.screenshot({ path: path.join(SCREENS, `machine-load-${device}.png`) });
      await context.close();
    }
  } finally {
    await browser.close();
    mini.kill();
    fs.rmSync(path.join(DIR, "node-mini.pid"), { force: true });
  }
  process.exit(failed ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
