#!/usr/bin/env node
// Drives P3b's approvals on the real page (CONTRACTS.md 13.5). An agent asks to pair a machine and to
// add an OpenCode Go account. Approving pair_code shows the pairing command once, to the user only.
// Approving add_account opens the Accounts form, where the user types the key the agent never sees.
// Both run at 1440x900 and 390x844, and each size gets one side-by-side picture of the two.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/surface-approvals.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes approvals-*.png into scripts/web-sweep/screens/.
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawn } = require("child_process");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname, "../../target"), "debug/pinned-server");
const SCREENS = path.join(__dirname, "screens");
const WORK = path.join(DIR, "home", "work");
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
/** The key scripts/web-sweep/mock-opencode-go accepts. */
const GO_KEY = "oc_sk_sweep";

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

/** The socket of the node whose pid `$DIR/node.pid` holds; the newest folder, since pids repeat. */
function nodeSock() {
  const pid = fs.readFileSync(path.join(DIR, "node.pid"), "utf8").trim();
  const dir = fs
    .readdirSync(os.tmpdir())
    .filter((d) => d.startsWith(`pinned-${pid}-`))
    .sort((a, b) => fs.statSync(path.join(os.tmpdir(), b)).mtimeMs - fs.statSync(path.join(os.tmpdir(), a)).mtimeMs)[0];
  if (!dir) throw new Error(`no socket folder for the node (${pid})`);
  return path.join(os.tmpdir(), dir, "pinned.sock");
}

/** The agent's `act` calls, in order, through `pinned-server mcp` as the session's claude runs it. */
function acts(tab, calls) {
  return new Promise((resolve, reject) => {
    const { PINNED_TASK, PINNED_ACCOUNT, PINNED_ROLE, CLAUDE_CONFIG_DIR, ...env } = process.env;
    const child = spawn(SERVER, ["mcp"], { env: { ...env, PINNED_SOCK: nodeSock(), PINNED_TAB: tab } });
    const requests = [
      { jsonrpc: "2.0", id: 0, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "sweep", version: "1" } } },
      ...calls.map((args, i) => ({ jsonrpc: "2.0", id: i + 1, method: "tools/call", params: { name: "act", arguments: args } })),
    ];
    const answers = new Map();
    let buffered = "";
    let sent = 0;
    const next = () => sent < requests.length && child.stdin.write(`${JSON.stringify(requests[sent++])}\n`);
    child.stdout.on("data", (chunk) => {
      buffered += chunk;
      for (let at; (at = buffered.indexOf("\n")) >= 0; ) {
        const message = JSON.parse(buffered.slice(0, at));
        buffered = buffered.slice(at + 1);
        answers.set(message.id, message);
        if (answers.size === requests.length) {
          child.stdin.end();
          resolve(requests.slice(1).map((r) => answers.get(r.id).result.content[0].text));
        } else next();
      }
    });
    child.on("error", reject);
    next();
    setTimeout(() => reject(new Error("the MCP server did not answer every call")), 120000);
  });
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

const openWaiting = (page) => page.evaluate(() => {
  ui.waitingOpen = true;
  renderSidebar();
});
const asksShown = (page) => page.evaluate(() => [...document.querySelectorAll("#thread-list .agent-confirm .title")].map((t) => t.textContent));
const threadText = (page) => page.evaluate(() => document.querySelector("#thread")?.innerText ?? "");

/** Two screenshots side by side, as one picture. */
async function sideBySide(browser, left, right, out, [width, height]) {
  const page = await browser.newPage({ viewport: { width: width * 2 + 48, height: height + 32 } });
  const img = (file) => `<img src="data:image/png;base64,${fs.readFileSync(file).toString("base64")}" style="width:${width}px;height:${height}px;border:1px solid #444">`;
  await page.setContent(`<body style="margin:0;padding:16px;display:flex;gap:16px;background:#111">${img(left)}${img(right)}</body>`);
  await page.screenshot({ path: out });
  await page.close();
  fs.rmSync(left);
  fs.rmSync(right);
}

