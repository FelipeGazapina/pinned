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
const routedText = (text, how = "Another workflow routed the Slack thread below here.") => text.replace("Someone mentioned you in the Slack thread below.", how);
const DECISION_MODEL = "typesafe-ai/jev";
const DEFAULT_CATEGORIES = [
  { name: "error", description: "Someone reports a bug or an error", workflow: null, mention: null },
  { name: "doubt", description: "Someone asks a question", workflow: null, mention: null },
  {
    name: "escalation",
    description: "Production is down or customers are blocked right now and someone on support must act; not questions, bug reports or feature requests",
    workflow: null,
    mention: "prod-engg",
  },
  { name: "ignore", description: "Anything else", workflow: null, mention: null },
];
const sessionSteps = (c, event) => [["ask", event], ["agent", ""], ["reply", ""], ["bug", ""], ...(c.fix === "off" ? [] : [["coding", "starts the fix"], ["review", "pull request opened"], ["proof", ""]])];
const TRIGGERS = {
  "linear-label": {
    text: "A Linear issue gets a label",
    pitch: "Claude resolves the issue, opens a pull request and moves the issue through your columns.",
    needs: "linear",
    step: "linear-trigger",
    steps: () => [["ask", "new issue"], ["picked", ""], ["agent", ""], ["coding", "starts changing code"], ["review", "pull request opened"], ["proof", ""]],
    subject: "issue",
    name: "Resolve labeled Linear issues",
    defaults: { worktree: true, columns: { picked: "Analyzing", coding: "In Dev", review: "Ready to Review" } },
    trigger: () => ({ kind: "linear-label", label: "claude" }),
    instructions: () => DEFAULT_INSTRUCTIONS,
    sentence: (c, where) => `When an issue gets the label “${c.trigger.label}”, ${who(c)} picks it up${where}.`,
  },
  "slack-mention": {
    text: "The bot is @mentioned in Slack",
    pitch: "Claude answers in the thread, and can file a Linear issue and fix the bug.",
    needs: "slack",
    step: "slack-trigger",
    steps: (c) => sessionSteps(c, "new thread"),
    subject: "thread",
    name: "Triage Slack mentions",
    trigger: (was) => ({ kind: "slack-mention", channel: was?.channel ?? "" }),
    instructions: (c) => (c.fix === "off" ? SLACK_INSTRUCTIONS : SLACK_FIX_INSTRUCTIONS),
    sentence: (c, where, then) => `When the bot is mentioned in #${c.trigger.channel.replace(/^#/, "")}, ${who(c)} ${then}${where}.`,
  },
  "slack-message": {
    text: "A Slack message, sorted by a decision model",
    pitch: "A model on the Vercel AI Gateway puts each new message in a category, and each category pings a group, runs its own workflow or stops.",
    needs: "slack",
    step: "message-trigger",
    steps: () => [["decide", "new message"]],
    subject: "thread",
    name: "Route Slack messages",
    trigger: (was) => ({
      kind: "slack-message",
      channel: was?.channel ?? "",
      decide: {
        model: DECISION_MODEL,
        instructions: "Categorize this Slack message. Choose escalation only for urgent production problems.",
        categories: clone(DEFAULT_CATEGORIES),
        fallback: null,
      },
    }),
    instructions: (c) => TRIGGERS["slack-mention"].instructions(c),
    sentence: (c) =>
      `When someone posts in #${c.trigger.channel.replace(/^#/, "")}, a decision model sorts the message into ${orList(c.trigger.decide.categories.map((cat) => cat.name))}.`,
  },
  called: {
    text: "Called by another workflow",
    pitch: "A Slack message workflow hands it the threads of one category, and Claude answers them as it does a mention.",
    needs: "slack",
    step: "called-trigger",
    steps: (c) => sessionSteps(c, "routed thread"),
    subject: "thread",
    name: "Answer routed Slack threads",
    trigger: () => ({ kind: "called" }),
    instructions: (c) => routedText(TRIGGERS["slack-mention"].instructions(c)),
    sentence: (c, where, then) => `When ${orList(routesTo(c).map((r) => `“${r.router}”`)) || "another workflow"} routes a Slack thread here, ${who(c)} ${then}${where}.`,
  },
};
const orList = (names) => new Intl.ListFormat("en", { type: "disjunction" }).format(names);

const settingsPage = document.getElementById("settings-page");
const LINEAR_KEYS = "https://linear.app/settings/account/security";
const GITHUB_TOKENS = "https://github.com/settings/tokens/new?scopes=repo&description=pinned";
const UPSTASH_CONSOLE = "https://console.upstash.com/";
const VERCEL_KEYS = "https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai-gateway%2Fapi-keys";
/** The Run on choice that gives each issue or thread its own Upstash Box, as the backend's `RunsOn::FreshBox` stores it. */
const BOX = "upstash-box";
const BOX_SIZES = { small: "Small", medium: "Medium", large: "Large" };

/** Mirrors the backend's `get_settings`. Unsaved connectors carry an empty id. */
let settings = { linear: null, slack: null, connectors: [], checked: null };
/** The settings sections in the nav's order, T3 Code's where it has the section. A file adds its own
 * with `settingsSection({ page, label, icon, show, search })`: `show` draws the page into settingsPage,
 * and `search` lists the settings it holds as `{ title, terms, id }`. A result scrolls to the element
 * `id`, or else to the heading or label on the page that reads `title`. */
const SETTINGS_ORDER = ["general", "appearance", "projects", "keybindings", "integrations", "accounts", "connectors", "account", "machines", "archive"];
const settingsSections = [];
function settingsSection(section) {
  const rank = (p) => (SETTINGS_ORDER.includes(p) ? SETTINGS_ORDER.indexOf(p) : SETTINGS_ORDER.length - 1.5);
  const at = settingsSections.findIndex((s) => rank(s.page) > rank(section.page));
  settingsSections.splice(at < 0 ? settingsSections.length : at, 0, section);
}
const settingsSectionOf = (name) => settingsSections.find((s) => s.page === name);
let page = load("settings-page") || "general";
/** Names from the connected Linear workspace and Slack channels, for the connector form's suggestions. */
let suggestions = { labels: [], columns: [], teams: [], channels: [], folders: [], repos: [], models: [] };

const settingsOpen = () => document.body.dataset.page === "settings";

const connectorCall = (cmd, args = {}) => (hub ? hub.hub({ cmd, ...args }) : invoke(cmd, args));

async function toggleSettings() {
  if (settingsOpen()) return active ? activate(active) : delete document.body.dataset.page;
  document.body.dataset.page = "settings";
  picker.hidden = true;
  renderSettingsNav();
  try {
    settings = await connectorCall("get_settings");
  } catch (e) {
    console.warn("get_settings", e);
  }
  showPage(page);
}

function onHubSettings(latest) {
  settings = latest;
  if (!settingsOpen()) return;
  if (page === "integrations") {
    const asks = document.getElementById("box-asks");
    if (asks) asks.innerHTML = boxAskRows();
    return;
  }
  if (page !== "connectors") return;
  if (view.name === "list") return redraw();
  const line = document.getElementById("checked");
  if (line) line.textContent = checkedText();
}

function renderSettingsNav() {
  document.querySelector("#settings-nav .links").innerHTML = settingsSections
    .map((s) => `<button type="button" data-page="${s.page}"${s.page === page ? ' class="active" aria-current="page"' : ""}>${icon(s.icon)}<span>${esc(s.label)}</span></button>`)
    .join("");
}

function showPage(name) {
  const section = settingsSectionOf(name) ?? settingsSections[0];
  name = page = section.page;
  save("settings-page", name);
  delete settingsPage.dataset.view;
  renderSettingsNav();
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
  if (name === "connectors" && settings.github && !suggestions.repos.length) {
    connectorCall("github_repos").then((repos) => {
      suggestions.repos = repos.map((r) => r.full_name);
      fillList("github-repos", suggestions.repos);
    }, console.warn);
  }
  if (name === "connectors" && !suggestions.models.length) {
    connectorCall("vercel_models").then((models) => {
      suggestions.models = models;
      fillVercelModels();
      if (view.name === "edit" && document.getElementById("save-connector")) refresh();
    }, console.warn);
  }
  settingsPage.scrollTop = 0;
  return section.show();
}

function fillList(id, names) {
  const list = document.getElementById(id);
  if (list) list.innerHTML = options(names);
}

function vercelModelOptions(models) {
  return models
    .map((model) => {
      const label = model.kind === "decision" ? `${model.id} · decision` : model.id;
      return `<option value="${esc(model.id)}">${esc(label)}</option>`;
    })
    .join("");
}

function fillVercelModels() {
  const list = document.getElementById("vercel-models");
  if (list) list.innerHTML = vercelModelOptions(suggestions.models);
}

const esc = (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const options = (values, labelOf = (v) => v, selected) =>
  values.map((v) => `<option value="${esc(v)}"${v === selected ? " selected" : ""}>${esc(labelOf(v))}</option>`).join("");

/** A row of chips that picks one of the machines, for pages that show one machine at a time. */
function machineChipsHtml(current, action) {
  const listed = [...machines.entries()].filter(([, m]) => !m.view.upstash_box);
  if (listed.length < 2) return "";
  const chips = listed.map(([id, m]) =>
    `<button class="chip${id === current ? " active" : ""}" data-action="${action}" data-machine="${esc(id)}"${m.view.online ? "" : " disabled"}>${esc(machineName(id))}</button>`);
  return `<div class="chips">${chips.join("")}</div>`;
}

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
  return `<h2>Connectors</h2>
    <p class="hint">Services the harness can read from and act on.</p>
    <div class="card">
      <div class="card-head"><span class="logo" data-tone="linear" data-logo="linear">${LOGOS.linear}</span><div><b>Linear</b><div class="hint">Issues, labels and comments</div></div>
        <span class="badge" data-on="${!!account}">${account ? "Connected" : "Not connected"}</span></div>
      ${body}
    </div>
    ${slackCardHtml()}
    ${githubCardHtml()}
    ${vercelCardHtml()}
    ${upstashCardHtml()}`;
}

function upstashCardHtml() {
  const connected = settings.upstash;
  const body = !account
    ? `<p class="hint">Sign in to pinned to run sessions in Upstash Boxes.</p>`
    : connected
      ? `<p class="connected">Box key <b>${esc(connected.key_hint)}</b> · Claude token <b>${esc(connected.claude_hint)}</b></p>
         ${boxPermissionsHtml()}
         <div class="actions"><span class="hint">Deletes the boxes running now</span><button class="secondary" data-action="disconnect-upstash">Disconnect</button></div>
         <p class="error" id="upstash-error"></p>`
      : `<ol class="steps">
           <li>Create a Box API key in the Box tab of the Upstash console.
             <button class="secondary" data-action="open-upstash-console">Open Upstash ↗</button>
             <div class="row"><input id="upstash-key" type="password" placeholder="box_…" autocomplete="off"></div></li>
           <li>Run <code>claude setup-token</code> in a terminal and paste the token it prints. Boxes then run Claude on your subscription.
             <div class="row"><input id="claude-token" type="password" placeholder="sk-ant-oat01-…" autocomplete="off">
             <button class="primary" data-action="connect-upstash">Connect</button></div></li>
         </ol>
         <p class="error" id="upstash-error"></p>`;
  return `<div class="card">
      <div class="card-head"><span class="logo upstash">▢</span><div><b>Upstash Box</b><div class="hint">Cloud sandboxes for runs. Pick who may open one.</div></div>
        <span class="badge" data-on="${!!connected}">${connected ? "Connected" : "Not connected"}</span></div>
      ${body}
    </div>`;
}

const BOX_MODES = [
  ["off", "Off"],
  ["fresh", "New box each time"],
  ["fixed", "One box, reused"],
  ["ask", "Ask first"],
];
const BOX_KINDS = [
  ["workflow", "Workflows", "Connector runs"],
  ["session", "Sessions", "Plain threads"],
  ["worker", "Workers", "Started by an orchestrator"],
  ["orchestrator", "Orchestrators", "The orchestrator"],
];

function boxAccess() {
  const stored = settings.boxes || {};
  const one = (kind, fallback) => {
    const mode = stored[kind]?.mode || fallback;
    const out = { mode };
    if (mode === "fixed" && stored[kind]?.row) out.row = stored[kind].row;
    return out;
  };
  return {
    workflow: one("workflow", "fresh"),
    session: one("session", "off"),
    worker: one("worker", "off"),
    orchestrator: one("orchestrator", "off"),
  };
}

function boxAskRows() {
  return (settings.box_asks || []).map((ask) => {
    const what = ask.subject.issue ? `issue ${ask.subject.issue}` : `thread ${ask.subject.thread}`;
    return `<p class="hint">Allow a box for ${esc(what)}?
      <button class="primary" data-action="answer-box" data-id="${esc(ask.id)}" data-allow="yes">Yes</button>
      <button class="secondary" data-action="answer-box" data-id="${esc(ask.id)}" data-allow="no">No</button></p>`;
  }).join("");
}

