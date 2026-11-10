/** The MCP servers page: the account's own MCP servers, which every session pinned starts loads
 * while they are on. Secrets are typed here only, and the hub answers with their names alone. */

const MCP_TYPES = { stdio: "Program (stdio)", http: "HTTP", sse: "SSE" };

/** The page's state: the hub's list, the repos a server can be kept to, the add or edit form, and
 * the import panel. A form opened by Approve on an agent's request carries that confirmation. */
const mcpStart = () => ({ list: [], loaded: false, error: "", repos: [], form: null, from: null, removing: null });
let mcpUi = mcpStart();

const mcpBlank = () => ({ name: "", transport: { type: "stdio", command: "", args: [], env: [] } });
/** Which names hold the server's secrets: a program's environment, or a URL's headers. */
const mcpKind = (server) => (server.transport.type === "stdio" ? "env" : "headers");
const mcpNames = (server) => server.transport[mcpKind(server)] ?? [];
const mcpScope = (server) => server.project ?? "Every project";
const mcpReach = (server) => (server.transport.type === "stdio" ? [server.transport.command, ...(server.transport.args ?? [])].join(" ") : server.transport.url);
/** An http or sse server that sends no header of its own may sign in with OAuth, which Claude Code
 * keeps per Claude account. */
const mcpMaySignIn = (server) => server.transport.type !== "stdio" && !mcpNames(server).length;
/** The projects a server can be kept to: every one, the repos the machines have, and its own. */
const mcpScopes = (repos, server) => ["", ...new Set([...repos, ...(server.project ? [server.project] : [])])];
const MCP_OAUTH = "If it signs in with OAuth, log in once per Claude account: run /mcp in a session of each account.";

function mcpRowHtml(entry) {
  const { server } = entry;
  const names = mcpNames(server);
  const secrets = names.length
    ? `<div class="mcp-secrets">${names.map((name) => `<span class="badge" data-on="true">${esc(name)} · set</span>`).join("")}</div>`
    : "";
  return `<div class="card mcp-server" data-mcp="${esc(entry.id)}">
      <div class="card-head"><div class="mcp-name"><b>${esc(server.name)}</b>
        <div class="hint">${esc(MCP_TYPES[server.transport.type])} · ${esc(mcpScope(server))}</div></div>
        <label class="switch" title="${entry.enabled ? "On" : "Off"}"><input type="checkbox" data-mcp-switch aria-label="Load ${esc(server.name)} in new sessions"${entry.enabled ? " checked" : ""}><span></span></label></div>
      <code class="mcp-reach">${esc(mcpReach(server))}</code>
      ${secrets}
      ${mcpMaySignIn(server) ? `<p class="hint">${esc(MCP_OAUTH)}</p>` : ""}
      <div class="actions start"><button type="button" class="secondary" data-action="mcp-edit">Edit</button>
        <button type="button" class="secondary danger" data-action="mcp-remove">Remove</button></div>
      ${mcpUi.removing === entry.id ? `<div class="confirm">Remove ${esc(server.name)}? New sessions stop loading it, and its secrets are deleted.
        <span class="actions start"><button type="button" class="secondary danger" data-action="mcp-remove-yes">Remove</button>
        <button type="button" class="secondary" data-action="mcp-remove-no">Cancel</button></span></div>` : ""}
    </div>`;
}

