#!/usr/bin/env node
// Drives Run now on the real page (CONTRACTS.md 13.12). A throwaway machine seeds three workflows:
// one that runs, one whose model the project's tier refuses, and a Slack sorter with no session of
// its own. At 1440x900 and 390x844 the sweep shows the Workflows list, clicks Run now, shows the
// disabled Starting… state, the started line with the session's machine, opens that session, and
// shows the limits' refusal in plain words.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/run-now.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes run-now-*.png into RUN_NOW_SHOTS, or scripts/web-sweep/screens/.
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const SHOTS = process.env.RUN_NOW_SHOTS || path.join(__dirname, "screens");
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";

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

async function api(route, body, token) {
  const res = await fetch(`${HUB}${route}`, {
    method: "POST",
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: JSON.stringify(body ?? {}),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${route} ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

const workflow = (id, name, model, folder, trigger = { kind: "linear-label", label: "nightly" }) => ({
  id,
  name,
  enabled: false,
  trigger,
  folder,
  model,
  effort: "medium",
  permissionMode: "acceptEdits",
  worktree: false,
  instructions: "Run the build and say what broke.",
  columns: { picked: "", coding: "", review: "" },
  fix: "off",
  proof: false,
});

/** An account's connectors are filled by a machine's import, as `sweep.js` seeds them. */
async function seed(folder) {
  const { token } = await api("/api/login", { email: EMAIL, password: PASSWORD });
  const machine = await api("/api/machines", { name: "seed", os: "linux" }, token);
  const ws = new WebSocket(`${HUB.replace("http", "ws")}/ws/machine`);
  const reply = new Promise((resolve, reject) => {
    ws.onmessage = (e) => {
      const frame = JSON.parse(e.data);
      if (frame.type === "reply") ("err" in frame ? reject(new Error(frame.err)) : resolve(frame.ok));
    };
    ws.onerror = () => reject(new Error("seed socket failed"));
  });
  await new Promise((r) => (ws.onopen = r));
  ws.send(JSON.stringify({ type: "auth", token: machine.token, name: "seed", os: "linux", version: "sweep" }));
  const store = {
    linear: { name: "Sweep Bot", email: "bot@example.test", organization: "Acme" },
    slack: { team: "Acme", userId: "U1", name: "pinned-bot" },
    connectors: [
      workflow("nightly", "Nightly build check", "claude-sonnet-5-5", folder),
      workflow("notes", "Release notes", "claude-opus-5-5", folder),
      workflow("sort", "Sort #support", "", folder, { kind: "slack-message", channel: "support", decide: { model: "", instructions: "", categories: [], fallback: null } }),
    ],
  };
  ws.send(JSON.stringify({ type: "call", id: 1, call: { cmd: "import", store, linear_key: "lin_api_fake", slack_key: "xoxb-fake" } }));
  check(await reply.finally(() => ws.close()), "the seed imported three workflows");
}

async function signIn(page) {
  await page.goto(HUB);
  if (await page.isVisible("#login [name=email]")) {
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await page.click("#login .primary");
  }
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  return waitFor("the sweep node", () => page.evaluate(() => [...machines.entries()].find(([, m]) => m.view.online && m.view.name === "sweep-node")?.[0]));
}

/** Types the password into the prompt a locked login opens, when one opens. */
async function unlockIfAsked(page) {
  const asked = await waitFor("the password prompt", () => page.evaluate(() => !!document.querySelector("dialog[open] form [name=password]")), 3000).catch(() => false);
  if (!asked) return;
  await page.fill("dialog[open] form [name=password]", PASSWORD);
  await page.click("dialog[open] form button.primary");
  await waitFor("the prompt to close", () => page.evaluate(() => !document.querySelector("dialog[open]")));
}

const card = (page, name) => page.locator("#settings-page .flow-card", { hasText: name });

async function openWorkflows(page, phone) {
  if (phone) await page.evaluate(() => toggleSidebar(true));
  await page.click((await page.isVisible("#gui-settings")) ? "#gui-settings" : "#settings-toggle");
  await page.click('#settings-nav [data-page="connectors"]');
  await page.locator("#settings-page .flow-card").first().waitFor();
}

async function sweep(browser, size, phone, shot) {
  const context = await browser.newContext({ viewport: size, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e.stack ?? e).split("\n").slice(0, 3).join(" <- ")));
  const node = await signIn(page);
  await openWorkflows(page, phone);
  const nightly = card(page, "Nightly build check");
  check((await nightly.locator(".run-now").textContent()) === "Run now", `${shot}: each workflow offers Run now`);
  check(await card(page, "Sort #support").locator(".run-now").isDisabled(), `${shot}: a Slack sorter's Run now is disabled`);
  await page.screenshot({ path: path.join(SHOTS, `run-now-list-${shot}.png`) });

  // Holds the call a moment, so the picture catches the button while the run starts.
  await page.evaluate(() => {
    const real = hub.hub.bind(hub);
    hub.hub = (call) => (call.cmd === "run_workflow" ? new Promise((r) => setTimeout(r, 2000)).then(() => real(call)) : real(call));
  });
  await nightly.locator('[data-action="run-now"]').click();
  await waitFor(`${shot}: Starting…`, async () => (await nightly.locator(".run-now").textContent()) === "Starting…");
  check(await nightly.locator(".run-now").isDisabled(), `${shot}: Run now is disabled while it starts`);
  await page.screenshot({ path: path.join(SHOTS, `run-now-starting-${shot}.png`) });
  const started = await waitFor(`${shot}: the started line`, () => nightly.locator(".run-now-outcome.started").textContent().catch(() => null), 60000);
  check(started.startsWith("Started on sweep-node."), `${shot}: ${started}`);
  await page.screenshot({ path: path.join(SHOTS, `run-now-started-${shot}.png`) });

  const tab = await nightly.locator('[data-action="run-now-open"]').getAttribute("data-tab");
  await nightly.locator('[data-action="run-now-open"]').click();
  await waitFor(`${shot}: the session in view`, () => page.evaluate((id) => active?.view.id === id && document.body.dataset.page !== "settings", tab));
  const view = await page.evaluate(() => active.view);
  check(view.connector === "nightly" && view.running, `${shot}: the session is the workflow's, running on ${node}`);
  await page.waitForTimeout(800);
  await page.screenshot({ path: path.join(SHOTS, `run-now-session-${shot}.png`) });

  await openWorkflows(page, phone);
  await card(page, "Release notes").locator('[data-action="run-now"]').click();
  const refused = await waitFor(`${shot}: the refusal`, () => card(page, "Release notes").locator(".run-now-outcome.refused").textContent().catch(() => null), 60000);
  check(refused === "Didn't start: claude-opus-5-5 is a high-tier model; this project allows up to mid. Pick a mid model or raise Tier in Settings › Way of working", `${shot}: ${refused}`);
  await card(page, "Release notes").scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(SHOTS, `run-now-refused-${shot}.png`) });
  // readMenu in terminal-menu.js reads past a fake session's last screen line. It predates Run
  // now and fires on any session the page paints, so it is listed, not counted.
  const known = errors.filter((e) => e.includes("terminal-menu.js"));
  if (known.length) console.log(`known, not Run now's: ${known.length} × ${known[0]}`);
  const others = errors.filter((e) => !e.includes("terminal-menu.js"));
  check(others.length === 0, `${shot}: no other page errors: ${others.join(" | ")}`);
  await context.close();
}

(async () => {
  fs.mkdirSync(SHOTS, { recursive: true });
  await seed(fs.realpathSync(path.join(DIR, "home", "work", "alpha")));
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
  const setup = await (await browser.newContext()).newPage();
  await signIn(setup);
  const limited = setup.evaluate(() => hub.hub({ cmd: "set_taste", scope: { kind: "captains" }, patch: { tier: "mid" } }));
  await unlockIfAsked(setup);
  await limited;
  check(true, "the account's projects allow models up to the mid tier");
  await sweep(browser, { width: 1440, height: 900 }, false, "desktop");
  await sweep(browser, { width: 390, height: 844 }, true, "phone");
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