function boxAsksHtml() {
  return `<div id="box-asks">${boxAskRows()}</div>`;
}

function boxPermissionsHtml() {
  const access = boxAccess();
  const rows = BOX_KINDS.map(([kind, label, hint]) => {
    const choices = BOX_MODES.map(([value, name]) => `<option value="${value}"${value === access[kind].mode ? " selected" : ""}>${name}</option>`).join("");
    return `<div class="look"><label for="box-${kind}">${label}</label><p class="meta">${hint}</p><select id="box-${kind}" data-box-kind="${kind}">${choices}</select></div>`;
  }).join("");
  return `<h2 class="looks-title">Who may use a box</h2>${rows}
    <p class="hint">Workflows follow this now. Sessions, workers, and orchestrators keep the choice and follow it in the next update.</p>
    ${boxAsksHtml()}`;
}

function githubCardHtml() {
  const account = settings.github;
  const body = account
    ? `<p class="connected">Connected as <b>@${esc(account.login)}</b>${account.name ? ` (${esc(account.name)})` : ""}.</p>
       <div class="actions"><button class="secondary" data-action="disconnect-github">Disconnect</button></div>`
    : `<ol class="steps">
         <li>Create a classic personal access token with the <code>repo</code> scope.
           <button class="secondary" data-action="open-github-tokens">Open GitHub ↗</button></li>
         <li>Paste the token here.
           <div class="row"><input id="github-token" type="password" placeholder="ghp_…" autocomplete="off">
           <button class="primary" data-action="connect-github">Connect</button></div></li>
       </ol>
       <p class="error" id="github-error"></p>`;
  return `<div class="card">
      <div class="card-head"><span class="logo" data-logo="github">${LOGOS.github}</span><div><b>GitHub</b><div class="hint">Clones your repositories on any machine that lacks them, private ones too</div></div>
        <span class="badge" data-on="${!!account}">${account ? "Connected" : "Not connected"}</span></div>
      ${body}
    </div>`;
}

function vercelCardHtml() {
  const account = settings.vercel;
  const body = account
    ? `<p class="connected">Key <b>${esc(account.key_hint)}</b></p>
       <div class="actions"><button class="secondary" data-action="disconnect-vercel">Disconnect</button></div>`
    : `<ol class="steps">
         <li>Create an AI Gateway API key in your Vercel team.
           <button class="secondary" data-action="open-vercel-keys">Open Vercel ↗</button></li>
         <li>Paste the key here.
           <div class="row"><input id="vercel-key" type="password" placeholder="vck_…" autocomplete="off">
           <button class="primary" data-action="connect-vercel">Connect</button></div></li>
       </ol>
       <p class="error" id="vercel-error"></p>`;
  return `<div class="card">
      <div class="card-head"><span class="logo" data-tone="vercel" data-logo="vercel">${LOGOS.vercel}</span><div><b>Vercel AI Gateway</b><div class="hint">Models that read, classify and sort what a workflow picks up</div></div>
        <span class="badge" data-on="${!!account}">${account ? "Connected" : "Not connected"}</span></div>
      ${body}
    </div>`;
}

function slackCardHtml() {
  const account = settings.slack;
  const body = account
    ? `<p class="connected">Connected as <b>@${esc(account.name)}</b> in ${esc(account.team)}.</p>
       <div class="actions"><button class="secondary" data-action="disconnect-slack">Disconnect</button></div>`
    : `<ol class="steps">
         <li>Create a Slack app at <code>api.slack.com/apps</code>. Under OAuth &amp; Permissions, give its bot the scopes
           <code>channels:history</code>, <code>groups:history</code>, <code>channels:read</code>, <code>groups:read</code>,
           <code>users:read</code>, <code>usergroups:read</code>, <code>chat:write</code> and <code>reactions:write</code>, then install it to your workspace.</li>
         <li>Invite the bot to the channel it should watch, with <code>/invite @your-bot</code>.</li>
         <li>Paste the Bot User OAuth Token here.
           <div class="row"><input id="slack-token" type="password" placeholder="xoxb-…" autocomplete="off">
           <button class="primary" data-action="connect-slack">Connect</button></div></li>
       </ol>
       <p class="error" id="slack-error"></p>`;
  return `<div class="card">
      <div class="card-head"><span class="logo" data-logo="slack">${LOGOS.slack}</span><div><b>Slack</b><div class="hint">Mentions of the bot, answered in their thread</div></div>
        <span class="badge" data-on="${!!account}">${account ? "Connected" : "Not connected"}</span></div>
      ${body}
    </div>`;
}


/** What the Workflows page shows: the list, the trigger choice for a new workflow, or the flow editor on a draft of one workflow. */
let view = { name: "list" };

const clone = (value) => JSON.parse(JSON.stringify(value));
const subject = (c) => TRIGGERS[c.trigger.kind].subject;
const folderName = (path) => path.split(/[\\/]/).filter(Boolean).pop() ?? "";

const input = (path, value, attrs = "") => `<input data-bind="${path}" value="${esc(value)}" ${attrs}>`;
const select = (path, values, labelOf, selected) => `<select data-bind="${path}">${options(values, labelOf, selected)}</select>`;
const check = (path, on, text) => `<label class="toggle"><input type="checkbox" data-bind="${path}"${on ? " checked" : ""}> ${text}</label>`;
const switchHtml = (attrs, on) => `<label class="switch" title="On: the workflow watches and starts sessions"><input type="checkbox" ${attrs}${on ? " checked" : ""}><span></span></label>`;
const SLACK_MARK = [
  ["red", "M5.042 15.165a2.528 2.528 0 0 1-2.52 2.523A2.528 2.528 0 0 1 0 15.165a2.527 2.527 0 0 1 2.522-2.52h2.52v2.52zm1.271 0a2.527 2.527 0 0 1 2.521-2.52 2.527 2.527 0 0 1 2.521 2.52v6.313A2.528 2.528 0 0 1 8.834 24a2.528 2.528 0 0 1-2.521-2.522v-6.313z"],
  ["blue", "M8.834 5.042a2.528 2.528 0 0 1-2.521-2.52A2.528 2.528 0 0 1 8.834 0a2.528 2.528 0 0 1 2.521 2.522v2.52H8.834zm0 1.271a2.528 2.528 0 0 1 2.521 2.521 2.528 2.528 0 0 1-2.521 2.521H2.522A2.528 2.528 0 0 1 0 8.834a2.528 2.528 0 0 1 2.522-2.521h6.312z"],
  ["green", "M18.956 8.834a2.528 2.528 0 0 1 2.522-2.521A2.528 2.528 0 0 1 24 8.834a2.528 2.528 0 0 1-2.522 2.521h-2.522V8.834zm-1.268 0a2.528 2.528 0 0 1-2.523 2.521 2.527 2.527 0 0 1-2.52-2.521V2.522A2.527 2.527 0 0 1 15.165 0a2.528 2.528 0 0 1 2.523 2.522v6.312z"],
  ["yellow", "M15.165 18.956a2.528 2.528 0 0 1 2.523 2.522A2.528 2.528 0 0 1 15.165 24a2.527 2.527 0 0 1-2.52-2.522v-2.522h2.52zm0-1.268a2.527 2.527 0 0 1-2.52-2.523 2.526 2.526 0 0 1 2.52-2.52h6.313A2.527 2.527 0 0 1 24 15.165a2.528 2.528 0 0 1-2.522 2.523h-6.313z"],
];
const CLAUDE_RAYS = Array.from({ length: 12 }, (_, i) => {
  const a = (i * Math.PI) / 6 + 0.2;
  const r = i % 2 ? 8.5 : 11;
  return `M${(12 + 2.6 * Math.cos(a)).toFixed(2)} ${(12 + 2.6 * Math.sin(a)).toFixed(2)}L${(12 + r * Math.cos(a)).toFixed(2)} ${(12 + r * Math.sin(a)).toFixed(2)}`;
}).join("");
const LOGOS = {
  slack: `<svg viewBox="0 0 24 24">${SLACK_MARK.map(([part, d]) => `<path class="${part}" d="${d}"/>`).join("")}</svg>`,
  linear: `<svg viewBox="0 0 24 24"><path d="M2.886 4.18A11.982 11.982 0 0 1 11.99 0C18.624 0 24 5.376 24 12.009c0 3.64-1.62 6.903-4.18 9.105L2.887 4.18ZM1.817 5.626l16.556 16.556c-.524.33-1.075.62-1.65.866L.951 7.277c.247-.575.537-1.126.866-1.65ZM.322 9.163l14.515 14.515c-.71.172-1.443.282-2.195.322L0 11.358a12 12 0 0 1 .322-2.195Zm-.17 4.862 9.823 9.824a12.02 12.02 0 0 1-9.824-9.824Z"/></svg>`,
  vercel: `<svg viewBox="0 0 24 24"><path d="M12 2.5 23 21.5H1z"/></svg>`,
  claude: `<svg viewBox="0 0 24 24"><path d="${CLAUDE_RAYS}"/></svg>`,
  github: `<svg viewBox="0 0 24 24"><path d="M12 .3a12 12 0 0 0-3.8 23.4c.6.1.8-.3.8-.6v-2c-3.3.7-4-1.6-4-1.6-.6-1.4-1.4-1.8-1.4-1.8-1-.7.1-.7.1-.7 1.2.1 1.8 1.2 1.8 1.2 1 1.8 2.8 1.3 3.5 1 0-.8.4-1.3.7-1.6-2.7-.3-5.5-1.3-5.5-6 0-1.2.5-2.3 1.3-3.1-.2-.4-.6-1.6 0-3.2 0 0 1-.3 3.4 1.2a11.5 11.5 0 0 1 6 0c2.3-1.5 3.3-1.2 3.3-1.2.6 1.6.2 2.8 0 3.2.9.8 1.3 1.9 1.3 3.2 0 4.6-2.8 5.6-5.5 5.9.5.4.9 1 .9 2.2v3.3c0 .3.1.7.8.6A12 12 0 0 0 12 .3"/></svg>`,
};
const iconHtml = (step) =>
  `<span class="icon" data-tone="${step.tone}"${LOGOS[step.tone] ? ` data-logo="${step.tone}"` : ""} title="${esc(`${step.app}: ${step.title}`)}">${LOGOS[step.tone] ?? step.icon}</span>`;
