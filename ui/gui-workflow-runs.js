/** The Workflow runs page, after Workflows: the account's workflow runs, newest first, as
 * `workflow_runs` answers them (crates/core/src/graph/runner.rs `view`). Each row opens to what
 * started the run, what its classify steps answered, its session and every step. Triggers and steps
 * hold text from Slack, Linear and PostHog, so every value of the answer goes through `esc`. */

/** `runs` is the last answer, null until it arrives; `error` stands in for the list when the call failed. */
let workflowRunsUi = { runs: null, error: null };

const WORKFLOW_RUN_STATUS = { running: "Running", waiting: "Waiting", done: "Done", failed: "Failed" };

/** An RFC 3339 time as `ago` words it, or a dash when the run has none. */
function workflowRunAgo(at) {
  if (!at) return "—";
  const since = ago(Date.parse(at) / 1000);
  return since === "now" ? "just now" : `${since} ago`;
}
const workflowRunStep = (step) => step.name || step.node;

/** What the run works on, read from its trigger: the issue, the error or the channel. */
function workflowRunSubject(run) {
  const trigger = run.trigger ?? {};
  if (trigger.kind === "issue") return [trigger.identifier, trigger.title].filter(Boolean).join(" ");
  if (trigger.kind === "observed") return trigger.title ?? "";
  if (trigger.kind === "slack") return `#${trigger.channelName ?? ""}`;
  return run.subject?.issue ?? [run.subject?.channel, run.subject?.thread].filter(Boolean).join(" · ");
}

function workflowRunOutcomeHtml(run) {
  if (run.status === "failed") return `<p class="error">Failed: ${esc(run.why)}</p>`;
  const on = run.waiting_on;
  if (!on) return "";
  const workflow = on.workflow === run.workflow ? run.workflow_name : on.workflow;
  return `<p>Waiting on <b>${esc(workflowRunStep(on))}</b> in ${esc(workflow)} · last step ${esc(workflowRunAgo(run.last_step_at))}</p>`;
}

function workflowRunClassifiedHtml(run) {
  if (!run.classified?.length) return `<p class="hint">No classify step answered</p>`;
  return run.classified
    .map((c) => `<p><b>${esc(workflowRunStep(c))}</b></p><ul>${c.answers.map(([question, answer]) => `<li>${esc(question)}: ${esc(answer)}</li>`).join("")}</ul>`)
    .join("");
}

/** A failed run whose tab this app has can also be forked or rewound from its card. */
function workflowRunSessionHtml(run) {
  const session = run.session;
  const claude = session.claude ? ` · Claude ${esc(session.claude)}` : "";
  if (session.state === "detached") return `<p>Tab closed${claude}</p>`;
  if (session.state !== "live") return `<p>No session</p>`;
  const machine = localIdOf(session.machine);
  const button = (action, label, extra = "") => ` <button type="button" class="secondary" data-action="${action}" data-machine="${esc(machine)}" data-tab="${esc(session.tab)}"${extra}>${label}</button>`;
  const here = tabByKey(keyOf(machine, session.tab));
  const open = here ? button("workflow-run-open", "Open thread") : "";
  // The conversation the run ran in: fork and rewind refuse once the tab has moved on to another.
  const sessionAttr = ` data-session="${esc(session.claude ?? "")}"`;
  const recover = here && run.status === "failed" ? button("workflow-run-fork", "Fork from here", sessionAttr) + button("workflow-run-rewind", "Rewind", sessionAttr) : "";
  return `<p class="workflow-run-session">${esc(machineName(machine))} · ${esc(session.tab)}${claude}${open}${recover}</p>`;
}

function workflowRunStepsHtml(run) {
  const steps = run.steps.map((step) => {
    const elsewhere = step.workflow === run.workflow ? "" : ` <span class="hint">${esc(step.workflow)}</span>`;
    return `<li><span class="hint">${esc(workflowRunAgo(step.at))}</span>${elsewhere} <b>${esc(workflowRunStep(step))}</b> <code>${esc(JSON.stringify(step.what))}</code></li>`;
  });
  return `<ol class="workflow-run-steps">${steps.join("")}</ol>`;
}

function workflowRunHtml(run) {
  return `<details class="card workflow-run" data-status="${esc(run.status)}">
      <summary><b>${esc(run.workflow_name)}</b> <span>${esc(workflowRunSubject(run))}</span>
        <span class="workflow-run-status">${esc(WORKFLOW_RUN_STATUS[run.status] ?? run.status)}</span>
        <span class="hint">Started ${esc(workflowRunAgo(run.started_at))} · last step ${esc(workflowRunAgo(run.last_step_at))}</span></summary>
      ${workflowRunOutcomeHtml(run)}
      <h3>Trigger</h3><pre>${esc(JSON.stringify(run.trigger, null, 2))}</pre>
      <h3>Classification</h3>${workflowRunClassifiedHtml(run)}
      <h3>Session</h3>${workflowRunSessionHtml(run)}
      <h3>Steps</h3>${workflowRunStepsHtml(run)}
    </details>`;
}

