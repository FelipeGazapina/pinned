#!/usr/bin/env node
// Drives P3c's MCP servers page on the real app (CONTRACTS.md 14). The user adds servers with their
// secrets; an agent asks to add one, and its secret is refused. Approving opens the page's form with
// the agent's server locked, and the secret the user types there approves it. Runs at 1440x900 and
// 390x844, and each size gets one side-by-side picture: the page, then the approval form.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/mcp-servers.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes mcp-desktop.png and mcp-phone.png into scripts/web-sweep/screens/.
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
/** Every value typed as a secret here, which no answer, trail row or notice may hold. */
const SECRETS = ["ctx-sweep-key-1", "docs-sweep-key-2", "c7-sweep-key-3", "agent-sweep-guess-4"];

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
const holdsNoSecret = (text) => !SECRETS.some((secret) => text.includes(secret));

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

const pageText = (view) => view.evaluate(() => document.getElementById("settings-page")?.innerText ?? "");

/** One run at one size: the page with the user's servers, then an agent's add approved from its form. */
async function run(view, agent, shot) {
  await view.evaluate(() => openSettings("mcp-servers"));
  await waitFor(`${shot}: the servers on the page`, async () => (await view.locator("#mcp-list .mcp-server").count()) >= 3);
  const shown = await pageText(view);
  check(shown.includes("API_KEY · set") && shown.includes("X-Api-Key · set") && holdsNoSecret(shown), `${shot}: the page shows each secret as set, never its value`);
  check((shown.match(/If it signs in with OAuth, log in once per Claude account/g) ?? []).length === 1, `${shot}: only linear, which sends no header, says to log in once per Claude account`);
  if (shot === "phone") {
    const wide = await view.evaluate(() => document.querySelector("#settings-page").scrollWidth - document.querySelector("#settings-page").clientWidth);
    check(wide <= 0, `${shot}: the page fits sideways (${wide}px)`);
  }
  await view.screenshot({ path: path.join(SCREENS, `mcp-page-${shot}.png`) });

  const server = { name: `context7-${shot}`, transport: { type: "stdio", command: "npx", args: ["-y", "@upstash/context7-mcp"], env: ["CONTEXT7_API_KEY"] } };
  const [asked, refused] = await acts(agent.id, [
    { action: "save_mcp_server", args: { server }, request_id: `c7-${shot}` },
    { action: "save_mcp_server", args: { server, secrets: { env: { CONTEXT7_API_KEY: "agent-sweep-guess-4" } } } },
  ]);
  const pending = JSON.parse(asked);
  check(pending.outcome === "pending", `${shot}: the agent's add waits for the user: ${asked}`);
  check(refused === "an agent never sends a password, key or token: secrets. The user enters it in the app.", `${shot}: an agent's secret is refused: ${refused}`);

  await view.evaluate(async () => {
    if (settingsOpen()) await toggleSettings();
    ui.waitingOpen = true;
    renderSidebar();
  });
  if (shot === "phone") await view.evaluate(() => toggleSidebar(true));
  const item = `#thread-list .agent-confirm[data-confirmation="${pending.id}"]`;
  await waitFor(`${shot}: the request in Waiting on you`, () => view.isVisible(item));
  check((await view.textContent(`${item} .title`)).trim() === `Add MCP server ${server.name}`, `${shot}: the item says what it adds`);
  await view.click(`${item} [data-agent=approve]`);
  await waitFor(`${shot}: the approval form`, () => view.evaluate(() => document.getElementById("agent-asking")?.textContent.includes("Entering its secrets here approves it.")));
  const form = await view.evaluate(() => {
    const f = document.getElementById("mcp-form");
    return { name: f.querySelector("[name=name]").value, locked: f.querySelector("[name=name]").readOnly && f.querySelector("[name=type]").disabled, secret: !!f.querySelector('[data-mcp-secret="CONTEXT7_API_KEY"]') };
  });
  check(form.name === server.name && form.locked && form.secret, `${shot}: the form holds the agent's server, locked, and a field for its secret: ${JSON.stringify(form)}`);
  if (shot === "phone") {
    const wide = await view.evaluate(() => document.querySelector("#settings-page").scrollWidth - document.querySelector("#settings-page").clientWidth);
    check(wide <= 0, `${shot}: the form fits sideways (${wide}px)`);
    await view.evaluate(() => document.getElementById("agent-asking").scrollIntoView({ block: "start" }));
  }
  await view.screenshot({ path: path.join(SCREENS, `mcp-form-${shot}.png`) });

  await view.fill('#mcp-form [data-mcp-secret="CONTEXT7_API_KEY"]', "c7-sweep-key-3");
  await view.click("#mcp-form button.primary");
  const failed = () => view.evaluate(() => document.getElementById("mcp-form-error")?.textContent ?? "");
  await waitFor(`${shot}: the added server in the list`, async () => {
    const error = await failed();
    if (error) throw new Error(`${shot}: the approval failed: ${error}`);
    return (await pageText(view)).includes(server.name) && !(await view.isVisible("#mcp-form"));
  });
  const decided = (await view.evaluate(() => hub.hub({ cmd: "confirmations" }))).find((c) => c.id === pending.id);
  check(decided?.outcome === "done", `${shot}: the approval ran the agent's add with the typed secret: ${decided?.outcome}`);
  return pending;
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

  for (const call of [
    { server: { name: "ctx", transport: { type: "stdio", command: "npx", args: ["-y", "@acme/ctx-mcp"], env: ["API_KEY"] } }, secrets: { env: { API_KEY: "ctx-sweep-key-1" } } },
    { server: { name: "linear", transport: { type: "http", url: "https://mcp.linear.app/mcp" } } },
    { server: { name: "docs", project: "github.com/acme/site", transport: { type: "sse", url: "https://docs.example.com/sse", headers: ["X-Api-Key"] } }, secrets: { headers: { "X-Api-Key": "docs-sweep-key-2" } } },
  ]) {
    const saved = await page.evaluate((c) => hub.hub({ cmd: "save_mcp_server", ...c }), call);
    check(holdsNoSecret(JSON.stringify(saved)), `the app's save of ${call.server.name} answers no secret`);
  }
  const docs = (await page.evaluate(() => hub.hub({ cmd: "list_mcp_servers" }))).find((e) => e.server.name === "docs");
  await page.evaluate((id) => hub.hub({ cmd: "enable_mcp_server", id, enabled: false }), docs.id);

  const agent = await page.evaluate(async ({ machine, work }) => {
    const tab = await revealTab(machine, await callOn(machine, { cmd: "open_tab", path: work, bypass: false }));
    activate(tab);
    await callOn(machine, { cmd: "start_claude", tab: tab.view.id, cols: 120, rows: 32 });
    return { id: tab.view.id, key: tab.key };
  }, { machine: node, work });
  await waitFor("the agent's session", () => page.evaluate((k) => tabByKey(k)?.view.running, agent.key));
  await page.evaluate(({ machine, tab }) => callOn(machine, { cmd: "prompt", tab, text: "Add the Context7 MCP server for every session." }), { machine: node, tab: agent.id });
  await waitFor("the agent's first turn to end", () => page.evaluate((k) => tabByKey(k)?.view.status === "done", agent.key));

  const desk = await run(page, agent, "desktop");
  await sideBySide(browser, path.join(SCREENS, "mcp-page-desktop.png"), path.join(SCREENS, "mcp-form-desktop.png"), path.join(SCREENS, "mcp-desktop.png"), [1440, 900]);

  const phone = await contextOf({ width: 390, height: 844 }, true);
  await signIn(phone);
  await run(phone, agent, "phone");
  await sideBySide(browser, path.join(SCREENS, "mcp-page-phone.png"), path.join(SCREENS, "mcp-form-phone.png"), path.join(SCREENS, "mcp-phone.png"), [390, 844]);

  const trail = JSON.stringify(await page.evaluate(() => hub.hub({ cmd: "agent_audit", limit: 200 })));
  check(trail.includes("save_mcp_server") && holdsNoSecret(trail), "the audit trail holds the acts and none of the secrets");
  const listed = JSON.stringify(await page.evaluate(() => hub.hub({ cmd: "list_mcp_servers" })));
  check(listed.includes(`"context7-desktop"`) && holdsNoSecret(listed), "the list holds the approved server and no secret");
  await page.evaluate((k) => activate(tabByKey(k)), agent.key);
  const told = await waitFor("the add's notice in the agent's thread", async () => {
    const text = await page.evaluate(() => document.querySelector("#thread")?.innerText ?? "");
    return text.includes(`The user approved your save_mcp_server request (${desk.id}): done.`) && text;
  }, 45000);
  check(holdsNoSecret(told), "the agent's notice holds no secret");
  check(errors.length === 0, `no page errors: ${errors.join(" | ")}`);
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