const kindHtml = (c) => `<label>Starts when ${select("trigger.kind", Object.keys(TRIGGERS), (t) => TRIGGERS[t].text, c.trigger.kind)}</label>`;

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
    <p class="hint">Matched by name, ignoring case, within the issue's team.${subject(c) === "thread" ? " A thread has no issue until the session files one, so this applies from then on." : ""}</p>`,
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
    issues: (c) => [!settings.linear && "Connect Linear under Connectors.", !c.trigger.label.trim() && "Name the label that starts a session."].filter(Boolean),
    fields: (c) => `${kindHtml(c)}
      <label>Label ${input("trigger.label", c.trigger.label, 'list="linear-labels" placeholder="claude"')}</label>
      <p class="hint">pinned checks Linear every minute, only for issues created after the workflow is saved, and starts each issue once.</p>`,
  },
  "slack-trigger": {
    tone: "slack",
    icon: "#",
    app: "Slack",
    title: "Bot is @mentioned",
    trigger: true,
    summary: (c) => (c.trigger.channel.trim() ? `in #${c.trigger.channel.trim().replace(/^#/, "")}` : "Name a channel"),
    issues: (c) => [!settings.slack && "Connect Slack under Connectors.", !c.trigger.channel.trim() && "Name the channel to watch."].filter(Boolean),
    fields: (c) => `${kindHtml(c)}
      <label>Channel ${input("trigger.channel", c.trigger.channel, 'list="slack-channels" placeholder="support"')}</label>
      <p class="hint">Invite the bot with <code>/invite @your-bot</code>. Each new thread that mentions it starts one session, checked every minute.</p>`,
  },
  "message-trigger": {
    tone: "slack",
    icon: "#",
    app: "Slack",
    title: "Message in channel",
    trigger: true,
    summary: (c) => (c.trigger.channel.trim() ? `in #${c.trigger.channel.trim().replace(/^#/, "")}` : "Name a channel"),
    issues: (c) => [!settings.slack && "Connect Slack under Connectors.", !c.trigger.channel.trim() && "Name the channel to watch."].filter(Boolean),
    fields: (c) => `${kindHtml(c)}
      <label>Channel ${input("trigger.channel", c.trigger.channel, 'list="slack-channels" placeholder="support"')}</label>
      <p class="hint">Invite the bot with <code>/invite @your-bot</code>. Every new message a person posts in the channel is sorted once, checked every minute. Thread replies and bots are skipped.</p>`,
  },
  "called-trigger": {
    tone: "called",
    icon: "↳",
    app: "Workflow",
    title: "Called by another workflow",
    trigger: true,
    summary: (c) => (routesTo(c).length ? `from ${[...new Set(routesTo(c).map((r) => r.router))].join(", ")}` : "No workflow routes here yet"),
    issues: () => [!settings.slack && "Connect Slack under Connectors."].filter(Boolean),
    fields: (c) => {
      const from = routesTo(c).map((r) => `“${esc(r.router)}” sorts a message into “${esc(r.category)}”`);
      return `${kindHtml(c)}
        <p class="hint">${
          from.length
            ? `Runs when ${orList(from)}. Claude answers in that message's thread as it does a mention.`
            : "No workflow routes threads here yet. In a Slack message workflow, choose this one as a category's route."
        }</p>`;
    },
  },
  decide: {
    tone: "vercel",
    icon: "◆",
    app: "Vercel AI Gateway",
    title: "Decide",
    summary: (c) => c.trigger.decide.model.trim() || "Name a model",
    issues: (c) => {
      const categories = c.trigger.decide.categories;
      const names = categories.map((cat) => cat.name.trim().toLowerCase());
      const twice = categories.find((cat, i) => names[i] && names.indexOf(names[i]) !== i);
      const vague = categories.find((cat) => cat.name.trim() && !cat.description.trim());
      const removed = categories.find((cat) => cat.workflow && !settings.connectors.some((k) => k.id === cat.workflow));
      const deaf = categories.find((cat) => !answersThreads(settings.connectors.find((k) => k.id === cat.workflow) ?? { trigger: { kind: "called" } }));
      const fallback = fallbackOf(c.trigger.decide);
      return [
        !settings.vercel && "Connect the Vercel AI Gateway under Connectors.",
        !c.trigger.decide.model.trim() && "Name the decision model.",
        suggestions.models.some((m) => m.id === c.trigger.decide.model.trim() && m.kind !== "decision") && `${c.trigger.decide.model.trim()} is a chat model: the Decide step needs a decision model.`,
        categories.length < 2 && "Add at least two categories.",
        names.some((name) => !name) && "Name every category.",
        twice && `Two categories are named “${twice.name.trim()}”.`,
        vague && `Describe the category “${vague.name.trim()}”.`,
        removed && `The workflow that “${removed.name.trim()}” routes to was deleted.`,
        deaf && `“${deaf.name.trim()}” routes to a workflow that can't answer a Slack thread.`,
        fallback?.mention?.trim() && `The fallback can't ping a group, so saving sets it back to “${BEST_GUESS}”.`,
      ].filter(Boolean);
    },
    fields: (c) => decideHtml(c),
    branches: (c) => c.trigger.decide.categories.map((cat, i) => ({ label: cat.name.trim() || "unnamed", nodes: branchNodes(cat, i), end: cat.workflow ? undefined : i })),
    stub: "＋ Add category",
  },
  picked: moveStep("picked", "Move the issue when pinned picks it up"),
  agent: {
    tone: "claude",
    icon: "✳",
    app: "Claude Code",
    title: "Run a session",
    summary: (c) =>
      [
        isBox(c) ? folderName(c.repo.replace(/\.git$/, "")) || "no repository" : folderName(c.folder) || folderName(c.repo.replace(/\.git$/, "")) || "no folder",
        (account || isBox(c)) && runOnName(c),
        connectorAccountName(c),
        c.model.trim() || "default model",
        c.effort,
        PERMISSIONS[c.permissionMode],
      ]
        .filter(Boolean)
        .join(" · "),
    issues: (c) => (isBox(c) ? boxIssues(c) : [...(c.folder.trim() || c.repo.trim() ? [] : ["Choose the folder or the repository the session opens in."]), ...codexProjectIssues(c)]),
    fields: (c) => `${account || isBox(c) ? `<label>Run on ${runOnHtml(c)}</label>` : ""}
      ${
        isBox(c)
          ? `<label>Size ${select("machine.size", Object.keys(BOX_SIZES), (size) => BOX_SIZES[size], c.machine.size)}</label>`
          : `<label>Folder${account ? ` on ${esc(machineName(folderMachine(c)))}` : ""}
        <span class="row">${input("folder", c.folder, 'list="folders" placeholder="/path/to/repo" spellcheck="false"')}
        ${folderMachine(c) === LOCAL ? `<button class="secondary" data-action="pick-folder">Choose…</button>` : ""}</span></label>`
      }
      <label>Repository ${input("repo", c.repo, `list="github-repos" placeholder="${settings.github ? "owner/name, from your GitHub" : "git URL"}" spellcheck="false"`)}</label>
      <p class="hint" id="repo">${repoHint(c)}</p>
      <div class="grid">
        <label>Model ${input("model", c.model, `list="models" placeholder="claude's default"`)}</label>
        <label>Effort ${select("effort", EFFORTS, undefined, c.effort)}</label>
        <label>Permissions ${select("permissionMode", Object.keys(PERMISSIONS), (p) => PERMISSIONS[p], c.permissionMode)}</label>
        ${isBox(c) ? "" : `<label>Account ${accountSelectHtml(c)}</label>`}
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
      c.fix === "off" ? [] : [!settings.linear && "Connect Linear under Connectors to file issues.", !c.team.trim() && "Name the Linear team issues are filed in."].filter(Boolean),
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
  ask: {
    tone: "vercel",
    icon: "▲",
    app: "Vercel",
    title: "Ask a model",
    ghost: "Ask a model before the session starts",
    optional: true,
    on: (c) => !!c.ask,
    add: (c) =>
      (c.ask = {
        model: "typesafe-ai/jev",
        instructions: "Classify the request below for an engineering team. Answer from the text only.",
        questions: [
          { name: "kind", ask: "What does it ask for?", choices: ["bug", "question", "critique", "enhancement"], stopOn: [] },
          { name: "agent", ask: "Can a coding agent handle it on its own, without a person deciding first?", choices: ["yes", "no"], stopOn: ["no"] },
        ],
      }),
    remove: (c) => (c.ask = null),
    summary: (c) => {
      const ask = c.ask;
      if (!ask) return "";
      const questions = ask.questions ?? [];
      const n = questions.length;
      const stops = questions
        .filter((q) => (q.stopOn ?? []).some((s) => s.trim()))
        .map((q) => `stops on ${q.name.trim() || "?"} = ${(q.stopOn ?? []).map((s) => s.trim()).filter(Boolean).join(", ")}`);
      return [`${ask.model.trim() || "no model"} · ${n} question${n === 1 ? "" : "s"}`, ...stops].join(" · ");
    },
    issues: (c) => {
      const ask = c.ask;
      if (!ask) return [];
      const questions = ask.questions ?? [];
      const model = ask.model.trim();
      const decision = suggestions.models.some((m) => m.id === model && m.kind === "decision");
      const bare = decision && questions.some((q) => !(q.choices ?? []).some((choice) => choice.trim()));
      const stray = questions.flatMap((q) =>
        (q.stopOn ?? [])
          .map((s) => s.trim())
          .filter(Boolean)
          .filter((s) => !(q.choices ?? []).some((choice) => choice.trim().toLowerCase() === s.toLowerCase()))
          .map((s) => `“${s}” is not a choice for ${q.name.trim() || "a question"}.`),
      );
      return [
        !settings.vercel && "Connect Vercel AI Gateway under Connectors.",
        !ask.model.trim() && "Name the model.",
        questions.some((q) => !q.name.trim()) && "Name each question.",
        questions.some((q) => !q.ask.trim()) && "Write what each question asks.",
        bare && `${model} is a decision model: give every question choices.`,
        ...stray,
      ].filter(Boolean);
    },
    fields: (c) => {
      const ask = c.ask;
      const rows = (ask?.questions ?? [])
        .map(
          (q, i) => `<div class="grid">
          <label>Name ${input(`ask.questions.${i}.name`, q.name, 'placeholder="kind" spellcheck="false"')}</label>
          <label>Question ${input(`ask.questions.${i}.ask`, q.ask, 'placeholder="What does it ask for?"')}</label>
        </div>
        <div class="grid">
          <label>Choices <input data-bind="ask.questions.${i}.choices" data-list="" value="${esc((q.choices ?? []).join(", "))}" placeholder="bug, question"></label>
          <label>Stop on <input data-bind="ask.questions.${i}.stopOn" data-list="" value="${esc((q.stopOn ?? []).join(", "))}" placeholder="none"></label>
        </div>
        <div class="actions start"><button class="secondary" data-action="remove-question" data-index="${i}">Remove</button></div>`,
        )
        .join("");
      return `<label>Model ${input("ask.model", ask?.model ?? "", 'list="vercel-models" placeholder="typesafe-ai/jev" spellcheck="false"')}</label>
        <label>Instructions
          <textarea data-bind="ask.instructions" rows="4">${esc(ask?.instructions ?? "")}</textarea></label>
        ${rows}
        <div class="actions start"><button class="secondary" data-action="add-question">Add question</button></div>`;
    },
  },
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
const flowOf = (c) => [[TRIGGERS[c.trigger.kind].step], ...TRIGGERS[c.trigger.kind].steps(c)];

const routesTo = (c) =>
  settings.connectors.flatMap((r) =>
    r.trigger.kind === "slack-message" && c.id ? r.trigger.decide.categories.filter((cat) => cat.workflow === c.id).map((cat) => ({ router: r.name, category: cat.name })) : [],
  );

function branchNodes(category, i) {
  const mention = (category.mention ?? "").trim().replace(/^@/, "");
  const nodes = mention ? [{ kind: "mention", category: i, tone: "slack", icon: "@", app: "Slack", title: `Mention @${mention}`, summary: "Pings the group" }] : [];
  const target = category.workflow && settings.connectors.find((k) => k.id === category.workflow);
  if (target) {
    const trigger = TRIGGERS[target.trigger.kind];
    const { tone, icon } = STEPS[trigger.step];
    nodes.push({ kind: "route", category: i, tone, icon, app: "Workflow", title: target.name, summary: trigger.text });
  } else if (category.workflow) {
    nodes.push({ kind: "route", category: i, tone: "", icon: "?", app: "Workflow", title: "A deleted workflow", summary: "Choose another route" });
  } else if (!mention) {
    nodes.push({ kind: "stop", category: i, tone: "", icon: "■", app: "", title: "Stop", summary: "Nothing else happens" });
  }
  return nodes;
}

const NEW_ROUTE = "＋new";

/** A route's target must answer the thread it is handed, as the backend's `Trigger::answers_threads`. */
const answersThreads = (k) => ["slack-mention", "called"].includes(k.trigger.kind);
const routeName = (id) => {
  const target = settings.connectors.find((k) => k.id === id);
  return target ? `${target.name} (can't answer threads)` : "A deleted workflow";
};

function decideHtml(c) {
  const decide = c.trigger.decide;
  const targets = settings.connectors.filter((k) => k.id !== c.id && answersThreads(k));
  const row = (cat, i) => {
    const at = `trigger.decide.categories.${i}`;
    const routes = Object.fromEntries([
      ["", "Stop"],
      ...targets.map((k) => [k.id, k.name]),
      ...(cat.workflow && !targets.some((k) => k.id === cat.workflow) ? [[cat.workflow, routeName(cat.workflow)]] : []),
      [NEW_ROUTE, "＋ New workflow for this category"],
    ]);
    return `<div class="category" data-category="${i}">
      <div class="row">${input(`${at}.name`, cat.name, 'class="name" placeholder="Category" aria-label="Category name" spellcheck="false"')}
        <button class="secondary" data-action="remove-category" title="Remove this category">✕</button></div>
      <textarea data-bind="${at}.description" rows="2" placeholder="What a message in it looks like" aria-label="Description">${esc(cat.description)}</textarea>
      <div class="grid">
        <label>Mention ${input(`${at}.mention`, cat.mention ?? "", 'placeholder="No one" spellcheck="false"')}</label>
        <label>Then ${select(`${at}.workflow`, Object.keys(routes), (id) => routes[id], cat.workflow ?? "")}</label>
      </div>
      <p class="hint mention-hint">Pings only when the model is at least 85% sure, once per thread, and at most once every 30 minutes per channel.</p>
    </div>`;
  };
  return `<label>Model ${input("trigger.decide.model", decide.model, `list="vercel-models" placeholder="${DECISION_MODEL}" spellcheck="false"`)}</label>
    <label>Instructions <textarea data-bind="trigger.decide.instructions" rows="3">${esc(decide.instructions)}</textarea></label>
    <div class="categories">${decide.categories.map(row).join("")}</div>
    <div class="actions start"><button class="secondary" data-action="add-category">＋ Add category</button></div>
    <label>When the model is less than 60% sure ${fallbackHtml(decide)}</label>
    <p class="hint">The model puts each message in one category, judged by its description. A mention pings a Slack user group, by its handle or ID, in the message's thread. A route hands the thread to a Slack mention or called workflow, which answers it there. When the model is unsure, the fallback category takes the message instead of its guess.</p>`;
}

const BEST_GUESS = "Use its best guess";
const fallbackOf = (decide) => (decide.fallback ? decide.categories.find((cat) => cat.name.trim() === decide.fallback) : undefined);

function fallbackHtml(decide) {
  const pingsNoOne = (cat) => !cat.mention?.trim();
  const chosen = (cat) => cat === fallbackOf(decide);
  const names = decide.categories.filter((cat) => cat.name.trim() && (pingsNoOne(cat) || chosen(cat))).map((cat) => cat.name.trim());
  return select("trigger.decide.fallback", ["", ...new Set(names)], (name) => name || BEST_GUESS, decide.fallback ?? "");
}

function routePanelHtml(c, i) {
  const category = c.trigger.decide.categories[i];
  const target = settings.connectors.find((k) => k.id === category?.workflow);
  if (!target) return "";
  return `<aside class="node-panel" aria-label="${esc(target.name)}">
    <div class="node-panel-head">${iconHtml(STEPS[TRIGGERS[target.trigger.kind].step])}<span class="text"><span class="app">Workflow</span><b>${esc(target.name)}</b></span>
      <button class="secondary close" data-action="close-step" title="Close (Esc)">✕</button></div>
    <div class="details">
      <p class="hint">Messages the decision model puts in “${esc(category.name.trim())}” go to this workflow${category.mention?.trim() ? " after the mention" : ""}, which answers in their thread.</p>
      <div class="actions start"><button class="primary" data-action="open-route" data-id="${esc(target.id)}">Open “${esc(target.name)}”</button></div>
      <p class="hint">Change the route in the Decide step.</p>
    </div>
  </aside>`;
}

const isOn = (c, id) => !STEPS[id].optional || STEPS[id].on(c);
/** Steps the editor draws as nodes: the ones that run, and an optional one the user just opened to fill in. */
const shown = (c, id) => isOn(c, id) || (view.name === "edit" && view.open === id);

const hubIdOf = (machine) => (machine === LOCAL ? account?.machine_id ?? "" : machine);
const localIdOf = (id) => (id && id === account?.machine_id ? LOCAL : id);
/** A connector's `machine` is a machine id, "" for any, or an object for a box per issue. */
const isBox = (c) => c.machine?.kind === BOX;
const runOnName = (c) => (isBox(c) ? `Upstash Box (${c.machine.size})` : c.machine ? machineName(localIdOf(c.machine)) : "any machine");

function folderMachine(c) {
  const preferred = isBox(c) ? "" : localIdOf(c.machine);
  if (preferred && machines.has(preferred)) return preferred;
  return machines.has(LOCAL) ? LOCAL : onlineMachines()[0];
}

function runOnHtml(c) {
  const known = [...machines.entries()].filter(([, m]) => !m.view.upstash_box).map(([id, m]) => [hubIdOf(id), `${machineName(id)}${m.view.online ? "" : " (offline)"}`]);
  if (c.machine && !isBox(c) && !known.some(([id]) => id === c.machine)) known.push([c.machine, "A removed machine"]);
  const labels = Object.fromEntries([["", "Any online machine"], ...known, [BOX, `A fresh Upstash Box per ${subject(c)}`]]);
  if (isBox(c)) {
    return `${select("machine", Object.keys(labels), (id) => labels[id], BOX)}
      <span class="hint">Each ${subject(c)} gets its own box. pinned deletes it 30 minutes after the session goes quiet, or when you close its tab, and after 4 hours at most.</span>`;
  }
  return `${select("machine", Object.keys(labels), (id) => labels[id], c.machine)}
    <span class="hint">${c.machine ? "Falls back to another online machine while this one is offline." : "Each run goes to whichever machine is online."}</span>`;
}

/** What a box run needs: the hub, which runs boxes, Upstash, and the repository the box clones and pushes with GitHub. */
function boxIssues(c) {
  if (!account) return ["Sign in to pinned to run sessions in Upstash Boxes."];
  return [
    !settings.upstash && "Connect Upstash Box under Connectors.",
    !c.repo.trim() && "Name the repository the box clones.",
    !settings.github && "Connect GitHub under Connectors: the box clones, pushes and opens the pull request with it.",
  ].filter(Boolean);
}

/** A run under the project's account fails when that account is Codex, which can't run a workflow. */
function codexProjectIssues(c) {
  if (c.account || isBox(c)) return [];
  const project = suggestions.projects?.find((p) => p.path === c.folder.trim());
  const account = project && accountOf(folderMachine(c), project.account);
  return account?.provider === "codex" ? [`${folderName(c.folder)} runs under ${account.name}, a Codex account, which can't run a workflow. Pick a Claude Code account under Account.`] : [];
}

const connectorAccountName = (c) => !isBox(c) && c.account && accountName(folderMachine(c), c.account);

/** The Claude Code accounts of the connector's machine; Codex can't run a connector, whose prompts rely on Claude Code's hooks. */
function accountSelectHtml(c) {
  const machine = folderMachine(c);
  const ids = ["", ...(machines.get(machine)?.accounts ?? []).filter((a) => a.provider === "claude").map((a) => a.id)];
  if (c.account && !ids.includes(c.account)) ids.push(c.account);
  const label = (id) => (!id ? "Project's account" : accountOf(machine, id)?.name ?? `${id} (not on ${machineName(machine)})`);
  return select("account", ids, label, c.account);
}

const repoHint = (c) =>
  isBox(c)
    ? `The box clones ${c.repo.trim() ? `<code>${esc(c.repo.trim())}</code>` : "the repository"} into <code>~/pinned</code> and pushes${settings.github ? " with your GitHub token" : ""}.`
    : !c.repo.trim()
    ? "A machine without the folder can't run this session."
    : `A machine without ${c.folder.trim() ? "this folder" : "a clone"} clones <code>${esc(c.repo.trim())}</code> into <code>~/pinned</code>${settings.github ? " with your GitHub token" : ""}.`;

async function newRoute(select) {
  const router = view.draft;
  const category = router.trigger.decide.categories[Number(select.closest("[data-category]").dataset.category)];
  const name = category.name.trim() || "a category";
  const draft = blank("called");
  draft.name = `${router.name.trim() || TRIGGERS["slack-message"].name}: ${name}`;
  draft.instructions = routedText(TRIGGERS["slack-mention"].instructions(draft), `A decision model sorted the Slack thread below into “${name}” and routed it here.`);
  try {
    const saved = await connectorCall("save_connector", { connector: tidy(draft) });
    if (!settings.connectors.some((k) => k.id === saved.id)) settings.connectors.push(saved);
    category.workflow = saved.id;
    redraw();
  } catch (err) {
    redraw();
    document.getElementById("editor-error").textContent = err;
  }
}

async function loadFolders(c) {
  const machine = folderMachine(c);
  suggestions.folders = [];
  suggestions.projects = [];
  try {
    suggestions.projects = await callOn(machine, { cmd: "list_projects" });
    suggestions.folders = suggestions.projects.map((p) => p.path);
  } catch {}
  fillList("folders", suggestions.folders);
  if (machine && !machines.get(machine)?.accounts) {
    await accountsOf(machine).catch(console.warn);
    if (page === "connectors" && view.name === "edit" && view.draft === c) redraw();
  }
}

async function fillRepo() {
  const draft = view.draft;
  const folder = draft.folder.trim();
  let repo = null;
  try {
    repo = folder ? await callOn(folderMachine(draft), { cmd: "git_remote", path: folder }) : null;
  } catch {}
  if (view.draft !== draft || draft.folder.trim() !== folder) return;
  if (repo) draft.repo = repo;
  redraw();
}

function connectorsHtml() {
  if (view.name === "edit") return editorHtml();
  if (view.name === "pick") return triggerChoiceHtml();
  const intro = `<h2>Workflows</h2>
    <p class="hint">A workflow watches a connector and opens a Claude Code tab on its own when something happens, so the agent starts before you ask.</p>`;
  if (!settings.linear && !settings.slack) {
    return `${intro}<div class="card"><p>Connect Linear or Slack first.</p>
      <div class="actions"><button class="primary" data-action="go-integrations">Go to Connectors</button></div></div>`;
  }
  return `${intro}
    ${settings.connectors.map(cardHtml).join("")}
    <button class="new-card" data-action="new-connector">＋ New workflow</button>
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

const who = (c) => (c.ask ? `${c.ask.model.trim() || "a model"} classifies it, then Claude` : "Claude");

function sentence(c) {
  const machine = (account || isBox(c)) && c.machine ? ` on ${isBox(c) ? "a fresh Upstash Box" : runOnName(c)}` : "";
  const as = connectorAccountName(c) ? ` with the ${connectorAccountName(c)} account` : "";
  const folder = isBox(c) ? folderName(c.repo.replace(/\.git$/, "")) : folderName(c.folder);
  const where = (folder ? ` in ${folder}${machine}` : machine) + as;
  const then = c.fix === "off" ? "answers in the thread" : "answers in the thread and fixes bugs";
  return TRIGGERS[c.trigger.kind].sentence(c, where, then);
}

function triggerChoiceHtml() {
  return `<div class="editor-bar"><button class="secondary" data-action="close-editor" title="Back to workflows">←</button><h2>New workflow</h2></div>
    <p class="hint">Pick what starts the agent. You set up the rest of the flow next.</p>
    ${Object.entries(TRIGGERS)
      .map(([kind, { text, pitch, needs, step }]) => {
        const connected = settings[needs];
        return `<button class="template" data-action="${connected ? "pick-trigger" : "go-integrations"}" data-kind="${kind}">
        ${iconHtml(STEPS[step])}
        <span><b>${text}</b><span class="hint">${connected ? pitch : `Connect ${needs === "slack" ? "Slack" : "Linear"} under Connectors first.`}</span></span>
      </button>`;
      })
      .join("")}`;
}

function editorHtml() {
  const c = view.draft;
  return `<datalist id="linear-labels">${options(suggestions.labels)}</datalist>
    <datalist id="linear-columns">${options(suggestions.columns)}</datalist>
    <datalist id="linear-teams">${options(suggestions.teams)}</datalist>
    <datalist id="slack-channels">${options(suggestions.channels)}</datalist>
    <datalist id="models">${options(MODELS)}</datalist>
    <datalist id="folders">${options(suggestions.folders)}</datalist>
    <datalist id="github-repos">${options(suggestions.repos)}</datalist>
    <datalist id="vercel-models">${vercelModelOptions(suggestions.models)}</datalist>
    <div class="editor-bar">
      <button class="secondary" data-action="close-editor" title="Back to workflows">←</button>
      ${input("name", c.name, 'class="title" placeholder="Workflow name"')}
      ${switchHtml('data-bind="enabled"', c.enabled)}
      ${c.id ? `<button class="secondary danger" data-action="delete-connector">Delete</button>` : ""}
      <button class="primary" id="save-connector" data-action="save-connector" title="Save (⌘S)"></button>
    </div>
    <div class="editor">
      <div class="canvas-wrap">
        <div class="canvas"></div>
        <div class="canvas-zoom" role="toolbar" aria-label="Zoom">
          <button data-action="zoom-fit" title="Fit to view">${icon("maximize")}</button>
          <button data-action="zoom-in" title="Zoom in">${icon("zoom-in")}</button>
          <button data-action="zoom-out" title="Zoom out">${icon("zoom-out")}</button>
        </div>
      </div>
      <p class="error" id="editor-error"></p>
      ${panelHtml(c)}
    </div>`;
}

const TILE = 84;
const TEXT_W = 150;
const TEXT_H = 56;
const COL = 230;
const PAD = 48;
const STUB = 40;
const END = 24;
const MENU_W = 280;

const graphApi = () => globalThis.PinnedGraph;
const WATCH_STEP = {
  "linear-label": "linear-trigger",
  "slack-mention": "slack-trigger",
  "slack-message": "message-trigger",
  called: "called-trigger",
};

const graphNodeKind = (node) => node.type ?? node.kind;

function syncTriggerFromWatch(c, watch) {
  if (!watch) return;
  if (watch.kind === "linear-label") c.trigger = { kind: "linear-label", label: watch.label ?? "" };
  else if (watch.kind === "slack-mention") c.trigger = { kind: "slack-mention", channel: watch.channel ?? "" };
  else if (watch.kind === "slack-message") c.trigger = { kind: "slack-message", channel: watch.channel ?? "", decide: c.trigger?.decide ?? { model: "", instructions: "", categories: [], fallback: null } };
  else if (watch.kind === "called") c.trigger = { kind: "called" };
}

function syncWatchFromTrigger(c) {
  const node = c.graph?.nodes?.find((n) => graphNodeKind(n) === "trigger");
  if (!node || !c.trigger) return;
  const t = c.trigger;
  if (t.kind === "linear-label") node.watch = { kind: "linear-label", label: t.label ?? "" };
  else if (t.kind === "slack-mention") node.watch = { kind: "slack-mention", channel: t.channel ?? "" };
  else if (t.kind === "slack-message") node.watch = { kind: "slack-message", channel: t.channel ?? "" };
  else node.watch = { kind: "called" };
}

const OLDER_HUB = "This workflow comes from an older hub; update the hub.";

function graphNodeUi(node) {
  const type = graphNodeKind(node);
  if (type === "trigger") return STEPS[WATCH_STEP[node.watch?.kind ?? "called"]] ?? STEPS["called-trigger"];
  if (type === "classify") return { tone: "vercel", icon: "▲", app: "Vercel AI Gateway", title: "Classify" };
  if (type === "switch") return { tone: "switch", icon: "⇉", app: "Workflow", title: "Switch" };
  if (type === "talk") return { tone: "claude", icon: "✳", app: "Claude Code", title: "Talk to the session" };
  if (type === "page") return { tone: "slack", icon: "@", app: "Slack", title: "Page a group" };
  if (type === "stage-picked") return { ...STEPS.picked, title: "Stage picked" };
  if (type === "call") return { tone: "called", icon: "↳", app: "Workflow", title: "Call workflow" };
  if (type === "create-ticket") return { tone: "linear", icon: "⚑", app: "Linear", title: "Create ticket" };
  if (type === "stop") return { tone: "", icon: "■", app: "", title: "Stop" };
  return { tone: "", icon: "?", app: "", title: node.name ?? "Node" };
}

function graphNodeSummary(node, c) {
  const type = graphNodeKind(node);
  if (type === "trigger") {
    const step = WATCH_STEP[node.watch?.kind ?? "called"];
    return STEPS[step]?.summary?.({ ...c, trigger: legacyTriggerFromWatch(node.watch) }) ?? node.name;
  }
  if (type === "classify") return node.model?.trim() || "Name a model";
  if (type === "switch") {
    const question = node.question?.trim();
    const equals = (node.cases ?? []).map((item) => item.equals?.trim()).filter(Boolean);
    if (!question) return "Pick a question";
    if (!equals.length) return `${question} continues`;
    return `${question} is ${equals.join(", ")}`;
  }
  if (type === "talk") return node.prompt?.trim() || "Session opens on the first talk";
  if (type === "page") return node.group?.trim() ? `@${node.group.trim().replace(/^@/, "")}` : "Name a group";
  if (type === "call") {
    const target = settings.connectors.find((k) => k.id === node.workflow);
    return target?.name ?? (node.workflow ? "Missing workflow" : "Pick a workflow");
  }
  if (type === "create-ticket") return node.team?.trim() || "Name a team";
  if (type === "stop") return "Ends this branch";
  return node.name ?? "";
}

function legacyTriggerFromWatch(watch) {
  if (watch?.kind === "linear-label") return { kind: "linear-label", label: watch.label ?? "" };
  if (watch?.kind === "slack-message") return { kind: "slack-message", channel: watch.channel ?? "", decide: { model: "", instructions: "", categories: [], fallback: null } };
  if (watch?.kind === "slack-mention") return { kind: "slack-mention", channel: watch.channel ?? "" };
  return { kind: "called" };
}

function graphNodeIssues(node, c) {
  const type = graphNodeKind(node);
  if (type === "trigger") return STEPS[WATCH_STEP[node.watch?.kind ?? "called"]]?.issues?.({ ...c, trigger: legacyTriggerFromWatch(node.watch) }) ?? [];
  if (type === "classify") {
    const pseudo = { ...c, ask: { model: node.model, instructions: node.instructions, questions: (node.questions ?? []).map((q) => ({ ...q, choices: (q.choices ?? []).map((ch) => ch.label ?? ch.id ?? ch) })) } };
    return STEPS.ask.issues(pseudo);
  }
  if (type === "switch") {
    const problem = graphApi()?.switchProblem(c.graph, node);
    return problem ? [problem] : [];
  }
  if (type === "talk") return [];
  if (type === "page") return [!node.group?.trim() && "Name the Slack user group to page."].filter(Boolean);
  if (type === "call") {
    const target = settings.connectors.find((k) => k.id === node.workflow);
    return [!node.workflow && "Choose a workflow.", node.workflow && !target && "That workflow was deleted."].filter(Boolean);
  }
  return [];
}

function graphNodeIndex(id) {
  return view.draft.graph.nodes.findIndex((n) => n.id === id);
}

function boundGraphNode(path) {
  const match = /^graph\.nodes\.(\d+)\b/.exec(path);
  if (!match) return null;
  return view.draft.graph?.nodes?.[Number(match[1])] ?? null;
}

function keepSwitchQuestion(node) {
  const target = view.draft.graph.nodes.find((n) => n.id === node.classify && graphNodeKind(n) === "classify");
  const names = (target?.questions ?? []).map((q) => q.name).filter(Boolean);
  if (!names.includes(node.question)) node.question = names[0] ?? "";
}

function retargetCasePort(node, index, button) {
  const item = node.cases?.[index];
  const api = graphApi();
  if (!item || !api) return;
  const wired = view.draft.graph.edges.some((e) => e.from === node.id && e.port === item.id);
  if (!wired) {
    item.id = api.nextCaseId(node.cases.filter((_, i) => i !== index), item.equals);
    if (button) button.dataset.port = item.id;
  }
  if (button) button.textContent = `${item.equals.trim() || "This case"} ends here`;
}

function graphLayout(c) {
  const api = graphApi();
  const graph = c.graph;
  if (!graph?.nodes?.length) return null;
  const nodes = graph.nodes.map((node) => {
    const portList = api ? api.ports(node) : [{ id: "out", label: "" }];
    const portCount = portList.length || 1;
    const h = api?.nodeHeight(node) ?? (portCount > 2 ? Math.max(TILE, portCount * 28 + 16) : TILE);
    return { node, id: node.id, cx: TEXT_W / 2 + node.position[0], cy: node.position[1], h, ports: portCount, portList };
  });
  const top = Math.min(...nodes.map((n) => n.cy - n.h / 2));
  const left = Math.min(...nodes.map((n) => n.cx - TEXT_W / 2));
  const dx = PAD - left;
  const dy = PAD - top;
  for (const n of nodes) {
    n.cx += dx;
    n.cy += dy;
  }
  const byId = Object.fromEntries(nodes.map((n) => [n.id, n]));
  const edges = graph.edges
    .map((e, index) => ({
      ...e,
      index,
      from: byId[e.from],
      to: byId[e.to],
      portLabel: byId[e.from]?.portList?.find((p) => p.id === e.port)?.label,
      selected: view.selectedEdge === index,
    }))
    .filter((e) => e.from && e.to);
  const right = Math.max(...nodes.map((n) => n.cx + TEXT_W / 2));
  const bottom = Math.max(...nodes.map((n) => n.cy + n.h / 2 + TEXT_H));
  return { nodes, edges, width: right + PAD, height: bottom + PAD, graph };
}

function graphPickerHtml(c, anchor) {
  if (!view.graphPick) return "";
  const api = graphApi();
  const types = api?.PICKER_TYPES ?? ["talk", "stop"];
  const items = types
    .flatMap((type) => {
      const ui = graphNodeUi({ type, watch: { kind: "called" } });
      const button = (label, backlog) => `<button data-action="graph-pick-type" data-type="${type}"${backlog ? ' data-backlog="true"' : ""}>${iconHtml(ui)}<span>${esc(label)}</span></button>`;
      if (type !== "create-ticket") return [button(type, false)];
      return [button("Create ticket", false), button("Create ticket on backlog", true)];
    })
    .join("");
  return `<div class="add-menu graph-picker" style="width:${MENU_W}px;left:${anchor.x}px;top:${anchor.y}px">${items}</div>`;
}

function flowHtml(c) {
  const laid = graphLayout(c);
  if (!laid) {
    return `<div class="flow graph-flow" data-width="720" data-height="240" style="width:720px;height:240px">
      <p class="hint" style="position:absolute;left:${PAD}px;top:${PAD}px;max-width:420px">${OLDER_HUB}</p>
    </div>`;
  }
  const { nodes, edges, width, height, graph } = laid;
  const api = graphApi();
  const dangling = api
    ? graph.nodes.flatMap((node) =>
        api.ports(node).flatMap((port) => {
          if (graph.edges.some((e) => e.from === node.id && e.port === port.id)) return [];
          const box = nodes.find((n) => n.id === node.id);
          const y = portY(box, port.id);
          return [{ node, port: port.id, label: port.label, x: box.cx + TILE / 2 + STUB, y, laid: box }];
        }),
      )
    : [];
  const flowWidth = Math.max(width, ...dangling.map((d) => d.x + END), 0) + (dangling.length ? PAD : 0);
  const picker = view.graphPick ? graphPickerHtml(c, view.graphPick) : "";
  return `<div class="flow graph-flow" data-width="${flowWidth}" data-height="${height}" style="width:${flowWidth}px;height:${height}px">
    ${nodes.map((n) => graphNodeHtml(c, n)).join("")}
    <svg class="edges" width="${flowWidth}" height="${height}" aria-hidden="true">
      <defs><marker id="flow-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0 1 9 5 0 9z"/></marker></defs>
      ${edges.map((edge) => graphEdgeSvg(edge)).join("")}
      ${dangling.map((d) => `<path d="M${d.laid.cx + TILE / 2} ${d.y}h${STUB}"/>`).join("")}
    </svg>
    ${edges.map((edge) => graphEdgeLabels(edge)).join("")}
    ${edges.map((edge) => graphEdgeAddButton(edge)).join("")}
    ${dangling.map((d) => graphDangleButton(d)).join("")}
    ${picker}
  </div>`;
}

function graphDangleButton({ node, port, x, y }) {
  return `<button class="end graph-port-add" data-action="graph-append" data-from="${esc(node.id)}" data-port="${esc(port)}" title="Add a step" style="left:${x}px;top:${y - END / 2}px">${icon("plus")}</button>`;
}

function graphEdgeAddButton(edge) {
  const [x1, y1, x2, y2] = graphEdgeEnds(edge);
  const x = (x1 + x2) / 2;
  const y = (y1 + y2) / 2;
  return `<button class="edge-add" data-action="graph-splice" data-edge="${edge.index}" title="Insert a step" style="left:${x - 12}px;top:${y - 12}px">${icon("plus")}</button>`;
}

const nodeAt = (n) => `left:${n.cx - TEXT_W / 2}px;top:${n.cy - n.h / 2}px`;

function graphNodeHtml(c, n) {
  const ui = graphNodeUi(n.node);
  const issues = graphNodeIssues(n.node, c);
  const open = view.open === n.id;
  const trigger = graphNodeKind(n.node) === "trigger";
  const portHandles = n.portList
    .map((port, portIndex) => {
      const y = (n.h * (portIndex + 0.5)) / n.ports;
      const x = (TEXT_W + TILE) / 2;
      return `<button type="button" class="port-handle" data-node="${esc(n.id)}" data-port="${esc(port.id)}" style="left:${x}px;top:${y}px;transform:translate(-50%,-50%)" title="${esc(port.label || port.id)}"></button>`;
    })
    .join("");
  return `<div class="step graph-node${open ? " open" : ""}" data-node="${esc(n.id)}" style="${nodeAt(n)}">
    ${portHandles}
    <button class="node${trigger ? " trigger" : ""}" data-action="graph-open-node" data-node="${esc(n.id)}" aria-expanded="${open}" title="${esc(`${ui.app}: ${ui.title}`)}" style="height:${n.h}px">
      ${iconHtml(ui)}${issues.length ? `<span class="warn" title="${esc(issues.join(" "))}">!</span>` : ""}
    </button>
    <span class="text"><b>${esc(n.node.name || ui.title)}</b><span class="summary">${esc(graphNodeSummary(n.node, c))}</span></span>
  </div>`;
}

function portY(laid, portId) {
  const portIndex = laid.portList.findIndex((p) => p.id === portId);
  const idx = portIndex < 0 ? 0 : portIndex;
  return laid.cy - laid.h / 2 + (laid.h * (idx + 0.5)) / laid.ports;
}

function graphEdgeEnds(edge) {
  const y1 = portY(edge.from, edge.port);
  return [edge.from.cx + TILE / 2, y1, edge.to.cx - TILE / 2, edge.to.cy];
}

function graphEdgeSvg(edge) {
  const [x1, y1, x2, y2] = graphEdgeEnds(edge);
  const bend = (x2 - x1) / 2;
  const cls = edge.selected ? "edge-selected" : "";
  return `<g class="edge-group${cls ? ` ${cls}` : ""}" data-edge="${edge.index}">
    <path class="edge-hit" data-action="graph-select-edge" data-edge="${edge.index}" d="M${x1} ${y1}C${x1 + bend} ${y1} ${x2 - bend} ${y2} ${x2 - 6} ${y2}"/>
    <path d="M${x1} ${y1}C${x1 + bend} ${y1} ${x2 - bend} ${y2} ${x2 - 6} ${y2}" marker-end="url(#flow-arrow)"/>
  </g>`;
}

function graphEdgeLabels(edge) {
  const [x1, y1, x2, y2] = graphEdgeEnds(edge);
  if (edge.portLabel) return `<span class="port-label" style="left:${x1 + 8}px;top:${y1}px">${esc(edge.portLabel)}</span>`;
  return "";
}

function classifyFields(node, c) {
  const i = graphNodeIndex(node.id);
  const base = `graph.nodes.${i}`;
  const rows = (node.questions ?? [])
    .map((q, qi) => {
      const choices = (q.choices ?? []).map((ch) => (typeof ch === "string" ? ch : ch.label ?? ch.id)).join(", ");
      return `<div class="grid">
          <label>Name ${input(`${base}.questions.${qi}.name`, q.name, 'placeholder="kind" spellcheck="false"')}</label>
          <label>Question ${input(`${base}.questions.${qi}.ask`, q.ask, "")}</label>
        </div>
        <label>Choices <input data-bind="${base}.questions.${qi}.choices" data-list="" value="${esc(choices)}" placeholder="bug, question"></label>
        <div class="actions start"><button class="secondary" data-action="graph-remove-question" data-index="${qi}">Remove question</button></div>`;
    })
    .join("");
  return `<label>Model ${input(`${base}.model`, node.model ?? "", 'list="vercel-models" spellcheck="false"')}</label>
    <label>Instructions<textarea data-bind="${base}.instructions" rows="3">${esc(node.instructions ?? "")}</textarea></label>
    ${rows}
    <div class="actions start"><button class="secondary" data-action="graph-add-question">Add question</button></div>
    <label>Branch on ${input(`${base}.branchOn`, node.branchOn ?? "", 'placeholder="kind" spellcheck="false"')}</label>
    <label>Gate ${select(`${base}.gate.kind`, ["direct", "decide"], (k) => (k === "decide" ? "Decide (thresholds)" : "Direct"), node.gate?.kind ?? "direct")}</label>
    ${
      node.gate?.kind === "decide"
        ? `<div class="grid">
        <label>Unsure below ${input(`${base}.gate.unsureBelow`, node.gate.unsureBelow ?? 0.6, 'type="number" step="0.01" min="0" max="1"')}</label>
        <label>Page at ${input(`${base}.gate.pageAt`, node.gate.pageAt ?? 0.85, 'type="number" step="0.01" min="0" max="1"')}</label>
      </div>`
        : ""
    }
    <label>Fallback ${select(`${base}.fallback.kind`, ["none", "extra", "existing"], (k) => k, node.fallback?.kind ?? "none")}</label>
    ${choiceEnders(node)}`;
}

function choiceLabels(question) {
  return (question?.choices ?? []).map((ch) => (typeof ch === "string" ? ch : ch.label ?? ch.id)).filter((label) => label != null && String(label) !== "");
}

function switchFields(node, c) {
  const i = graphNodeIndex(node.id);
  const base = `graph.nodes.${i}`;
  const classifies = (c.graph?.nodes ?? []).filter((n) => graphNodeKind(n) === "classify");
  const target = classifies.find((n) => n.id === node.classify);
  const questions = target?.questions ?? [];
  const names = questions.map((q) => q.name).filter(Boolean);
  const question = questions.find((q) => q.name === node.question);
  const labels = choiceLabels(question);
  const classifyIds = classifies.map((n) => n.id);
  const classifyValues = classifyIds.includes(node.classify) ? classifyIds : ["", ...classifyIds];
  const questionValues = names.includes(node.question) ? names : ["", ...names];
  const classifySelect = classifies.length
    ? `<label>Classify ${select(`${base}.classify`, classifyValues, (id) => classifies.find((n) => n.id === id)?.name || "Choose a classify", node.classify ?? "")}</label>`
    : `<p class="hint">Add a classify node first. A switch reads an answer that node already gave.</p>`;
  const questionSelect = !target
    ? ""
    : names.length
      ? `<label>Question ${select(`${base}.question`, questionValues, (name) => name || "Choose a question", node.question ?? "")}</label>`
      : `<p class="hint">That classify has no questions yet.</p>`;
  const cases = (node.cases ?? [])
    .map((item, ci) => {
      const listId = `switch-choices-${i}-${ci}`;
      const datalist = labels.length ? `<datalist id="${listId}">${labels.map((label) => `<option value="${esc(label)}"></option>`).join("")}</datalist>` : "";
      const shown = item.equals?.trim() || "This case";
      const listAttr = labels.length ? `list="${listId}" ` : "";
      return `<div class="category">
        <label>Equals ${input(`${base}.cases.${ci}.equals`, item.equals ?? "", `${listAttr}placeholder="no" spellcheck="false"`)}${datalist}</label>
        <div class="actions start">
          <button class="secondary" data-action="graph-end-choice" data-port="${esc(item.id)}">${esc(shown)} ends here</button>
          <button class="secondary" data-action="graph-remove-case" data-index="${ci}">Remove case</button>
        </div>
      </div>`;
    })
    .join("");
  return `${classifySelect}
    ${questionSelect}
    ${cases}
    <div class="actions start"><button class="secondary" data-action="graph-add-case">Add case</button></div>
    <p class="hint">The first case that matches the answer, ignoring case, takes that port. Anything else leaves through otherwise.</p>`;
}

function choiceEnders(node) {
  const api = graphApi();
  const skip = new Set([api?.FALLBACK ?? "fallback", api?.NOWHERE ?? "nowhere"]);
  const buttons = (api?.ports(node) ?? [])
    .filter((port) => port.label && !skip.has(port.id))
    .map((port) => `<button class="secondary" data-action="graph-end-choice" data-port="${esc(port.id)}">${esc(port.label)} ends here</button>`)
    .join("");
  return buttons ? `<div class="actions start">${buttons}</div>` : "";
}

function sessionFields(c) {
  return STEPS.agent.fields(c);
}

function graphNodePanel(c) {
  if (view.open === "session") {
    return `<aside class="node-panel" aria-label="Session">
      <div class="node-panel-head">${iconHtml(STEPS.agent)}<span class="text"><span class="app">Claude Code</span><b>Session</b></span>
        <button class="secondary close" data-action="close-step" title="Close (Esc)">✕</button></div>
      <div class="details">${sessionFields(c)}</div>
    </aside>`;
  }
  const node = view.draft.graph?.nodes?.find((n) => n.id === view.open);
  if (!node) return "";
  const ui = graphNodeUi(node);
  const type = graphNodeKind(node);
  let body = "";
  if (type === "trigger") body = STEPS[WATCH_STEP[c.trigger?.kind] ?? WATCH_STEP[node.watch?.kind ?? "called"]].fields(c);
  else if (type === "classify") body = classifyFields(node, c);
  else if (type === "switch") body = switchFields(node, c);
  else if (type === "talk") {
    const i = graphNodeIndex(node.id);
    body = `<label>Prompt<textarea data-bind="graph.nodes.${i}.prompt" rows="6">${esc(node.prompt ?? "")}</textarea></label>
      <div class="actions start"><button class="secondary" data-action="graph-open-session">Session settings (folder, model, fix…)</button></div>`;
  } else if (type === "page") {
    const i = graphNodeIndex(node.id);
    body = `<label>Group ${input(`graph.nodes.${i}.group`, node.group ?? "", 'placeholder="@oncall" spellcheck="false"')}</label>`;
  } else if (type === "call") {
    const i = graphNodeIndex(node.id);
    const targets = settings.connectors.filter((k) => k.id !== c.id);
    body = `<label>Workflow ${select(`graph.nodes.${i}.workflow`, ["", ...targets.map((k) => k.id)], (id) => targets.find((k) => k.id === id)?.name ?? id, node.workflow ?? "")}</label>`;
  } else if (type === "stage-picked") body = `<p class="hint">Moves the Linear issue to the connector's picked column when the session opens.</p>`;
  else if (type === "create-ticket") {
    const i = graphNodeIndex(node.id);
    body = `<label>Team ${input(`graph.nodes.${i}.team`, node.team ?? "", 'spellcheck="false"')}</label>
      <label>Label ${input(`graph.nodes.${i}.label`, node.label ?? "", 'spellcheck="false"')}</label>
      <p class="hint">An empty label adds no label.</p>
      <label>Column ${input(`graph.nodes.${i}.column`, node.column ?? "", 'spellcheck="false"')}</label>
      <p class="hint">An empty column uses the team's default.</p>
      <label>Instructions<textarea data-bind="graph.nodes.${i}.instructions" rows="4">${esc(node.instructions ?? "")}</textarea></label>`;
  }
  const removable = type !== "trigger";
  return `<aside class="node-panel" aria-label="${esc(ui.title)}">
    <div class="node-panel-head">${iconHtml(ui)}<span class="text"><span class="app">${ui.app}</span><b>${esc(node.name || ui.title)}</b></span>
      <button class="secondary close" data-action="close-step" title="Close (Esc)">✕</button></div>
    <div class="details">${body}${removable ? `<div class="actions start"><button class="secondary danger" data-action="graph-delete-node">Delete node</button></div>` : ""}</div>
  </aside>`;
}

function panelHtml(c) {
  if (view.name !== "edit") return "";
  return graphNodePanel(c);
}

function applyGraphPick(type, backlog) {
  const api = graphApi();
  if (!api || !view.graphPick) return;
  const edge = view.graphPick.mode === "splice" ? view.draft.graph.edges[view.graphPick.edgeIndex] : null;
  const from = edge?.from ?? view.graphPick.from;
  const port = edge?.port ?? view.graphPick.port;
  if (type === "create-ticket") {
    const before = view.draft.graph.nodes.map((node) => node.id);
    view.draft.graph = api.insertCreateTicket(view.draft.graph, from, port, { backlog });
    view.open = view.draft.graph.nodes.find((node) => node.type === "create-ticket" && !before.includes(node.id))?.id ?? null;
    view.graphPick = null;
    redraw();
    return;
  }
  const fresh = api.newNode(type, view.draft.graph, from);
  if (view.graphPick.mode === "splice") {
    view.draft.graph = api.splice(view.draft.graph, edge, fresh);
  } else {
    view.draft.graph = api.append(view.draft.graph, from, port, fresh);
  }
  view.graphPick = null;
  view.open = fresh.id;
  redraw();
}

function syncLegacyFromGraph(c) {
  const trigger = c.graph?.nodes?.find((n) => graphNodeKind(n) === "trigger");
  if (trigger?.watch) syncTriggerFromWatch(c, trigger.watch);
}

const READABLE_ZOOM = 0.75;
const MIN_ZOOM = 0.5;
const MAX_ZOOM = 1.5;

function fitZoom(floor) {
  const canvas = settingsPage.querySelector(".canvas");
  const flow = canvas.firstElementChild;
  return Math.max(floor, Math.min(1, canvas.clientWidth / flow.dataset.width, canvas.clientHeight / flow.dataset.height));
}

function applyZoom() {
  const canvas = settingsPage.querySelector(".canvas");
  const flow = canvas?.firstElementChild;
  if (!flow) return;
  const zoom = view.zoom ?? fitZoom(READABLE_ZOOM);
  flow.style.zoom = zoom;
  if (view.pan) [canvas.scrollLeft, canvas.scrollTop] = view.pan;
  settingsPage.querySelector('[data-action="zoom-in"]').disabled = zoom >= MAX_ZOOM;
  settingsPage.querySelector('[data-action="zoom-out"]').disabled = zoom <= MIN_ZOOM;
}

function zoomAround(zoom, at) {
  const canvas = settingsPage.querySelector(".canvas");
  const flow = canvas.firstElementChild;
  const box = canvas.getBoundingClientRect();
  const [x, y] = at ?? [box.left + canvas.clientWidth / 2, box.top + canvas.clientHeight / 2];
  const was = flow.getBoundingClientRect();
  const scale = Number(flow.style.zoom) || 1;
  view.zoom = Math.round(Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom)) * 100) / 100;
  view.pan = null;
  applyZoom();
  const now = flow.getBoundingClientRect();
  canvas.scrollLeft += now.left + ((x - was.left) / scale) * view.zoom - x;
  canvas.scrollTop += now.top + ((y - was.top) / scale) * view.zoom - y;
  view.pan = [canvas.scrollLeft, canvas.scrollTop];
}