function workflowRunsHtml() {
  const { runs, error } = workflowRunsUi;
  let body;
  if (error) body = `<p class="error">${esc(error)}</p>`;
  else if (!runs) body = `<p class="hint">Loading…</p>`;
  else if (!runs.length) body = `<p class="hint">No workflow has run yet.</p>`;
  else body = runs.map(workflowRunHtml).join("");
  return `<h2>Workflow runs</h2><p class="hint">Every run of the account's workflows, newest first: what started it, what it decided, its session and each step.</p>${body}`;
}

function drawWorkflowRuns() {
  if (!settingsOpen() || page !== "workflow-runs") return;
  settingsPage.innerHTML = workflowRunsHtml();
}

async function showWorkflowRuns() {
  const asked = (workflowRunsUi = { runs: null, error: null });
  drawWorkflowRuns();
  let answer;
  try {
    answer = { runs: await connectorCall("workflow_runs"), error: null };
  } catch (e) {
    answer = { runs: null, error: isOlderPinned(e) ? "Update pinned on the hub to see workflow runs." : String(e?.message ?? e) };
  }
  // A later visit asked again, and its answer is the one to draw.
  if (workflowRunsUi !== asked) return;
  workflowRunsUi = answer;
  drawWorkflowRuns();
}

/** Writes why a card action stopped on its session line, as text. */
function workflowRunSay(button, text) {
  const line = button.closest(".workflow-run-session");
  if (line) line.textContent = text;
}

actions["workflow-run-open"] = (button) =>
  revealTab(button.dataset.machine, button.dataset.tab).catch((err) => workflowRunSay(button, `Couldn't open that session: ${err?.message ?? err}`));

/** The sessions a fork or rewind is working on now, by machine and tab: a second click on one does nothing until the first is done. */
const workflowRunBusy = new Set();

/** Runs `act` for the card's session, unless a fork or rewind of it is already under way. */
async function workflowRunOnce(button, act) {
  const key = keyOf(button.dataset.machine, button.dataset.tab);
  if (workflowRunBusy.has(key)) return;
  workflowRunBusy.add(key);
  try {
    await act();
  } finally {
    workflowRunBusy.delete(key);
  }
}

/** The run's tab and the turns of its conversation, read first when this app hasn't yet, or why the
 * card can't fork or rewind it. */
async function workflowRunTurns(button) {
  const tab = tabByKey(keyOf(button.dataset.machine, button.dataset.tab));
  if (!tab) return { why: "That session's tab is gone." };
  if (!threadOf(tab).loaded) await fetchThread(tab);
  const t = threadOf(tab);
  if (t.error) return { why: `Couldn't read that session: ${t.error}` };
  // A read already in flight returns at once from fetchThread, with the thread still unloaded.
  if (!t.loaded) return { why: "Still reading that session. Try again in a moment." };
  if (t.session && t.session !== button.dataset.session) return { why: "That session has moved on to another conversation since this run." };
  const turns = turnsFor(t);
  if (!turns.some((turn) => turn.user)) return { why: "That session has no conversation yet." };
  if (turnLive(tab)) return { why: "That session is working. Try again once it stops." };
  return { tab, turns };
}

// Forks through the newest settled response, the one that carries "Fork from this response".
actions["workflow-run-fork"] = (button) =>
  workflowRunOnce(button, async () => {
    const { tab, turns, why } = await workflowRunTurns(button);
    if (why) return workflowRunSay(button, why);
    const answer = turns.findLast((turn) => turn.answer)?.answer;
    if (!answer) return workflowRunSay(button, "Nothing to fork yet: claude hasn't finished a response.");
    try {
      await forkTab(tab, answer.id);
    } catch (err) {
      workflowRunSay(button, `Couldn't fork that session: ${err?.message ?? err}`);
    }
  });

// Opens the thread on "Edit from here" for its newest prompt; its own dialog asks before anything rewinds.
actions["workflow-run-rewind"] = (button) =>
  workflowRunOnce(button, async () => {
    const { turns, why } = await workflowRunTurns(button);
    if (why) return workflowRunSay(button, why);
    const user = turns.findLast((turn) => turn.user).user;
    try {
      await revealTab(button.dataset.machine, button.dataset.tab);
    } catch (err) {
      return workflowRunSay(button, `Couldn't open that session: ${err?.message ?? err}`);
    }
    await editFromHere(user.id);
  });

settingsSection({ page: "workflow-runs", label: "Workflow runs", icon: "rows-3", show: () => showWorkflowRuns(), search: [{ title: "Workflow runs", terms: ["workflow runs history trigger classify steps failed stalled"] }] });
