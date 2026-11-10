/** What agents do as the user (CONTRACTS.md 13): the actions they ask the user to approve, shown in
 * Waiting on you and on the Agent activity page, and the audit trail of every act. */

/** The hub's `Confirmation`s that wait on the user, oldest first, as its last push listed them. */
let agentConfirmations = [];
/** Confirmation ids whose Approve or Deny is on its way to the hub. */
const agentDeciding = new Set();
/** What an approved action handed the user to see once, `{ action, result }`, as `create_invite`'s
 * `NewInvite` or `pair_code`'s `PairCode`. It shows nowhere else, so it stays until the user closes it. */
let agentShown = null;
/** The actions whose answer the agent never sees, and how the app shows it: a heading, the line
 * under it, and the field of the result to show and copy. */
const AGENT_SHOWN_ONCE = {
  create_invite: {
    title: "Invite code",
    show: "code",
    hint: (r) => `Send it to the person you invite. It works once until ${inviteDate(r.expires_at)} and shows only until you press Done.`,
  },
  pair_code: {
    title: "Pairing command",
    show: "command",
    hint: (r) => `Run it yourself on the new machine. Its code works once, until ${clockTime(r.expires_at)}, and shows only until you press Done.`,
  },
};
/** The credential actions whose Approve opens the app's own form for the call instead of deciding:
 * the settings page it lives on, the input to focus, the fields the user types there (sent to the
 * hub as `entered`, never seen by the agent), what to call them, and the arguments that pick which
 * form on the page is the one. */
const AGENT_FORMS = {
  connect_linear: { page: "integrations", focus: "linear-key", entered: ["key"], noun: "key" },
  connect_slack: { page: "integrations", focus: "slack-token", entered: ["token"], noun: "token" },
  connect_github: { page: "integrations", focus: "github-token", entered: ["token"], noun: "token" },
  connect_vercel: { page: "integrations", focus: "vercel-key", entered: ["key"], noun: "key" },
  connect_upstash: { page: "integrations", focus: "upstash-key", entered: ["key", "claude_token"], noun: "key and token" },
  set_box_claude_token: { page: "integrations", focus: "box-token", entered: ["token"], noun: "token" },
  add_account: { page: "accounts", entered: ["key"], noun: "key" },
  set_account_key: { page: "accounts", entered: ["key"], noun: "key", same: ["account"] },
};
/** The confirmation whose Approve opened its form, until that form approves it, Cancel, sign-out, or
 * the hub no longer lists it as waiting. */
let agentApproving = null;
/** The Agent activity page: the audit rows loaded so far, newest first, and the page's last word. */
const agentActivityStart = () => ({ rows: [], more: false, loading: false, notice: null });
let agentActivity = agentActivityStart();
const AGENT_AUDIT_PAGE = 50;
/** Each audit outcome's badge tone. */
const AGENT_OUTCOME_TONES = { done: "success", approved: "success", pending: "primary", failed: "error", refused: "warning", denied: "muted" };

/** Actions whose words alone read wrong. */
const AGENT_LABELS = { update_self: "Update pinned", pair_code: "Pair a machine", set_box_claude_token: "Keep box Claude token", remove_box_claude_token: "Forget box Claude token" };
const AGENT_NAMES = { linear: "Linear", slack: "Slack", github: "GitHub", vercel: "Vercel", upstash: "Upstash" };
/** `remove_machine` as "Remove machine", `connect_github` as "Connect GitHub". */
function agentWords(name) {
  if (AGENT_LABELS[name]) return AGENT_LABELS[name];
  const words = name.split("_").map((word) => AGENT_NAMES[word] ?? word);
  return `${words[0].charAt(0).toUpperCase()}${words.join(" ").slice(1)}`;
}
const agentSince = (at) => (ago(at) === "now" ? "just now" : `${ago(at)} ago`);

/** The name the app knows for the account's machine `id`, or "". */
function agentMachineName(id) {
  if (typeof id !== "string" || id === LOCAL) return "";
  if (id === account?.machine_id) return machines.get(LOCAL)?.hubView?.name ?? "";
  return machines.get(id)?.view.name ?? "";
}