const zoomNow = () => Number(settingsPage.querySelector(".flow").style.zoom);

const changed = () => JSON.stringify(view.draft) !== view.original;

function refresh() {
  const c = view.draft;
  settingsPage.querySelector(".canvas").innerHTML = flowHtml(c);
  applyZoom();
  const command = document.getElementById("command");
  if (command) command.textContent = commandPreview(c);
  const repo = document.getElementById("repo");
  if (repo) repo.innerHTML = repoHint(c);
  const fallback = settingsPage.querySelector('.node-panel [data-bind="trigger.decide.fallback"]');
  if (fallback && fallback !== document.activeElement) fallback.outerHTML = fallbackHtml(c.trigger.decide);
  const save = document.getElementById("save-connector");
  save.disabled = !!c.id && !changed();
  save.textContent = !c.id ? "Create" : changed() ? "Save" : "Saved";
}

function redraw() {
  settingsPage.dataset.view = view.name;
  settingsPage.innerHTML = connectorsHtml();
  if (view.name === "edit") refresh();
}

function edit(index, draft = clone(settings.connectors[index]), open = null) {
  const trigger = draft.graph?.nodes?.find((n) => graphNodeKind(n) === "trigger");
  if (trigger?.watch) syncTriggerFromWatch(draft, trigger.watch);
  const first = draft.graph?.nodes?.[0]?.id ?? null;
  view = { name: "edit", index, draft, original: JSON.stringify(draft), open: open ?? first, zoom: null, pan: null, menu: false, graphPick: null, selectedEdge: null, connectFrom: null, drag: null };
  loadFolders(draft);
}

