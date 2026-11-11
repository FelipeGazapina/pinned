Object.assign(ICON_PATHS, {
  "circle-stop": `<circle cx="12" cy="12" r="10"/><rect x="9" y="9" width="6" height="6" rx="1"/>`,
  worker: `<rect x="4" y="8" width="16" height="12" rx="2"/><path d="M12 8V4"/><path d="M9 14v2"/><path d="M15 14v2"/>`,
});

/** The agents claude spawned in the thread, as the last page listed them. An older pinned lists none. */
const agentsOf = (tab) => threadOf(tab).agents ?? [];
const runningAgents = (tab) => agentsOf(tab).filter((a) => a.state === "running");
/** The key of the tab that started `t`: on `t`'s own machine, or on the machine `spawned_on` names
 * for a worker another machine's thread delegated there. */
function spawnerKey(t) {
  if (!t.view.spawned_by) return null;
  return `${t.view.spawned_on ? localIdOf(t.view.spawned_on) : t.machine}:${t.view.spawned_by}`;
}
/** The worker threads `tab` started that are still at work, as far down as `depth` threads. */
const workingChildren = (tab, depth = 8) =>
  depth ? tabs.filter((t) => spawnerKey(t) === `${tab.machine}:${tab.view.id}` && t !== tab && (statusOf(t) === "working" || workingChildren(t, depth - 1).length || runningAgents(t).length)) : [];
/** How many of the thread's agents and worker threads are at work. */
const busyUnder = (tab) => runningAgents(tab).length + workingChildren(tab).length;
const agentsWorking = (n) => `${n} ${n === 1 ? "agent" : "agents"} working`;

/** Keeps a page's agents on the thread, and answers whether that changes what the cards show. */
function takeAgents(t, agents = []) {
  const stepKey = (step) => step ? [step.kind, step.id, step.at, step.text, step.summary] : "";
  const key = JSON.stringify(agents.map((a) => [a.id, a.state, a.updated, a.error ?? "", a.undelivered ? 1 : 0, stepKey(a.steps?.at(-1))]));
  t.agents = agents;
  if (t.agentsKey === key) return false;
  t.agentsKey = key;
  return true;
}

/** The `Agent` tool use that spawned `agent`: in the thread, or in its parent agent's steps. */
const spawnOf = (t, agent) =>
  (agent.parent ? (t.agents ?? []).find((a) => a.id === agent.parent)?.steps ?? [] : t.items).find((i) => i.kind === "tool" && i.id === agent.tool);
const spawnedAt = (t, agent) => Date.parse(spawnOf(t, agent)?.at ?? "");
/** When a delegated task started, or when its tool use was written, in ms. */
const startedAt = (tab, agent) => {
  const started = Date.parse(agent.started ?? "");
  return Number.isFinite(started) ? started : spawnedAt(threadOf(tab), agent);
};
/** When the earliest running agent or worker thread started, in ms, or NaN. */
const delegatedSince = (tab) =>
  Math.min(...[...runningAgents(tab).map((a) => startedAt(tab, a)), ...workingChildren(tab).map((t) => (t.view.updated ?? NaN) * 1000)].filter(Number.isFinite));
/** The last thing the thread or one of its agents did, in unix seconds. */
const lastActivity = (tab) => Math.max(tab.view.updated ?? 0, ...agentsOf(tab).map((a) => Date.parse(a.updated) / 1000 || 0));

function agentSteps(agent) {
  const results = new Map(agent.steps.filter((s) => s.kind === "result").map((r) => [r.tool, r]));
  return agent.steps.filter((s) => s.kind !== "result").map((s) => (s.kind === "tool" ? { ...s, result: results.get(s.id) } : s));
}

