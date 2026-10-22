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
  "linear-label": "A Linear issue gets a label",
  "slack-mention": "The bot is @mentioned in Slack",
};

const settingsPage = document.getElementById("settings-page");
const LINEAR_KEYS = "https://linear.app/settings/account/security";

/** Mirrors the backend's `get_settings`. Unsaved connectors carry an empty id. */
let settings = { linear: null, slack: null, connectors: [], checked: null };
const PAGES = ["general", "integrations", "projects", "connectors", "account", "machines"];
let page = PAGES.includes(load("settings-page")) && (TAURI || load("settings-page") !== "general") ? load("settings-page") : "integrations";
/** Names from the connected Linear workspace and Slack channels, for the connector form's suggestions. */
let suggestions = { labels: [], columns: [], teams: [], channels: [], folders: [] };

const settingsOpen = () => document.body.dataset.page === "settings";

const connectorCall = (cmd, args = {}) => (hub ? hub.hub({ cmd, ...args }) : invoke(cmd, args));

async function toggleSettings() {
  if (settingsOpen()) return active ? activate(active) : delete document.body.dataset.page;
  document.body.dataset.page = "settings";
  picker.hidden = true;
  try {
    settings = await connectorCall("get_settings");
  } catch (e) {
    console.warn("get_settings", e);
  }
  showPage(page);
}

function onHubSettings(latest) {
  settings = latest;
  if (!settingsOpen() || page !== "connectors") return;
  if (view.name === "list") return redraw();
  const line = document.getElementById("checked");
  if (line) line.textContent = checkedText();
}