/** A fresh connector of `kind`, filled with the defaults that make it work once a folder is chosen. */
function blank(kind) {
  const machine = active ? hubIdOf(active.machine) : "";
  const { name, trigger, defaults } = TRIGGERS[kind];
  const c = {
    id: "",
    account: "",
    enabled: true,
    name,
    trigger: trigger(),
    folder: active?.view.cwd ?? "",
    machine,
    repo: "",
    model: "opus",
    effort: "high",
    permissionMode: "auto",
    proof: true,
    team: "",
    label: "",
    fix: "off",
    worktree: false,
    columns: { picked: "", coding: "", review: "" },
    ...clone(defaults ?? {}),
  };
  return { ...c, instructions: defaultText(c) };
}

function commandPreview(c) {
  const parts = [connectorAccountName(c) && c.account !== DEFAULT_ACCOUNT ? `CLAUDE_CONFIG_DIR=<${connectorAccountName(c)}'s home> claude` : "claude"];
  if (c.model) parts.push(`--model ${c.model}`);
  parts.push(`--effort ${c.effort}`, `--permission-mode ${c.permissionMode}`);
  if (c.worktree) parts.push(`--worktree=<${subject(c)}>`);
  return [...parts, `"<instructions + ${subject(c)}>"`].join(" ");
}

