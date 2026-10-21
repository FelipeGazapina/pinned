const MODELS = ["opus", "sonnet", "haiku", "fable"];
const EFFORTS = ["low", "medium", "high", "xhigh", "max"];
const PERMISSIONS = {
  manual: "Ask me",
  acceptEdits: "Accept edits",
  auto: "Auto",
  dontAsk: "Don't ask",
  bypassPermissions: "Bypass all checks",
  plan: "Plan only",
};
/** The steps of a connector's run that move the issue, matching the backend's `Stage`. */
const STAGES = [
  ["picked", "When pinned picks it up"],
  ["coding", "When Claude starts changing code"],
  ["review", "When the pull request is open"],
];
const DEFAULT_INSTRUCTIONS = `Resolve the Linear issue below in this repository, end to end, without waiting for me.

1. Read the issue and the code it touches. Create a branch named after the issue identifier.
2. Implement the change and run the project's existing tests and checks.
3. Prove it works in the running app: write a Playwright test that exercises the fix with video recording on (video: 'on'), run it, and keep the recorded video.
4. Commit the code, and commit the video under .github/evidence/<issue identifier>/.
5. Push and open a pull request with gh pr create. Put the issue identifier in the title, summarize the change, and link the committed video in the body as proof that it works.
6. If a Linear tool is available, comment the pull request link on the issue.`;

const settingsPage = document.getElementById("settings-page");

/** Mirrors the backend's `get_settings`. Unsaved connectors carry an empty id. */
let settings = { linear: null, connectors: [], checked: null };
let page = ["general", "connectors", "projects"].includes(load("settings-page")) ? load("settings-page") : "integrations";
/** Names from the connected Linear workspace, for the connector form's suggestions. */
let suggestions = { labels: [], columns: [] };

const settingsOpen = () => document.body.dataset.page === "settings";

async function toggleSettings() {
  if (settingsOpen()) return active && activate(active);
  document.body.dataset.page = "settings";
  picker.hidden = true;
  settings = await invoke("get_settings");
  showPage(page);
}

function showPage(name) {
  page = name;
  save("settings-page", name);
  for (const b of document.querySelectorAll("#settings-nav button")) b.classList.toggle("active", b.dataset.page === name);
  if (name === "connectors" && settings.linear && !suggestions.labels.length) {
    invoke("linear_suggestions").then((found) => {
      suggestions = found;
      for (const [id, names] of [["linear-labels", found.labels], ["linear-columns", found.columns]]) {
        const list = document.getElementById(id);
        if (list) list.innerHTML = options(names);
      }
    }, console.warn);
  }
  if (name === "projects") return showProjects();
  if (name === "general") return showGeneral();
  settingsPage.innerHTML = name === "integrations" ? integrationsHtml() : connectorsHtml();
}

