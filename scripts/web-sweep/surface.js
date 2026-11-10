#!/usr/bin/env node
// Drives the agent capability surface (CONTRACTS.md 13) on the real page. An agent session lists its
// actions and acts through `pinned-server mcp` as its claude runs it: a read, a write, and memory of
// another project. It asks for two confirm actions, which wait in Waiting on you. The user approves
// them there: the removal runs and the agent's thread gets the notice, and only the user sees the
// invite's code. Settings › Agent activity lists every act. At 1440x900 and 390x844.
//
//   SWEEP_SIGNUP=invite SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/surface.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes surface-*.png into scripts/web-sweep/screens/ and the agent's MCP answers into
// $SWEEP_DIR/surface-mcp.json.
const fs = require("fs");
const net = require("net");
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

/** The agent's tool calls, through `pinned-server mcp` as the session's claude runs it. */
function mcp(sock, tab, calls) {
  return new Promise((resolve, reject) => {
    const { PINNED_TASK, PINNED_ACCOUNT, PINNED_ROLE, CLAUDE_CONFIG_DIR, ...env } = process.env;
    const child = spawn(SERVER, ["mcp"], { env: { ...env, PINNED_SOCK: sock, PINNED_TAB: tab } });
    const requests = [
      { jsonrpc: "2.0", id: 0, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "sweep", version: "1" } } },
      { jsonrpc: "2.0", id: 1, method: "tools/list" },
      ...calls.map(([name, args], i) => ({ jsonrpc: "2.0", id: i + 2, method: "tools/call", params: { name, arguments: args } })),
    ];
    const answers = new Map();
    let buffered = "";
    child.stdout.on("data", (chunk) => {
      buffered += chunk;
      for (let at; (at = buffered.indexOf("\n")) >= 0; ) {
        const message = JSON.parse(buffered.slice(0, at));
        buffered = buffered.slice(at + 1);
        answers.set(message.id, message);
        if (answers.size === requests.length) {
          child.stdin.end();
          resolve(requests.map((r) => answers.get(r.id)));
        }
      }
    });
    child.on("error", reject);
    // One call at a time, in order, so the audit trail reads as the agent acted.
    let sent = 0;
    const next = () => {
      if (sent >= requests.length) return;
      child.stdin.write(`${JSON.stringify(requests[sent++])}\n`);
    };
    const pace = setInterval(() => {
      if (answers.size >= sent) next();
      if (sent >= requests.length) clearInterval(pace);
    }, 50);
    setTimeout(() => reject(new Error("the MCP server did not answer every call")), 120000);
  });
}
const text = (answer) => answer.result.content[0].text;
const parsed = (answer) => JSON.parse(text(answer));

async function post(route, body, token) {
  const res = await fetch(`${HUB}${route}`, { method: "POST", headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) }, body: JSON.stringify(body) });
  if (!res.ok) throw new Error(`${route}: ${res.status} ${await res.text()}`);
  return res.json();
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