function checkedText() {
  const checked = settings.checked;
  if (!checked) return "Not checked yet. Connectors are checked every minute.";
  const at = new Date(checked.at).toLocaleTimeString();
  return checked.error ? `Last check at ${at} failed: ${checked.error}` : `Last checked at ${at}. Connectors are checked every minute.`;
}

const trimmed = (s) => (s ?? "").trim();

function tidyTrigger({ kind, label, channel, decide }) {
  const trigger = { kind };
  if (label !== undefined) trigger.label = trimmed(label);
  if (channel !== undefined) trigger.channel = trimmed(channel);
  if (decide) {
    trigger.decide = {
      model: trimmed(decide.model),
      instructions: trimmed(decide.instructions),
      categories: decide.categories.map((cat) => ({
        name: trimmed(cat.name),
        description: trimmed(cat.description),
        workflow: cat.workflow || null,
        mention: trimmed(cat.mention).replace(/^@/, "") || null,
      })),
      fallback: decide.fallback || null,
    };
  }
  return trigger;
}

/** The draft with its typed names trimmed, as the backend matches them. */
function tidy(c) {
  const t = trimmed;
  const list = (values) => (values ?? []).map(t).filter(Boolean);
  const out = {
    ...c,
    name: t(c.name),
    folder: t(c.folder),
    repo: t(c.repo),
    model: t(c.model),
    account: t(c.account),
    team: t(c.team),
    label: t(c.label),
    trigger: tidyTrigger(c.trigger),
    columns: Object.fromEntries(Object.entries(c.columns).map(([stage, column]) => [stage, t(column)])),
  };
  if (c.ask) {
    out.ask = {
      model: t(c.ask.model),
      instructions: t(c.ask.instructions),
      questions: (c.ask.questions ?? []).map((q) => ({ name: t(q.name), ask: t(q.ask), choices: list(q.choices), stopOn: list(q.stopOn) })),
    };
  } else delete out.ask;
  if (c.graph) {
    syncLegacyFromGraph(c);
    out.graph = c.graph;
  }
  return out;
}