/** The action as a person reads it, with the name of a machine its arguments point at. `args` is
 * the confirmation's object or the audit row's JSON summary. */
function agentActionTitle(action, args) {
  if (typeof args === "string") {
    try {
      args = JSON.parse(args);
    } catch {
      args = null;
    }
  }
  const machine = Object.values(args ?? {}).map(agentMachineName).find(Boolean);
  return machine ? `${agentWords(action)} ${machine}` : agentWords(action);
}

function agentConfirmationHtml(c) {
  const busy = agentDeciding.has(c.id) ? " disabled" : "";
  const who = `Asked by ${c.title || c.tab} · ${agentWords(c.role)} on ${c.machine_name || "removed machine"} · ${agentSince(c.at)}`;
  return `<div class="agent-confirm" data-confirmation="${esc(c.id)}" role="group" aria-label="${esc(`Approve ${c.action}?`)}">
    <span class="dot"></span>
    <div class="texts">
      <b class="title" title="${esc(c.about)}">${esc(agentActionTitle(c.action, c.args))}${c.target ? ` on ${esc(agentMachineName(c.target) || c.target)}` : ""}</b>
      ${Object.keys(c.args ?? {}).length ? `<pre class="args">${esc(JSON.stringify(c.args, null, 2))}</pre>` : ""}
      <span class="who">${esc(who)}</span>
      <div class="actions start"><button type="button" class="primary" data-agent="approve"${busy}>Approve</button><button type="button" class="outline" data-agent="deny"${busy}>Deny</button></div>
    </div>
  </div>`;
}

function agentShownHtml() {
  if (!agentShown) return "";
  const { title, show, hint } = AGENT_SHOWN_ONCE[agentShown.action];
  return `<div class="agent-once">
    <b>${esc(title)}</b>
    <span class="hint">${esc(hint(agentShown.result))}</span>
    <div class="row"><code>${esc(agentShown.result[show])}</code><button type="button" class="outline" data-agent="copy-code">Copy</button><button type="button" class="outline" data-agent="dismiss-code">Done</button></div>
  </div>`;
}

/** Waiting on you's agent items, above its threads: what an approval handed the user to see once,
 * then each waiting confirmation, in one block, or nothing. */
function agentWaitingRows() {
  if (!agentShown && !agentConfirmations.length) return [];
  const asks = document.createElement("div");
  asks.className = "agent-asks";
  asks.innerHTML = `${agentShownHtml()}${agentConfirmations.map(agentConfirmationHtml).join("")}`;
  return [asks];
}

/** The confirmation whose Approve opened `action`'s form, or null. */
const agentAsking = (action) => (agentApproving?.action === action ? agentApproving : null);

/** The line in `action`'s form, while Approve opened it, that names who asked and how to approve. */
function agentAskingHtml(action) {
  const c = agentAsking(action);
  if (!c) return "";
  const keyless = action === "add_account" && c.args.auth !== "opencode-go";
  const how = keyless ? "Adding the account here approves it." : `Entering the ${AGENT_FORMS[action].noun} here approves it.`;
  return `<p class="agent-asking" id="agent-asking" role="status">${esc(`Asked by ${c.title || c.tab} on ${c.machine_name || "a removed machine"}. ${how}`)}
    <button type="button" class="secondary" data-action="cancel-agent-approval">Cancel</button></p>`;
}

/** Approve on a credential action: the app's own form for the call, on that page, with what the
 * agent asked for filled in. The form approves it with what the user types there. */
async function openAgentForm(c) {
  agentApproving = c;
  const form = AGENT_FORMS[c.action];
  if (form.page === "accounts") accountsUi = { machine: localIdOf(c.target ?? c.machine), removing: null, replacing: c.args.account ?? null, error: "" };
  await openSettings(form.page);
  if (c.action === "set_box_claude_token" && !settings.upstash) {
    agentApproving = null;
    return agentSay("Connect Upstash before you approve a box Claude token.", true);
  }
  if (form.focus) document.getElementById(form.focus)?.focus();
}

