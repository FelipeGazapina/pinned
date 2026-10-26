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
const sessionSteps = (c, event) => [["agent", event], ["reply", ""], ["bug", ""], ...(c.fix === "off" ? [] : [["coding", "starts the fix"], ["review", "pull request opened"], ["proof", ""]])];
const TRIGGERS = {
  "linear-label": {
    text: "A Linear issue gets a label",
    pitch: "Claude resolves the issue, opens a pull request and moves the issue through your columns.",
    needs: "linear",
    step: "linear-trigger",
    steps: () => [["picked", "new issue"], ["agent", ""], ["coding", "starts changing code"], ["review", "pull request opened"], ["proof", ""]],
    subject: "issue",
    name: "Resolve labeled Linear issues",
    defaults: { worktree: true, columns: { picked: "Analyzing", coding: "In Dev", review: "Ready to Review" } },
    trigger: () => ({ kind: "linear-label", label: "claude" }),
    instructions: () => DEFAULT_INSTRUCTIONS,
    sentence: (c, where) => `When an issue gets the label “${c.trigger.label}”, Claude picks it up${where}.`,
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
    sentence: (c, where, then) => `When the bot is mentioned in #${c.trigger.channel.replace(/^#/, "")}, Claude ${then}${where}.`,
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
    sentence: (c, where, then) => `When ${orList(routesTo(c).map((r) => `“${r.router}”`)) || "another workflow"} routes a Slack thread here, Claude ${then}${where}.`,
  },
};
const orList = (names) => new Intl.ListFormat("en", { type: "disjunction" }).format(names);

const settingsPage = document.getElementById("settings-page");
const LINEAR_KEYS = "https://linear.app/settings/account/security";
const GITHUB_TOKENS = "https://github.com/settings/tokens/new?scopes=repo&description=pinned";
const VERCEL_KEYS = "https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai-gateway%2Fapi-keys";

/** Mirrors the backend's `get_settings`. Unsaved connectors carry an empty id. */
let settings = { linear: null, slack: null, vercel: null, connectors: [], checked: null };
const PAGES = ["general", "integrations", "projects", "connectors", "account", "machines", "archive"];
let page = PAGES.includes(load("settings-page")) ? load("settings-page") : "integrations";
let suggestions = { labels: [], columns: [], teams: [], channels: [], folders: [], repos: [], decisionModels: [] };

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
  delete settingsPage.dataset.view;
  for (const b of document.querySelectorAll("#settings-nav [data-page]")) b.classList.toggle("active", b.dataset.page === name);
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
  if (name === "connectors" && settings.vercel && !suggestions.decisionModels.length) {
    connectorCall("vercel_models").then((models) => {
      suggestions.decisionModels = models;
      fillList("vercel-models", models);
    }, console.warn);
  }
  if (name === "connectors" && settings.github && !suggestions.repos.length) {
    connectorCall("github_repos").then((repos) => {
      suggestions.repos = repos.map((r) => r.full_name);
      fillList("github-repos", suggestions.repos);
    }, console.warn);
  }
  if (name === "projects") return showProjectsPage();
  if (name === "general") return showGeneral();
  if (name === "account") return showAccount();
  if (name === "machines") return showMachinesPage();
  if (name === "archive") return showArchive();
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
  return `<h2>Connectors</h2>
    <p class="hint">Services the harness can read from and act on.</p>
    <div class="card">
      <div class="card-head"><span class="logo" data-tone="linear" data-logo="linear">${LOGOS.linear}</span><div><b>Linear</b><div class="hint">Issues, labels and comments</div></div>
        <span class="badge" data-on="${!!account}">${account ? "Connected" : "Not connected"}</span></div>
      ${body}
    </div>
    ${slackCardHtml()}
    ${vercelCardHtml()}
    ${githubCardHtml()}`;
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
    ? `<p class="connected">Connected. The team's AI Gateway balance is <b>$${esc(account.balance)}</b>.</p>
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
      <div class="card-head"><span class="logo" data-tone="vercel" data-logo="vercel">${LOGOS.vercel}</span><div><b>Vercel AI Gateway</b><div class="hint">Decision models that sort Slack messages into categories</div></div>
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
           <code>users:read</code>, <code>chat:write</code> and <code>reactions:write</code>, then install it to your workspace.</li>
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
const folderName = (path) => path.split("/").filter(Boolean).pop() ?? "";

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
/** Each app's mark, drawn in its theme-aware colours by the stylesheet. Steps of other apps keep their glyph. */
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
      const fallback = fallbackOf(c.trigger.decide);
      return [
        !settings.vercel && "Connect the Vercel AI Gateway under Connectors.",
        !c.trigger.decide.model.trim() && "Name the decision model.",
        categories.length < 2 && "Add at least two categories.",
        names.some((name) => !name) && "Name every category.",
        twice && `Two categories are named “${twice.name.trim()}”.`,
        vague && `Describe the category “${vague.name.trim()}”.`,
        removed && `The workflow that “${removed.name.trim()}” routes to was deleted.`,
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
      [folderName(c.folder) || folderName(c.repo.replace(/\.git$/, "")) || "no folder", account && runOnName(c), c.model.trim() || "default model", c.effort, PERMISSIONS[c.permissionMode]]
        .filter(Boolean)
        .join(" · "),
    issues: (c) => (c.folder.trim() || c.repo.trim() ? [] : ["Choose the folder or the repository the session opens in."]),
    fields: (c) => `${account ? `<label>Run on ${runOnHtml(c)}</label>` : ""}
      <label>Folder${account ? ` on ${esc(machineName(folderMachine(c)))}` : ""}
        <span class="row">${input("folder", c.folder, 'list="folders" placeholder="/path/to/repo" spellcheck="false"')}
        ${folderMachine(c) === LOCAL ? `<button class="secondary" data-action="pick-folder">Choose…</button>` : ""}</span></label>
      <label>Repository ${input("repo", c.repo, `list="github-repos" placeholder="${settings.github ? "owner/name, from your GitHub" : "git URL"}" spellcheck="false"`)}</label>
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

