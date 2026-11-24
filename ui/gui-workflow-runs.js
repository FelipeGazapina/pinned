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

function workflowRunSessionHtml(session) {
  const claude = session.claude ? ` · Claude ${esc(session.claude)}` : "";
  if (session.state === "detached") return `<p>Tab closed${claude}</p>`;
  if (session.state !== "live") return `<p>No session</p>`;
  const machine = localIdOf(session.machine);
  const open = tabByKey(keyOf(machine, session.tab))
    ? ` <button type="button" class="secondary" data-action="workflow-run-open" data-machine="${esc(machine)}" data-tab="${esc(session.tab)}">Open thread</button>`
    : "";
  return `<p class="workflow-run-session">${esc(machineName(machine))} · ${esc(session.tab)}${claude}${open}</p>`;
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
      <h3>Session</h3>${workflowRunSessionHtml(run.session)}
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

actions["workflow-run-open"] = (button) =>
  revealTab(button.dataset.machine, button.dataset.tab).catch((err) => {
    const line = button.closest(".workflow-run-session");
    if (line) line.textContent = `Couldn't open that session: ${err?.message ?? err}`;
  });

settingsSection({ page: "workflow-runs", label: "Workflow runs", icon: "rows-3", show: () => showWorkflowRuns(), search: [{ title: "Workflow runs", terms: ["workflow runs history trigger classify steps failed stalled"] }] });