/** The add or edit form. An agent's request fills it and locks all but the secrets. */
function mcpFormHtml(form, repos) {
  const { server, asking } = form;
  const lock = asking ? " readonly" : "";
  const kind = mcpKind(server);
  const t = server.transport;
  const stored = new Set(form.stored ?? []);
  const field = (label, html, wide) => `<label class="mcp-field${wide ? " wide" : ""}"><span>${label}</span>${html}</label>`;
  const scopes = mcpScopes(repos, server);
  const secretInputs = mcpNames(server).map((name) => field(esc(name), `<input type="password" data-mcp-secret="${esc(name)}" autocomplete="off" spellcheck="false" placeholder="${stored.has(name) ? "Set · leave blank to keep" : "Value"}">`, true));
  const title = asking ? (asking.action === "import_mcp_server" ? "Import an MCP server" : form.id ? "Change an MCP server" : "Add an MCP server") : form.id ? `Edit ${server.name}` : "Add an MCP server";
  return `<form class="card mcp-form" id="mcp-form">
      <div class="card-head"><div><b>${esc(title)}</b><div class="hint">Every session pinned starts loads it while it is on, alongside pinned's own server.</div></div></div>
      ${asking ? agentAskingHtml(asking.action) : ""}
      ${form.loading ? `<p class="hint">Reading ${esc(asking?.args.account ?? "the account")}'s servers…</p>` : `<div class="mcp-fields">
        ${field("Name", `<input name="name" value="${esc(server.name)}" placeholder="context7" autocomplete="off" spellcheck="false"${lock}>`)}
        ${field("Projects", `<select name="project"${asking ? " disabled" : ""}>${options(scopes, (repo) => repo || "Every project", server.project ?? "")}</select>`)}
        ${field("Transport", `<select name="type"${asking ? " disabled" : ""}>${options(Object.keys(MCP_TYPES), (type) => MCP_TYPES[type], t.type)}</select>`)}
        ${t.type === "stdio"
          ? `${field("Command", `<input name="command" value="${esc(t.command)}" placeholder="npx" spellcheck="false"${lock}>`)}
             ${field("Arguments, one per line", `<textarea name="args" rows="2" spellcheck="false"${lock}>${esc((t.args ?? []).join("\n"))}</textarea>`, true)}`
          : field("URL", `<input name="url" value="${esc(t.url ?? "")}" placeholder="https://mcp.example.com/mcp" spellcheck="false"${lock}>`, true)}
        ${field(kind === "env" ? "Environment variables, by name" : "Headers, by name", `<input name="names" value="${esc(mcpNames(server).join(", "))}" placeholder="${kind === "env" ? "API_KEY" : "Authorization"}" spellcheck="false"${lock}>`, true)}
        ${secretInputs.length ? `<p class="hint wide">Values are kept sealed on the hub. Agents and lists only see that each is set.</p>${secretInputs.join("")}` : ""}
      </div>
      ${mcpMaySignIn(server) && (t.url ?? "") ? `<p class="hint">${esc(MCP_OAUTH)}</p>` : ""}
      <div class="actions start"><button class="primary">${asking ? "Approve" : "Save"}</button>
        <button type="button" class="secondary" data-action="mcp-cancel">Cancel</button></div>`}
      <p class="error" id="mcp-form-error">${esc(form.error ?? "")}</p>
    </form>`;
}

/** The import panel: a Claude account's own servers on one machine, each ready to add. */
function mcpFromHtml(from) {
  const claude = (from.accounts ?? []).filter((a) => a.provider === "claude");
  const rows = from.found.map((found, i) => `<div class="row mcp-found"><span><b>${esc(found.server.name)}</b> <span class="hint">${esc(mcpReach(found.server))}</span></span>
      <button type="button" class="secondary" data-action="mcp-import-one" data-index="${i}">Import</button></div>`);
  return `<div class="card mcp-from">
      <div class="card-head"><div><b>Import from a Claude account</b><div class="hint">The servers in its .claude.json. You type their secrets here.</div></div></div>
      ${machineChipsHtml(from.machine, "mcp-from-machine")}
      <div class="row"><select id="mcp-from-account">${options(claude.map((a) => a.id), (id) => claude.find((a) => a.id === id)?.name ?? id, from.account)}</select></div>
      ${from.loading ? `<p class="hint">Reading its servers…</p>` : rows.length ? rows.join("") : `<p class="hint">${esc(from.error || "It has no MCP servers of its own.")}</p>`}
    </div>`;
}

function mcpPageHtml() {
  if (!hub || !account) {
    return `<h2>MCP servers</h2>
      <p class="hint">Sign in under Account to keep MCP servers that every session pinned starts loads, on every machine.</p>
      <div class="actions start"><button class="primary" data-action="go-account">Go to Account</button></div>`;
  }
  const { list, loaded, error, form, from } = mcpUi;
  const servers = list.length ? list.map(mcpRowHtml).join("") : `<p class="hint">${loaded ? "No MCP servers yet." : "Loading…"}</p>`;
  return `<h2>MCP servers</h2>
    <p class="hint">Servers every session pinned starts loads, for Claude Code and Codex, on every machine and box of the account. A server its Claude account already configures keeps the account's own.</p>
    ${error ? `<p class="error">${esc(error)}</p>` : ""}
    ${form ? mcpFormHtml(form, mcpUi.repos) : `<div class="actions start"><button type="button" class="primary" data-action="mcp-add">Add a server</button>
      <button type="button" class="secondary" data-action="mcp-from">Import from a Claude account</button></div>`}
    ${from && !form ? mcpFromHtml(from) : ""}
    <div id="mcp-list">${servers}</div>`;
}

function drawMcpPage() {
  if (mcpUi.form?.asking && agentApproving?.id !== mcpUi.form.asking.id) mcpUi.form = null;
  if (!settingsOpen() || page !== "mcp-servers") return;
  settingsPage.innerHTML = mcpPageHtml();
  const form = mcpUi.form && document.getElementById("mcp-form");
  if (form) {
    form.onsubmit = (e) => (e.preventDefault(), mcpSave());
    form.onchange = (e) => e.target.matches("[name=type], [name=names]") && mcpRedrawForm();
    const secret = form.querySelector("[data-mcp-secret]");
    if (mcpUi.form.asking && secret) secret.focus();
  }
  const pick = document.getElementById("mcp-from-account");
  if (pick) pick.onchange = () => mcpLoadFrom(mcpUi.from.machine, pick.value);
  for (const toggle of settingsPage.querySelectorAll("[data-mcp-switch]")) toggle.onchange = () => mcpEnable(toggle.closest("[data-mcp]").dataset.mcp, toggle.checked);
}

