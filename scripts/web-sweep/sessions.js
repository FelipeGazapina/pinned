#!/usr/bin/env node
// Seeds the case behind the global orchestrator's sessions tool, then calls sessions, read_session
// and waiting through the session MCP server the orchestrator's claude runs, and shoots the
// sidebar's Waiting on you on desktop and phone.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   # a second node named sweep-mini on the same hub, HOME=$SWEEP_DIR/home2, config $SWEEP_DIR/node2
//   SWEEP_DIR=... PLAYWRIGHT=... node scripts/web-sweep/sessions.js
//
// sweep-node hosts the orchestrator and has a client-apps checkout whose project runs under a Bloom
// account: three "Someone mentioned you in the Slack thread" threads and two new threads. sweep-mini
// has a client-apps checkout on the default account with a five-day-old Slack thread whose
// transcript is past 8 MiB. Writes the MCP answers to $SWEEP_DIR/sessions-mcp.json and
// sessions-{desktop,phone}.png into scripts/web-sweep/screens/.
const fs = require("fs");
const net = require("net");
const os = require("os");
const path = require("path");
const { execFileSync, spawn } = require("child_process");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const SERVER = path.join(__dirname, "../../target/debug/pinned-server");
const SCREENS = path.join(__dirname, "screens");
const SLACK = "Someone mentioned you in the Slack thread #client-apps-ops";
const BRANCH = "felipega/fix-login-and-ops";
const HOUR = 3600;

const check = (ok, what) => {
  if (!ok) throw new Error(what);
};
const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const found = await fn();
    if (found) return found;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`timed out waiting for ${what}`);
};
const iso = (unix) => new Date(unix * 1000).toISOString();
const now = Math.floor(Date.now() / 1000);

/** The socket of the node whose pid `$DIR/<name>.pid` holds. */
function sockOf(name) {
  const pid = fs.readFileSync(path.join(DIR, `${name}.pid`), "utf8").trim();
  // A pid can repeat, so an older folder may carry the same prefix: the newest is this node's.
  const dir = fs
    .readdirSync(os.tmpdir())
    .filter((d) => d.startsWith(`pinned-${pid}-`))
    .sort((a, b) => fs.statSync(path.join(os.tmpdir(), b)).mtimeMs - fs.statSync(path.join(os.tmpdir(), a)).mtimeMs)[0];
  check(dir, `no socket folder for ${name} (${pid})`);
  return path.join(os.tmpdir(), dir, "pinned.sock");
}

/** One line to the node's socket, as claude's hooks send it, and its answer. */
const tell = (sock, message) =>
  new Promise((resolve, reject) => {
    const c = net.createConnection(sock, () => c.write(`${JSON.stringify(message)}\n`));
    let reply = "";
    c.on("data", (d) => {
      reply += d;
      if (reply.includes("\n")) {
        c.destroy();
        resolve(reply.trim());
      }
    });
    c.on("error", reject);
  });
const hook = (sock, event) => {
  console.log("hook", JSON.stringify(event));
  return tell(sock, { kind: "agent", ...event });
};

/** A transcript as claude writes it: `turns` is [kind, text, unix time] with kind user, assistant or pr. */
function transcript(file, turns, padding = 0) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const line = (kind, content, at, i) => JSON.stringify({ parentUuid: null, isSidechain: false, cwd: "/", sessionId: path.basename(file, ".jsonl"), gitBranch: BRANCH, type: kind, message: { role: kind, content }, uuid: `${kind}-${i}`, timestamp: iso(at) });
  const lines = [];
  for (let i = 0; i < padding; i++) lines.push(line("assistant", [{ type: "text", text: "x".repeat(1 << 20) }], turns[0][2] - 600 + i, `pad-${i}`));
  turns.forEach(([kind, text, at], i) => {
    if (kind === "pr") lines.push(JSON.stringify({ type: "pr-link", sessionId: "s", prNumber: text, prUrl: `https://github.com/example/client-apps/pull/${text}`, prRepository: "example/client-apps", timestamp: iso(at) }));
    else lines.push(line(kind, kind === "user" ? text : [{ type: "text", text }], at, i));
  });
  fs.writeFileSync(file, `${lines.join("\n")}\n`);
}