const defaultText = (c) => TRIGGERS[c.trigger.kind].instructions(c);

/** Fields that change which steps the flow has, so the draft is reshaped and the editor redrawn. */
const RESHAPES = {
  "trigger.kind": (c, kind) => ({ ...c, trigger: TRIGGERS[kind].trigger(c.trigger) }),
  fix: (c, fix) => ({ ...c, fix }),
  machine: (c, machine) => ({ ...c, machine: machine === BOX ? { kind: BOX, size: "medium" } : machine }),
};

/** Applies `change` to the draft, swapping in the new defaults where the old ones were untouched. */
function reshape(change) {
  const before = view.draft;
  const after = change(clone(before));
  if (before.name === TRIGGERS[before.trigger.kind].name) after.name = TRIGGERS[after.trigger.kind].name;
  if (before.instructions.trim() === defaultText(before).trim()) after.instructions = defaultText(after);
  if (before.trigger.kind !== after.trigger.kind) {
    syncWatchFromTrigger(after);
    view.open = after.graph?.nodes?.find((n) => graphNodeKind(n) === "trigger")?.id ?? view.open;
  }
  view.draft = after;
  if (JSON.stringify(before.machine) !== JSON.stringify(after.machine)) {
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
  view.graphPick = null;
  redraw();
  settingsPage.querySelector(".flow .open")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  if (focus) settingsPage.querySelector(".node-panel .details [data-bind]")?.focus();
}

const stepOf = (button) => button.dataset.step ?? button.closest(".step").dataset.step;

function focusCategory(category, field) {
  showStep("decide");
  const input = settingsPage.querySelector(`.node-panel [data-bind="trigger.decide.categories.${category}.${field}"]`);
  input?.scrollIntoView({ block: "nearest" });
  input?.focus();
}

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
  "open-upstash-console": () => (TAURI ? invoke("open_url", { url: UPSTASH_CONSOLE }) : window.open(UPSTASH_CONSOLE, "_blank", "noopener")),
  async "connect-upstash"() {
    const key = document.getElementById("upstash-key").value;
    const claude_token = document.getElementById("claude-token").value;
    if (!key.trim() || !claude_token.trim()) return;
    try {
      settings.upstash = await connectorCall("connect_upstash", { key, claude_token });
      showPage("integrations");
    } catch (e) {
      document.getElementById("upstash-error").textContent = e;
    }
  },
  async "answer-box"(button) {
    const id = button.dataset.id;
    const allow = button.dataset.allow === "yes";
    try {
      await connectorCall("answer_box", { id, allow });
      settings.box_asks = (settings.box_asks || []).filter((ask) => ask.id !== id);
      showPage("integrations");
    } catch (e) {
      const line = document.getElementById("upstash-error");
      if (line) line.textContent = e;
    }
  },
  async "disconnect-upstash"(button) {
    if (!armed(button, "Delete the boxes?")) return;
    try {
      await connectorCall("disconnect_upstash");
    } catch (e) {
      // The hub keeps the key until every box is deleted, so a later try can still delete them.
      document.getElementById("upstash-error").textContent = e;
      return;
    }
    settings.upstash = null;
    showPage("integrations");
  },
  "open-github-tokens": () => (TAURI ? invoke("open_github_tokens") : window.open(GITHUB_TOKENS, "_blank", "noopener")),
  async "connect-github"() {
    const token = document.getElementById("github-token").value;
    if (!token.trim()) return;
    try {
      settings.github = await connectorCall("connect_github", { token });
      suggestions.repos = [];
      showPage("integrations");
    } catch (e) {
      document.getElementById("github-error").textContent = e;
    }
  },
  async "disconnect-github"() {
    await connectorCall("disconnect_github");
    settings.github = null;
    suggestions.repos = [];
    showPage("integrations");
  },
  "open-vercel-keys": () => (TAURI ? invoke("open_url", { url: VERCEL_KEYS }) : window.open(VERCEL_KEYS, "_blank", "noopener")),
  async "connect-vercel"() {
    const key = document.getElementById("vercel-key").value;
    if (!key.trim()) return;
    try {
      settings.vercel = await connectorCall("connect_vercel", { key });
      showPage("integrations");
    } catch (e) {
      document.getElementById("vercel-error").textContent = e;
    }
  },
  async "disconnect-vercel"() {
    await connectorCall("disconnect_vercel");
    settings.vercel = null;
    showPage("integrations");
  },
  "add-question"() {
    view.draft.ask.questions.push({ name: "", ask: "", choices: [], stopOn: [] });
    redraw();
  },
  "remove-question"(button) {
    view.draft.ask.questions.splice(Number(button.dataset.index), 1);
    redraw();
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
    if (graphApi()) draft.graph = graphApi().startingGraph(draft);
    edit(-1, draft);
    showStep(draft.graph.nodes[0]?.id, true);
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
  "graph-open-node"(button) {
    const id = button.dataset.node;
    showStep(view.open === id ? null : id);
  },
  "graph-open-session"() {
    view.open = "session";
    redraw();
  },
  "graph-splice"(button) {
    const flow = settingsPage.querySelector(".flow");
    const box = button.getBoundingClientRect();
    const origin = flow.getBoundingClientRect();
    const zoom = zoomNow() || 1;
    view.graphPick = { mode: "splice", edgeIndex: Number(button.dataset.edge), x: (box.left - origin.left) / zoom, y: (box.bottom - origin.top) / zoom + 6 };
    refresh();
  },
  "graph-append"(button) {
    const flow = settingsPage.querySelector(".flow");
    const box = button.getBoundingClientRect();
    const origin = flow.getBoundingClientRect();
    const zoom = zoomNow() || 1;
    view.graphPick = { mode: "append", from: button.dataset.from, port: button.dataset.port, x: (box.left - origin.left) / zoom, y: (box.bottom - origin.top) / zoom + 6 };
    refresh();
  },
  "graph-pick-type"(button) {
    applyGraphPick(button.dataset.type, button.dataset.backlog === "true");
  },
  "graph-select-edge"(button) {
    view.selectedEdge = Number(button.dataset.edge);
    refresh();
  },
  "graph-delete-node"() {
    const api = graphApi();
    if (!api || !view.open || view.open === "session") return;
    view.draft.graph = api.remove(view.draft.graph, view.open);
    view.open = null;
    redraw();
  },
  "graph-add-case"() {
    const api = graphApi();
    const node = view.draft.graph.nodes.find((n) => n.id === view.open);
    if (!api || !node || graphNodeKind(node) !== "switch") return;
    if (!node.cases) node.cases = [];
    node.cases.push({ id: api.nextCaseId(node.cases), equals: "" });
    const index = node.cases.length - 1;
    redraw();
    settingsPage.querySelector(`.node-panel [data-bind$=".cases.${index}.equals"]`)?.focus();
  },
  "graph-remove-case"(button) {
    const node = view.draft.graph.nodes.find((n) => n.id === view.open);
    if (!node?.cases) return;
    const index = Number(button.dataset.index);
    const id = node.cases[index]?.id;
    node.cases.splice(index, 1);
    if (id) view.draft.graph.edges = view.draft.graph.edges.filter((e) => !(e.from === node.id && e.port === id));
    redraw();
  },
  "graph-add-question"() {
    const node = view.draft.graph.nodes.find((n) => n.id === view.open);
    if (!node?.questions) node.questions = [];
    node.questions.push({ name: "", ask: "", choices: [] });
    redraw();
  },
  "graph-remove-question"(button) {
    const node = view.draft.graph.nodes.find((n) => n.id === view.open);
    node.questions.splice(Number(button.dataset.index), 1);
    redraw();
  },
  "graph-end-choice"(button) {
    const api = graphApi();
    const from = view.open;
    const port = button.dataset.port;
    if (!api || !from || !view.draft.graph) return;
    let graph = view.draft.graph;
    const existing = graph.edges.filter((e) => e.from === from && e.port === port);
    const onlyStop = existing.length === 1 && graphNodeKind(graph.nodes.find((n) => n.id === existing[0].to) ?? {}) === "stop";
    if (onlyStop) return;
    for (const edge of existing) graph = api.disconnect(graph, edge);
    const stop = api.newNode("stop", graph);
    view.draft.graph = api.append(graph, from, port, stop);
    redraw();
  },
  "add-step"(button) {
    const id = stepOf(button);
    STEPS[id].add?.(view.draft);
    showStep(id, true);
  },
  "close-step": () => showStep(null),
  "add-category"() {
    const categories = view.draft.trigger.decide.categories;
    categories.push({ name: "", description: "", workflow: null, mention: null });
    showStep("decide");
    settingsPage.querySelector(`.node-panel [data-bind="trigger.decide.categories.${categories.length - 1}.name"]`)?.focus();
  },
  "remove-category"(button) {
    const decide = view.draft.trigger.decide;
    const [removed] = decide.categories.splice(Number(button.closest("[data-category]").dataset.category), 1);
    if (removed.name.trim() === decide.fallback) decide.fallback = null;
    showStep("decide");
  },
  "open-branch"(button) {
    const { branch, category } = button.closest(".branch").dataset;
    if (branch === "route") return showStep(`route:${category}`);
    focusCategory(category, branch === "mention" ? "mention" : "workflow");
  },
  "choose-route": (button) => focusCategory(button.dataset.category, "workflow"),
  "open-add-menu"() {
    view.menu = !view.menu;
    refresh();
  },
  "zoom-in": () => zoomAround(Math.floor(zoomNow() * 10 + 1e-6) / 10 + 0.1),
  "zoom-out": () => zoomAround(Math.ceil(zoomNow() * 10 - 1e-6) / 10 - 0.1),
  "zoom-fit"() {
    view.zoom = fitZoom(MIN_ZOOM);
    view.pan = [0, 0];
    applyZoom();
  },
  "open-route"(button) {
    if ((!view.draft.id || changed()) && !armed(button, "Discard changes and open?")) return;
    edit(settings.connectors.findIndex((k) => k.id === button.dataset.id));
    redraw();
  },
  "remove-step"() {
    STEPS[view.open].remove(view.draft);
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
    const error = document.getElementById("editor-error");
    if (!view.draft.graph?.nodes?.length) {
      error.textContent = OLDER_HUB;
      return;
    }
    syncWatchFromTrigger(view.draft);
    const decide = view.draft.trigger.decide;
    if (decide && fallbackOf(decide)?.mention?.trim()) decide.fallback = null;
    const c = tidy(view.draft);
    syncWatchFromTrigger(c);
    if (!c.name) {
      error.textContent = "Name the workflow.";
      return settingsPage.querySelector(".editor-bar .title").focus();
    }
    const api = graphApi();
    const graphErr = api?.validate(c.graph, (id) => settings.connectors.some((k) => k.id === id && k.trigger?.kind === "called"));
    if (graphErr) {
      document.getElementById("editor-error").textContent = graphErr;
      return;
    }
    const bad = c.graph.nodes.find((n) => graphNodeIssues(n, c).length);
    if (bad) {
      showStep(bad.id, true);
      document.getElementById("editor-error").textContent = graphNodeIssues(bad, c)[0];
      return;
    }
    try {
      const saved = await connectorCall("save_connector", { connector: c });
      // The hub pushes its settings before it answers, so a new workflow may already be listed.
      let index = settings.connectors.findIndex((known) => known.id === saved.id);
      if (index < 0) index = settings.connectors.push(saved) - 1;
      settings.connectors[index] = saved;
      Object.assign(view, { index, draft: clone(saved), original: JSON.stringify(saved) });
      redraw();
    } catch (err) {
      error.textContent = err;
    }
  },
  async "delete-connector"(button) {
    if (!armed(button, "Click again to delete")) return;
    await connectorCall("delete_connector", { id: view.draft.id });
    settings.connectors = settings.connectors.filter((known) => known.id !== view.draft.id);
    view = { name: "list" };
    redraw();
  },
};

let canvasPan = null;
let canvasPanned = false;

settingsPage.addEventListener("click", (e) => {
  const button = e.target.closest("[data-action]");
  if (button) {
    e.preventDefault();
    return actions[button.dataset.action](button);
  }
  if (canvasPanned || !e.target.closest(".canvas") || e.target.closest(".step, .branch, .add-menu, .graph-picker, .edge-add, .graph-port-add, .port-handle")) return;
  if (view.graphPick) {
    view.graphPick = null;
    return refresh();
  }
  if (view.open) showStep(null);
});

settingsPage.addEventListener("pointerdown", (e) => {
  canvasPanned = false;
  const handle = e.target.closest(".port-handle");
  if (handle && view.name === "edit") {
    view.connectFrom = { node: handle.dataset.node, port: handle.dataset.port };
    handle.setPointerCapture(e.pointerId);
    return;
  }
  const nodeBtn = e.target.closest(".graph-node .node");
  if (nodeBtn && view.name === "edit" && !e.target.closest(".warn")) {
    const id = nodeBtn.closest(".graph-node").dataset.node;
    const node = view.draft.graph.nodes.find((n) => n.id === id);
    if (node) {
      view.graphDrag = { id, pointer: e.pointerId, x: e.clientX, y: e.clientY, pos: [...node.position] };
      nodeBtn.setPointerCapture(e.pointerId);
      return;
    }
  }
  const canvas = e.target.closest(".canvas");
  if (!canvas || e.button !== 0 || e.pointerType === "touch" || e.target.closest(".step, .branch, button, .add-menu, .graph-picker, .edge-add, .port-handle")) return;
  canvasPan = { canvas, id: e.pointerId, x: e.clientX, y: e.clientY, left: canvas.scrollLeft, top: canvas.scrollTop };
});

settingsPage.addEventListener("pointermove", (e) => {
  if (view.graphDrag?.pointer === e.pointerId) {
    const api = graphApi();
    const zoom = zoomNow() || 1;
    const dx = (e.clientX - view.graphDrag.x) / zoom;
    const dy = (e.clientY - view.graphDrag.y) / zoom;
    view.draft.graph = api.move(view.draft.graph, view.graphDrag.id, [Math.round(view.graphDrag.pos[0] + dx), Math.round(view.graphDrag.pos[1] + dy)]);
    refresh();
    return;
  }
  if (canvasPan?.id !== e.pointerId) return;
  const dx = e.clientX - canvasPan.x;
  const dy = e.clientY - canvasPan.y;
  if (!canvasPanned && Math.hypot(dx, dy) < 4) return;
  if (!canvasPanned) {
    canvasPanned = true;
    canvasPan.canvas.setPointerCapture(e.pointerId);
    canvasPan.canvas.classList.add("panning");
  }
  canvasPan.canvas.scrollLeft = canvasPan.left - dx;
  canvasPan.canvas.scrollTop = canvasPan.top - dy;
});

for (const type of ["pointerup", "pointercancel"]) {
  settingsPage.addEventListener(type, (e) => {
    if (view.graphDrag?.pointer === e.pointerId) {
      view.graphDrag = null;
      return;
    }
    if (view.connectFrom) {
      const target = [...document.elementsFromPoint(e.clientX, e.clientY)].map((el) => el.closest?.(".graph-node")).find(Boolean);
      const api = graphApi();
      if (target && api) {
        try {
          view.draft.graph = api.connect(view.draft.graph, view.connectFrom.node, view.connectFrom.port, target.dataset.node);
        } catch {}
      }
      view.connectFrom = null;
      refresh();
    }
    if (canvasPan?.id !== e.pointerId) return;
    canvasPan.canvas.classList.remove("panning");
    canvasPan = null;
  });
}

settingsPage.addEventListener(
  "wheel",
  (e) => {
    if (!(e.ctrlKey || e.metaKey) || !e.target.closest(".canvas")) return;
    e.preventDefault();
    // A mouse notch is about 100 and a pinch frame a few units, so both zoom by at most 10% a frame.
    zoomAround(zoomNow() * Math.exp(-Math.max(-10, Math.min(10, e.deltaY)) / 100), [e.clientX, e.clientY]);
  },
  { passive: false },
);

settingsPage.addEventListener(
  "scroll",
  (e) => {
    if (e.target.classList?.contains("canvas") && view.name === "edit") view.pan = [e.target.scrollLeft, e.target.scrollTop];
  },
  true,
);

settingsPage.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && e.target.id === "linear-key") actions["connect-linear"]();
  if (e.key === "Enter" && e.target.id === "slack-token") actions["connect-slack"]();
  if (e.key === "Enter" && e.target.id === "vercel-key") actions["connect-vercel"]();
  if (e.key === "Enter" && e.target.id === "claude-token") actions["connect-upstash"]();
  if (view.name !== "edit" || page !== "connectors") return;
  if (e.key === "s" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    actions["save-connector"]();
  }
  if (e.key === "Escape" && view.menu) {
    view.menu = false;
    return refresh();
  }
  if (e.key === "Escape" && view.open) showStep(null);
  if ((e.key === "Delete" || e.key === "Backspace") && view.selectedEdge != null && !e.target.closest("input, textarea, select")) {
    const api = graphApi();
    const edge = view.draft.graph.edges[view.selectedEdge];
    if (api && edge) {
      e.preventDefault();
      view.draft.graph = api.disconnect(view.draft.graph, edge);
      view.selectedEdge = null;
      refresh();
    }
  }
});