/** The server as the form's fields describe it: `fields` holds each input's text. */
function mcpServerOf(fields) {
  const names = fields.names.split(/[\s,]+/).filter(Boolean);
  const transport =
    fields.type === "stdio"
      ? { type: "stdio", command: fields.command.trim(), args: fields.args.split("\n").map((a) => a.trim()).filter(Boolean), env: names }
      : { type: fields.type, url: fields.url.trim(), headers: names };
  return { name: fields.name.trim(), ...(fields.project ? { project: fields.project } : {}), transport };
}

/** The secrets typed in the form, by kind and name, without the ones left blank. */
function mcpSecretsOf(server, typed) {
  const values = Object.fromEntries(Object.entries(typed).filter(([name, value]) => mcpNames(server).includes(name) && value.trim()));
  return Object.keys(values).length ? { [mcpKind(server)]: values } : {};
}

/** The call the form makes: an agent's own request with the secrets typed here, or the user's save. */
function mcpCallOf(form, server, secrets) {
  const typed = Object.keys(secrets).length ? { secrets } : {};
  if (form.asking) return { cmd: form.asking.action, ...form.asking.args, ...typed };
  return { cmd: "save_mcp_server", ...(form.id ? { id: form.id } : {}), server, ...typed };
}

function mcpReadForm() {
  const el = document.getElementById("mcp-form");
  const value = (name) => el.querySelector(`[name=${name}]`)?.value ?? "";
  const fields = { name: value("name"), project: value("project"), type: value("type"), command: value("command"), args: value("args"), url: value("url"), names: value("names") };
  const typed = Object.fromEntries([...el.querySelectorAll("[data-mcp-secret]")].map((input) => [input.dataset.mcpSecret, input.value]));
  return { server: mcpUi.form.asking ? mcpUi.form.server : mcpServerOf(fields), typed };
}

/** Draws the form again from what its fields hold now, as after a change of transport or names,
 * keeping the secrets typed so far. */
function mcpRedrawForm() {
  const { server, typed } = mcpReadForm();
  mcpUi.form.server = server;
  drawMcpPage();
  for (const [name, value] of Object.entries(typed)) {
    const input = document.querySelector(`#mcp-form [data-mcp-secret="${CSS.escape(name)}"]`);
    if (input) input.value = value;
  }
}

async function mcpSave() {
  const form = mcpUi.form;
  const { server, typed } = mcpReadForm();
  const secrets = mcpSecretsOf(server, typed);
  const call = mcpCallOf(form, server, secrets);
  const button = document.querySelector("#mcp-form button.primary");
  if (button) button.disabled = true;
  try {
    await (agentApproval(call) ?? hub.hub(call));
    mcpUi.form = null;
    await loadMcpServers();
  } catch (e) {
    form.server = server;
    form.error = String(e);
    drawMcpPage();
  }
}

async function mcpEnable(id, enabled) {
  try {
    const changed = await hub.hub({ cmd: "enable_mcp_server", id, enabled });
    mcpUi.list = mcpUi.list.map((entry) => (entry.id === id ? changed : entry));
    mcpUi.error = "";
  } catch (e) {
    mcpUi.error = String(e);
  }
  drawMcpPage();
}

async function loadMcpServers() {
  if (!hub) return drawMcpPage();
  try {
    mcpUi.list = await hub.hub({ cmd: "list_mcp_servers" });
    mcpUi.error = "";
  } catch (e) {
    mcpUi.error = `Couldn't load the MCP servers: ${e}`;
  }
  mcpUi.loaded = true;
  drawMcpPage();
}

/** The repos a server can be kept to: the projects the online machines have, as the Way of working
 * page lists them. */
async function loadMcpRepos() {
  const online = onlineMachines();
  const lists = await Promise.all(online.map((m) => callOn(m, { cmd: "list_projects" }).catch(() => [])));
  mcpUi.repos = [...new Set(lists.flat().map((p) => p.repo).filter(Boolean))].sort();
  const select = mcpUi.form && document.querySelector("#mcp-form [name=project]");
  if (select) select.innerHTML = options(mcpScopes(mcpUi.repos, mcpUi.form.server), (repo) => repo || "Every project", mcpUi.form.server.project ?? "");
  else if (!mcpUi.form) drawMcpPage();
}

