/** The Convex connector: its card on the Connectors page, the banner in a thread whose project uses
 * Convex, and the Read Convex data workflow step. The hub keeps each deploy key; the card only sends
 * what is typed into `#convex-key`, and every other view shows the key's hint. */

const CONVEX_KEYS = "https://dashboard.convex.dev/deployment/settings";
const CONVEX_PERMISSIONS = ["deployment:data:view", "deployment:functions:runInternalQueries", "deployment:functions:runTestQuery", "deployment:logs:view"];
LOGOS.convex = `<svg viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="5" rx="1.6"/><rect x="3" y="9.5" width="18" height="5" rx="1.6"/><rect x="3" y="16" width="18" height="5" rx="1.6"/></svg>`;

/** `prefill` is the repo a banner asked the card to connect. `tables` holds each connection's table
 * names for the step's panel: "loading", "failed", or the names. `filter` narrows the panel's list. */
const convexUi = { prefill: null, tables: {}, filter: "" };

const convexConnections = () => settings.convex ?? [];
const convexServing = (repo) => convexConnections().find((c) => (c.repos ?? []).includes(repo)) ?? null;
/** `dev:quick-lemur-789`, as a project's dotenv names it, as the deployment `quick-lemur-789`. */
const convexDeployment = (value) => String(value ?? "").split(":").pop();

/** Each project with a git origin on the account's machines, once per repo key, with the deployment
 * its dotenv names. A box's clones are left out: the user works in their own folders. */
function convexProjects() {
  const byRepo = new Map();
  for (const m of machines.values()) {
    if (m.view?.upstash_box) continue;
    for (const p of m.projects ?? []) {
      const seen = p.repo && byRepo.get(p.repo);
      if (p.repo && (!seen || (!seen.convex && p.convex))) byRepo.set(p.repo, { repo: p.repo, name: p.name, path: p.path, convex: p.convex ?? null, deployment: p.convex ? convexDeployment(p.convex) : null });
    }
  }
  return [...byRepo.values()];
}

const convexProjectLabel = (projects, repo) => {
  const p = projects.find((x) => x.repo === repo);
  return p ? `${p.name} · ${tilde(p.path)}` : repo;
};

function convexCardHtml() {
  const list = convexConnections();
  const body = !account
    ? `<p class="hint">Sign in to pinned to connect Convex. The hub keeps the deploy keys.</p>`
    : `${list.length ? `<ul class="agent-list convex-list">${list.map(convexRowHtml).join("")}</ul>` : ""}${convexFoundHtml()}${convexFormHtml()}`;
  return `<div class="card" id="convex">
      <div class="card-head"><span class="logo" data-tone="convex" data-logo="convex">${LOGOS.convex}</span><div><b>Convex</b><div class="hint">Tables, documents and logs your agents can read</div></div>
        <span class="badge" data-on="${list.length > 0}">${list.length ? `${list.length} connected` : "Not connected"}</span></div>
      ${body}
    </div>`;
}

function convexRowHtml(c) {
  const projects = convexProjects();
  const repos = c.repos ?? [];
  const nameOf = (repo) => projects.find((p) => p.repo === repo)?.name ?? repo.split("/").pop();
  const chips = repos.map((repo) => `<span class="chip convex-repo" title="${esc(repo)}">${esc(nameOf(repo))}<button type="button" data-action="convex-remove-repo" data-id="${esc(c.id)}" data-repo="${esc(repo)}" aria-label="Remove ${esc(nameOf(repo))}">×</button></span>`);
  const addable = projects.filter((p) => !repos.includes(p.repo)).map((p) => p.repo);
  const add = addable.length
    ? `<select data-convex-add="${esc(c.id)}" aria-label="Add a project">${options(["", ...addable], (repo) => (repo ? convexProjectLabel(projects, repo) : "Add a project…"), "")}</select>`
    : "";
  return `<li class="agent-row convex-row">
      <div><b>${esc(c.project || c.id)}</b> <code>${esc(c.id)}</code> <span class="badge convex-kind" data-kind="${esc(c.kind)}">${esc(c.kind)}</span> <span class="hint">key ${esc(c.key_hint)}</span>
        <div class="chips">${chips.join("") || `<span class="hint">No project reads it yet.</span>`}${add}</div></div>
      <button class="secondary" data-action="disconnect-convex" data-id="${esc(c.id)}">Disconnect</button>
    </li>`;
}

