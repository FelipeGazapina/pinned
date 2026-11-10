/** What agents do as the user (CONTRACTS.md 13): the actions they ask the user to approve, shown in
 * Waiting on you and on the Agent activity page, and the audit trail of every act. */

/** The hub's `Confirmation`s that wait on the user, oldest first, as its last push listed them. */
let agentConfirmations = [];
/** Confirmation ids whose Approve or Deny is on its way to the hub. */
const agentDeciding = new Set();
/** The `NewInvite` an approved `create_invite` made. Its code shows nowhere else, so it stays until the user closes it. */
let agentInviteCode = null;
/** The Agent activity page: the audit rows loaded so far, newest first, and the page's last word. */
const agentActivityStart = () => ({ rows: [], more: false, loading: false, notice: null });
let agentActivity = agentActivityStart();
const AGENT_AUDIT_PAGE = 50;
/** Each audit outcome's badge tone. */
const AGENT_OUTCOME_TONES = { done: "success", approved: "success", pending: "primary", failed: "error", refused: "warning", denied: "muted" };

/** `remove_machine` as "Remove machine". */
const agentWords = (name) => `${name.charAt(0).toUpperCase()}${name.slice(1).replace(/_/g, " ")}`;
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
      <pre class="args">${esc(JSON.stringify(c.args, null, 2))}</pre>
      <span class="who">${esc(who)}</span>
      <div class="actions start"><button type="button" class="primary" data-agent="approve"${busy}>Approve</button><button type="button" class="outline" data-agent="deny"${busy}>Deny</button></div>
    </div>
  </div>`;
}

function agentInviteHtml() {
  return `<div class="agent-invite">
    <b>Invite code</b>
    <span class="hint">Send it to the person you invite. It works once until ${esc(inviteDate(agentInviteCode.expires_at))} and shows only until you press Done.</span>
    <div class="row"><code>${esc(agentInviteCode.code)}</code><button type="button" class="outline" data-agent="copy-code">Copy</button><button type="button" class="outline" data-agent="dismiss-code">Done</button></div>
  </div>`;
}

/** Waiting on you's agent items, above its threads: the invite code an approval made, then each
 * waiting confirmation, in one block, or nothing. */
function agentWaitingRows() {
  if (!agentInviteCode && !agentConfirmations.length) return [];
  const asks = document.createElement("div");
  asks.className = "agent-asks";
  asks.innerHTML = `${agentInviteCode ? agentInviteHtml() : ""}${agentConfirmations.map(agentConfirmationHtml).join("")}`;
  return [asks];
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
  agentSurfaceChanged();
  if (settingsOpen() && page === "agent-activity") loadAgentAudit();
}

function resetAgentSurface() {
  agentConfirmations = [];
  agentInviteCode = null;
  agentActivity = agentActivityStart();
}

/** The outcome goes to the sidebar's notice, and to the Agent activity page, which covers the sidebar. */
function agentSay(text, error = false) {
  tell(esc(text), error ? 8000 : 5000);
  agentActivity.notice = { text, error };
}

/** The account's password, which the hub asks for before a call only the owner makes, such as an
 * approval; `error` is why the last one failed. Null when the user cancels. */
function askOwnerPassword(error) {
  return askDialog(
    `<h3>Enter your password</h3>
    <p class="about">Approving an agent's request, and changes only you make, need it. It holds until you close this window, for at most 12 hours.</p>
    <label>Password<input type="password" name="password" autocomplete="current-password"></label>
    <p class="error" role="alert">${esc(error)}</p>
    <div class="actions"><button type="button" class="outline" data-cancel>Cancel</button><button class="primary">Unlock</button></div>`,
    (form) => form.password.value || undefined,
    (form) => setTimeout(() => form.password.focus()),
  );
}

async function decideAgentConfirmation(id, approve) {
  const asked = agentConfirmations.find((c) => c.id === id);
  if (!asked || agentDeciding.has(id)) return;
  agentDeciding.add(id);
  agentSurfaceChanged();
  try {
    const decided = await hub.hub({ cmd: "decide_confirmation", id, approve });
    agentConfirmations = agentConfirmations.filter((c) => c.id !== id);
    if (decided.decision === "approved" && asked.action === "create_invite" && decided.result?.code) agentInviteCode = decided.result;
    agentSay(decided.decision === "approved" ? `Approved ${asked.action}: ${decided.outcome}` : `Denied ${asked.action}`);
  } catch (e) {
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

const AGENT_CLICKS = {
  approve: (button) => decideAgentConfirmation(button.closest("[data-confirmation]").dataset.confirmation, true),
  deny: (button) => decideAgentConfirmation(button.closest("[data-confirmation]").dataset.confirmation, false),
  more: () => loadAgentAudit(agentActivity.rows.at(-1)?.at),
  "copy-code": (button) =>
    navigator.clipboard.writeText(agentInviteCode.code).then(
      () => (button.textContent = "Copied"),
      () => getSelection().selectAllChildren(button.previousElementSibling),
    ),
  "dismiss-code": () => {
    agentInviteCode = null;
    agentSurfaceChanged();
  },
};

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
    ${agentInviteCode ? agentInviteHtml() : ""}
    ${agentConfirmations.length ? `<h3 class="agent-heading">Waiting on you</h3><div class="agent-confirms">${agentConfirmations.map(agentConfirmationHtml).join("")}</div>` : ""}
    <h3 class="agent-heading">Actions</h3>
    ${trail}
    ${more ? `<div class="actions start"><button type="button" class="secondary" data-agent="more"${loading ? " disabled" : ""}>${loading ? "Loading…" : "Show more"}</button></div>` : ""}
  </section>`;
  document.getElementById("agent-activity").onclick = agentSurfaceClick;
}

settingsSection({ page: "agent-activity", label: "Agent activity", icon: "shield", show: () => showAgentActivity(), search: [{ title: "Agent activity", terms: ["audit agent actions approvals confirmations trail approve deny"] }] });
