/** The Convex connector's card on the Connectors page. The hub keeps each deploy key; the card only
 * sends what is typed into `#convex-key`, and every other view shows the key's hint. */

const CONVEX_KEYS = "https://dashboard.convex.dev/deployment/settings";
const CONVEX_PERMISSIONS = ["deployment:data:view", "deployment:functions:runInternalQueries", "deployment:functions:runTestQuery", "deployment:logs:view"];
LOGOS.convex = `<svg viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="5" rx="1.6"/><rect x="3" y="9.5" width="18" height="5" rx="1.6"/><rect x="3" y="16" width="18" height="5" rx="1.6"/></svg>`;

/** `prefill` is the repo the card was asked to connect. */
const convexUi = { prefill: null };

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
  const chips = repos.map((repo) => `<span class="chip convex-repo" title="${esc(repo)}">${esc(nameOf(repo))}<button type="button" data-action="convex-remove-repo" data-id="${esc(c.id)}" data-repo="${esc(repo)}" aria-label="Stop reading it in ${esc(nameOf(repo))}">×</button></span>`);
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

/** Opens the card with `repo` picked. */
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