/** Projects whose dotenv names a deployment while no connection serves their repo. When that
 * deployment is already connected for another project, one click adds this one to it. */
function convexFoundHtml() {
  return convexProjects()
    .filter((p) => p.deployment && !convexServing(p.repo))
    .map((p) => {
      const connected = convexConnections().find((c) => c.id === p.deployment);
      const add = connected ? ` <button class="secondary" data-action="convex-add-repo" data-id="${esc(connected.id)}" data-repo="${esc(p.repo)}">Add ${esc(p.name)} to it</button>` : "";
      return `<p class="hint convex-found">Pinned found <code>${esc(p.deployment)}</code> in <code>${esc(tilde(p.path))}</code>.${add}</p>`;
    })
    .join("");
}

function convexFormHtml() {
  const asking = agentAsking("connect_convex");
  const projects = convexProjects();
  const chosen = asking ? asking.args?.repos?.[0] ?? "" : convexUi.prefill ?? projects.find((p) => p.deployment && !convexServing(p.repo))?.repo ?? "";
  const repos = ["", ...projects.map((p) => p.repo)];
  if (chosen && !repos.includes(chosen)) repos.push(chosen);
  const found = projects.find((p) => p.repo === chosen)?.convex;
  return `${agentAskingHtml("connect_convex")}<ol class="steps">
      <li>Open the deployment's settings in Convex and create a deploy key with these permissions.
        <button class="secondary" data-action="open-convex-keys">Open Convex ↗</button>
        <ul class="convex-permissions">${CONVEX_PERMISSIONS.map((name) => `<li><code>${name}</code></li>`).join("")}</ul></li>
      <li>Pick the project whose agents read it, and paste the key. Pinned reads the deployment from the key.
        <div class="row"><select id="convex-project" aria-label="Project"${asking ? " disabled" : ""}>${options(repos, (repo) => (repo ? convexProjectLabel(projects, repo) : "No project yet"), chosen)}</select></div>
        <div class="row"><input id="convex-key" type="password" placeholder="${esc(found ?? "dev:…")}|…" autocomplete="off" spellcheck="false">
        <button class="primary" data-action="connect-convex">Connect</button></div></li>
    </ol>
    <p class="error" id="convex-error"></p>`;
}

function redrawConvexCard() {
  const card = document.getElementById("convex");
  if (card) card.outerHTML = convexCardHtml();
}

function convexError(e) {
  const line = document.getElementById("convex-error");
  if (line) line.textContent = String(e);
}

/** Keeps a connection the hub answered with. The hub moved its repos off every other connection. */
function convexKeep(saved) {
  const kept = convexConnections().map((c) => (c.id === saved.id ? saved : { ...c, repos: (c.repos ?? []).filter((repo) => !saved.repos.includes(repo)) }));
  settings.convex = kept.some((c) => c.id === saved.id) ? kept : [...kept, saved];
  redrawConvexCard();
}

async function convexSetRepos(id, change) {
  const c = convexConnections().find((x) => x.id === id);
  if (!c) return;
  try {
    convexKeep(await connectorCall("set_convex_repos", { id, repos: change(c.repos ?? []) }));
  } catch (e) {
    convexError(e);
  }
}

