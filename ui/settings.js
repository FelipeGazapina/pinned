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
3. Commit, push and open a pull request with gh pr create. Put the issue identifier in the title and summarize the change.
4. If a Linear tool is available, comment the pull request link on the issue.`;
const SLACK_INSTRUCTIONS = `Someone mentioned you in the Slack thread below. Triage it in this repository, without waiting for me.

- If it reports a bug, reproduce it or trace it through the code, and reply with your analysis: the cause, where it lives, and how to fix it.
- If existing behavior or a known fix already solves it, reply with that solution and stop.
- If it is a question, answer it from the code.

Never edit, commit or push code. Only read, run and reply.`;
const SLACK_FIX_INSTRUCTIONS = `Someone mentioned you in the Slack thread below. Handle it in this repository, without waiting for me.

- If it is a question, answer it from the code.
- If existing behavior or a known fix already solves it, reply with that solution.
- If it reports a bug, reproduce it or trace it through the code before you decide whether to fix it.
- When you fix a bug, create a branch named after the Linear issue identifier, run the project's existing tests and checks, then push and open a pull request with gh pr create. Put the issue identifier in the title and link the Slack thread in the body.`;
/** How far a Slack connector's session goes with a bug, matching the backend's `Fix`. */
const FIXES = {
  off: "Only analyze and reply",
  ask: "Analyze, then ask me in the thread before filing",
  auto: "Fix obvious bugs on its own, ask for the rest",
};
/** What can start a connector, matching the backend's `Trigger`. */
const TRIGGERS = {
  "linear-label": "a Linear issue is created with label",
  "slack-mention": "the bot is @mentioned in Slack channel",
};

const settingsPage = document.getElementById("settings-page");

/** Mirrors the backend's `get_settings`. Unsaved connectors carry an empty id. */
let settings = { linear: null, slack: null, connectors: [], checked: null };
let page = ["general", "connectors", "projects"].includes(load("settings-page")) ? load("settings-page") : "integrations";
/** Names from the connected Linear workspace and Slack channels, for the connector form's suggestions. */
let suggestions = { labels: [], columns: [], teams: [], channels: [] };

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
      Object.assign(suggestions, found);
      fillList("linear-labels", found.labels);
      fillList("linear-columns", found.columns);
      fillList("linear-teams", found.teams);
    }, console.warn);
  }
  if (name === "connectors" && settings.slack && !suggestions.channels.length) {
    invoke("slack_suggestions").then((channels) => {
      suggestions.channels = channels;
      fillList("slack-channels", channels);
    }, console.warn);
  }
  if (name === "projects") return showProjectsPage();
  if (name === "general") return showGeneral();
  settingsPage.innerHTML = name === "integrations" ? integrationsHtml() : connectorsHtml();
}