function decideHtml(c) {
  const decide = c.trigger.decide;
  const targets = settings.connectors.filter((k) => k.id !== c.id && ["slack-mention", "called"].includes(k.trigger.kind));
  const row = (cat, i) => {
    const at = `trigger.decide.categories.${i}`;
    const routes = Object.fromEntries([
      ["", "Stop"],
      ...targets.map((k) => [k.id, k.name]),
      ...(cat.workflow && !targets.some((k) => k.id === cat.workflow) ? [[cat.workflow, "A deleted workflow"]] : []),
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

/** Only a category that pings no one can take the unsure messages; one that gained a mention stays listed so its issue reads. */
function fallbackHtml(decide) {
  const names = decide.categories.filter((cat) => cat.name.trim() && (!cat.mention?.trim() || cat === fallbackOf(decide))).map((cat) => cat.name.trim());
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
  !c.repo.trim()
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

function sentence(c) {
  const machine = account && c.machine ? ` on ${runOnName(c)}` : "";
  const where = folderName(c.folder) ? ` in ${folderName(c.folder)}${machine}` : machine;
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
    <datalist id="vercel-models">${options(suggestions.decisionModels)}</datalist>
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
        <div class="zoom" role="toolbar" aria-label="Zoom">
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
const ROW = 150;
const PAD = 48;
const STUB = 40;
const END = 24;
const MENU_W = 280;

/** Optional steps of the flow that are off, which the ＋ after the chain's last node offers. */
const offSteps = (c) => flowOf(c).map(([id]) => id).filter((id) => !shown(c, id));

function layoutOf(c) {
  const nodes = [];
  const edges = [];
  const ends = [];
  const place = (node, col, row, h = node.ports > 2 ? Math.max(TILE, node.ports * 28 + 16) : TILE) => {
    nodes.push({ ...node, cx: PAD + TEXT_W / 2 + col * COL, cy: row * ROW, h });
    return nodes.at(-1);
  };
  let prev = null;
  flowOf(c).forEach(([id, label], col) => {
    const step = STEPS[id];
    const branches = shown(c, id) ? step.branches?.(c) ?? [] : [];
    const node = place({ step: id, ghost: !shown(c, id), ports: branches.length || 1 }, col, 0);
    if (prev) edges.push({ from: prev, to: node, label, dashed: prev.ghost || node.ghost });
    prev = node;
    const middle = (branches.length - 1) / 2;
    branches.forEach((branch, port) => {
      let from = node;
      branch.nodes.forEach((target, i) => {
        const placed = place(target, col + 1 + i, port - middle);
        edges.push(i ? { from, to: placed } : { from, to: placed, port, portLabel: branch.label });
        from = placed;
      });
      if (branch.end !== undefined) ends.push({ from, category: branch.end });
    });
    if (branches.length && step.stub) place({ stub: step.stub }, col + 1, middle + 0.9, 32);
  });
  if (offSteps(c).length) ends.push({ from: prev, menu: true });
  const top = Math.min(...nodes.map((n) => n.cy - n.h / 2));
  const bottom = Math.max(...nodes.map((n) => n.cy + n.h / 2 + TEXT_H));
  for (const n of nodes) n.cy += PAD - top;
  const right = Math.max(...nodes.map((n) => n.cx + TEXT_W / 2), ...ends.map((e) => e.from.cx + TILE / 2 + STUB + END));
  return { nodes, edges, ends, width: right + PAD, height: bottom - top + 2 * PAD };
}

function flowHtml(c) {
  const { nodes, edges, ends, width, height } = layoutOf(c);
  return `<div class="flow" data-width="${width}" data-height="${height}" style="width:${width}px;height:${height}px">
    ${nodes.map((n) => nodeHtml(c, n)).join("")}
    <svg class="edges" width="${width}" height="${height}" aria-hidden="true">
      <defs><marker id="flow-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0 1 9 5 0 9z"/></marker></defs>
      ${edges.map(edgeHtml).join("")}
      ${ends.map(({ from }) => `<path d="M${from.cx + TILE / 2} ${from.cy}h${STUB}"/><circle cx="${from.cx + TILE / 2}" cy="${from.cy}" r="4.5"/>`).join("")}
    </svg>
    ${edges.map(edgeLabelHtml).join("")}
    ${ends.map((end) => endHtml(c, end)).join("")}
  </div>`;
}

/** The ＋ at an open end: after the chain it lists the steps that are off, after a category that stops it picks what comes next. */
function endHtml(c, { from, menu, category }) {
  const x = from.cx + TILE / 2 + STUB;
  const at = `left:${x}px;top:${from.cy - END / 2}px`;
  if (!menu) {
    const name = c.trigger.decide.categories[category].name.trim();
    return `<button class="end" data-action="choose-route" data-category="${category}" title="${esc(`Choose what happens after “${name}”`)}" style="${at}">${icon("plus")}</button>`;
  }
  const items = offSteps(c).map((id) => `<button data-action="add-step" data-step="${id}">${iconHtml(STEPS[id])}<span>${esc(STEPS[id].ghost)}</span></button>`);
  return `<button class="end" data-action="open-add-menu" aria-expanded="${!!view.menu}" title="Add a step" style="${at}">${icon("plus")}</button>
    ${view.menu ? `<div class="add-menu" style="width:${MENU_W}px;left:${x + END - MENU_W}px;top:${from.cy + END / 2 + 6}px">${items.join("")}</div>` : ""}`;
}

const nodeAt = (n) => `left:${n.cx - TEXT_W / 2}px;top:${n.cy - n.h / 2}px`;

function nodeHtml(c, n) {
  if (n.stub) return `<div class="stub" style="${nodeAt(n)}"><button data-action="add-category">${esc(n.stub)}</button></div>`;
  if (!n.step) {
    const open = n.kind === "route" && view.open === `route:${n.category}`;
    return `<div class="branch${open ? " open" : ""}" data-branch="${n.kind}" data-category="${n.category}" style="${nodeAt(n)}">
      <button class="node" data-action="open-branch" title="${esc(n.title)}">${iconHtml(n)}</button>
      <span class="text"><b>${esc(n.title)}</b><span class="summary">${esc(n.summary)}</span></span>
    </div>`;
  }
  const step = STEPS[n.step];
  if (n.ghost) {
    return `<div class="step" data-step="${n.step}" style="${nodeAt(n)}">
      <button class="node ghost" data-action="add-step" title="${esc(step.ghost)}"><span class="icon">＋</span></button>
      <span class="text"><span class="summary">${esc(step.ghost)}</span></span></div>`;
  }
  const issues = step.issues?.(c) ?? [];
  const open = view.open === n.step;
  return `<div class="step${open ? " open" : ""}" data-step="${n.step}" style="${nodeAt(n)}">
    <button class="node${step.trigger ? " trigger" : ""}" data-action="toggle-step" aria-expanded="${open}" title="${esc(`${step.app}: ${step.title}`)}" style="height:${n.h}px">
      ${iconHtml(step)}${issues.length ? `<span class="warn" title="${esc(issues.join(" "))}">!</span>` : ""}
    </button>
    <span class="text"><b>${step.title}</b><span class="summary">${esc(step.summary(c))}</span></span>
  </div>`;
}

function ends({ from, to, port }) {
  const y1 = port === undefined ? from.cy : from.cy - from.h / 2 + (from.h * (port + 0.5)) / from.ports;
  return [from.cx + TILE / 2, y1, to.cx - TILE / 2, to.cy];
}

function edgeHtml(edge) {
  const [x1, y1, x2, y2] = ends(edge);
  const bend = (x2 - x1) / 2;
  return `<path${edge.dashed ? ' class="dashed"' : ""} d="M${x1} ${y1}C${x1 + bend} ${y1} ${x2 - bend} ${y2} ${x2 - 6} ${y2}" marker-end="url(#flow-arrow)"/>
    <circle cx="${x1}" cy="${y1}" r="4.5"/><circle cx="${x2}" cy="${y2}" r="4.5"/>`;
}

function edgeLabelHtml(edge) {
  const [x1, y1, x2, y2] = ends(edge);
  if (edge.portLabel !== undefined) return `<span class="port-label" style="left:${x1 + 8}px;top:${y1}px">${esc(edge.portLabel)}</span>`;
  return edge.label ? `<span class="edge-label" style="left:${(x1 + x2) / 2}px;top:${(y1 + y2) / 2}px">${esc(edge.label)}</span>` : "";
}

function panelHtml(c) {
  const route = /^route:(\d+)$/.exec(view.open ?? "");
  if (route) return routePanelHtml(c, Number(route[1]));
  const step = STEPS[view.open];
  if (!step || !shown(c, view.open)) return "";
  return `<aside class="node-panel" aria-label="${esc(step.title)}">
    <div class="node-panel-head">${iconHtml(step)}<span class="text"><span class="app">${step.app}</span><b>${step.title}</b></span>
      <button class="secondary close" data-action="close-step" title="Close (Esc)">✕</button></div>
    <div class="details">${step.fields(c)}${step.optional ? `<div class="actions start"><button class="secondary" data-action="remove-step">Remove step</button></div>` : ""}</div>
  </aside>`;
}

const READABLE_ZOOM = 0.75;
const MIN_ZOOM = 0.5;
const MAX_ZOOM = 1.5;

/** The zoom that shows the whole flow, never above 1. Opening a workflow floors it at a readable size; the Fit button doesn't. */
function fitZoom(floor) {
  const canvas = settingsPage.querySelector(".canvas");
  const flow = canvas.firstElementChild;
  return Math.max(floor, Math.min(1, canvas.clientWidth / flow.dataset.width, canvas.clientHeight / flow.dataset.height));
}

/** Draws the flow at the view's zoom and pan; a null zoom fits it. */
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

/** Zooms keeping the point under `at` (the canvas's centre by default) where it is. */
function zoomTo(zoom, at) {
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
  view = { name: "edit", index, draft, original: JSON.stringify(draft), open, zoom: null, pan: null, menu: false };
  loadFolders(draft);
}

/** A fresh connector of `kind`, filled with the defaults that make it work once a folder is chosen. */
function blank(kind) {
  const machine = active ? hubIdOf(active.machine) : "";
  const { name, trigger, defaults } = TRIGGERS[kind];
  const c = {
    id: "",
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
  const parts = ["claude"];
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
  return {
    ...c,
    name: t(c.name),
    folder: t(c.folder),
    repo: t(c.repo),
    model: t(c.model),
    team: t(c.team),
    label: t(c.label),
    trigger: tidyTrigger(c.trigger),
    columns: Object.fromEntries(Object.entries(c.columns).map(([stage, column]) => [stage, t(column)])),
  };
}

const defaultText = (c) => TRIGGERS[c.trigger.kind].instructions(c);

/** Fields that change which steps the flow has, so the draft is reshaped and the editor redrawn. */
const RESHAPES = {
  "trigger.kind": (c, kind) => ({ ...c, trigger: TRIGGERS[kind].trigger(c.trigger) }),
  fix: (c, fix) => ({ ...c, fix }),
  machine: (c, machine) => ({ ...c, machine }),
};

/** Applies `change` to the draft, swapping in the new defaults where the old ones were untouched. */
function reshape(change) {
  const before = view.draft;
  const after = change(clone(before));
  if (before.name === TRIGGERS[before.trigger.kind].name) after.name = TRIGGERS[after.trigger.kind].name;
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
  view.menu = false;
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
      suggestions.decisionModels = [];
      showPage("integrations");
    } catch (e) {
      document.getElementById("vercel-error").textContent = e;
    }
  },
  async "disconnect-vercel"() {
    await connectorCall("disconnect_vercel");
    settings.vercel = null;
    suggestions.decisionModels = [];
    showPage("integrations");
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
  "zoom-in": () => zoomTo(Math.floor(zoomNow() * 10 + 1e-6) / 10 + 0.1),
  "zoom-out": () => zoomTo(Math.ceil(zoomNow() * 10 - 1e-6) / 10 - 0.1),
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
    const decide = view.draft.trigger.decide;
    if (decide && fallbackOf(decide)?.mention?.trim()) decide.fallback = null;
    const c = tidy(view.draft);
    const error = document.getElementById("editor-error");
    if (!c.name) {
      error.textContent = "Name the workflow.";
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

/** A mouse drag on the canvas's ground pans it, and the click that ends it does nothing. Touch pans by scrolling. */
let canvasPan = null;
let canvasPanned = false;

settingsPage.addEventListener("click", (e) => {
  const button = e.target.closest("[data-action]");
  if (button) {
    e.preventDefault();
    return actions[button.dataset.action](button);
  }
  if (canvasPanned || !e.target.closest(".canvas") || e.target.closest(".step, .branch, .add-menu")) return;
  if (view.menu) {
    view.menu = false;
    return refresh();
  }
  if (view.open) showStep(null);
});

settingsPage.addEventListener("pointerdown", (e) => {
  canvasPanned = false;
  const canvas = e.target.closest(".canvas");
  if (!canvas || e.button !== 0 || e.pointerType === "touch" || e.target.closest(".step, .branch, button, .add-menu")) return;
  canvasPan = { canvas, id: e.pointerId, x: e.clientX, y: e.clientY, left: canvas.scrollLeft, top: canvas.scrollTop };
});

settingsPage.addEventListener("pointermove", (e) => {
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
    zoomTo(zoomNow() * Math.exp(-Math.max(-10, Math.min(10, e.deltaY)) / 100), [e.clientX, e.clientY]);
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
});

settingsPage.addEventListener("input", (e) => {
  const path = e.target.dataset.bind;
  if (!path || path in RESHAPES) return;
  if (e.target.tagName === "SELECT" && e.target.value === NEW_ROUTE) return newRoute(e.target);
  const decide = view.draft.trigger.decide;
  const renamed = /^trigger\.decide\.categories\.\d+\.name$/.test(path) && decide.fallback && fallbackOf(decide);
  bind(path, e.target.type === "checkbox" ? e.target.checked : e.target.value);
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

for (const b of document.querySelectorAll("#settings-nav [data-page]")) b.onclick = () => showPage(b.dataset.page);
document.getElementById("settings-toggle").onclick = toggleSettings;
document.getElementById("settings-back").onclick = toggleSettings;

listen("connectors-checked", (payload) => {
  settings.checked = payload;
  const line = document.getElementById("checked");
  if (line) line.textContent = checkedText();
});
