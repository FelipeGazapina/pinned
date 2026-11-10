#!/usr/bin/env node
// Shoots "Waiting on you" with approvals from two machines, at 1440x900 and 390x844: a Captain on
// sweep-node delegates a worker to a second node (sweep-mini), the worker hits a permission prompt,
// and two loose threads ask a question and fail. The rows say Approval, Question and Failed in
// words, name their machine and show what is asked. Then the worker is answered and its row goes.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... PLAYWRIGHT=<dir>/node_modules/playwright node scripts/web-sweep/approvals-everywhere.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// The script starts sweep-mini itself and stops it at the end. Everything runs on the fake claude,
// whose `/ask` types a Bash permission prompt, `/questions` an AskUserQuestion and `limited` a
// usage-limit failure. Writes approvals-<screen>-<desktop|phone>.png into $APPROVALS_SHOTS
// (default scripts/web-sweep/screens/approvals-everywhere).
const { execFileSync, spawn } = require("child_process");
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const WORK = path.join(DIR, "home", "work");
const SCREENS = process.env.APPROVALS_SHOTS || path.join(__dirname, "screens", "approvals-everywhere");
const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname, "../../target"), "debug", "pinned-server");
const ALPHA = "https://github.com/example/alpha.git";
const SIZES = [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }]];

const check = (ok, what) => {
  if (!ok) throw new Error(what);
  console.log(`ok: ${what}`);
};
const waitFor = async (what, fn, timeout = 30000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`timed out waiting for ${what}`);
};

/** The second node of the sweep's account, with a checkout of alpha in its clones folder. */
function startMini() {
  const home = path.join(DIR, "home2");
  const config = path.join(DIR, "node2");
  const alpha = path.join(home, "pinned", "alpha");
  fs.mkdirSync(alpha, { recursive: true });
  execFileSync("git", ["-C", alpha, "init", "-q"]);
  execFileSync("git", ["-C", alpha, "remote", "add", "origin", ALPHA]);
  const env = { ...process.env, HOME: home, SHELL: path.join(DIR, "bin", "shell"), PATH: `${path.join(DIR, "bin")}:${process.env.PATH}` };
  for (const key of ["PINNED_SOCK", "PINNED_TASK", "PINNED_ACCOUNT", "PINNED_TAB", "CLAUDE_CONFIG_DIR"]) delete env[key];
  execFileSync(SERVER, ["login", HUB, EMAIL, "--name", "sweep-mini", "--config-dir", config], { env: { ...env, PINNED_PASSWORD: PASSWORD }, stdio: "ignore" });
  const log = fs.openSync(path.join(DIR, "node2.log"), "a");
  const proc = spawn(SERVER, ["node", "--config-dir", config], { env, stdio: ["ignore", log, log] });
  fs.writeFileSync(path.join(DIR, "node2.pid"), String(proc.pid));
  return proc;
}

// Runs in the page: the waiting area as text, in order.
const waitingArea = () =>
  [...document.querySelectorAll("#thread-list .waiting-row:not(.offer)")].map((el) => el.textContent.replace(/\s+/g, " ").trim());

const send = (page, key, text) =>
  page.evaluate(async ({ key, text }) => {
    const tab = tabByKey(key);
    activate(tab);
    if (!tab.view.running) startTab(tab);
    const start = Date.now();
    while (!tab.view.running && Date.now() - start < 15000) await new Promise((r) => setTimeout(r, 100));
    prompt.value = text;
    send();
  }, { key, text });