/** The confirmation's Approve with what the user typed, when `call` is the one Approve opened the
 * form for. It answers as the call itself would. Else null, and the form makes its own call. */
function agentApproval(call) {
  const c = agentApproving;
  if (c?.action !== call.cmd) return null;
  const form = AGENT_FORMS[c.action];
  if (form.same?.some((arg) => call[arg] !== c.args[arg])) return null;
  const entered = Object.fromEntries(form.entered.filter((field) => call[field] !== undefined).map((field) => [field, call[field]]));
  return decideAgentConfirmation(c.id, true, entered).then((decided) => decided.result);
}

function forgetAgentApproval() {
  agentApproving = null;
  document.getElementById("agent-asking")?.remove();
}

function agentAuditRowHtml(entry) {
  const when = new Date(entry.at * 1000);
  const who = [agentWords(entry.role), entry.tab, entry.machine_name || "removed machine"].join(" · ");
  return `<div class="agent-audit-row">
    <div class="head"><b>${esc(agentActionTitle(entry.action, entry.args))}</b><span class="badge agent-outcome" data-tone="${AGENT_OUTCOME_TONES[entry.outcome] ?? "muted"}">${esc(entry.outcome)}</span><time class="meta" datetime="${when.toISOString()}">${esc(when.toLocaleString())}</time></div>
    <span class="meta">${esc(who)}</span>
    <code class="args">${esc(entry.args)}</code>
    ${entry.detail ? `<p class="detail">${esc(entry.detail)}</p>` : ""}
  </div>`;
}

function agentSurfaceChanged() {
  if (guiOn()) renderSidebar();
  drawAgentActivity();
}

/** The hub's push of the waiting confirmations, after hello and after every change. */
function onAgentConfirmations(items) {
  agentConfirmations = items;
  if (agentApproving && !items.some((c) => c.id === agentApproving.id)) forgetAgentApproval();
  agentSurfaceChanged();
  if (settingsOpen() && page === "agent-activity") loadAgentAudit();
}

function resetAgentSurface() {
  agentConfirmations = [];
  agentShown = null;
  agentApproving = null;
  agentActivity = agentActivityStart();
}

/** The outcome goes to the sidebar's notice, and to the Agent activity page, which covers the sidebar. */
function agentSay(text, error = false) {
  tell(esc(text), error ? 8000 : 5000);
  agentActivity.notice = { text, error };
}

/** Approve or Deny. `entered` is what the user typed in a credential action's form, and its failure
 * is thrown for that form to show, while the confirmation keeps waiting. */
async function decideAgentConfirmation(id, approve, entered) {
  const asked = agentConfirmations.find((c) => c.id === id);
  if (!asked || agentDeciding.has(id)) {
    if (entered) throw asked ? "It is being approved already." : "The agent's request no longer waits.";
    return;
  }
  agentDeciding.add(id);
  agentSurfaceChanged();
  try {
    const decided = await hub.hub({ cmd: "decide_confirmation", id, approve, ...(entered ? { entered } : {}) });
    agentConfirmations = agentConfirmations.filter((c) => c.id !== id);
    if (agentApproving?.id === id) forgetAgentApproval();
    if (decided.decision === "approved" && AGENT_SHOWN_ONCE[asked.action] && decided.result) agentShown = { action: asked.action, result: decided.result };
    agentSay(decided.decision === "approved" ? `Approved ${asked.action}: ${decided.outcome}` : `Denied ${asked.action}`);
    return decided;
  } catch (e) {
    if (entered) throw e;
    agentSay(`Couldn't ${approve ? "approve" : "deny"} ${asked.action}: ${e}`, true);
  } finally {
    agentDeciding.delete(id);
    agentSurfaceChanged();
  }
}

/** The newest page of the trail, put above the rows shown, or the page from unix second `before` and
 * earlier, put below them. `before` takes in its own second, so rows already shown are dropped, and a
 * page that brings nothing new ends the trail. */