function stepLine(step, cwd) {
  if (!step) return { icon: "bot", label: "Starting…" };
  if (step.kind === "thinking") return { icon: "brain", label: thoughtLine(step.text) || "Thinking" };
  if (step.kind === "text") return { icon: "message-circle", label: step.text.split("\n").map((l) => l.replace(/^[\s#>*-]+/, "").trim()).find(Boolean) ?? "" };
  return { icon: toolIcon(step), label: toolLabel(step, cwd) };
}

const STATE_LABEL = { running: "working", done: "done", failed: "failed", cancelled: "cancelled", stopped: "stopped" };
const stateLabel = (state) => STATE_LABEL[state] ?? state;
const taskActions = (agent) => {
  const running = agent.state === "running" && agent.id.startsWith("task-");
  const cancel = running ? `<button type="button" class="outline" data-cancel-task="${esc(agent.id)}">Cancel</button>` : "";
  const send = agent.undelivered ? `<button type="button" class="outline" data-send-task="${esc(agent.id)}">Send to session</button>` : "";
  const open = agent.id.startsWith("task-") && agent.state !== "running" && agent.conversation
    ? `<button type="button" class="outline" data-open-task="${esc(agent.id)}">Open as thread</button>` : "";
  return cancel + send + open;
};
const agentError = (agent) => agent.state === "failed" && agent.error ? `<div class="agent-error">${esc(agent.error)}</div>` : "";
const sendErrorHtml = (tab, agent) => {
  const text = agent.undelivered && threadOf(tab).sendErrors?.[agent.id];
  return text ? `<div class="agent-error">${esc(text)}</div>` : "";
};

/** Codex and Cursor tabs have no transcript, so their children sit in a strip with Open as thread. */
function delegatedStrip(tab) {
  const agents = agentsOf(tab).filter((a) => !a.parent && a.id.startsWith("task-"));
  if (!agents.length) return "";
  return `<div class="delegated-strip">${agents.map((a) => {
    const chip = [a.kind, a.model].filter(Boolean).join(" · ");
    return `<div class="delegated" data-state="${esc(a.state)}"><span class="name">${esc(a.name || "Agent")}</span>${chip ? `<span class="chip">${esc(chip)}</span>` : ""}<span class="state-label">${esc(stateLabel(a.state))}</span>${taskActions(a)}${sendErrorHtml(tab, a)}${agentError(a)}</div>`;
  }).join("")}</div>`;
}

/** A Claude model's slug as its name: claude-haiku-5-5 reads Haiku 5.5. Any other name stays. */
function modelLabel(name) {
  const m = /^claude-([a-z]+)-(\d+)(?:-(\d{1,2}))?(?:-\d{8})?$/.exec(name ?? "");
  return m ? `${m[1][0].toUpperCase()}${m[1].slice(1)} ${m[2]}${m[3] ? `.${m[3]}` : ""}` : name;
}

const tickerHtml = (since) => `<span class="ticker" data-since="${since}">${ticking(clockMs() - since)}</span>`;
/** The thread that started a worker, while it is open. */
const parentOf = (tab) => {
  const key = spawnerKey(tab);
  return key ? tabByKey(key) : null;
};

/** What a worker row says under its title: the command it asks to run, why it failed, or its latest step. */
function workerLine(tab, row) {
  if (row.state === "needs") {
    const ask = row.ask;
    if (ask?.kind === "approval" && ask.command) return { text: `Asks to run ${ask.command.split("\n")[0]}`, tone: "needs" };
    if (ask?.kind === "question") return { text: ask.text ? `Asks: ${ask.text.split("\n")[0]}` : "Asks a question", tone: "needs" };
    if (ask?.kind === "plan") return { text: "Waits on its plan", tone: "needs" };
    return { text: "Needs your input", tone: "needs" };
  }
  const error = row.agent?.error ?? (row.tab && row.state === "failed" ? row.tab.view.message : null);
  if (row.state === "failed" && error) return { text: error, tone: "error" };
  if (row.agent && !row.agent.steps?.length) {
    const spawn = spawnOf(threadOf(tab), row.agent);
    if (spawn?.kind === "tool" && (spawn.detail || spawn.summary)) return { text: `Delegated ${spawn.summary || row.agent.name || "task"}`, tone: "" };
  }
  const items = row.tab && row.state === "working" ? threadOf(row.tab).items : [];
  const last = items.findLast((i) => i.kind === "tool" || i.kind === "text" || i.kind === "thinking");
  const running = last?.kind === "tool" && last.name === "Bash" && !items.some((i) => i.kind === "result" && i.tool === last.id);
  return { text: running && row.step ? `Running ${row.step}` : row.step ?? "", tone: "" };
}

const WORKER_STEPS = 5;
/** A worker's latest tool steps, each with its result once it has one. */
function workerSteps(row) {
  if (row.agent) return agentSteps(row.agent).filter((s) => s.kind === "tool").slice(-WORKER_STEPS);
  const items = threadOf(row.tab).items.slice(-400);
  const results = new Map(items.filter((i) => i.kind === "result").map((r) => [r.tool, r]));
  return items.filter((i) => i.kind === "tool").slice(-WORKER_STEPS).map((s) => ({ ...s, result: results.get(s.id) }));
}

function workerStepLineHtml(step, row, cwd, last) {
  const live = last && (row.state === "working" || row.state === "needs");
  const [tag, cls] = step.result ? (step.result.error ? ["Failed", "failed"] : ["Done", "done"]) : live ? ["Now", "now"] : ["Stopped", "stopped"];
  return `<div class="worker-line ${cls}"><span class="tag">${tag}</span><span class="what">${esc(toolLabel(step, cwd))}</span></div>`;
}

/** Messages typed into a worker row's line, kept across the redraws that replace the input. */
const workerDrafts = new Map();
function focusWorkerSay(key) {
  const input = threadView.querySelector(`[data-worker-say="${CSS.escape(key)}"]`);
  if (!input) return;
  input.focus({ preventScroll: true });
  input.setSelectionRange(input.value.length, input.value.length);
}

/** Sends a worker row's line to that worker's session as its next prompt, unless it waits on the
 * user (waitsOnUser), whose ask only its own thread answers. True once it went out; the line then
 * closes, and keeps its words if not. `quiet` leaves the saying to the caller. */
async function sayToWorker(key, text, quiet = false) {
  const worker = tabByKey(key);
  const words = text.trim();
  if (!worker || !words) return false;
  if (waitsOnUser(worker)) {
    if (!quiet) showComposerNote(`${threadTitle(worker)} is waiting on you. Open it to answer.`, true);
    rerender();
    return false;
  }
  try {
    worker.prompts = (worker.prompts ?? 0) + 1;
    await callOn(worker.machine, { cmd: "prompt", tab: worker.view.id, text: words });
    workerDrafts.delete(key);
    threadOf(active).open.delete(`say:${key}`);
    if (!quiet) tell(`Sent to ${esc(threadTitle(worker))}.`, 3000);
    rerender();
    setTimeout(() => fetchThread(worker), 300);
    return true;
  } catch (e) {
    showComposerNote(`Couldn't send it to ${threadTitle(worker)}: ${e.message ?? e}`, true);
    return false;
  }
}

function workerActionsHtml(tab, row) {
  const id = esc(row.key);
  if (row.tab) {
    return `<button type="button" class="outline" data-worker-open="${id}">Open thread</button><button type="button" class="outline" data-toggle="${esc(`say:${row.key}`)}">Send it a message</button>${row.state === "working" ? `<button type="button" class="outline danger" data-worker-cancel="${id}">Cancel</button>` : ""}`;
  }
  const agent = row.agent;
  const subagent = row.kind === "subagent" && agent.tool ? `<button type="button" class="outline" data-subagent="${esc(agent.tool)}" data-title="${esc(agent.name || "Subagent")}">Open subagent</button>` : "";
  return taskActions(agent).replace('class="outline" data-cancel-task', 'class="outline danger" data-cancel-task') + subagent;
}

/** One worker in the block: its state, title and latest step, opening to its recent steps and actions. */
function workerRowHtml(tab, row, t) {
  const key = `worker:${row.key}`;
  const open = t.open.has(key);
  const id = esc(row.key);
  const cwd = row.tab?.view.cwd ?? tab.view.cwd;
  const line = workerLine(tab, row);
  const model = modelLabel(row.model);
  const time = row.state === "working" && row.since ? tickerHtml(row.since) : row.elapsedMs != null ? duration(row.elapsedMs) : row.since ? tickerHtml(row.since) : "";
  const review = row.state === "needs" && row.tab ? `<button type="button" class="worker-review" data-worker-open="${id}">Review</button>` : "";
  const head = `<div class="worker-head"><button type="button" class="worker-toggle" data-toggle="${esc(key)}" aria-expanded="${open}" aria-label="${open ? "Collapse" : "Expand"} ${esc(row.title)}"></button>${glyphHtml(row.state, 12)}
    <span class="worker-text"><span class="worker-title">${esc(row.title || "Agent")}</span>${line.text ? `<span class="worker-step${line.tone ? ` ${line.tone}` : ""}" title="${esc(line.text)}">${esc(line.text)}</span>` : ""}</span>
    ${review}${model ? `<span class="worker-model">${esc(model)}</span>` : ""}${time ? `<span class="worker-time">${time}</span>` : ""}${icon("chevron-right", "chev")}</div>`;
  const attrs = `${row.agent ? ` data-agent="${esc(row.agent.id)}"` : ""}${row.depth ? ` data-depth="${row.depth}" style="--depth: ${row.depth}"` : ""}`;
  if (!open) return `<div class="worker-row" data-worker="${id}" data-state="${row.state}"${attrs}>${head}</div>`;
  const steps = workerSteps(row);
  const spawn = row.agent && !steps.length ? spawnOf(threadOf(tab), row.agent) : null;
  const brief = spawn?.kind === "tool" && spawn.detail ? `<div class="narration md">${markdown(spawn.detail)}</div>` : "";
  const saying = !row.tab || !t.open.has(`say:${row.key}`) ? ""
    : row.state === "needs" || waitsOnUser(row.tab)
      ? `<div class="worker-say-waiting" role="status"><span>${esc(row.title)} is waiting on you. A message would answer its ask, so answer it in its thread.</span><button type="button" class="outline" data-worker-open="${id}">Open</button></div>`
      : `<input type="text" class="worker-say" data-worker-say="${id}" value="${esc(workerDrafts.get(row.key) ?? "")}" placeholder="Message ${esc(row.title)}. Enter sends it, Esc closes." aria-label="Message ${esc(row.title)}">`;
  const body = `<div class="worker-body">${steps.map((s, i) => workerStepLineHtml(s, row, cwd, i === steps.length - 1)).join("")}${brief}${row.agent ? sendErrorHtml(tab, row.agent) : ""}
    <div class="worker-actions">${workerActionsHtml(tab, row)}</div>${saying}</div>`;
  return `<div class="worker-row" data-worker="${id}" data-state="${row.state}" data-open${attrs}>${head}${body}</div>`;
}

/** Writes each drawn worker row's latest step in place, which workersKey leaves out for a collapsed
 * row so that a step does not repaint the whole thread. */
function patchWorkerSteps(tab, rows) {
  for (const row of rows) {
    const el = threadView.querySelector(`.worker-row[data-worker="${CSS.escape(row.key)}"] > .worker-head .worker-step`);
    const { text } = workerLine(tab, row);
    if (!el || el.textContent === text) continue;
    el.textContent = text;
    el.title = text;
  }
}

/** The thread's workers in one block: how many are done, at work and waiting on the user, then a row each. */
function workersBlockHtml(tab, t) {
  const rows = workersNow(tab);
  if (!rows.length) return "";
  const s = workersSummary(rows);
  const working = s.working ? `<span class="workers-working">${s.working} working${s.since ? ` · ${tickerHtml(s.since)}` : ""}</span>` : "";
  const needs = s.needs ? `<span class="workers-needs">${s.needs} needs you</span>` : "";
  return `<section class="workers" aria-label="Workers"><div class="workers-head">${icon("worker", "lead")}<span class="workers-title">Workers</span><span class="workers-count">${s.done} of ${s.total} done</span><span class="spacer"></span>${working}${needs}</div>
    <div class="worker-rows">${rows.map((row) => workerRowHtml(tab, row, t)).join("")}</div></section>`;
}

/** The turn the Workers block sits in: the last one that started a worker or an agent, else the last. */
const SPAWNS = /(^|__)delegate_task$|^(Agent|Task)$/;
const workersHome = (turns) => turns.findLast((x) => x.steps.some((s) => s.kind === "tool" && SPAWNS.test(s.name))) ?? turns.at(-1);

/** The notices pinned types into a thread for its workers, worded as machine.rs report_worker and
 * the delegate registry's notice_line write them: a line that starts one, and what it reads as. */
const NOTICES = [
  [/^\[pinned\] Worker (?:[^\s"]+ )?"(.*?)" finished\b/, "reported back"],
  [/^\[pinned\] Worker (?:[^\s"]+ )?"(.*?)" needs input\b/, "needs input"],
  [/^\[pinned\] Worker (?:[^\s"]+ )?"(.*?)" stopped on an error\b/, "stopped on an error"],
  [/^\[pinned\] Worker "(.*?)" on .+? waits on the user for /, "needs input"],
  [/^\[pinned\] Delegated task \S+ "(.*?)" \(.*?\) finished: result available/, "reported back"],
  [/^\[pinned\] Delegated task \S+ "(.*?)" \(.*?\) finished: failed/, "failed"],
  [/^\[pinned\] Delegated task \S+ "(.*?)" \(.*?\) finished: cancelled/, "was cancelled"],
  [/^\[pinned\] Delegated task \S+ "(.*?)" \(.*?\) (?:is still working|replied and still runs)/, "is still working"],
];
function noticeAt(line) {
  for (const [pattern, what] of NOTICES) {
    const m = pattern.exec(line);
    if (m) return { title: m[1], what };
  }
  return null;
}

/** The worker notices in a prompt, each running to the next one's line, and the words before the
 * first, or null when the prompt holds none. */
function noticesIn(text) {
  const lines = text.split("\n");
  const starts = lines.map((l, i) => [i, noticeAt(l)]).filter(([, n]) => n);
  if (!starts.length) return null;
  const notices = starts.map(([i, n], k) => ({ ...n, text: lines.slice(i, starts[k + 1]?.[0] ?? lines.length).join("\n").trim() }));
  return { mine: lines.slice(0, starts[0][0]).join("\n").trim(), notices };
}
/** Whether a prompt is only worker notices, which the minimap does not count as the user's. */
const onlyNotices = (text) => {
  const found = /^\s*\[pinned\] (Worker|Delegated task) /.test(text) && noticesIn(text);
  return !!found && !found.mine;
};

/** One notice as a rule across the lane, "<title> reported back · 6:47 PM", that opens to its text. */
function noticeHtml(notice, at, t, key) {
  const when = dateOf(at) ? ` · ${dayAware(at)}` : "";
  return `<details class="reported" data-fold="${esc(key)}" data-what="${esc(notice.what)}"${t.open.has(key) ? " open" : ""}><summary><span class="rule"></span><span class="what">${esc(`${notice.title} ${notice.what}${when}`)}</span><span class="rule"></span></summary>
    <div class="reported-body md">${markdown(notice.text, promptMarked)}</div></details>`;
}

/** The model a worker runs: as its transcript names it, else as its parent's delegate_task call did. */
const workerModel = (tab, parent) =>
  modelOf(tab)?.name ?? threadOf(tab).items.findLast((i) => i.kind === "model")?.name ?? (parent && delegatedModels(parent).get(tab.view.id)) ?? null;
/** Whether a worker runs in a worktree of its own rather than in its parent's folder. */
const ownWorktree = (tab, parent) => !!threadOf(tab).workspace?.worktree && tab.view.cwd !== parent?.view.cwd;

/** A worker's first prompt: the brief its parent wrote, under the model it runs and where. */
function briefHtml(user, t, tab) {
  const lines = user.text.trim().split("\n");
  const titled = lines[0].trim() === threadTitle(tab).trim() && lines.slice(1).join("\n").trim();
  const text = titled ? lines.slice(1).join("\n").trim() : user.text.trim();
  const key = `full:${user.id}`;
  const long = isLongText(text) || text.length > 280 || text.split("\n").length > 3;
  const clamped = long && !t.open.has(key);
  const parent = parentOf(tab);
  const from = !parent || parent.view.role === "orchestrator" ? "Brief from the Captain" : `Brief from ${threadTitle(parent)}`;
  const meta = [modelLabel(workerModel(tab, parent)), ownWorktree(tab, parent) && "own worktree"].filter(Boolean).join(" · ");
  return `<div class="brief"><div class="brief-head"><span class="brief-from">${esc(from)}</span>${meta ? `<span class="brief-meta">${esc(meta)}</span>` : ""}<span class="spacer"></span>
    ${long ? `<button type="button" class="brief-full" data-toggle="${esc(key)}" aria-expanded="${!clamped}">${clamped ? "Show full brief" : "Show less"}</button>` : ""}</div>
    <div class="brief-text md${clamped ? " clamped" : ""}">${markdown(text, promptMarked)}</div></div>`;
}

/** The strip at the top of a worker's thread that names its parent and opens it. */
function workerOfHtml(tab) {
  const parent = tab.view.role === "worker" && parentOf(tab);
  if (!parent) return "";
  return `<div class="worker-of">${icon("worker", "lead")}<span class="text">Worker of <b>${esc(threadTitle(parent))}</b>. Its report goes back to that thread.</span>
    <button type="button" class="open-parent" data-open-parent>Open parent thread</button></div>`;
}

/** What a worker's strip and brief draw from outside its items, for renderThread's key. */
function workerViewKey(tab) {
  if (tab.view.role !== "worker") return null;
  const parent = parentOf(tab);
  return [parent?.key ?? null, parent && threadTitle(parent), parent?.view.role ?? null, workerModel(tab, parent), ownWorktree(tab, parent)];
}