(async () => {
  const mini = startMini();
  const errors = [];
  const browser = await playwright.chromium.launch({ headless: true, channel: process.env.SWEEP_CHANNEL || undefined, executablePath: process.env.CHROME_BIN || undefined });
  try {
    const alpha = fs.realpathSync(path.join(WORK, "alpha"));
    const context = await browser.newContext({ viewport: SIZES[0][1], colorScheme: "dark" });
    const page = await context.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto(HUB);
    if (await page.isVisible("#login [name=email]")) {
      await page.fill("#login [name=email]", EMAIL);
      await page.fill("#login [name=password]", PASSWORD);
      await page.click("#login .primary");
    }
    await waitFor("both machines online", () => page.evaluate(() => document.body.dataset.hub === "online" && [...machines.values()].filter((m) => m.view.online).length === 2), 60000);
    await page.evaluate(() => setMode("gui"));
    const ids = await page.evaluate(() => Object.fromEntries([...machines].filter(([, m]) => m.view.online).map(([k, m]) => [m.view.name, k])));
    const node = ids["sweep-node"];

    // The Captain on sweep-node starts a worker on sweep-mini, which stays at work.
    fs.writeFileSync(path.join(DIR, "home", "delegate-calls.jsonl"), JSON.stringify({ task: "Apply the orders migration. [hold]", title: "Apply the orders migration", target: { machine: "sweep-mini" } }));
    const captain = await page.evaluate(async ({ machine, alpha }) => {
      const tab = await revealTab(machine, await callOn(machine, { cmd: "open_orchestrator", path: alpha }));
      activate(tab);
      startTab(tab);
      return { key: tab.key, id: tab.view.id };
    }, { machine: node, alpha });
    await waitFor("the Captain's session", () => page.evaluate((k) => tabByKey(k)?.view.running === true, captain.key));
    await send(page, captain.key, "Ship the orders migration on the mini.");
    const worker = await waitFor("the worker on sweep-mini", () => page.evaluate((id) => tabs.find((t) => t.view.spawned_by === id && t.view.role === "worker")?.key ?? null, captain.id), 60000);
    await waitFor("the worker at work", () => page.evaluate((k) => statusOf(tabByKey(k)) === "working", worker), 60000);
    check(await page.evaluate((k) => machines.get(tabByKey(k).machine).view.name, worker) === "sweep-mini", "the worker runs on sweep-mini");

    // Two loose threads on sweep-node: one asks a question, one hits a usage limit.
    const loose = async (title, command) => {
      const key = await page.evaluate(async ({ machine, alpha }) => (await revealTab(machine, await callOn(machine, { cmd: "open_tab", path: alpha, bypass: false }))).key, { machine: node, alpha });
      await send(page, key, title);
      await waitFor(`${title} to answer`, () => page.evaluate((k) => statusOf(tabByKey(k)) === "done", key));
      await send(page, key, command);
      return key;
    };
    await loose("Pick who opens the PR", "/questions");
    await loose("Fix the flaky checkout test", "limited");

    // The worker hits its permission prompt: type /ask into its thread, on its own machine.
    await send(page, worker, "/ask");
    await waitFor("three rows in Waiting on you", () => page.evaluate(() => document.querySelectorAll("#thread-list .waiting-row:not(.offer)").length >= 3), 60000);

    for (const [size, viewport] of SIZES) {
      const phone = size === "phone";
      const view = phone ? await browser.newContext({ viewport, deviceScaleFactor: 2, colorScheme: "dark", isMobile: phone, hasTouch: phone }) : context;
      const shot = phone ? await view.newPage() : page;
      if (phone) {
        shot.on("pageerror", (e) => errors.push(String(e)));
        await shot.goto(HUB);
        await waitFor("the hub on the phone", () => shot.evaluate(() => document.body.dataset.hub === "online"));
        await shot.evaluate(() => setMode("gui"));
        await waitFor("the rows on the phone", () => shot.evaluate(() => document.querySelectorAll("#thread-list .waiting-row:not(.offer)").length >= 3), 60000);
        await shot.evaluate(() => toggleSidebar(true));
      }
      await shot.evaluate(() => {
        ui.waitingOpen = true;
        renderSidebar();
        document.getElementById("thread-list").scrollTo(0, 0);
      });
      const rows = await shot.evaluate(waitingArea);
      console.log(`${size}:\n  ${rows.join("\n  ")}`);
      await shot.waitForTimeout(500);
      fs.mkdirSync(SCREENS, { recursive: true });
      await shot.screenshot({ path: path.join(SCREENS, `real-${size === "phone" ? "390x844" : "1440x900"}.png`) });
      if (!phone) {
        const text = rows.join("\n");
        check(/Approval · alpha · sweep-mini · \w+Bash\s?rm -rf build/.test(rows.find((r) => /sweep-mini/.test(r)) ?? ""), `the worker's approval reads Approval, names sweep-mini and shows Bash rm -rf build: ${rows.find((r) => /sweep-mini/.test(r))}`);
        check(/Question · alpha · sweep-node/.test(text), "the question reads Question and names its machine");
        check(/Failed · alpha · sweep-node/.test(text), "the failure reads Failed and names its machine");
      }
      if (phone) await view.close();
    }

    // The Captain's thread heard about it, as a task's end arrives.
    await page.evaluate((k) => activate(tabByKey(k)), captain.key);
    await waitFor("the notice in the Captain's thread", () => page.evaluate(() => /waits on the user for approval to use Bash: rm -rf build/.test(document.getElementById("thread").innerText)), 60000);
    check(true, "the Captain's thread got the notice that the worker waits on the user for approval");
    await page.waitForTimeout(400);
    await page.screenshot({ path: path.join(SCREENS, "real-captain-notice-1440x900.png") });

    check(!errors.length, `no page errors${errors.length ? `:\n${errors.slice(0, 5).join("\n")}` : ""}`);
    console.log("approvals everywhere: every check passed");
  } finally {
    await browser.close();
    mini.kill("SIGTERM");
  }
})().catch((e) => {
  console.error(e.stack || String(e));
  process.exit(1);
});