settingsPage.addEventListener("change", (e) => {
  const kind = e.target.dataset.boxKind;
  if (!kind) return;
  const access = boxAccess();
  const mode = e.target.value;
  access[kind] = { mode };
  if (mode === "fixed" && settings.boxes?.[kind]?.row) access[kind].row = settings.boxes[kind].row;
  connectorCall("set_box_access", { access }).then((saved) => {
    settings.boxes = saved;
    showPage("integrations");
  }, (err) => {
    const line = document.getElementById("upstash-error");
    if (line) line.textContent = err;
  });
});

settingsPage.addEventListener("input", (e) => {
  const path = e.target.dataset.bind;
  if (!path || path in RESHAPES) return;
  if (e.target.tagName === "SELECT" && e.target.value === NEW_ROUTE) return newRoute(e.target);
  const decide = view.draft.trigger.decide;
  const renamed = /^trigger\.decide\.categories\.\d+\.name$/.test(path) && decide.fallback && fallbackOf(decide);
  const listed = "list" in e.target.dataset;
  let value = e.target.type === "checkbox" ? e.target.checked : listed ? e.target.value.split(",").map((s) => s.trim()) : e.target.value;
  if (path.includes(".choices") && path.startsWith("graph.nodes")) {
    value = value.map((label) => ({ id: label, label, description: "" }));
  }
  bind(path, value);
  if (path.startsWith("graph.nodes.") && (path.endsWith(".classify") || path.endsWith(".question") || /\.cases\.\d+\.equals$/.test(path))) {
    const node = boundGraphNode(path);
    if (node && graphNodeKind(node) === "switch") {
      if (path.endsWith(".classify")) keepSwitchQuestion(node);
      const caseMatch = /\.cases\.(\d+)\.equals$/.exec(path);
      if (caseMatch) retargetCasePort(node, Number(caseMatch[1]), e.target.closest(".category")?.querySelector('[data-action="graph-end-choice"]'));
      if (path.endsWith(".classify") || path.endsWith(".question")) {
        redraw();
        return;
      }
    }
  }
  if (path === "trigger.label" || path === "trigger.channel") syncWatchFromTrigger(view.draft);
  if (path.startsWith("graph.nodes") && path.includes(".watch.")) syncLegacyFromGraph(view.draft);
  if (renamed) decide.fallback = renamed.name.trim() || null;
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

addEventListener("resize", applyZoom);

document.querySelector("#settings-nav .links").addEventListener("click", (e) => {
  const b = e.target.closest("[data-page]");
  if (b) showPage(b.dataset.page);
});
settingsSection({ page: "general", label: "General", icon: "settings", show: () => showGeneral(), search: [
  { title: "Keep this Mac awake", terms: ["sleep display power"] },
  { title: "Organization", terms: ["sidebar working section auto settle merged inactive threads pull request"] },
  { title: "Send shortcut", terms: ["enter new line multiline cmd ctrl composer"] },
] });
settingsSection({ page: "integrations", label: "Connectors", icon: "plug", show: () => (settingsPage.innerHTML = integrationsHtml()), search: [
  { title: "Linear", terms: ["api key issues labels connector"] },
  { title: "Slack", terms: ["bot token channels mentions connector"] },
  { title: "GitHub", terms: ["token repositories clone connector"] },
  { title: "Vercel AI Gateway", terms: ["api key models classify connector"] },
  { title: "Upstash Box", terms: ["sandbox cloud box claude setup-token subscription connector permission fresh fixed ask"] },
] });
settingsSection({ page: "projects", label: "Projects", icon: "folder", show: () => showProjectsPage(), search: [{ title: "Projects", terms: ["history turns changed files"] }] });
settingsSection({ page: "connectors", label: "Workflows", icon: "workflow", show: () => redraw(), search: [{ title: "Workflows", terms: ["linear label slack mention automation trigger"] }] });
settingsSection({ page: "account", label: "Account", icon: "circle-user", show: () => showAccount(), search: [{ title: "Password", terms: ["account change sign in out email"] }] });
settingsSection({ page: "machines", label: "Machines", icon: "server", show: () => showMachinesPage(), search: [{ title: "Machines", terms: ["pair rename remove server node computer"] }] });
settingsSection({ page: "archive", label: "Archive", icon: "archive", show: () => showArchive(), search: [{ title: "Archive", terms: ["archived threads unarchive delete restore"] }] });
document.getElementById("settings-toggle").onclick = toggleSettings;
document.getElementById("settings-back").onclick = toggleSettings;

listen("connectors-checked", (payload) => {
  settings.checked = payload;
  const line = document.getElementById("checked");
  if (line) line.textContent = checkedText();
});