/** Re-reads each machine's projects, so a folder that started using Convex since boot is listed. */
async function loadConvexProjects() {
  if (!account) return;
  const before = JSON.stringify(convexProjects());
  const asked = [...machines.entries()]
    .filter(([id, m]) => (id === LOCAL || m.view?.online) && !m.view?.upstash_box)
    .map(([id]) => callOn(id, { cmd: "list_projects" }).then((list) => noteProjects(id, Array.isArray(list) ? list : []), () => {}));
  await Promise.all(asked);
  if (JSON.stringify(convexProjects()) !== before) redrawConvexCard();
}

async function convexOpenCard(repo) {
  convexUi.prefill = repo;
  await openSettings("integrations");
  document.getElementById("convex")?.scrollIntoView({ block: "center" });
  document.getElementById(account ? "convex-key" : "convex")?.focus();
}

Object.assign(actions, {
  "open-convex-keys": () => (TAURI ? invoke("open_url", { url: CONVEX_KEYS }) : window.open(CONVEX_KEYS, "_blank", "noopener")),
  async "connect-convex"() {
    const key = document.getElementById("convex-key").value.trim();
    if (!key) return;
    const repo = document.getElementById("convex-project")?.value;
    try {
      const saved = await connectorCall("connect_convex", { key, repos: repo ? [repo] : [] });
      convexUi.prefill = null;
      delete convexUi.tables[saved.id];
      convexKeep(saved);
    } catch (e) {
      convexError(e);
    }
  },
  async "disconnect-convex"(button) {
    if (!armed(button, "Click again to disconnect")) return;
    const { id } = button.dataset;
    try {
      await connectorCall("disconnect_convex", { id });
      settings.convex = convexConnections().filter((c) => c.id !== id);
      redrawConvexCard();
    } catch (e) {
      convexError(e);
    }
  },
  "convex-remove-repo": ({ dataset: { id, repo } }) => convexSetRepos(id, (repos) => repos.filter((r) => r !== repo)),
  "convex-add-repo": ({ dataset: { id, repo } }) => convexSetRepos(id, (repos) => [...repos, repo]),
});

settingsPage.addEventListener("change", (e) => {
  const id = e.target.dataset.convexAdd;
  if (id && e.target.value) convexSetRepos(id, (repos) => [...repos, e.target.value]);
});

/** Repos whose banner was dismissed this boot. */
const convexDismissed = new Set();

function convexBannerFor(tab) {
  if (!account || !tab) return null;
  const path = projectPathOf(tab);
  const project = (machines.get(tab.machine)?.projects ?? []).find((p) => p.path === path);
  if (!project?.convex || !project.repo || convexServing(project.repo) || convexDismissed.has(project.repo)) return null;
  return { repo: project.repo, deployment: convexDeployment(project.convex) };
}

function convexBannerHtml(banner) {
  return `<div class="resume-banner convex-banner" role="status"><span class="logo" data-tone="convex" data-logo="convex">${LOGOS.convex}</span><div><b>This project uses Convex.</b>
    <span>Connect ${esc(banner.deployment)} so agents here can read its data.</span></div>
    <button type="button" class="primary" data-convex-connect="${esc(banner.repo)}">Connect Convex</button>
    <button type="button" class="icon" data-dismiss-convex="${esc(banner.repo)}" aria-label="Dismiss">${icon("x")}</button></div>`;
}

const CONVEX_STEP = { tone: "convex", icon: "▤", app: "Convex", title: "Read Convex data", about: "Lets this run's session read the project's Convex data." };

/** Whether a workflow's repository, as typed or as `git_remote` gives it, is the repo key `key`. */
const convexSameRepo = (url, key) => {
  const text = String(url ?? "").trim().toLowerCase().replace(/\/+$/, "").replace(/\.git$/, "").replaceAll(":", "/");
  return text === key || text.endsWith(`/${key}`) || text.endsWith(`@${key}`);
};