const esc = (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const options = (values, labelOf = (v) => v, selected) =>
  values.map((v) => `<option value="${esc(v)}"${v === selected ? " selected" : ""}>${esc(labelOf(v))}</option>`).join("");

function integrationsHtml() {
  const account = settings.linear;
  const body = account
    ? `<p class="connected">Connected as <b>${esc(account.name)}</b> (${esc(account.email)}) in ${esc(account.organization)}.</p>
       <div class="actions"><button class="secondary" data-action="disconnect-linear">Disconnect</button></div>`
    : `<ol class="steps">
         <li>Open Linear in your browser, where you're already signed in, and create a personal API key.
           <button class="secondary" data-action="open-linear-keys">Open Linear ↗</button></li>
         <li>Paste the key here.
           <div class="row"><input id="linear-key" type="password" placeholder="lin_api_…" autocomplete="off">
           <button class="primary" data-action="connect-linear">Connect</button></div></li>
       </ol>
       <p class="error" id="linear-error"></p>`;
  return `<h2>Integrations</h2>
    <p class="hint">Services the harness can read from and act on.</p>
    <div class="card">
      <div class="card-head"><span class="logo">◐</span><div><b>Linear</b><div class="hint">Issues, labels and comments</div></div>
        <span class="badge" data-on="${!!account}">${account ? "Connected" : "Not connected"}</span></div>
      ${body}
    </div>`;
}

function connectorsHtml() {
  const intro = `<h2>Connectors</h2>
    <p class="hint">A connector watches an integration and opens a Claude Code tab on its own when something happens, so the agent starts before you ask.</p>`;
  if (!settings.linear) {
    return `${intro}<div class="card"><p>Connect Linear first.</p>
      <div class="actions"><button class="primary" data-action="go-integrations">Go to Integrations</button></div></div>`;
  }
  return `${intro}
    <datalist id="linear-labels">${options(suggestions.labels)}</datalist>
    <datalist id="linear-columns">${options(suggestions.columns)}</datalist>
    <datalist id="models">${options(MODELS)}</datalist>
    ${settings.connectors.map(connectorHtml).join("")}
    <div class="actions start"><button class="secondary" data-action="new-connector">＋ New connector</button></div>
    <p class="hint" id="checked">${checkedText()}</p>`;
}

function connectorHtml(c, i) {
  return `<form class="card connector" data-index="${i}">
    <div class="card-head">
      <input name="name" class="title" value="${esc(c.name)}" placeholder="Connector name" required>
      <label class="toggle"><input type="checkbox" name="enabled"${c.enabled ? " checked" : ""}> On</label>
    </div>
    <div class="grid">
      <label>When a Linear issue is created with label
        <input name="label" list="linear-labels" value="${esc(c.trigger.label)}" placeholder="claude" required></label>
      <label>Open a session in
        <span class="row"><input name="folder" value="${esc(c.folder)}" placeholder="/path/to/repo" required>
        <button type="button" class="secondary" data-action="pick-folder">Choose…</button></span></label>
      <label>Model <input name="model" list="models" value="${esc(c.model)}" placeholder="claude's default"></label>
      <label>Effort <select name="effort">${options(EFFORTS, undefined, c.effort)}</select></label>
      <label>Permissions <select name="permissionMode">${options(Object.keys(PERMISSIONS), (p) => PERMISSIONS[p], c.permissionMode)}</select></label>
      <label class="toggle"><input type="checkbox" name="worktree"${c.worktree ? " checked" : ""}> Separate git worktree per issue</label>
    </div>
    <fieldset class="columns">
      <legend>Move the Linear issue to the column</legend>
      ${STAGES.map(([stage, when]) => `<label>${when}
        <input name="column-${stage}" list="linear-columns" value="${esc(c.columns?.[stage])}" placeholder="leave where it is"></label>`).join("")}
    </fieldset>
    <label>Instructions sent before the issue
      <textarea name="instructions" rows="8">${esc(c.instructions)}</textarea></label>
    <p class="hint">Runs <code>${esc(commandPreview(c))}</code></p>
    <div class="actions">
      <span class="error"></span>
      <button type="button" class="secondary danger" data-action="delete-connector">Delete</button>
      <button class="primary">${c.id ? "Save" : "Create"}</button>
    </div>
  </form>`;
}

function commandPreview(c) {
  const parts = ["claude"];
  if (c.model) parts.push(`--model ${c.model}`);
  parts.push(`--effort ${c.effort}`, `--permission-mode ${c.permissionMode}`);
  if (c.worktree) parts.push("--worktree=<issue>");
  return [...parts, '"<instructions + issue>"'].join(" ");
}

function checkedText() {
  const checked = settings.checked;
  if (!checked) return "Not checked yet. Linear is checked every minute.";
  const at = new Date(checked.at).toLocaleTimeString();
  return checked.error ? `Last check at ${at} failed: ${checked.error}` : `Last checked at ${at}. Linear is checked every minute.`;
}

function readConnector(form) {
  const f = form.elements;
  return {
    ...settings.connectors[form.dataset.index],
    name: f.name.value.trim(),
    enabled: f.enabled.checked,
    trigger: { kind: "linear-label", label: f.label.value.trim() },
    folder: f.folder.value.trim(),
    model: f.model.value.trim(),
    effort: f.effort.value,
    permissionMode: f.permissionMode.value,
    worktree: f.worktree.checked,
    instructions: f.instructions.value,
    columns: Object.fromEntries(STAGES.map(([stage]) => [stage, f[`column-${stage}`].value.trim()])),
  };
}

const actions = {
  "go-integrations": () => showPage("integrations"),
  "open-linear-keys": () => invoke("open_linear_keys"),
  async "connect-linear"() {
    const key = document.getElementById("linear-key").value;
    if (!key.trim()) return;
    try {
      settings.linear = await invoke("connect_linear", { key });
      showPage("integrations");
    } catch (e) {
      document.getElementById("linear-error").textContent = e;
    }
  },
  async "disconnect-linear"() {
    await invoke("disconnect_linear");
    settings.linear = null;
    suggestions = { labels: [], columns: [] };
    showPage("integrations");
  },
  "new-connector"() {
    settings.connectors.push({
      id: "",
      name: "Resolve labeled Linear issues",
      enabled: true,
      trigger: { kind: "linear-label", label: "claude" },
      folder: active?.cwd ?? "",
      model: "opus",
      effort: "high",
      permissionMode: "auto",
      worktree: true,
      instructions: DEFAULT_INSTRUCTIONS,
      columns: { picked: "Analyzing", coding: "In Dev", review: "Ready to Review" },
    });
    showPage("connectors");
    settingsPage.querySelector("form.connector:last-of-type [name=name]").focus();
  },
  async "pick-folder"(button) {
    const input = button.closest("form").elements.folder;
    const picked = await invoke("pick_folder", { start: input.value });
    if (picked) input.value = picked;
  },
  async "delete-connector"(button) {
    const form = button.closest("form");
    const { id } = settings.connectors[form.dataset.index];
    if (id) await invoke("delete_connector", { id });
    settings.connectors.splice(form.dataset.index, 1);
    showPage("connectors");
  },
};

settingsPage.addEventListener("click", (e) => {
  const button = e.target.closest("[data-action]");
  if (!button) return;
  e.preventDefault();
  actions[button.dataset.action](button);
});

settingsPage.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && e.target.id === "linear-key") actions["connect-linear"]();
});

settingsPage.addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = e.target;
  try {
    settings.connectors[form.dataset.index] = await invoke("save_connector", { connector: readConnector(form) });
    showPage("connectors");
  } catch (err) {
    form.querySelector(".error").textContent = err;
  }
});

for (const b of document.querySelectorAll("#settings-nav button")) b.onclick = () => showPage(b.dataset.page);
document.getElementById("settings-toggle").onclick = toggleSettings;

win.listen("connectors-checked", ({ payload }) => {
  settings.checked = payload;
  const line = document.getElementById("checked");
  if (line) line.textContent = checkedText();
});