/** The import panel on `machine`, reading the Claude account `accountId`'s servers. */
async function mcpLoadFrom(machine, accountId) {
  mcpUi.from = { machine, account: accountId, accounts: machines.get(machine)?.accounts ?? [], found: [], loading: true, error: "" };
  drawMcpPage();
  try {
    const accounts = await accountsOf(machine);
    const claude = accounts.filter((a) => a.provider === "claude");
    const chosen = claude.find((a) => a.id === accountId)?.id ?? claude[0]?.id;
    mcpUi.from = { ...mcpUi.from, accounts, account: chosen };
    mcpUi.from.found = chosen ? await callOn(machine, { cmd: "mcp_importable", account: chosen }) : [];
  } catch (e) {
    mcpUi.from.error = `Couldn't read its servers: ${e}`;
  }
  mcpUi.from.loading = false;
  drawMcpPage();
}

/** The machine the app knows by `named`, an id or a name as an agent wrote it, or null. */
function mcpMachine(named) {
  const want = String(named ?? "").toLowerCase();
  if (account && (named === account.machine_id || machineName(LOCAL).toLowerCase() === want || machines.get(LOCAL)?.hubView?.name?.toLowerCase() === want)) return LOCAL;
  return [...machines.entries()].find(([id, m]) => id === named || m.view?.name?.toLowerCase() === want)?.[0] ?? null;
}

/** Approve on an agent's add, edit or import: the form, filled with its request. An import reads the
 * server from the machine and account it named first. */
async function openMcpApproval(c) {
  mcpUi.from = null;
  if (c.action === "save_mcp_server") {
    const stored = mcpUi.list.find((entry) => entry.id === c.args.id);
    mcpUi.form = { id: c.args.id ?? null, server: c.args.server, stored: stored && mcpKind(stored.server) === mcpKind(c.args.server) ? mcpNames(stored.server) : [], asking: c };
    return;
  }
  mcpUi.form = { id: null, server: { name: c.args.name, ...(c.args.project ? { project: c.args.project } : {}), transport: { type: "stdio", command: "", env: [] } }, asking: c, loading: true };
  const machine = mcpMachine(c.args.machine);
  try {
    if (!machine) throw `this app doesn't know machine ${c.args.machine}`;
    const found = (await callOn(machine, { cmd: "mcp_importable", account: c.args.account, ...(c.args.path ? { path: c.args.path } : {}) })).find((f) => f.server.name === c.args.name);
    if (!found) throw `${c.args.account} on ${machineName(machine)} has no MCP server ${c.args.name}`;
    mcpUi.form = { ...mcpUi.form, server: { ...found.server, ...(c.args.project ? { project: c.args.project } : {}) }, loading: false };
  } catch (e) {
    mcpUi.form = { ...mcpUi.form, loading: false, error: String(e) };
  }
  drawMcpPage();
}

function showMcpServers() {
  drawMcpPage();
  loadMcpServers();
  if (hub && account) loadMcpRepos();
}

Object.assign(actions, {
  "mcp-add"() {
    mcpUi = { ...mcpUi, form: { id: null, server: mcpBlank(), stored: [] }, from: null };
    drawMcpPage();
  },
  "mcp-edit"(button) {
    const entry = mcpUi.list.find((e) => e.id === button.closest("[data-mcp]").dataset.mcp);
    mcpUi = { ...mcpUi, form: { id: entry.id, server: entry.server, stored: mcpNames(entry.server) }, from: null };
    drawMcpPage();
  },
  "mcp-cancel"() {
    if (mcpUi.form?.asking) forgetAgentApproval();
    mcpUi.form = null;
    drawMcpPage();
  },
  "mcp-remove"(button) {
    mcpUi.removing = button.closest("[data-mcp]").dataset.mcp;
    drawMcpPage();
  },
  "mcp-remove-no"() {
    mcpUi.removing = null;
    drawMcpPage();
  },
  async "mcp-remove-yes"() {
    const id = mcpUi.removing;
    mcpUi.removing = null;
    try {
      await hub.hub({ cmd: "remove_mcp_server", id });
    } catch (e) {
      mcpUi.error = String(e);
    }
    loadMcpServers();
  },
  "mcp-from"() {
    mcpLoadFrom(onlineMachines()[0] ?? LOCAL, null);
  },
  "mcp-from-machine"(button) {
    mcpLoadFrom(button.dataset.machine, null);
  },
  "mcp-import-one"(button) {
    const found = mcpUi.from.found[Number(button.dataset.index)];
    mcpUi = { ...mcpUi, form: { id: null, server: found.server, stored: [] }, from: null };
    drawMcpPage();
  },
});

settingsSection({ page: "mcp-servers", label: "MCP servers", icon: "wrench", show: () => showMcpServers(), search: [{ title: "MCP servers", terms: ["mcp model context protocol server tools stdio http sse import claude.json secrets env headers oauth"] }] });
