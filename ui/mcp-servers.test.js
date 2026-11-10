import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
// Values built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => JSON.parse(JSON.stringify(value));

const ctx = { name: "ctx", transport: { type: "stdio", command: "npx", args: ["-y", "@acme/ctx"], env: ["API_KEY"] } };
const linear = { name: "linear", transport: { type: "http", url: "https://mcp.linear.app/mcp" } };
const asked = { id: "c-7", at: 1794249424, machine: "this-mac", machine_name: "MacBook", tab: "tab-4", title: "Wire up the docs", role: "worker", about: "", request_id: "r-1" };

function element(id) {
  return { id, value: "", innerHTML: "", textContent: "", hidden: false, dataset: {}, focused: false, addEventListener() {}, classList: { contains: () => false, add() {}, remove() {}, toggle() {} }, focus() { this.focused = true; }, remove() {}, querySelector: () => null, querySelectorAll: () => [] };
}

/** The MCP form as the page drew it, holding `fields` by name and a secret input for each of `typed`. */
function mcpForm(fields, typed) {
  const form = element("mcp-form");
  const inputs = Object.entries(typed).map(([name, value]) => Object.assign(element(), { value, dataset: { mcpSecret: name } }));
  form.querySelector = (selector) => {
    const named = selector.match(/^\[name=(\w+)\]$/);
    if (named) return named[1] in fields ? Object.assign(element(), { value: fields[named[1]] }) : null;
    return selector === "[data-mcp-secret]" ? inputs[0] ?? null : null;
  };
  form.querySelectorAll = (selector) => (selector === "[data-mcp-secret]" ? inputs : []);
  return form;
}

/** settings.js, agent-surface.js and mcp-servers.js as the page runs them, with the hub played by
 * `answer` and the machines' calls by `onMachine`. Settings is open. */
