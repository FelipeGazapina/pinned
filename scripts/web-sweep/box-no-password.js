#!/usr/bin/env node
// Drives 0.17.23 G and I on the real page, against a stub of the Box API. The app's login is locked.
// An agent session's open_box under Ask first files a question on the Upstash Box card, and the
// owner answers it with one click and no password, at 1440x900 and on a 390x844 phone. Then the card
// lists the Claude accounts signed in on the machine, and Use for boxes asks for the password, waits
// for the approval in the browser, and keeps the token the machine's claude setup-token printed.
//
//   UPSTASH_BOX_BASE_URL=http://127.0.0.1:$SWEEP_BOX_PORT SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... SHOTS=<folder> node scripts/web-sweep/box-no-password.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
const fs = require("fs");
const http = require("http");
const os = require("os");
const path = require("path");
const { spawn } = require("child_process");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const BOX_PORT = Number(process.env.SWEEP_BOX_PORT || 8799);
const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname, "../../target"), "debug/pinned-server");
const SHOTS = process.env.SHOTS || path.join(__dirname, "screens");
const WORK = path.join(DIR, "home", "work");
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const PROMPT = "dialog[open] form";
const YES = '[data-action="answer-box"][data-allow="yes"]';

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

/** The Box API as far as connecting needs it: a box_ key lists no boxes; anything else is refused. */
function boxApi() {
  const server = http.createServer((req, res) => {
    const ok = (req.headers["x-box-api-key"] || "").startsWith("box_") && req.method === "GET";
    res.writeHead(ok ? 200 : 401, { "content-type": "application/json" });
    res.end(ok ? "[]" : JSON.stringify({ error: "the sweep's Box API opens no box" }));
  });
  return new Promise((resolve) => server.listen(BOX_PORT, "127.0.0.1", () => resolve(server)));
}

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

/** The agent's `act` call through `pinned-server mcp`, as the session's claude runs it: its text. */
function act(tab, args) {
  return new Promise((resolve, reject) => {
    const { PINNED_TASK, PINNED_ACCOUNT, PINNED_ROLE, CLAUDE_CONFIG_DIR, ...env } = process.env;
    const child = spawn(SERVER, ["mcp"], { env: { ...env, PINNED_SOCK: nodeSock(), PINNED_TAB: tab } });
    const requests = [
      { jsonrpc: "2.0", id: 0, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "sweep", version: "1" } } },
      { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "act", arguments: args } },
    ];
    let buffered = "";
    child.stdout.on("data", (chunk) => {
      buffered += chunk;
      for (let at; (at = buffered.indexOf("\n")) >= 0; ) {
        const message = JSON.parse(buffered.slice(0, at));
        buffered = buffered.slice(at + 1);
        if (message.id === 0) child.stdin.write(`${JSON.stringify(requests[1])}\n`);
        if (message.id === 1) {
          child.stdin.end();
          resolve(message.result.content[0].text);
        }
      }
    });
    child.on("error", reject);
    child.stdin.write(`${JSON.stringify(requests[0])}\n`);
    setTimeout(() => reject(new Error("the MCP server did not answer")), 120000);
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

async function typePassword(page) {
  await waitFor("the password prompt", () => page.evaluate((p) => !!document.querySelector(`${p} [name=password]`), PROMPT));
  await page.fill(`${PROMPT} [name=password]`, PASSWORD);
  await page.click(`${PROMPT} button.primary`);
  await waitFor("the prompt to close", () => page.evaluate(() => !document.querySelector("dialog[open]")));
}

const openCard = async (page) => {
  await page.evaluate(() => {
    openSettings();
    showPage("integrations");
  });
  await waitFor("the Upstash Box card", () => page.evaluate(() => !!document.querySelector("#box-asks")));
};

/** An agent session of the node in project `name`, known to the hub, and the question its open_box files. */
async function askingSession(page, node, name) {
  const folder = fs.realpathSync(path.join(WORK, name));
  const tab = await page.evaluate(({ node, folder }) => callOn(node, { cmd: "open_tab", path: folder, bypass: false }), { node, folder });
  await waitFor("the session's tab", () => page.evaluate((id) => tabs.some((t) => t.view.id === id), tab));
  const asked = await act(tab, { action: "open_box", args: { repo: `example/${name}`, size: "small", budget_usd: 1 } });
  check(/waiting for a yes on the Upstash card/.test(asked), `the agent's box waits for the owner's yes: ${asked}`);
  return tab;
}

