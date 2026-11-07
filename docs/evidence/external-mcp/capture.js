// Captures the external MCP evidence against a local stack from scripts/web-sweep/stack.sh: a real
// hub, Postgres and paired node whose `claude` is scripts/web-sweep/fake-claude. This script plays
// the external MCP client itself (it is not Grok): it registers, sends the signed-in browser
// through /oauth/authorize, receives the code on a loopback callback, and calls /mcp.
//
//   SWEEP_DIR=… SWEEP_PG_PORT=… SWEEP_HUB_PORT=… scripts/web-sweep/stack.sh start
//   HUB=http://127.0.0.1:<hub port> SWEEP_DIR=… PLAYWRIGHT=… node docs/evidence/external-mcp/capture.js
//
// Writes PNGs and mcp-transcript.json (tokens redacted) next to this file.
const crypto = require("crypto");
const fs = require("fs");
const http = require("http");
const path = require("path");
const { chromium } = require(process.env.PLAYWRIGHT || "playwright");

const HUB = process.env.HUB || "http://127.0.0.1:8797";
const OUT = __dirname;
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const HOME = path.join(process.env.SWEEP_DIR, "home");
const transcript = [];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const SECRETS = ["access_token", "refresh_token", "code", "code_verifier"];
const redact = (value) => JSON.parse(JSON.stringify(value, (k, v) => (SECRETS.includes(k) && typeof v === "string" ? "[redacted]" : v)));

async function post(route, body, headers = {}) {
  const res = await fetch(`${HUB}${route}`, { method: "POST", headers: { "content-type": "application/json", ...headers }, body: typeof body === "string" ? body : JSON.stringify(body) });
  const text = await res.text();
  return { status: res.status, headers: Object.fromEntries(res.headers), body: text ? JSON.parse(text) : null };
}

/** The account's client socket, for setup the user would otherwise do by hand: opening two projects. */
async function clientSocket(token) {
  const ws = new WebSocket(`${HUB.replace("http", "ws")}/ws/client`);
  const waiting = new Map();
  let hello;
  const ready = new Promise((resolve) => {
    ws.onmessage = (e) => {
      const frame = JSON.parse(e.data);
      if (frame.type === "hello") (hello = frame), resolve();
      if (frame.type === "reply" && waiting.has(frame.id)) waiting.get(frame.id)(frame);
    };
  });
  ws.onopen = () => ws.send(JSON.stringify({ type: "auth", token }));
  await ready;
  let next = 1;
  const call = (machine, call) =>
    new Promise((resolve, reject) => {
      const id = next++;
      waiting.set(id, (f) => ("err" in f ? reject(new Error(f.err)) : resolve(f.ok)));
      ws.send(JSON.stringify({ type: "call", id, machine, call }));
    });
  return { ws, hello, call };
}

let accessToken = null;
let rpcId = 1;
async function mcp(method, params, note) {
  const res = await post("/mcp", { jsonrpc: "2.0", id: rpcId++, method, params }, accessToken ? { authorization: `Bearer ${accessToken}` } : {});
  if (note === null) return res;
  transcript.push(redact({ note, request: { method, params }, status: res.status, ...(res.headers["www-authenticate"] ? { www_authenticate: res.headers["www-authenticate"] } : {}), response: res.body }));
  return res;
}
const tool = async (name, args, note) => (await mcp("tools/call", { name, arguments: args }, note)).body.result;