/** The connection a new step reads: the one serving the workflow's repository, else the only one. */
function convexDefaultConnection(c) {
  const list = convexConnections();
  const urls = [c.repo, ...(c.graph?.nodes ?? []).filter((n) => graphNodeKind(n) === "session").map((n) => n.repo)].filter((url) => url?.trim());
  const serving = list.find((conn) => (conn.repos ?? []).some((key) => urls.some((url) => convexSameRepo(url, key))));
  return serving?.id ?? (list.length === 1 ? list[0].id : "");
}

function convexStepSummary(node) {
  const c = convexConnections().find((x) => x.id === node.connection);
  if (!c) return !convexConnections().length ? "Connect Convex" : node.connection ? "Missing connection" : "Choose a connection";
  const name = c.project || c.id;
  if (!Array.isArray(node.tables)) return `${name} · all tables`;
  return node.tables.length ? `${name} · ${node.tables.join(", ")}` : `${name} · pick tables`;
}

function convexStepIssues(node) {
  const list = convexConnections();
  if (!list.length) return ["Connect Convex under Connectors."];
  return [
    !node.connection && "Choose the Convex connection this run reads.",
    node.connection && !list.some((c) => c.id === node.connection) && `${node.connection} is no longer connected. Choose another connection.`,
    Array.isArray(node.tables) && !node.tables.length && "Pick at least one table, or read all tables.",
  ].filter(Boolean);
}

const convexOpenNode = () => view.draft?.graph?.nodes?.find((n) => n.id === view.open && graphNodeKind(n) === "convex-data") ?? null;

/** Connection `id`'s table names, fetched the first time a panel asks. The answer fills the panel's
 * list in place, so a field being typed in keeps its focus. */
function convexTablesOf(id) {
  if (!(id in convexUi.tables)) {
    convexUi.tables[id] = "loading";
    connectorCall("convex_tables", { id })
      .then((names) => (convexUi.tables[id] = Array.isArray(names) ? names : "failed"), () => (convexUi.tables[id] = "failed"))
      .then(() => {
        const node = convexOpenNode();
        const box = document.getElementById("convex-step-tables");
        if (box && node?.connection === id) box.innerHTML = convexTablesHtml(node);
      });
  }
  return convexUi.tables[id];
}

/** The names the step's list offers: the connection's tables, then any picked table it no longer lists. */
function convexTableNames(node) {
  const known = convexTablesOf(node.connection);
  return Array.isArray(known) ? [...new Set([...known, ...(node.tables ?? [])])] : [];
}

const convexChosenHtml = (node) =>
  (node.tables ?? []).map((name) => `<span class="chip convex-table" title="${esc(name)}">${esc(name)}<button type="button" data-action="convex-remove-table" data-table="${esc(name)}" aria-label="Remove ${esc(name)}">×</button></span>`).join("") ||
  `<span class="hint">No table picked yet.</span>`;

function convexTableListHtml(node) {
  const query = convexUi.filter.trim().toLowerCase();
  const shown = convexTableNames(node).filter((name) => name.toLowerCase().includes(query));
  if (!shown.length) return `<p class="hint">No table matches “${esc(convexUi.filter.trim())}”.</p>`;
  const chosen = node.tables ?? [];
  return shown.map((name) => `<label class="toggle"><input type="checkbox" data-convex-table="${esc(name)}"${chosen.includes(name) ? " checked" : ""}> <code>${esc(name)}</code></label>`).join("");
}

/** The picked tables as chips over a filter and a scrolling list, which a deployment of over a
 * hundred tables needs. When the names can't be listed, the tables are typed instead. */