function showPage(name) {
  page = name;
  save("settings-page", name);
  for (const b of document.querySelectorAll("#settings-nav button")) b.classList.toggle("active", b.dataset.page === name);
  if (name === "connectors" && settings.linear && !suggestions.labels.length) {
    connectorCall("linear_suggestions").then((found) => {
      Object.assign(suggestions, found);
      fillList("linear-labels", found.labels);
      fillList("linear-columns", found.columns);
      fillList("linear-teams", found.teams);
    }, console.warn);
  }
  if (name === "connectors" && settings.slack && !suggestions.channels.length) {
    connectorCall("slack_suggestions").then((channels) => {
      suggestions.channels = channels;
      fillList("slack-channels", channels);
    }, console.warn);
  }
  if (name === "projects") return showProjectsPage();
  if (name === "general") return showGeneral();
  if (name === "account") return showAccount();
  if (name === "machines") return showMachinesPage();
  if (name === "integrations") settingsPage.innerHTML = integrationsHtml();
  else redraw();
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


/** What the Connectors page shows: the list, the trigger choice for a new connector, or the flow editor on a draft of one connector. */
let view = { name: "list" };

const clone = (value) => JSON.parse(JSON.stringify(value));
const isSlack = (c) => c.trigger.kind === "slack-mention";
const subject = (c) => (isSlack(c) ? "thread" : "issue");
const folderName = (path) => path.split("/").filter(Boolean).pop() ?? "";

const input = (path, value, attrs = "") => `<input data-bind="${path}" value="${esc(value)}" ${attrs}>`;
const select = (path, values, labelOf, selected) => `<select data-bind="${path}">${options(values, labelOf, selected)}</select>`;
const check = (path, on, text) => `<label class="toggle"><input type="checkbox" data-bind="${path}"${on ? " checked" : ""}> ${text}</label>`;
const switchHtml = (attrs, on) => `<label class="switch" title="On: the connector watches and starts sessions"><input type="checkbox" ${attrs}${on ? " checked" : ""}><span></span></label>`;
const iconHtml = (step) => `<span class="icon" data-tone="${step.tone}" title="${esc(`${step.app}: ${step.title}`)}">${step.icon}</span>`;
const kindHtml = (c) => `<label>Starts when ${select("trigger.kind", Object.keys(TRIGGERS), (t) => TRIGGERS[t], c.trigger.kind)}</label>`;

/** An optional step that moves the Linear issue to a column at `stage`, matching the backend's `Stage`. Off while it names no column. */
const moveStep = (stage, ghost) => ({
  tone: "linear",
  icon: "⇥",
  app: "Linear",
  title: "Move the issue",
  ghost,
  optional: true,
  on: (c) => !!c.columns[stage].trim(),
  remove: (c) => (c.columns[stage] = ""),
  summary: (c) => (c.columns[stage].trim() ? `to “${c.columns[stage].trim()}”` : "Name a column"),
  fields: (c) => `<label>Column ${input(`columns.${stage}`, c.columns[stage], 'list="linear-columns" placeholder="In Dev"')}</label>
    <p class="hint">Matched by name, ignoring case, within the issue's team.${isSlack(c) ? " A thread has no issue until the session files one, so this applies from then on." : ""}</p>`,
});

/** Every step a connector can run. `flowOf` puts them in order; the backend runs them. */
const STEPS = {
  "linear-trigger": {
    tone: "linear",
    icon: "◐",
    app: "Linear",
    title: "Issue gets a label",
    trigger: true,
    summary: (c) => (c.trigger.label.trim() ? `Label “${c.trigger.label.trim()}”` : "Name a label"),
    issues: (c) => [!settings.linear && "Connect Linear under Integrations.", !c.trigger.label.trim() && "Name the label that starts a session."].filter(Boolean),
    fields: (c) => `${kindHtml(c)}
      <label>Label ${input("trigger.label", c.trigger.label, 'list="linear-labels" placeholder="claude"')}</label>
      <p class="hint">pinned checks Linear every minute, only for issues created after the connector is saved, and starts each issue once.</p>`,
  },
  "slack-trigger": {
    tone: "slack",
    icon: "#",
    app: "Slack",
    title: "Bot is @mentioned",
    trigger: true,
    summary: (c) => (c.trigger.channel.trim() ? `in #${c.trigger.channel.trim().replace(/^#/, "")}` : "Name a channel"),
    issues: (c) => [!settings.slack && "Connect Slack under Integrations.", !c.trigger.channel.trim() && "Name the channel to watch."].filter(Boolean),
    fields: (c) => `${kindHtml(c)}
      <label>Channel ${input("trigger.channel", c.trigger.channel, 'list="slack-channels" placeholder="support"')}</label>
      <p class="hint">Invite the bot with <code>/invite @your-bot</code>. Each new thread that mentions it starts one session, checked every minute.</p>`,
  },
  picked: moveStep("picked", "Move the issue when pinned picks it up"),
  agent: {
    tone: "claude",
    icon: "✳",
    app: "Claude Code",
    title: "Run a session",
    summary: (c) =>
      [folderName(c.folder) || "no folder", account && runOnName(c), c.model.trim() || "default model", c.effort, PERMISSIONS[c.permissionMode]]
        .filter(Boolean)
        .join(" · "),
    issues: (c) => (c.folder.trim() ? [] : ["Choose the folder the session opens in."]),
    fields: (c) => `${account ? `<label>Run on ${runOnHtml(c)}</label>` : ""}
      <label>Folder${account ? ` on ${esc(machineName(folderMachine(c)))}` : ""}
        <span class="row">${input("folder", c.folder, 'list="folders" placeholder="/path/to/repo" spellcheck="false"')}
        ${folderMachine(c) === LOCAL ? `<button class="secondary" data-action="pick-folder">Choose…</button>` : ""}</span></label>
      <p class="hint" id="repo">${repoHint(c)}</p>
      <div class="grid">
        <label>Model ${input("model", c.model, `list="models" placeholder="claude's default"`)}</label>
        <label>Effort ${select("effort", EFFORTS, undefined, c.effort)}</label>
        <label>Permissions ${select("permissionMode", Object.keys(PERMISSIONS), (p) => PERMISSIONS[p], c.permissionMode)}</label>
      </div>
      ${check("worktree", c.worktree, `Separate git worktree per ${subject(c)}`)}
      <label>Instructions sent before the ${subject(c)}
        <textarea data-bind="instructions" rows="8">${esc(c.instructions)}</textarea></label>
      <p class="hint">Runs <code id="command">${esc(commandPreview(c))}</code></p>`,
  },
  reply: {
    tone: "slack",
    icon: "↩",
    app: "Slack",
    title: "Reply in the thread",
    summary: () => "Follow-ups reach the same session",
    fields: () => `<p class="hint">The session answers with <code>pinned slack reply</code>, so the bot token stays in pinned. Later messages in the thread are typed into the same tab. If that tab is gone, pinned starts a new session with the whole thread.</p>`,
  },
  bug: {
    tone: "linear",
    icon: "⚑",
    app: "Linear",
    title: "If it reports a bug",
    summary: (c) => FIXES[c.fix],
    issues: (c) =>
      c.fix === "off" ? [] : [!settings.linear && "Connect Linear under Integrations to file issues.", !c.team.trim() && "Name the Linear team issues are filed in."].filter(Boolean),
    fields: (c) => `<label>The session ${select("fix", Object.keys(FIXES), (f) => FIXES[f], c.fix)}</label>
      ${
        c.fix === "off"
          ? `<p class="hint">The session never changes code, and pinned rejects <code>pinned linear create</code>.</p>`
          : `<div class="grid">
               <label>Team ${input("team", c.team, 'list="linear-teams" placeholder="Engineering"')}</label>
               <label>Label ${input("label", c.label, 'list="linear-labels" placeholder="none"')}</label>
             </div>
             <p class="hint">To fix a bug, the session files one Linear issue in this team with a link to the thread, fixes the bug and opens a pull request.</p>`
      }`,
  },
  coding: moveStep("coding", "Move the issue when Claude starts changing code"),
  review: moveStep("review", "Move the issue when the pull request is open"),
  proof: {
    tone: "proof",
    icon: "▶",
    app: "Playwright",
    title: "Prove it works",
    ghost: "Require a proof video on the pull request",
    optional: true,
    on: (c) => c.proof !== false,
    add: (c) => (c.proof = true),
    remove: (c) => (c.proof = false),
    summary: () => "Video posted on the pull request",
    fields: () => `<p class="hint">The session drives the change in a Playwright script, records a video and posts it as a comment on the pull request. The video is uploaded to the repository's <code>pinned-evidence</code> prerelease and the comment links it, so it opens or downloads instead of playing inline: GitHub has no API for inline video. Nothing is committed to the repository.</p>`,
  },
};

/** A connector's steps in run order, each with the event that leads into it. */
function flowOf(c) {
  if (!isSlack(c)) {
    return [["linear-trigger"], ["picked", "new issue"], ["agent", ""], ["coding", "starts changing code"], ["review", "pull request opened"], ["proof", ""]];
  }
  const fixing = c.fix === "off" ? [] : [["coding", "starts the fix"], ["review", "pull request opened"], ["proof", ""]];
  return [["slack-trigger"], ["agent", "new thread"], ["reply", ""], ["bug", ""], ...fixing];
}

const isOn = (c, id) => !STEPS[id].optional || STEPS[id].on(c);
/** Steps the editor draws as nodes: the ones that run, and an optional one the user just opened to fill in. */
const shown = (c, id) => isOn(c, id) || (view.name === "edit" && view.open === id);

const hubIdOf = (machine) => (machine === LOCAL ? account?.machine_id ?? "" : machine);
const localIdOf = (id) => (id && id === account?.machine_id ? LOCAL : id);
const runOnName = (c) => (c.machine ? machineName(localIdOf(c.machine)) : "any machine");

function folderMachine(c) {
  const preferred = localIdOf(c.machine);
  if (preferred && machines.has(preferred)) return preferred;
  return machines.has(LOCAL) ? LOCAL : onlineMachines()[0];
}

function runOnHtml(c) {
  const known = [...machines.entries()].map(([id, m]) => [hubIdOf(id), `${machineName(id)}${m.view.online ? "" : " (offline)"}`]);
  if (c.machine && !known.some(([id]) => id === c.machine)) known.push([c.machine, "A removed machine"]);
  const labels = Object.fromEntries([["", "Any online machine"], ...known]);
  return `${select("machine", Object.keys(labels), (id) => labels[id], c.machine)}
    <span class="hint">${c.machine ? "Falls back to another online machine while this one is offline." : "Each run goes to whichever machine is online."}</span>`;
}

const repoHint = (c) =>
  c.repo ? `A machine without this folder finds or clones <code>${esc(c.repo)}</code>.` : "";

async function loadFolders(c) {
  const machine = folderMachine(c);
  suggestions.folders = [];
  try {
    suggestions.folders = (await callOn(machine, { cmd: "list_projects" })).map((p) => p.path);
  } catch {}
  fillList("folders", suggestions.folders);
}

async function fillRepo() {
  const draft = view.draft;
  const folder = draft.folder.trim();
  let repo = null;
  try {
    repo = folder ? await callOn(folderMachine(draft), { cmd: "git_remote", path: folder }) : null;
  } catch {}
  if (view.draft !== draft || draft.folder.trim() !== folder) return;
  draft.repo = repo ?? "";
  refresh();
}

function connectorsHtml() {
  if (view.name === "edit") return editorHtml();
  if (view.name === "pick") return triggerChoiceHtml();
  const intro = `<h2>Connectors</h2>
    <p class="hint">A connector watches an integration and opens a Claude Code tab on its own when something happens, so the agent starts before you ask.</p>`;
  if (!settings.linear && !settings.slack) {
    return `${intro}<div class="card"><p>Connect Linear or Slack first.</p>
      <div class="actions"><button class="primary" data-action="go-integrations">Go to Integrations</button></div></div>`;
  }
  return `${intro}
    ${settings.connectors.map(cardHtml).join("")}
    <button class="new-card" data-action="new-connector">＋ New connector</button>
    <p class="hint" id="checked">${esc(checkedText())}</p>`;
}

function cardHtml(c, i) {
  const steps = flowOf(c).filter(([id]) => isOn(c, id));
  return `<div class="flow-card" data-index="${i}" data-on="${c.enabled}">
    <button class="open" data-action="edit-connector">
      <span class="chain">${steps.map(([id]) => iconHtml(STEPS[id])).join("<i></i>")}</span>
      <b>${esc(c.name)}</b>
      <span class="hint">${esc(sentence(c))}</span>
    </button>
    ${switchHtml('data-toggle="enabled"', c.enabled)}
  </div>`;
}

function sentence(c) {
  const machine = account && c.machine ? ` on ${runOnName(c)}` : "";
  const where = folderName(c.folder) ? ` in ${folderName(c.folder)}${machine}` : machine;
  if (!isSlack(c)) return `When an issue gets the label “${c.trigger.label}”, Claude picks it up${where}.`;
  const then = c.fix === "off" ? "answers in the thread" : "answers in the thread and fixes bugs";
  return `When the bot is mentioned in #${c.trigger.channel.replace(/^#/, "")}, Claude ${then}${where}.`;
}

const TEMPLATES = [
  { kind: "linear-label", integration: "linear", text: "Claude resolves the issue, opens a pull request and moves the issue through your columns." },
  { kind: "slack-mention", integration: "slack", text: "Claude answers in the thread, and can file a Linear issue and fix the bug." },
];

function triggerChoiceHtml() {
  return `<div class="editor-bar"><button class="secondary" data-action="close-editor" title="Back to connectors">←</button><h2>New connector</h2></div>
    <p class="hint">Pick what starts the agent. You set up the rest of the flow next.</p>
    ${TEMPLATES.map(({ kind, integration, text }) => {
      const step = STEPS[`${integration}-trigger`];
      const connected = settings[integration];
      return `<button class="template" data-action="${connected ? "pick-trigger" : "go-integrations"}" data-kind="${kind}">
        ${iconHtml(step)}
        <span><b>${TRIGGERS[kind]}</b><span class="hint">${connected ? text : `Connect ${step.app} under Integrations first.`}</span></span>
      </button>`;
    }).join("")}`;
}

function editorHtml() {
  const c = view.draft;
  return `<datalist id="linear-labels">${options(suggestions.labels)}</datalist>
    <datalist id="linear-columns">${options(suggestions.columns)}</datalist>
    <datalist id="linear-teams">${options(suggestions.teams)}</datalist>
    <datalist id="slack-channels">${options(suggestions.channels)}</datalist>
    <datalist id="models">${options(MODELS)}</datalist>
    <datalist id="folders">${options(suggestions.folders)}</datalist>
    <div class="editor-bar">
      <button class="secondary" data-action="close-editor" title="Back to connectors">←</button>
      ${input("name", c.name, 'class="title" placeholder="Connector name"')}
      ${switchHtml('data-bind="enabled"', c.enabled)}
      <button class="primary" id="save-connector" data-action="save-connector" title="Save (⌘S)"></button>
    </div>
    <p class="error" id="editor-error"></p>
    <div class="flow">${flowOf(c).map(([id, edge]) => stepHtml(c, id, edge)).join("")}</div>
    ${c.id ? `<div class="actions start"><button class="secondary danger" data-action="delete-connector">Delete connector</button></div>` : ""}`;
}

function stepHtml(c, id, edge) {
  const step = STEPS[id];
  const edgeHtml = edge === undefined ? "" : `<div class="edge">${edge ? `<span>${esc(edge)}</span>` : ""}</div>`;
  if (!shown(c, id)) {
    return `${edgeHtml}<div class="step" data-step="${id}">
      <button class="node ghost" data-action="add-step"><span class="icon">＋</span><span class="text">${esc(step.ghost)}</span></button></div>`;
  }
  const open = view.open === id;
  return `${edgeHtml}<div class="step${open ? " open" : ""}" data-step="${id}">
    <button class="node${step.trigger ? " trigger" : ""}" data-action="toggle-step" aria-expanded="${open}">
      ${iconHtml(step)}
      <span class="text"><span class="app">${step.app}</span><b>${step.title}</b><span class="summary"></span></span>
      <span class="warn" hidden>!</span>
      <span class="chevron">›</span>
    </button>
    ${open ? `<div class="details">${step.fields(c)}${step.optional ? `<div class="actions start"><button class="secondary" data-action="remove-step">Remove step</button></div>` : ""}</div>` : ""}
  </div>`;
}

const changed = () => JSON.stringify(view.draft) !== view.original;

/** Updates the parts of the editor that follow the draft without redrawing it, so the field being typed in keeps focus. */
function refresh() {
  const c = view.draft;
  for (const el of settingsPage.querySelectorAll(".step")) {
    const step = STEPS[el.dataset.step];
    const summary = el.querySelector(".summary");
    if (summary) summary.textContent = step.summary(c);
    const warn = el.querySelector(".warn");
    if (warn) {
      const issues = step.issues?.(c) ?? [];
      warn.hidden = !issues.length;
      warn.title = issues.join(" ");
    }
  }
  const command = document.getElementById("command");
  if (command) command.textContent = commandPreview(c);
  const repo = document.getElementById("repo");
  if (repo) repo.innerHTML = repoHint(c);
  const save = document.getElementById("save-connector");
  save.disabled = !!c.id && !changed();
  save.textContent = !c.id ? "Create" : changed() ? "Save" : "Saved";
}

function redraw() {
  settingsPage.innerHTML = connectorsHtml();
  if (view.name === "edit") refresh();
}

function edit(index, draft = clone(settings.connectors[index]), open = null) {
  view = { name: "edit", index, draft, original: JSON.stringify(draft), open };
  loadFolders(draft);
}

/** A fresh connector of `kind`, filled with the defaults that make it work once a folder is chosen. */
function blank(kind) {
  const machine = active ? hubIdOf(active.machine) : "";
  const common = { id: "", enabled: true, folder: active?.view.cwd ?? "", machine, repo: "", model: "opus", effort: "high", permissionMode: "auto", proof: true, team: "", label: "", fix: "off" };
  return kind === "linear-label"
    ? {
        ...common,
        name: DEFAULT_NAMES[kind],
        trigger: { kind, label: "claude" },
        worktree: true,
        instructions: DEFAULT_INSTRUCTIONS,
        columns: { picked: "Analyzing", coding: "In Dev", review: "Ready to Review" },
      }
    : { ...common, name: DEFAULT_NAMES[kind], trigger: { kind, channel: "" }, worktree: false, instructions: SLACK_INSTRUCTIONS, columns: { picked: "", coding: "", review: "" } };
}

function commandPreview(c) {
  const parts = ["claude"];
  if (c.model) parts.push(`--model ${c.model}`);
  parts.push(`--effort ${c.effort}`, `--permission-mode ${c.permissionMode}`);
  if (c.worktree) parts.push(`--worktree=<${subject(c)}>`);
  return [...parts, `"<instructions + ${subject(c)}>"`].join(" ");
}

function checkedText() {
  const checked = settings.checked;
  if (!checked) return "Not checked yet. Integrations are checked every minute.";
  const at = new Date(checked.at).toLocaleTimeString();
  return checked.error ? `Last check at ${at} failed: ${checked.error}` : `Last checked at ${at}. Integrations are checked every minute.`;
}

/** The draft with its typed names trimmed, as the backend matches them. */
function tidy(c) {
  const t = (s) => (s ?? "").trim();
  return {
    ...c,
    name: t(c.name),
    folder: t(c.folder),
    repo: t(c.repo),
    model: t(c.model),
    team: t(c.team),
    label: t(c.label),
    trigger: isSlack(c) ? { kind: c.trigger.kind, channel: t(c.trigger.channel) } : { kind: c.trigger.kind, label: t(c.trigger.label) },
    columns: Object.fromEntries(Object.entries(c.columns).map(([stage, column]) => [stage, t(column)])),
  };
}

const DEFAULT_NAMES = { "linear-label": "Resolve labeled Linear issues", "slack-mention": "Triage Slack mentions" };
const defaultText = (c) =>
  c.trigger.kind === "linear-label" ? DEFAULT_INSTRUCTIONS : c.fix === "off" ? SLACK_INSTRUCTIONS : SLACK_FIX_INSTRUCTIONS;

/** Fields that change which steps the flow has, so the draft is reshaped and the editor redrawn. */
const RESHAPES = {
  "trigger.kind": (c, kind) => ({ ...c, trigger: kind === "slack-mention" ? { kind, channel: "" } : { kind, label: "" } }),
  fix: (c, fix) => ({ ...c, fix }),
  machine: (c, machine) => ({ ...c, machine }),
};

/** Applies `change` to the draft, swapping in the new defaults where the old ones were untouched. */
function reshape(change) {
  const before = view.draft;
  const after = change(clone(before));
  if (before.name === DEFAULT_NAMES[before.trigger.kind]) after.name = DEFAULT_NAMES[after.trigger.kind];
  if (before.instructions.trim() === defaultText(before).trim()) after.instructions = defaultText(after);
  if (before.trigger.kind !== after.trigger.kind) view.open = flowOf(after)[0][0];
  view.draft = after;
  if (before.machine !== after.machine) {
    loadFolders(after);
    fillRepo();
  }
  redraw();
}

function bind(path, value) {
  const keys = path.split(".");
  const last = keys.pop();
  keys.reduce((o, key) => o[key], view.draft)[last] = value;
}

/** Makes a destructive button take two clicks: the first relabels it, a second within 3 seconds acts. */
function armed(button, text) {
  if ("armed" in button.dataset) return true;
  const label = button.textContent;
  button.dataset.armed = "";
  button.textContent = text;
  setTimeout(() => {
    delete button.dataset.armed;
    button.textContent = label;
  }, 3000);
  return false;
}

function showStep(id, focus) {
  view.open = id;
  redraw();
  const step = settingsPage.querySelector(".step.open");
  step?.scrollIntoView({ block: "nearest" });
  if (focus) step?.querySelector(".details [data-bind]")?.focus();
}

const stepOf = (button) => button.closest(".step").dataset.step;

const actions = {
  "go-integrations": () => showPage("integrations"),
  "open-linear-keys": () => (TAURI ? invoke("open_linear_keys") : window.open(LINEAR_KEYS, "_blank", "noopener")),
  async "connect-linear"() {
    const key = document.getElementById("linear-key").value;
    if (!key.trim()) return;
    try {
      settings.linear = await connectorCall("connect_linear", { key });
      showPage("integrations");
    } catch (e) {
      document.getElementById("linear-error").textContent = e;
    }
  },
  async "disconnect-linear"() {
    await connectorCall("disconnect_linear");
    settings.linear = null;
    suggestions = { ...suggestions, labels: [], columns: [] };
    showPage("integrations");
  },
  async "connect-slack"() {
    const token = document.getElementById("slack-token").value;
    if (!token.trim()) return;
    try {
      settings.slack = await connectorCall("connect_slack", { token });
      showPage("integrations");
    } catch (e) {
      document.getElementById("slack-error").textContent = e;
    }
  },
  async "disconnect-slack"() {
    await connectorCall("disconnect_slack");
    settings.slack = null;
    suggestions.channels = [];
    showPage("integrations");
  },
  "new-connector"() {
    view = { name: "pick" };
    redraw();
  },
  "pick-trigger"(button) {
    const draft = blank(button.dataset.kind);
    edit(-1, draft);
    showStep(flowOf(draft)[0][0], true);
  },
  "edit-connector"(button) {
    edit(Number(button.closest(".flow-card").dataset.index));
    redraw();
  },
  "close-editor"(button) {
    if (view.name === "edit" && (!view.draft.id || changed()) && !armed(button, "Discard changes?")) return;
    view = { name: "list" };
    redraw();
  },
  "toggle-step"(button) {
    const id = stepOf(button);
    showStep(view.open === id ? null : id);
  },
  "add-step"(button) {
    const id = stepOf(button);
    STEPS[id].add?.(view.draft);
    showStep(id, true);
  },
  "remove-step"(button) {
    STEPS[stepOf(button)].remove(view.draft);
    showStep(null);
  },
  async "pick-folder"() {
    const picked = await invoke("pick_folder", { start: view.draft.folder });
    if (!picked) return;
    view.draft.folder = picked;
    redraw();
    fillRepo();
  },
  async "save-connector"() {
    const c = tidy(view.draft);
    const error = document.getElementById("editor-error");
    if (!c.name) {
      error.textContent = "Name the connector.";
      return settingsPage.querySelector(".editor-bar .title").focus();
    }
    const incomplete = flowOf(c).find(([id]) => shown(c, id) && STEPS[id].issues?.(c).length);
    if (incomplete) {
      const [id] = incomplete;
      showStep(id, true);
      document.getElementById("editor-error").textContent = STEPS[id].issues(c)[0];
      return;
    }
    try {
      const saved = await connectorCall("save_connector", { connector: c });
      const index = view.index < 0 ? settings.connectors.push(saved) - 1 : view.index;
      settings.connectors[index] = saved;
      edit(index, clone(saved), view.open);
      redraw();
    } catch (err) {
      error.textContent = err;
    }
  },
  async "delete-connector"(button) {
    if (!armed(button, "Click again to delete")) return;
    await connectorCall("delete_connector", { id: view.draft.id });
    settings.connectors.splice(view.index, 1);
    view = { name: "list" };
    redraw();
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
  if (view.name !== "edit" || page !== "connectors") return;
  if (e.key === "s" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    actions["save-connector"]();
  }
  if (e.key === "Escape" && view.open) showStep(null);
});

settingsPage.addEventListener("input", (e) => {
  const path = e.target.dataset.bind;
  if (!path || path in RESHAPES) return;
  bind(path, e.target.type === "checkbox" ? e.target.checked : e.target.value);
  refresh();
});

settingsPage.addEventListener("change", async (e) => {
  const path = e.target.dataset.bind;
  if (path in RESHAPES) return reshape((c) => RESHAPES[path](c, e.target.value));
  if (path === "folder") return fillRepo();
  if (e.target.dataset.toggle !== "enabled") return;
  // The list's switch saves right away, as the connector's other fields are already saved.
  const i = e.target.closest(".flow-card").dataset.index;
  try {
    settings.connectors[i] = await connectorCall("save_connector", { connector: { ...settings.connectors[i], enabled: e.target.checked } });
  } catch (err) {
    e.target.checked = !e.target.checked;
    document.getElementById("checked").textContent = String(err);
    return;
  }
  e.target.closest(".flow-card").dataset.on = e.target.checked;
});

for (const b of document.querySelectorAll("#settings-nav button")) b.onclick = () => showPage(b.dataset.page);
document.getElementById("settings-toggle").onclick = toggleSettings;

listen("connectors-checked", (payload) => {
  settings.checked = payload;
  const line = document.getElementById("checked");
  if (line) line.textContent = checkedText();
});