async function main() {
  // The test client's loopback callback, as a desktop MCP client registers.
  let gotCallback;
  const callback = new Promise((resolve) => (gotCallback = resolve));
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    res.setHeader("content-type", "text/html; charset=utf-8");
    res.end(`<!doctype html><body style="font:16px system-ui;margin:3em;background:#111;color:#eee"><h2>Test MCP client callback (local evidence harness, not Grok)</h2><p>Received an authorization code for state <code>${url.searchParams.get("state")}</code>.</p><p>The test client now trades it for tokens at <code>/oauth/token</code> with its PKCE verifier.</p>`);
    gotCallback(url);
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const redirect = `http://127.0.0.1:${server.address().port}/callback`;

  const { token } = (await post("/api/login", { email: EMAIL, password: PASSWORD })).body;
  const socket = await clientSocket(token);
  const node = socket.hello.machines.find((m) => m.name === "sweep-node");
  for (const project of ["alpha", "beta"]) {
    const tab = await socket.call(node.id, { cmd: "open_tab", path: path.join(HOME, "work", project), bypass: false });
    await socket.call(node.id, { cmd: "close_tab", tab });
  }

  // Discovery and registration, as the client does before the user sees anything.
  accessToken = null;
  await mcp("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "evidence", version: "1" } }, "unauthenticated: 401 with resource metadata");
  const resource = await (await fetch(`${HUB}/.well-known/oauth-protected-resource/mcp`)).json();
  const as = await (await fetch(`${HUB}/.well-known/oauth-authorization-server`)).json();
  transcript.push({ note: "discovery", protected_resource: resource, authorization_server: as });
  const registered = await post("/oauth/register", { client_name: "Evidence test agent", redirect_uris: [redirect] });
  transcript.push(redact({ note: "dynamic client registration", status: registered.status, response: registered.body }));
  const clientId = registered.body.client_id;
  const verifier = crypto.randomBytes(48).toString("base64url");
  const challenge = crypto.createHash("sha256").update(verifier).digest("base64url");
  const authorize = new URL(`${HUB}/oauth/authorize`);
  for (const [k, v] of Object.entries({ response_type: "code", client_id: clientId, redirect_uri: redirect, code_challenge: challenge, code_challenge_method: "S256", state: "evidence-state", scope: "sessions orchestrator", resource: `${HUB}/mcp` })) authorize.searchParams.set(k, v);

  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, colorScheme: "dark" });
  const page = await context.newPage();
  await page.goto(HUB);
  await page.fill("#login [name=email]", EMAIL);
  await page.fill("#login [name=password]", PASSWORD);
  await page.click("#login .primary");
  await page.waitForFunction(() => typeof hub !== "undefined" && !!hub?.online, null, { timeout: 30000 });

  // The client sends the browser to /oauth/authorize; the hub opens the web app on the request.
  await page.goto(authorize.toString());
  await page.waitForSelector("#agent-consent [data-action=approve-agent]", { timeout: 30000 });
  await page.waitForSelector('#agent-consent input[name="agent-project"]', { timeout: 30000 });
  await sleep(500);
  await page.locator("#agent-consent").screenshot({ path: path.join(OUT, "01-consent-card.png") });
  await page.screenshot({ path: path.join(OUT, "02-consent-in-connectors.png") });

  await page.locator('#agent-consent input[name="agent-scope"][value="sessions"]').check();
  await page.locator('#agent-consent label.check', { hasText: "alpha" }).locator("input").check();
  await page.locator("#agent-consent").screenshot({ path: path.join(OUT, "03-consent-picked.png") });
  await page.click("#agent-consent [data-action=approve-agent]");
  const back = await callback;
  await page.waitForLoadState("load");
  await page.screenshot({ path: path.join(OUT, "04-test-client-callback.png") });
  const exchanged = await post("/oauth/token", new URLSearchParams({ grant_type: "authorization_code", client_id: clientId, code: back.searchParams.get("code"), code_verifier: verifier, redirect_uri: redirect, resource: `${HUB}/mcp` }).toString(), { "content-type": "application/x-www-form-urlencoded" });
  transcript.push(redact({ note: "token exchange (PKCE S256)", status: exchanged.status, response: exchanged.body, callback_params: Object.fromEntries([...back.searchParams].map(([k, v]) => [k, k === "code" ? "[redacted]" : v])) }));
  accessToken = exchanged.body.access_token;

  // MCP over the issued token.
  await mcp("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "evidence", version: "1" } }, "initialize");
  await mcp("tools/list", {}, "tools/list");
  const projects = await tool("list_projects", {}, "list_projects");
  const alpha = projects.structuredContent.projects.find((p) => p.name === "alpha");
  const started = await tool("start_session", { project: alpha.id, task: "Look at the README and summarize it.", title: "Summarize the README", request_id: "evidence-start-1" }, "start_session");
  const session = started.structuredContent.session_id;
  await tool("start_session", { project: alpha.id, task: "Look at the README and summarize it.", title: "Summarize the README", request_id: "evidence-start-1" }, "start_session retried with the same request_id: replayed, no second session");
  await tool("send_message", { session_id: session, message: "hello from the test agent", request_id: "evidence-msg-0" }, "send_message refused while the session has not reported it is at its prompt");
  // fake-claude ignores its argv task, which real Claude runs as the first turn. The harness types the
  // task once, as the user, so fake-claude runs that turn and fires its UserPromptSubmit and Stop hooks.
  await sleep(1500);
  const firstTab = (await socket.call(node.id, { cmd: "tabs" })).find((t) => t.connector === `mcp:${session}`);
  await socket.call(node.id, { cmd: "prompt", tab: firstTab.id, text: "Look at the README and summarize it." });
  for (let i = 0; i < 40 && (await tool("read_session", { session_id: session }, null)).structuredContent?.state !== "done"; i++) await sleep(250);
  await tool("send_message", { session_id: session, message: "hello from the test agent", request_id: "evidence-msg-1" }, "send_message once the session is idle");
  await sleep(2000);
  await tool("read_session", { session_id: session }, "read_session after the reply");
  await tool("message_orchestrator", { project: alpha.id, message: "What is planned for alpha?", request_id: "evidence-orch-1" }, "message_orchestrator: starts the orchestrator and types nothing yet");
  await tool("read_orchestrator", { project: alpha.id }, "read_orchestrator while it starts");
  // As for the session: the harness gives the fake orchestrator its first turn, which real Claude's idle notice stands for.
  await sleep(1500);
  const orchestratorTab = (await socket.call(node.id, { cmd: "tabs" })).find((t) => t.role === "orchestrator");
  await socket.call(node.id, { cmd: "prompt", tab: orchestratorTab.id, text: "Ready." });
  for (let i = 0; i < 40 && (await tool("read_orchestrator", { project: alpha.id }, null)).structuredContent?.state !== "done"; i++) await sleep(250);
  await tool("message_orchestrator", { project: alpha.id, message: "What is planned for alpha?", request_id: "evidence-orch-1" }, "message_orchestrator again with the same request_id once it is idle: delivered");
  await sleep(2000);
  await tool("read_orchestrator", { project: alpha.id }, "read_orchestrator");

  // The Connectors page lists the agent with a Revoke button.
  await page.goto(HUB);
  await page.waitForFunction(() => typeof hub !== "undefined" && !!hub?.online, null, { timeout: 30000 });
  await page.evaluate(() => {
    page = "integrations";
    save("settings-page", page);
    return toggleSettings();
  });
  await page.waitForSelector("#external-agents .agent-row", { timeout: 30000 });
  await page.locator("#external-agents").scrollIntoViewIfNeeded();
  await page.locator("#external-agents").screenshot({ path: path.join(OUT, "05-connectors-agent-listed.png") });

  // The session the agent started, as the user sees it in Pinned.
  await page.evaluate(() => toggleSettings());
  await page.getByText("Summarize the README").first().click({ timeout: 15000 }).catch(() => {});
  await sleep(2500);
  await page.screenshot({ path: path.join(OUT, "06-agent-session-in-pinned.png") });

  // The user makes the session ask for permission (fake claude's /ask); the agent sees it and can't answer it.
  const sessionTab = (await socket.call(node.id, { cmd: "tabs" })).find((t) => t.connector === `mcp:${session}`);
  await socket.call(node.id, { cmd: "prompt", tab: sessionTab.id, text: "/ask" });
  // Polled unrecorded until the hook lands, then read once for the transcript.
  for (let i = 0; i < 40 && (await tool("read_session", { session_id: session }, null)).structuredContent?.state !== "needs_permission"; i++) await sleep(250);
  await tool("read_session", { session_id: session }, "read_session while the session asks the user");
  await tool("send_message", { session_id: session, message: "yes, go ahead", request_id: "evidence-msg-2" }, "send_message refused while the user is asked for permission");
  await sleep(1500);
  await page.screenshot({ path: path.join(OUT, "07-permission-stays-with-the-user.png") });

  // The orchestrator thread, with the agent's message.
  await page.evaluate(() => activate(tabs.find((t) => t.view.role === "orchestrator")));
  await sleep(1500);
  await page.screenshot({ path: path.join(OUT, "08-orchestrator-message.png") });

  // Revoking from the Connectors page stops the token at once.
  await page.evaluate(() => {
    page = "integrations";
    save("settings-page", page);
    return toggleSettings();
  });
  await page.waitForSelector("#external-agents [data-action=revoke-agent]");
  await page.click("#external-agents [data-action=revoke-agent]");
  await page.waitForFunction(() => !document.querySelector("#external-agents .agent-row"), null, { timeout: 15000 });
  await page.locator("#external-agents").screenshot({ path: path.join(OUT, "09-connectors-after-revoke.png") });
  await mcp("tools/list", {}, "tools/list after revoke: 401");

  fs.writeFileSync(path.join(OUT, "mcp-transcript.json"), JSON.stringify(transcript, null, 2) + "\n");
  await browser.close();
  server.close();
  socket.ws.close();
}

main().catch((e) => {
  fs.writeFileSync(path.join(OUT, "mcp-transcript.partial.json"), JSON.stringify(transcript, null, 2) + "\n");
  console.error(e);
  process.exit(1);
});