async function loadAgentAudit(before) {
  const asked = hub;
  if (!asked) return;
  agentActivity.loading = true;
  drawAgentActivity();
  try {
    const rows = await asked.hub({ cmd: "agent_audit", ...(before != null ? { before } : {}), limit: AGENT_AUDIT_PAGE });
    if (hub !== asked) return;
    const first = !agentActivity.rows.length;
    const shown = new Set(agentActivity.rows.map((row) => row.id));
    const fresh = rows.filter((row) => !shown.has(row.id));
    agentActivity.rows = before != null ? [...agentActivity.rows, ...fresh] : [...fresh, ...agentActivity.rows];
    if (before != null || first) agentActivity.more = rows.length === AGENT_AUDIT_PAGE && fresh.length > 0;
  } catch (e) {
    agentActivity.notice = { text: `Couldn't load the agent activity: ${e}`, error: true };
  } finally {
    agentActivity.loading = false;
    drawAgentActivity();
  }
}

function approveAgentConfirmation(id) {
  const asked = agentConfirmations.find((c) => c.id === id);
  return asked && AGENT_FORMS[asked.action] ? openAgentForm(asked) : decideAgentConfirmation(id, true);
}

const AGENT_CLICKS = {
  approve: (button) => approveAgentConfirmation(button.closest("[data-confirmation]").dataset.confirmation),
  deny: (button) => decideAgentConfirmation(button.closest("[data-confirmation]").dataset.confirmation, false),
  more: () => loadAgentAudit(agentActivity.rows.at(-1)?.at),
  "copy-code": (button) =>
    navigator.clipboard.writeText(agentShown.result[AGENT_SHOWN_ONCE[agentShown.action].show]).then(
      () => (button.textContent = "Copied"),
      () => getSelection().selectAllChildren(button.previousElementSibling),
    ),
  "dismiss-code": () => {
    agentShown = null;
    agentSurfaceChanged();
  },
};

actions["cancel-agent-approval"] = forgetAgentApproval;

function agentSurfaceClick(e) {
  const button = e.target.closest("[data-agent]");
  if (button && !button.disabled) AGENT_CLICKS[button.dataset.agent](button);
}

function showAgentActivity() {
  agentActivity = agentActivityStart();
  drawAgentActivity();
  loadAgentAudit();
}

function drawAgentActivity() {
  if (!settingsOpen() || page !== "agent-activity") return;
  if (!hub || !account) {
    settingsPage.innerHTML = `<h2>Agent activity</h2>
      <p class="hint">Sign in under Account to see what agents did as you and approve what they ask for.</p>
      <div class="actions start"><button class="primary" data-action="go-account">Go to Account</button></div>`;
    return;
  }
  const { rows, more, loading, notice } = agentActivity;
  const trail = rows.length ? `<div class="agent-audit">${rows.map(agentAuditRowHtml).join("")}</div>` : `<p class="hint">${loading ? "Loading…" : "No agent has acted as you yet."}</p>`;
  settingsPage.innerHTML = `<h2>Agent activity</h2><section id="agent-activity">
    <p class="hint">Every action an agent took as you, newest first. Actions that need your approval wait in Waiting on you.</p>
    ${notice ? `<p class="${notice.error ? "error" : "hint"}" role="status">${esc(notice.text)}</p>` : ""}
    ${agentShownHtml()}
    ${agentConfirmations.length ? `<h3 class="agent-heading">Waiting on you</h3><div class="agent-confirms">${agentConfirmations.map(agentConfirmationHtml).join("")}</div>` : ""}
    <h3 class="agent-heading">Actions</h3>
    ${trail}
    ${more ? `<div class="actions start"><button type="button" class="secondary" data-agent="more"${loading ? " disabled" : ""}>${loading ? "Loading…" : "Show more"}</button></div>` : ""}
  </section>`;
  document.getElementById("agent-activity").onclick = agentSurfaceClick;
}

settingsSection({ page: "agent-activity", label: "Agent activity", icon: "shield", show: () => showAgentActivity(), search: [{ title: "Agent activity", terms: ["audit agent actions approvals confirmations trail approve deny"] }] });