(async () => {
  const { token } = await post("/api/login", { email: EMAIL, password: PASSWORD });
  const spare = (await post("/api/machines", { name: "old-imac", os: "macos" }, token)).machine;
  const alpha = fs.realpathSync(path.join(WORK, "alpha"));
  const beta = fs.realpathSync(path.join(WORK, "beta"));
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

  // The agent: a session in alpha that already ran a turn, and beta opened as another project.
  const agent = await page.evaluate(async ({ machine, alpha, beta }) => {
    await callOn(machine, { cmd: "open_tab", path: beta, bypass: false });
    const tab = await revealTab(machine, await callOn(machine, { cmd: "open_tab", path: alpha, bypass: false }));
    activate(tab);
    await callOn(machine, { cmd: "start_claude", tab: tab.view.id, cols: 120, rows: 32 });
    return { id: tab.view.id, key: tab.key };
  }, { machine: node, alpha, beta });
  await waitFor("the agent's session", () => page.evaluate((k) => tabByKey(k)?.view.running, agent.key));
  await page.evaluate(({ machine, tab }) => callOn(machine, { cmd: "prompt", tab, text: "Tidy the account's machines and invite Bea." }), { machine: node, tab: agent.id });
  await waitFor("the agent's first turn to end", () => page.evaluate((k) => tabByKey(k)?.view.status === "done", agent.key));

  const answers = await mcp(nodeSock(), agent.id, [
    ["actions", { search: "machine" }],
    ["act", { action: "machines" }],
    ["act", { action: "rename_machine", args: { id: spare.id, name: "Old iMac" } }],
    ["act", { action: "memory_save", args: { path: beta, title: "Releases", text: "Beta ships from tags." } }],
    ["act", { action: "write_input", args: { tab: agent.id, data: "y" } }],
    ["act", { action: "remove_machine", args: { id: spare.id }, request_id: "sweep-remove" }],
    ["act", { action: "create_invite", request_id: "sweep-invite" }],
  ]);
  fs.writeFileSync(path.join(DIR, "surface-mcp.json"), JSON.stringify(answers, null, 2));
  const offered = answers[1].result.tools.map((t) => t.name);
  check(offered.includes("actions") && offered.includes("act"), `the session is offered actions and act: ${offered.join(", ")}`);
  const listed = parsed(answers[2]).actions.map((a) => `${a.name}:${a.class}`);
  check(["machines:read", "rename_machine:write", "remove_machine:confirm"].every((a) => listed.includes(a)), `actions lists the machine actions by class: ${listed.join(", ")}`);
  const read = parsed(answers[3]);
  check(read.outcome === "done" && read.result.map((m) => m.name).sort().join() === "old-imac,sweep-node", `the read answers the account's machines: ${text(answers[3])}`);
  check(text(answers[4]) === '{"outcome":"done","result":null}', `the write renames the machine: ${text(answers[4])}`);
  const memory = parsed(answers[5]);
  check(memory.outcome === "done" && memory.result.title === "Releases", `the memory went to beta, another project: ${text(answers[5])}`);
  check(answers[6].result.isError && text(answers[6]) === "write_input is not open to agents: the app's terminal plumbing", `an excluded action is refused: ${text(answers[6])}`);
  const remove = parsed(answers[7]);
  const invite = parsed(answers[8]);
  check(remove.outcome === "pending" && invite.outcome === "pending", `both confirm actions wait: ${text(answers[7])} ${text(answers[8])}`);
  check(await page.evaluate(async () => (await hub.hub({ cmd: "machines" })).some((m) => m.name === "Old iMac")), "the app sees the agent's rename, and the removal has not run");

  await openWaiting(page);
  await waitFor("both confirmations in Waiting on you", async () => (await asksShown(page)).length === 2);
  check((await asksShown(page)).join(" | ") === "Remove machine Old iMac | Create invite", `the items name the action: ${(await asksShown(page)).join(" | ")}`);
  const args = await page.evaluate((id) => document.querySelector(`#thread-list .agent-confirm[data-confirmation="${id}"] .args`).textContent, remove.id);
  check(JSON.parse(args).id === spare.id, `the item shows the exact arguments: ${args}`);
  const [count, threads] = await page.evaluate(() => [document.querySelector("#thread-list .waiting-head .count")?.textContent, document.querySelectorAll("#thread-list .waiting-row").length]);
  check(count === String(2 + threads), `Waiting on you counts them with the ${threads} waiting threads: ${count}`);
  await page.screenshot({ path: path.join(SCREENS, "surface-waiting-desktop.png") });

  const phone = await contextOf({ width: 390, height: 844 }, true);
  await signIn(phone);
  await phone.evaluate(() => toggleSidebar(true));
  await openWaiting(phone);
  await waitFor("the phone's confirmations", async () => (await asksShown(phone)).length === 2);
  const overflow = await phone.evaluate(() => {
    const item = document.querySelector("#thread-list .agent-confirm");
    return item.scrollWidth - item.clientWidth;
  });
  check(overflow <= 0, `a confirmation fits the phone's drawer (${overflow}px)`);
  await phone.screenshot({ path: path.join(SCREENS, "surface-waiting-phone.png") });

  await page.click(`#thread-list .agent-confirm[data-confirmation="${remove.id}"] [data-agent=approve]`);
  await waitFor("the removal to leave Waiting on you", async () => (await asksShown(page)).length === 1);
  check(await page.evaluate(async () => !(await hub.hub({ cmd: "machines" })).some((m) => m.name === "Old iMac")), "approving ran the removal");
  const told = `[pinned] The user approved your remove_machine request (${remove.id}): done.`;
  await page.evaluate((k) => activate(tabByKey(k)), agent.key);
  await waitFor("the notice in the agent's thread", () => page.evaluate((t) => document.querySelector("#thread")?.innerText.includes(t), told), 45000);
  check(true, "the agent's session got the notice through its queue");
  await page.screenshot({ path: path.join(SCREENS, "surface-notice-desktop.png") });

  await page.click(`#thread-list .agent-confirm[data-confirmation="${invite.id}"] [data-agent=approve]`);
  const code = await waitFor("the invite's code for the user", () => page.evaluate(() => document.querySelector("#thread-list .agent-once code")?.textContent));
  check(/^[A-Z0-9]{4}(-[A-Z0-9]{4}){3}$/.test(code), `the user sees the invite code: ${code}`);
  const invited = `[pinned] The user approved your create_invite request (${invite.id}): done.`;
  await waitFor("the invite's notice in the agent's thread", () => page.evaluate((t) => document.querySelector("#thread")?.innerText.includes(t), invited), 45000);
  const thread = await page.evaluate(() => document.querySelector("#thread").innerText);
  check(!thread.includes(code), "the agent's notice holds no invite code");
  await page.screenshot({ path: path.join(SCREENS, "surface-invite-desktop.png") });

  for (const [shot, view] of [["desktop", page], ["phone", phone]]) {
    await view.evaluate(() => {
      openSettings();
      showPage("agent-activity");
    });
    await waitFor("the audit trail", () => view.evaluate(() => document.querySelectorAll(".agent-audit-row").length >= 10));
    const rows = await view.evaluate(() => [...document.querySelectorAll(".agent-audit-row")].map((r) => [r.querySelector(".head b").textContent, r.querySelector(".agent-outcome").textContent]));
    console.log(`${shot} trail:`, JSON.stringify(rows));
    const expected = [["Create invite", "done"], ["Create invite", "approved"], ["Remove machine", "done"], ["Remove machine", "approved"], ["Create invite", "pending"], ["Remove machine", "pending"], ["Write input", "refused"], ["Memory save", "done"], ["Rename machine", "done"], ["Machines", "done"]];
    check(rows.length === expected.length && expected.every(([title, outcome], i) => rows[i][0].startsWith(title) && rows[i][1] === outcome), `${shot}: the trail lists every act, newest first`);
    if (shot === "phone") {
      const wide = await view.evaluate(() => document.querySelector("#settings-page").scrollWidth - document.querySelector("#settings-page").clientWidth);
      check(wide <= 0, `nothing overflows sideways on the phone (${wide}px)`);
    }
    await view.screenshot({ path: path.join(SCREENS, `surface-activity-${shot}.png`), fullPage: shot === "desktop" });
  }
  check(errors.length === 0, `no page errors: ${errors.join(" | ")}`);
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