/** The orchestrator's tool calls, through `pinned-server mcp` as its claude runs it. */
function mcp(sock, tab, calls) {
  return new Promise((resolve, reject) => {
    const { PINNED_TASK, PINNED_ACCOUNT, CLAUDE_CONFIG_DIR, ...env } = process.env;
    const child = spawn(SERVER, ["mcp"], { env: { ...env, PINNED_SOCK: sock, PINNED_TAB: tab, PINNED_ROLE: "global-orchestrator" } });
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
    for (const r of requests) child.stdin.write(`${JSON.stringify(r)}\n`);
    setTimeout(() => reject(new Error("the MCP server did not answer every call")), 90000);
  });
}

(async () => {
  // CHROME_BIN runs an installed Chrome when playwright's own browser is not downloaded.
  const browser = await playwright.chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : {}) });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(HUB);
  await page.fill("#login [name=email]", "sweep@example.test");
  await page.fill("#login [name=password]", "sweep password 1");
  await page.click("#login .primary");
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  const ids = await waitFor("sweep-node and sweep-mini", () =>
    page.evaluate(() => {
      const byName = Object.fromEntries([...machines.entries()].filter(([, m]) => m.view.online).map(([id, m]) => [m.view.name, id]));
      return byName["sweep-node"] && byName["sweep-mini"] ? byName : null;
    }));
  const [dev, mini] = [ids["sweep-node"], ids["sweep-mini"]];
  const call = (machine, c) => {
    console.log("call", machine === dev ? "sweep-node" : "sweep-mini", JSON.stringify(c));
    const late = new Promise((_, reject) => setTimeout(() => reject(new Error(`no answer to ${JSON.stringify(c)}`)), 20000));
    return Promise.race([page.evaluate(([m, c]) => callOn(m, c), [machine, c]), late]);
  };
  const [devSock, miniSock] = [sockOf("node"), sockOf("node2")];

  // sweep-node: client-apps under Bloom, as Felipe's dev machine has it.
  const devApps = path.join(DIR, "home/work/client-apps");
  execFileSync("git", ["init", "-q", devApps]);
  execFileSync("git", ["-C", devApps, "remote", "add", "origin", "https://github.com/example/client-apps.git"]);
  await call(dev, { cmd: "add_account", provider: "claude", name: "Bloom" });
  fs.writeFileSync(path.join(DIR, "node/accounts/bloom/signed-in"), "");
  await call(dev, { cmd: "close_tab", tab: await call(dev, { cmd: "open_tab", path: devApps, bypass: false }) });
  await call(dev, { cmd: "set_project_account", path: devApps, account: "bloom" });
  const running = async (machine, folder) => {
    const tab = await call(machine, { cmd: "open_tab", path: folder, bypass: false });
    await call(machine, { cmd: "start_claude", tab, cols: 100, rows: 30 });
    return tab;
  };
  const bloomProjects = path.join(DIR, "node/accounts/bloom/projects/client-apps");
  const asked = await running(dev, devApps);
  transcript(path.join(bloomProjects, "s-asked.jsonl"), [
    ["user", "Someone mentioned you in the Slack thread #client-apps-ops: the login page loops on Safari.", now - 1 * HOUR - 300],
    ["assistant", "Found it: the session cookie is SameSite=None without Secure on staging, so Safari drops it and the login redirects forever. The fix is one line in `auth/cookies.ts`. Should I open the PR against `main` now?", now - 1 * HOUR],
  ]);
  await hook(devSock, { status: "working", tab: asked, session: "s-asked", title: SLACK });
  await hook(devSock, { status: "done", tab: asked, session: "s-asked" });
  const permission = await running(dev, devApps);
  transcript(path.join(bloomProjects, "s-permission.jsonl"), [["user", "Someone mentioned you in the Slack thread #client-apps-ops: run the ops migration on staging.", now - 1.5 * HOUR]]);
  await hook(devSock, { status: "working", tab: permission, session: "s-permission", title: SLACK });
  await hook(devSock, { status: "needs-input", tab: permission, message: "Claude needs your permission to use Bash" });
  const working = await running(dev, devApps);
  transcript(path.join(bloomProjects, "s-working.jsonl"), [
    ["user", "Someone mentioned you in the Slack thread #client-apps-ops: the ops dashboard 500s.", now - 2 * HOUR],
    ["pr", 418, now - 2 * HOUR + 600],
    ["assistant", "PR #418 fixes the 500. Now running the e2e suite.", now - 2 * HOUR + 900],
  ]);
  // No hook names it, as a tab restored after a restart has lost its title: its first prompt stands in.
  await hook(devSock, { status: "working", tab: working, session: "s-working" });
  const fresh = [await running(dev, devApps), await running(dev, devApps)];

  // sweep-mini: the same repo on the default account, with a thread five days old past 8 MiB.
  const miniApps = path.join(DIR, "home2/work/client-apps");
  execFileSync("git", ["init", "-q", miniApps]);
  execFileSync("git", ["-C", miniApps, "remote", "add", "origin", "https://github.com/example/client-apps.git"]);
  const old = await running(mini, miniApps);
  transcript(
    path.join(DIR, "home2/.claude/projects/client-apps/s-old.jsonl"),
    [
      ["user", "Someone mentioned you in the Slack thread #client-apps-ops: is the Android build green?", now - 5 * 24 * HOUR],
      ["assistant", "The Android build is green on main; the flaky screenshot test is quarantined in #402.", now - 5 * 24 * HOUR + 120],
    ],
    10,
  );
  await hook(miniSock, { status: "done", tab: old, session: "s-old" });

  await call(dev, { cmd: "set_global_orchestrator_settings", settings: { host: true, name: "Orchestrator" } });
  const global = await call(dev, { cmd: "open_global_orchestrator" });
  const settled = async () => {
    const [, , listed] = await mcp(devSock, global, [["sessions", { project: "client-apps" }]]);
    const sessions = JSON.parse(listed.result.content[0].text).sessions;
    return sessions.length === 6 && sessions.some((s) => s.id === working && s.pr) && sessions.some((s) => s.id === old && s.branch) ? sessions : null;
  };
  await waitFor("every seeded session on the hub with its transcript read", settled, 30000);
  const answers = await mcp(devSock, global, [
    ["sessions", { project: "client-apps" }],
    ["read_session", { id: old, items: 3 }],
    ["read_session", { id: asked, items: 2 }],
    ["waiting", {}],
  ]);
  const tools = answers[1].result.tools.map((t) => t.name);
  check(tools.includes("sessions") && tools.includes("read_session"), `the orchestrator is offered sessions and read_session: ${tools}`);
  const [sessions, oldRead, askedRead, waiting] = answers.slice(2).map((a) => JSON.parse(a.result.content[0].text));
  const out = { tools, sessions, read_session_old: oldRead, read_session_asked: askedRead, waiting, ids: { asked, permission, working, fresh, old, global, dev, mini } };
  fs.writeFileSync(path.join(DIR, "sessions-mcp.json"), JSON.stringify(out, null, 2));
  console.log(JSON.stringify(out, null, 2));
  check(sessions.sessions.length === 6, "six sessions in client-apps across both machines and accounts");
  check(oldRead.conversation.items.at(-1).text.startsWith("The Android build is green"), "read_session reaches the end of a transcript past 8 MiB");

  // Waiting on you, desktop then phone.
  await page.evaluate(() => setMode("gui"));
  await waitFor("Waiting on you", () => page.evaluate(() => !!document.querySelector(".waiting-head")));
  fs.mkdirSync(SCREENS, { recursive: true });
  await page.screenshot({ path: path.join(SCREENS, "sessions-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => toggleSidebar(true));
  await page.waitForTimeout(500);
  await page.screenshot({ path: path.join(SCREENS, "sessions-phone.png") });
  const rows = await page.evaluate(() => [...document.querySelectorAll(".waiting-row")].map((r) => r.textContent.trim()));
  console.log("waiting rows:", JSON.stringify(rows));
  check(!errors.length, `page errors: ${errors.join("; ")}`);
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