/** One run at one size: the agent asks, the user approves both, and the agent learns no secret. */
async function approveBoth(view, agent, shot, name) {
  const [paired, added, refused] = await acts(agent.id, [
    { action: "pair_code", request_id: `pair-${shot}` },
    { action: "add_account", args: { provider: "claude", name, auth: "opencode-go" }, request_id: `go-${shot}` },
    { action: "add_account", args: { provider: "claude", name, auth: "opencode-go", key: GO_KEY } },
  ]);
  const pair = JSON.parse(paired);
  const account = JSON.parse(added);
  check(pair.outcome === "pending" && account.outcome === "pending", `${shot}: both wait for the user: ${paired} ${added}`);
  check(refused === "an agent never sends a password, key or token: key. The user enters it in the app.", `${shot}: an agent's key is refused: ${refused}`);

  await openWaiting(view);
  await waitFor(`${shot}: both in Waiting on you`, async () => (await asksShown(view)).length === 2);
  check((await asksShown(view)).join(" | ") === "Pair a machine | Add account", `${shot}: the items name the action in words: ${(await asksShown(view)).join(" | ")}`);

  await view.click(`#thread-list .agent-confirm[data-confirmation="${pair.id}"] [data-agent=approve]`);
  const command = await waitFor(`${shot}: the pairing command`, () => view.evaluate(() => document.querySelector("#thread-list .agent-once code")?.textContent));
  check(/^curl -fsSL http:\/\/127\.0\.0\.1:\d+\/install\.sh \| sh -s -- \S+$/.test(command), `${shot}: the user sees the pairing command: ${command}`);
  const code = command.split(" ").at(-1);
  await view.screenshot({ path: path.join(SCREENS, `approvals-pair-${shot}.png`) });

  await view.click(`#thread-list .agent-confirm[data-confirmation="${account.id}"] [data-agent=approve]`);
  await waitFor(`${shot}: the Accounts form`, () => view.evaluate(() => document.getElementById("agent-asking")?.textContent.includes("Entering the key here approves it.")));
  const form = await view.evaluate(() => {
    const f = document.querySelector("form.add-account");
    return { provider: f.provider.value, locked: f.provider.disabled, name: f.name.value, readonly: f.name.readOnly, key: !f.key.hidden };
  });
  check(form.provider === "opencode-go" && form.locked && form.name === name && form.readonly && form.key, `${shot}: the form holds what the agent asked, and a key field: ${JSON.stringify(form)}`);
  if (shot === "phone") {
    const wide = await view.evaluate(() => document.querySelector("#settings-page").scrollWidth - document.querySelector("#settings-page").clientWidth);
    check(wide <= 0, `${shot}: the form fits sideways (${wide}px)`);
  }
  await view.screenshot({ path: path.join(SCREENS, `approvals-form-${shot}.png`) });

  await view.fill("form.add-account [name=key]", GO_KEY);
  await view.click("form.add-account button.primary");
  await waitFor(`${shot}: the account the approval added`, () => view.evaluate((n) => document.getElementById("account-list")?.innerText.includes(n), name));
  check(await view.evaluate(() => !document.getElementById("agent-asking")), `${shot}: the form forgets the request once it ran`);
  return { pair, account, code };
}

(async () => {
  const work = fs.realpathSync(path.join(WORK, "alpha"));
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
  const errors = [];
  const contextOf = async (viewport, phone) => {
    const context = await browser.newContext({ viewport, deviceScaleFactor: phone ? 2 : 1, colorScheme: "dark", isMobile: phone, hasTouch: phone });
    const page = await context.newPage();
    page.on("pageerror", (e) => errors.push(String(e)));
    return page;
  };
  const page = await contextOf({ width: 1440, height: 900 }, false);
  const node = await signIn(page);

  const agent = await page.evaluate(async ({ machine, work }) => {
    const tab = await revealTab(machine, await callOn(machine, { cmd: "open_tab", path: work, bypass: false }));
    activate(tab);
    await callOn(machine, { cmd: "start_claude", tab: tab.view.id, cols: 120, rows: 32 });
    return { id: tab.view.id, key: tab.key };
  }, { machine: node, work });
  await waitFor("the agent's session", () => page.evaluate((k) => tabByKey(k)?.view.running, agent.key));
  await page.evaluate(({ machine, tab }) => callOn(machine, { cmd: "prompt", tab, text: "Pair the new server and add my OpenCode Go account." }), { machine: node, tab: agent.id });
  await waitFor("the agent's first turn to end", () => page.evaluate((k) => tabByKey(k)?.view.status === "done", agent.key));

  const desk = await approveBoth(page, agent, "desktop", "Go desk");
  await page.evaluate((k) => activate(tabByKey(k)), agent.key);
  // The first notice is typed into the idle session; the second rides that turn's end as hook context.
  await waitFor("the pair_code notice in the agent's thread", async () => (await threadText(page)).includes(`The user approved your pair_code request (${desk.pair.id}): done.`), 45000);
  const decided = await page.evaluate(() => hub.hub({ cmd: "confirmations" }));
  const outcome = decided.find((c) => c.id === desk.account.id)?.outcome;
  check(outcome === "done", `the add_account approval ran with the typed key: ${outcome}`);
  const told = await threadText(page);
  check(!told.includes(desk.code) && !told.includes(GO_KEY), "the agent's notices hold neither the pairing code nor the key");
  const trail = await page.evaluate(() => hub.hub({ cmd: "agent_audit", limit: 200 }));
  const audited = JSON.stringify(trail);
  check(!audited.includes(desk.code) && !audited.includes(GO_KEY), "the audit trail holds neither the pairing code nor the key");
  await sideBySide(browser, path.join(SCREENS, "approvals-pair-desktop.png"), path.join(SCREENS, "approvals-form-desktop.png"), path.join(SCREENS, "approvals-desktop.png"), [1440, 900]);

  const phone = await contextOf({ width: 390, height: 844 }, true);
  await signIn(phone);
  await phone.evaluate(() => toggleSidebar(true));
  await approveBoth(phone, agent, "phone", "Go phone");
  await sideBySide(browser, path.join(SCREENS, "approvals-pair-phone.png"), path.join(SCREENS, "approvals-form-phone.png"), path.join(SCREENS, "approvals-phone.png"), [390, 844]);

  check(errors.length === 0, `no page errors: ${errors.join(" | ")}`);
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