/** Shoots the question, answers it with one click, and checks that no password prompt opened. */
async function answer(page, tab, shot) {
  await openCard(page);
  await waitFor("the question on the card", () => page.evaluate(() => !!document.querySelector('[data-action="answer-box"]')));
  const asked = await page.evaluate(() => document.querySelector("#box-asks").innerText);
  check(asked.includes(`sessions of sweep-node ${tab}`), `${shot}: the card asks about the agent's session: ${asked}`);
  await page.evaluate(() => document.querySelector("#box-asks").scrollIntoView({ block: "center" }));
  await page.screenshot({ path: path.join(SHOTS, `box-ask-${shot}.png`), scale: "css" });
  await page.click(YES);
  await waitFor(`${shot}: the question answered`, () => page.evaluate(() => !document.querySelector('[data-action="answer-box"]')));
  await new Promise((r) => setTimeout(r, 800));
  check(!(await page.evaluate(() => !!document.querySelector("dialog[open]"))), `${shot}: the yes asked for no password`);
  await page.evaluate(() => document.querySelector("#box-asks").closest(".card").scrollIntoView({ block: "center" }));
  await page.screenshot({ path: path.join(SHOTS, `box-ask-answered-${shot}.png`), scale: "css" });
  const opened = JSON.parse(await act(tab, { action: "open_box", args: { repo: "example/alpha", size: "small", budget_usd: 1 } }));
  check(opened.outcome === "done", `${shot}: after the yes the agent's open_box runs: ${JSON.stringify(opened)}`);
}

(async () => {
  fs.mkdirSync(SHOTS, { recursive: true });
  const stub = await boxApi();
  const executablePath = process.env.CHROME_BIN || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  const browser = await playwright.chromium.launch({ executablePath, headless: true });
  const errors = [];
  try {
    const desktop = await (await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" })).newPage();
    desktop.on("pageerror", (e) => errors.push(String(e)));
    const node = await signIn(desktop);
    const connecting = desktop.evaluate(() => connectorCall("connect_upstash", { key: "box_sweepkey", claude_token: "sk-ant-oat01-sweepdefault" }));
    await typePassword(desktop);
    await connecting;
    check(true, "connecting Upstash took the password on the locked app");
    await desktop.evaluate(() => connectorCall("set_box_access", { access: { workflow: { mode: "fresh" }, session: { mode: "ask" }, worker: { mode: "off" }, orchestrator: { mode: "off" } } }));

    await answer(desktop, await askingSession(desktop, node, "alpha"), "desktop");

    const phone = await (await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true, colorScheme: "dark" })).newPage();
    phone.on("pageerror", (e) => errors.push(String(e)));
    await signIn(phone);
    await answer(phone, await askingSession(desktop, node, "beta"), "phone");

    await desktop.evaluate((node) => callOn(node, { cmd: "add_account", provider: "claude", name: "Bloom" }), node);
    const homes = path.join(DIR, "node", "accounts", "bloom");
    await waitFor("bloom's home", () => fs.existsSync(homes));
    fs.writeFileSync(path.join(homes, "signed-in"), "");
    const listed = async (page) => {
      await openCard(page);
      await page.evaluate(async (node) => {
        await accountsOf(node, true);
        await loadBoxLogins();
      }, node);
      await waitFor("both accounts on the card", () => page.evaluate(() => document.querySelectorAll(".box-login").length === 2));
      await page.evaluate(() => document.querySelector(".box-logins").scrollIntoView({ block: "start" }));
      return page.evaluate(() => [...document.querySelectorAll(".box-login span:first-child")].map((s) => s.innerText.replace(/\s+/g, " ")));
    };
    const rows = await listed(desktop);
    check(JSON.stringify(rows) === JSON.stringify(["claude personal@example.test · max · on sweep-node", "bloom bloom@example.test · max · on sweep-node"]), `each account with its email and plan: ${JSON.stringify(rows)}`);
    await desktop.screenshot({ path: path.join(SHOTS, "box-logins-desktop.png"), scale: "css" });
    await listed(phone);
    await phone.screenshot({ path: path.join(SHOTS, "box-logins-phone.png"), scale: "css" });

    await phone.click('[data-action="use-for-boxes"][data-account="bloom"]');
    await waitFor("the password prompt", () => phone.evaluate((p) => !!document.querySelector(`${p} [name=password]`), PROMPT));
    check(true, "Use for boxes asks for the password on the locked phone");
    await phone.screenshot({ path: path.join(SHOTS, "box-logins-unlock-phone.png"), scale: "css" });
    await typePassword(phone);
    await waitFor("the row to wait on the browser", () => phone.evaluate(() => /Approve in the browser on sweep-node, signed in as bloom@example.test/.test(document.querySelector("#box-logins").innerText)));
    await phone.screenshot({ path: path.join(SHOTS, "box-logins-waiting-phone.png"), scale: "css" });
    fs.writeFileSync(path.join(homes, "approve-setup-token"), "");
    await waitFor("the token kept", () => phone.evaluate(() => /Used for boxes/.test(document.querySelector("#box-logins").innerText) && !!settings.box_claude_tokens?.bloom), 60000);
    await phone.evaluate(() => document.querySelector(".box-logins").scrollIntoView({ block: "start" }));
    await phone.screenshot({ path: path.join(SHOTS, "box-logins-kept-phone.png"), scale: "css" });
    const seen = await phone.evaluate(() => document.documentElement.outerHTML + JSON.stringify(settings));
    check(!seen.includes("sk-ant-oat01-sweep-bloom"), "the page holds the hint only, never the token");

    check(!errors.length, `no page errors: ${errors.slice(0, 3).join(" | ")}`);
    console.log("box-no-password: every check passed");
  } finally {
    await browser.close();
    stub.close();
  }
})().catch((error) => {
  console.error(error.stack || String(error));
  process.exit(1);
});