function fillList(id, names) {
  const list = document.getElementById(id);
  if (list) list.innerHTML = options(names);
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
    </div>
    ${slackCardHtml()}`;
}

function slackCardHtml() {
  const account = settings.slack;
  const body = account
    ? `<p class="connected">Connected as <b>@${esc(account.name)}</b> in ${esc(account.team)}.</p>
       <div class="actions"><button class="secondary" data-action="disconnect-slack">Disconnect</button></div>`
    : `<ol class="steps">
         <li>Create a Slack app at <code>api.slack.com/apps</code>. Under OAuth &amp; Permissions, give its bot the scopes
           <code>channels:history</code>, <code>groups:history</code>, <code>channels:read</code>, <code>groups:read</code>,
           <code>users:read</code> and <code>chat:write</code>, then install it to your workspace.</li>
         <li>Invite the bot to the channel it should watch, with <code>/invite @your-bot</code>.</li>
         <li>Paste the Bot User OAuth Token here.
           <div class="row"><input id="slack-token" type="password" placeholder="xoxb-…" autocomplete="off">
           <button class="primary" data-action="connect-slack">Connect</button></div></li>
       </ol>
       <p class="error" id="slack-error"></p>`;
  return `<div class="card">
      <div class="card-head"><span class="logo slack">#</span><div><b>Slack</b><div class="hint">Mentions of the bot, answered in their thread</div></div>
        <span class="badge" data-on="${!!account}">${account ? "Connected" : "Not connected"}</span></div>
      ${body}
    </div>`;
}

function connectorsHtml() {
  const intro = `<h2>Connectors</h2>
    <p class="hint">A connector watches an integration and opens a Claude Code tab on its own when something happens, so the agent starts before you ask.</p>`;
  if (!settings.linear && !settings.slack) {
    return `${intro}<div class="card"><p>Connect Linear or Slack first.</p>
      <div class="actions"><button class="primary" data-action="go-integrations">Go to Integrations</button></div></div>`;
  }
  return `${intro}
    <datalist id="linear-labels">${options(suggestions.labels)}</datalist>
    <datalist id="linear-columns">${options(suggestions.columns)}</datalist>
    <datalist id="linear-teams">${options(suggestions.teams)}</datalist>
    <datalist id="slack-channels">${options(suggestions.channels)}</datalist>
    <datalist id="models">${options(MODELS)}</datalist>
    ${settings.connectors.map(connectorHtml).join("")}
    <div class="actions start"><button class="secondary" data-action="new-connector">＋ New connector</button></div>
    <p class="hint" id="checked">${checkedText()}</p>`;
}

function connectorHtml(c, i) {
  const slack = c.trigger.kind === "slack-mention";
  const proofs = !slack || c.fix !== "off";
  const what = slack
    ? `<input name="channel" list="slack-channels" value="${esc(c.trigger.channel)}" placeholder="support" required>`
    : `<input name="label" list="linear-labels" value="${esc(c.trigger.label)}" placeholder="claude" required>`;
  return `<form class="card connector" data-index="${i}">
    <div class="card-head">
      <input name="name" class="title" value="${esc(c.name)}" placeholder="Connector name" required>
      <label class="toggle"><input type="checkbox" name="enabled"${c.enabled ? " checked" : ""}> On</label>
    </div>
    <div class="grid">
      <label>When
        <span class="row"><select name="kind">${options(Object.keys(TRIGGERS), (t) => TRIGGERS[t], c.trigger.kind)}</select>
        ${what}</span></label>
      <label>Open a session in
        <span class="row"><input name="folder" value="${esc(c.folder)}" placeholder="/path/to/repo" required>
        <button type="button" class="secondary" data-action="pick-folder">Choose…</button></span></label>
      <label>Model <input name="model" list="models" value="${esc(c.model)}" placeholder="claude's default"></label>
      <label>Effort <select name="effort">${options(EFFORTS, undefined, c.effort)}</select></label>
      <label>Permissions <select name="permissionMode">${options(Object.keys(PERMISSIONS), (p) => PERMISSIONS[p], c.permissionMode)}</select></label>
      <label class="toggle"><input type="checkbox" name="worktree"${c.worktree ? " checked" : ""}> Separate git worktree per ${slack ? "thread" : "issue"}</label>
      ${proofs ? `<label class="toggle"><input type="checkbox" name="proof"${c.proof === false ? "" : " checked"}> Require a Playwright video, posted as a comment on the pull request</label>` : ""}
    </div>
    ${proofs ? `<p class="hint">The video is uploaded to the repository's <code>pinned-evidence</code> prerelease and the comment links it, so it opens or downloads instead of playing inline: GitHub has no API for inline video. Nothing is committed to the repository.</p>` : ""}
    ${slack ? issuesHtml(c) : ""}
    <fieldset class="columns">
      <legend>Move the Linear issue to the column</legend>
      ${(slack ? STAGES.filter(([stage]) => stage !== "picked") : STAGES).map(([stage, when]) => `<label>${when}
        <input name="column-${stage}" list="linear-columns" value="${esc(c.columns?.[stage])}" placeholder="leave where it is"></label>`).join("")}
      ${slack ? `<p class="hint">A thread has no Linear issue when pinned picks it up, so there is no first column. These two apply once the session files the issue.</p>` : ""}
    </fieldset>
    <label>Instructions sent before the ${slack ? "thread" : "issue"}
      <textarea name="instructions" rows="8">${esc(c.instructions)}</textarea></label>
    <p class="hint">Runs <code>${esc(commandPreview(c))}</code></p>
    <div class="actions">
      <span class="error"></span>
      <button type="button" class="secondary danger" data-action="delete-connector">Delete</button>
      <button class="primary">${c.id ? "Save" : "Create"}</button>
    </div>
  </form>`;
}

/** The Slack connector's Linear issue settings, which the backend's `create` files issues with. */
function issuesHtml(c) {
  return `<fieldset class="columns">
      <legend>Linear issues</legend>
      <label class="wide">When the thread reports a bug <select name="fix">${options(Object.keys(FIXES), (f) => FIXES[f], c.fix)}</select></label>
      <label>Team <input name="team" list="linear-teams" value="${esc(c.team)}" placeholder="Engineering"${c.fix !== "off" ? " required" : ""}></label>
      <label>Label <input name="issueLabel" list="linear-labels" value="${esc(c.label)}" placeholder="none"></label>
      ${settings.linear ? "" : `<p class="hint">Connect Linear under Integrations to file issues from Slack threads.</p>`}
    </fieldset>`;
}

function commandPreview(c) {
  const parts = ["claude"];
  if (c.model) parts.push(`--model ${c.model}`);
  parts.push(`--effort ${c.effort}`, `--permission-mode ${c.permissionMode}`);
  const subject = c.trigger.kind === "slack-mention" ? "thread" : "issue";
  if (c.worktree) parts.push(`--worktree=<${subject}>`);
  return [...parts, `"<instructions + ${subject}>"`].join(" ");
}

function checkedText() {
  const checked = settings.checked;
  if (!checked) return "Not checked yet. Integrations are checked every minute.";
  const at = new Date(checked.at).toLocaleTimeString();
  return checked.error ? `Last check at ${at} failed: ${checked.error}` : `Last checked at ${at}. Integrations are checked every minute.`;
}

function readConnector(form) {
  const f = form.elements;
  const stored = settings.connectors[form.dataset.index];
  const c = {
    ...stored,
    name: f.name.value.trim(),
    enabled: f.enabled.checked,
    trigger: f.label
      ? { kind: "linear-label", label: f.label.value.trim() }
      : { kind: "slack-mention", channel: f.channel.value.trim() },
    folder: f.folder.value.trim(),
    model: f.model.value.trim(),
    effort: f.effort.value,
    permissionMode: f.permissionMode.value,
    worktree: f.worktree.checked,
    instructions: f.instructions.value,
    // Hidden while a Slack connector only replies, where it keeps the stored choice.
    proof: f.proof?.checked ?? stored.proof ?? true,
    // A Slack form has no picked column, so it keeps the stored one.
    columns: Object.fromEntries(STAGES.map(([stage]) => [stage, f[`column-${stage}`]?.value.trim() ?? stored.columns?.[stage] ?? ""])),
  };
  return f.label ? c : { ...c, team: f.team.value.trim(), label: f.issueLabel.value.trim(), fix: f.fix.value };
}

const DEFAULT_NAMES = { "linear-label": "Resolve labeled Linear issues", "slack-mention": "Triage Slack mentions" };
const defaultText = (c) =>
  c.trigger.kind === "linear-label" ? DEFAULT_INSTRUCTIONS : c.fix === "off" ? SLACK_INSTRUCTIONS : SLACK_FIX_INSTRUCTIONS;

/** Redraws the form after `change` edits what it is, swapping in the new defaults where the old ones were untouched. */
function reshape(form, change) {
  const before = readConnector(form);
  const after = change({ ...before });
  if (before.name === DEFAULT_NAMES[before.trigger.kind]) after.name = DEFAULT_NAMES[after.trigger.kind];
  if (before.instructions.trim() === defaultText(before).trim()) after.instructions = defaultText(after);
  settings.connectors[form.dataset.index] = after;
  showPage("connectors");
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
    suggestions = { ...suggestions, labels: [], columns: [] };
    showPage("integrations");
  },
  async "connect-slack"() {
    const token = document.getElementById("slack-token").value;
    if (!token.trim()) return;
    try {
      settings.slack = await invoke("connect_slack", { token });
      showPage("integrations");
    } catch (e) {
      document.getElementById("slack-error").textContent = e;
    }
  },
  async "disconnect-slack"() {
    await invoke("disconnect_slack");
    settings.slack = null;
    suggestions.channels = [];
    showPage("integrations");
  },
  "new-connector"() {
    const common = { id: "", enabled: true, folder: active?.cwd ?? "", model: "opus", effort: "high", permissionMode: "auto", proof: true };
    settings.connectors.push(
      settings.linear
        ? {
            ...common,
            name: DEFAULT_NAMES["linear-label"],
            trigger: { kind: "linear-label", label: "claude" },
            worktree: true,
            instructions: DEFAULT_INSTRUCTIONS,
            columns: { picked: "Analyzing", coding: "In Dev", review: "Ready to Review" },
          }
        : {
            ...common,
            name: DEFAULT_NAMES["slack-mention"],
            trigger: { kind: "slack-mention", channel: "" },
            worktree: false,
            instructions: SLACK_INSTRUCTIONS,
            columns: { picked: "", coding: "", review: "" },
            team: "",
            label: "",
            fix: "off",
          },
    );
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
  if (e.key === "Enter" && e.target.id === "slack-token") actions["connect-slack"]();
});

settingsPage.addEventListener("change", (e) => {
  if (e.target.name === "kind") {
    const kind = e.target.value;
    reshape(e.target.closest("form"), (c) => ({ ...c, trigger: kind === "slack-mention" ? { kind, channel: "" } : { kind, label: "" } }));
  }
  if (e.target.name === "fix") {
    // Read the form as it was before the change, so its untouched default text is recognised.
    const form = e.target.closest("form");
    const fix = e.target.value;
    e.target.value = settings.connectors[form.dataset.index].fix;
    reshape(form, (c) => ({ ...c, fix }));
  }
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