function convexTablesHtml(node) {
  const chosen = node.tables ?? [];
  if (!convexConnections().some((c) => c.id === node.connection)) return `<p class="hint">Choose the connection first.</p>`;
  const known = convexTablesOf(node.connection);
  if (known === "loading") return `<p class="hint">Listing the tables…</p>`;
  const names = convexTableNames(node);
  if (!names.length) {
    return `<input data-convex-tables-text value="${esc(chosen.join(", "))}" placeholder="invoices, customers" spellcheck="false" aria-label="Tables">
      <p class="hint">${known === "failed" ? "Pinned could not list the tables." : "The deployment lists no tables yet."} Type their names, separated by commas.</p>`;
  }
  return `<div class="chips" id="convex-chosen">${convexChosenHtml(node)}</div>
    <input type="search" data-convex-filter value="${esc(convexUi.filter)}" placeholder="Filter ${names.length} tables" spellcheck="false" aria-label="Filter the tables">
    <div class="convex-table-list" id="convex-table-list">${convexTableListHtml(node)}</div>`;
}

/** Redraws the chips, and the list when asked, leaving the filter being typed in alone. */
function convexRedrawTables(node, list) {
  const chips = document.getElementById("convex-chosen");
  if (chips) chips.innerHTML = convexChosenHtml(node);
  const box = list && document.getElementById("convex-table-list");
  if (box) box.innerHTML = convexTableListHtml(node);
}

function convexStepPanel(node) {
  const list = convexConnections();
  if (!list.length) {
    return `<p class="hint">No Convex deployment is connected yet. The Convex card under Connectors connects one.</p>
      <div class="actions start"><button class="secondary" data-action="convex-open-card">Connect Convex</button></div>`;
  }
  const ids = ["", ...list.map((c) => c.id)];
  if (node.connection && !ids.includes(node.connection)) ids.push(node.connection);
  const label = (id) => {
    const c = list.find((x) => x.id === id);
    return !id ? "Choose a connection" : c ? `${c.project || c.id} · ${c.id}` : `${id} (disconnected)`;
  };
  const only = Array.isArray(node.tables);
  return `<label>Connection <select data-convex-connection>${options(ids, label, node.connection ?? "")}</select></label>
    <div class="convex-scope">Tables
      <div class="chips"><button type="button" class="chip${only ? "" : " active"}" data-action="convex-all-tables">All tables</button><button type="button" class="chip${only ? " active" : ""}" data-action="convex-only-tables">Only these</button></div>
      ${only ? `<div id="convex-step-tables">${convexTablesHtml(node)}</div>` : ""}
    </div>
    <p class="hint">With a table list, queries and logs are off for this run.</p>`;
}

/** All tables (`null`) or a list. Picking Only these again keeps the list already picked. */
function convexSetTables(tables) {
  const node = convexOpenNode();
  if (!node || Array.isArray(node.tables) === Array.isArray(tables)) return;
  node.tables = tables;
  redraw();
}

Object.assign(actions, {
  "convex-open-card": () => convexOpenCard(null),
  "convex-all-tables": () => convexSetTables(null),
  "convex-only-tables": () => convexSetTables([]),
  "convex-remove-table"(button) {
    const node = convexOpenNode();
    if (!node) return;
    node.tables = (node.tables ?? []).filter((name) => name !== button.dataset.table);
    convexRedrawTables(node, true);
    refresh();
  },
});

settingsPage.addEventListener("change", (e) => {
  const node = convexOpenNode();
  if (!node) return;
  if ("convexConnection" in e.target.dataset) {
    node.connection = e.target.value;
    convexUi.filter = "";
    return redraw();
  }
  const table = e.target.dataset.convexTable;
  if (table === undefined) return;
  const rest = (node.tables ?? []).filter((name) => name !== table);
  node.tables = e.target.checked ? [...rest, table] : rest;
  convexRedrawTables(node, false);
  refresh();
});

settingsPage.addEventListener("input", (e) => {
  const { dataset } = e.target;
  if (!("convexTablesText" in dataset) && !("convexFilter" in dataset)) return;
  const node = convexOpenNode();
  if (!node) return;
  if ("convexFilter" in dataset) {
    convexUi.filter = e.target.value;
    return convexRedrawTables(node, true);
  }
  node.tables = e.target.value.split(",").map((name) => name.trim()).filter(Boolean);
  refresh();
});