function app({ answer = async () => [], onMachine = async () => [] } = {}) {
  const calls = [];
  const machineCalls = [];
  const dom = {};
  const byId = (id) => (dom[id] ??= element(id));
  const context = {
    console,
    addEventListener() {},
    load: () => null,
    save() {},
    listen() {},
    LOCAL: "local",
    account: { machine_id: "this-mac", email: "ana@acme.dev" },
    machines: new Map([["local", { view: { online: true, name: "MacBook" }, hubView: { name: "MacBook" }, accounts: [{ id: "claude", provider: "claude", name: "Claude" }] }]]),
    hub: { hub: (call) => (calls.push(call), answer(call)) },
    callOn: async (machine, call) => (machineCalls.push({ machine, call }), onMachine(call)),
    accountsOf: async (machine) => context.machines.get(machine).accounts,
    picker: {},
    active: null,
    icon: () => "",
    guiOn: () => false,
    renderSidebar() {},
    render() {},
    tell() {},
    ago: () => "5m",
    inviteDate: () => "Nov 16",
    clockTime: () => "14:30",
    machineName: (id) => ({ local: "MacBook" })[id] ?? "machine",
    agentConsentHtml: () => "",
    externalAgentsCardHtml: () => "",
    loadExternalAgents() {},
    document: { body: { dataset: { page: "settings" } }, getElementById: byId, querySelector: () => element(), createElement: () => element() },
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(
    [
      read("settings.js"),
      `const onlineMachines = () => [...machines.entries()].filter(([, m]) => m.view.online).map(([id]) => id);`,
      fn(read("gui-shell.js"), "openSettings"),
      read("agent-surface.js"),
      read("mcp-servers.js"),
    ].join("\n"),
    context,
  );
  const state = (name) => vm.runInContext(name, context);
  return { context, calls, machineCalls, dom, state, actions: state("actions") };
}

function approve(context, confirmation) {
  const row = { dataset: { confirmation } };
  const button = { dataset: { agent: "approve" }, disabled: false, closest: (selector) => (selector === "[data-agent]" ? button : row) };
  context.agentSurfaceClick({ target: button });
  return flush();
}

test("a server's card shows what it runs, where it loads, and that each secret is set, never a value", () => {
  const { context } = app();
  const card = context.mcpRowHtml({ id: "mcp-1", enabled: true, server: { ...ctx, project: "github.com/acme/site" } });
  assert.match(card, /<b>ctx<\/b>\s*<div class="hint">Program \(stdio\) · github\.com\/acme\/site<\/div>/);
  assert.match(card, /<code class="mcp-reach">npx -y @acme\/ctx<\/code>/);
  assert.match(card, /<span class="badge" data-on="true">API_KEY · set<\/span>/);
  assert.match(card, /<input type="checkbox" data-mcp-switch aria-label="Load ctx in new sessions" checked>/);
  assert.doesNotMatch(card, /OAuth/, "a program never signs in with OAuth");
  const remote = context.mcpRowHtml({ id: "mcp-2", enabled: false, server: linear });
  assert.match(remote, /Every project/);
  assert.match(remote, /If it signs in with OAuth, log in once per Claude account: run \/mcp in a session of each account\./);
  assert.doesNotMatch(remote, / checked>/);
  const keyed = context.mcpRowHtml({ id: "mcp-3", enabled: true, server: { ...linear, transport: { ...linear.transport, headers: ["X-Api-Key"] } } });
  assert.doesNotMatch(keyed, /OAuth/, "a server that sends a header of its own signs in with it");
});

test("the form's fields become the server, and only the secrets typed go with it", () => {
  const { context } = app();
  const server = context.mcpServerOf({ name: " ctx ", project: "", type: "stdio", command: "npx ", args: "-y\n @acme/ctx \n", url: "", names: "API_KEY, REGION" });
  assert.deepEqual(here(server), { name: "ctx", transport: { type: "stdio", command: "npx", args: ["-y", "@acme/ctx"], env: ["API_KEY", "REGION"] } });
  const secrets = context.mcpSecretsOf(server, { API_KEY: "k-1", REGION: "  ", OTHER: "x" });
  assert.deepEqual(here(secrets), { env: { API_KEY: "k-1" } }, "a blank value keeps the stored one, and a name the server lacks goes nowhere");
  assert.deepEqual(here(context.mcpCallOf({ id: "mcp-1" }, server, secrets)), { cmd: "save_mcp_server", id: "mcp-1", server: here(server), secrets: { env: { API_KEY: "k-1" } } });
  const remote = context.mcpServerOf({ name: "lin", project: "github.com/acme/site", type: "http", command: "", args: "", url: " https://mcp.linear.app/mcp", names: "Authorization" });
  assert.deepEqual(here(remote), { name: "lin", project: "github.com/acme/site", transport: { type: "http", url: "https://mcp.linear.app/mcp", headers: ["Authorization"] } });
  assert.deepEqual(here(context.mcpCallOf({}, remote, {})), { cmd: "save_mcp_server", server: here(remote) });
});

test("each MCP action reads as what it does in Waiting on you and the trail", () => {
  const { context } = app();
  vm.runInContext(`mcpUi.list = [{ id: "mcp-1", enabled: true, server: { name: "ctx", transport: { type: "stdio", command: "npx" } } }]`, context);
  assert.equal(context.agentActionTitle("save_mcp_server", { server: ctx }), "Add MCP server ctx");
  assert.equal(context.agentActionTitle("save_mcp_server", { id: "mcp-1", server: ctx }), "Change MCP server ctx");
  assert.equal(context.agentActionTitle("import_mcp_server", { machine: "this-mac", account: "work", name: "files" }), "Import MCP server files from work");
  assert.equal(context.agentActionTitle("remove_mcp_server", '{"id":"mcp-1"}'), "Remove MCP server ctx");
  assert.equal(context.agentActionTitle("enable_mcp_server", { id: "mcp-9", enabled: false }), "Turn off MCP server mcp-9");
  const paths = vm.runInContext("agentEnteredPaths", context);
  assert.deepEqual(here(Object.fromEntries(paths("secrets", { env: { API_KEY: "k" }, headers: { "X-Org": "o" } }))), { "secrets.env.API_KEY": "k", "secrets.headers.X-Org": "o" });
  assert.deepEqual(here(paths("key", "lin_api_x")), [["key", "lin_api_x"]], "a typed key keeps its name");
});

test("Approve on an agent's add opens the form with its server locked, and the secret typed there approves it", async () => {
  const decided = { decision: "approved", outcome: "done", result: { id: "mcp-1", enabled: true, server: ctx } };
  const { context, calls, dom, state } = app({ answer: async (call) => (call.cmd === "decide_confirmation" ? decided : []) });
  context.onAgentConfirmations([{ ...asked, action: "save_mcp_server", args: { server: ctx } }]);
  await approve(context, "c-7");
  assert.equal(state("page"), "mcp-servers");
  const html = dom["settings-page"].innerHTML;
  assert.match(html, /<b>Add an MCP server<\/b>/);
  assert.match(html, /Asked by “Wire up the docs” on MacBook\. Entering its secrets here approves it\./);
  assert.match(html, /<input name="name" value="ctx" placeholder="context7" autocomplete="off" spellcheck="false" readonly>/);
  assert.match(html, /<select name="type" disabled>/);
  assert.match(html, /<input type="password" data-mcp-secret="API_KEY" autocomplete="off" spellcheck="false" placeholder="Value">/);
  assert.match(html, /<button class="primary">Approve<\/button>/);
  assert.deepEqual(calls.map((c) => c.cmd), ["list_mcp_servers"], "opening the form decides nothing");

  dom["mcp-form"] = mcpForm({ name: "changed-by-hand", type: "stdio", command: "evil" }, { API_KEY: "typed-key-1" });
  await context.mcpSave();
  const decide = calls.find((c) => c.cmd === "decide_confirmation");
  assert.deepEqual(here(decide), { cmd: "decide_confirmation", id: "c-7", approve: true, entered: { "secrets.env.API_KEY": "typed-key-1" } }, "the agent's own request runs, with the secret the user typed");
  assert.ok(!calls.some((c) => c.cmd === "save_mcp_server"), "the form never makes its own save for an agent's request");
  assert.equal(state("mcpUi.form"), null);
});

test("Approve on an agent's import reads the server from the machine it named and sends the typed secret", async () => {
  const found = [{ scope: "user", server: { name: "files", transport: { type: "stdio", command: "files-mcp", args: ["--root", "/data"], env: ["API_TOKEN"] } } }];
  const decided = { decision: "approved", outcome: "done", result: {} };
  const { context, calls, machineCalls, dom } = app({
    answer: async (call) => (call.cmd === "decide_confirmation" ? decided : []),
    onMachine: async (call) => (call.cmd === "mcp_importable" ? found : []),
  });
  const args = { machine: "MacBook", account: "work", name: "files", project: "github.com/acme/site" };
  context.onAgentConfirmations([{ ...asked, action: "import_mcp_server", args }]);
  await approve(context, "c-7");
  await flush();
  assert.deepEqual(here(machineCalls.find((m) => m.call.cmd === "mcp_importable")), { machine: "local", call: { cmd: "mcp_importable", account: "work" } });
  const html = dom["settings-page"].innerHTML;
  assert.match(html, /<b>Import an MCP server<\/b>/);
  assert.match(html, /<input name="command" value="files-mcp" placeholder="npx" spellcheck="false" readonly>/);
  assert.match(html, /<option value="github\.com\/acme\/site" selected>github\.com\/acme\/site<\/option>/);
  assert.match(html, /data-mcp-secret="API_TOKEN"/);

  dom["mcp-form"] = mcpForm({}, { API_TOKEN: "typed-token-2" });
  await context.mcpSave();
  assert.deepEqual(here(calls.find((c) => c.cmd === "decide_confirmation")), { cmd: "decide_confirmation", id: "c-7", approve: true, entered: { "secrets.env.API_TOKEN": "typed-token-2" } });
});

test("a server with no secrets is approved by saving it, and an import of a server the account lacks says so", async () => {
  const { context, dom } = app({ onMachine: async () => [] });
  context.onAgentConfirmations([{ ...asked, action: "save_mcp_server", args: { server: linear } }]);
  await approve(context, "c-7");
  assert.match(dom["settings-page"].innerHTML, /Saving it here approves it\./);
  assert.match(dom["settings-page"].innerHTML, /If it signs in with OAuth/);
  context.onAgentConfirmations([{ ...asked, id: "c-8", action: "import_mcp_server", args: { machine: "this-mac", account: "work", name: "gone" } }]);
  await approve(context, "c-8");
  await flush();
  assert.match(dom["settings-page"].innerHTML, /<p class="error" id="mcp-form-error">work on MacBook has no MCP server gone<\/p>/);
});

test("the form shows the hub's refusal of a server that carries a key, and keeps what was typed", async () => {
  const refusal = "files holds a key in its command, arguments or URL, which lists show: put it in an environment variable or a header";
  const { context, calls, dom, state } = app({ answer: async (call) => { if (call.cmd === "save_mcp_server") throw refusal; return []; } });
  vm.runInContext(`page = "mcp-servers"; mcpUi.form = { id: null, server: { name: "", transport: { type: "stdio", command: "", env: [] } } }`, context);
  dom["mcp-form"] = mcpForm({ name: "files", project: "", type: "stdio", command: "files-mcp", args: "--token\n3f9a8c1e77d2", url: "", names: "" }, {});
  await context.mcpSave();
  assert.deepEqual(here(calls.find((c) => c.cmd === "save_mcp_server").server), { name: "files", transport: { type: "stdio", command: "files-mcp", args: ["--token", "3f9a8c1e77d2"], env: [] } });
  assert.match(dom["settings-page"].innerHTML, new RegExp(`<p class="error" id="mcp-form-error">${refusal.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}</p>`));
  assert.equal(state("mcpUi.form.server.transport.command"), "files-mcp", "the form stays open with the server as typed");
});
