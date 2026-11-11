// What every screen draws about threads, derived once from the state the page already keeps, so no
// screen re-derives it with its own conditionals. Times are unix milliseconds. A field the app has
// no data for is null.
//
// threadState(tab) -> "working" | "needs" | "unread" | "done" | "idle"
//   One status per thread, first match wins: needs (an ask or a failure waits on the user: a
//   `waitingKindOf`, or a menu or question on screen), working (its turn runs, or a worker or agent
//   it started does), unread (a finished turn the user has not seen; never a worker, whose result
//   went up to its parent), done (a finished worker), idle (anything else).
//
// pendingAsk(tab) -> null | { kind: "approval" | "question" | "plan", text, command, choices, typed }
//   What the thread asks right now. `command` is what an approval runs; `text` is the approval's
//   description, the question, or the plan. `choices` lists the answers one tap gives, each
//   { label, about, choice } with `choice` ready for answerAsk; it is null when only opening the
//   thread can answer: no menu parsed off its screen yet, several questions in a row, or a
//   multi-select. `typed` says answerAsk takes { text } (decline with a reason, a free answer, or
//   a plan refinement).
//
// workersOf(tab) -> [{ kind, key, tab, agent, title, state, step, ask, model, elapsedMs, since }]
//   A Captain's Workers block: its worker threads (`spawned_by`, oldest first), then its delegated
//   `task-` agents and CLI subagents (top level only). `kind` is "thread" | "task" | "subagent";
//   `tab` is set for a thread, `agent` for the others. `state` is "working" | "needs" | "done" |
//   "failed" | "stopped". `step` is the latest step in one line (a done worker's last answer line).
//   `ask` is pendingAsk of a worker thread. `model` is the model's name once the transcript or the
//   task names it, else the model the parent's delegate_task call started the worker thread on. `elapsedMs` is the time it ran (`view.stats.active_ms` for a thread), `since`
//   when its current run or wait began, for a `.ticker`.
//
// workersSummary(rows) -> { total, done, working, needs, since }
//   Counts for the block's header and the phone pill; `since` is the earliest working row's.
//
// needsYouItems() -> [{ tab, kind, title, project, machine, since, ask, limit }], newest first
//   Every thread on every machine that waits on the user, on top of `waitingKindOf`: kind
//   "approval" | "question" | "plan" (with `ask`), "limit" (a usage limit stopped it; `limit` is
//   { resumesAt, account, held, switchTo: [{ id, name }] }, what #limit-note offers), "failed" (any
//   other failure), and "review" (a finished thread the user has not seen). A snoozed thread is
//   left out until it wakes or asks something (`snoozedNow`); an ask always shows.
//
// navCounts() -> { needsYou, orchestrator, reviews, usage, machinesOnline, machinesTotal }
//   `needsYou` counts needsYouItems() without reviews. `orchestrator` counts the Captains' reports
//   in the global orchestrator's thread since the user last saw it. `reviews` is the open pull
//   requests that ask the user's review, null until the first answer; it is fetched when tabs first
//   arrive and then at most every 5 minutes while the page shows. `usage` is { percent, resetsAt,
//   account, provider, week } of the 5-hour window of the active thread's account, else the most
//   used one, or null; `week` is that account's 7-day window { percent, resetsAt }, or null.
//
// answerAsk(tab, choice) -> Promise<boolean>
//   Answers any thread's ask without opening it and without touching the open thread's composer.
//   `choice` is "approve" | "decline" | "always" | { option: n } | { text }. False when the menu is
//   gone or changed, the choice is not offered, or the ask needs the thread open.
//
// workersKey(tab), onModelChange(fn), modelChanged()
//   workersKey is in renderThread's `content` key, so a worker's change repaints its parent.
//   modelChanged runs after every sidebar render, which follows every tab event, screen change and
//   thread read: it repaints the open thread when its workers changed and calls each onModelChange
//   listener. onModelChange returns a function that unsubscribes.
//
// glyphHtml(state, size = 10) -> the status dot for a threadState or a worker state (gui-glyph.css).

function threadState(tab) {
  const phase = phaseOf(tab);
  if (waitingKindOf(tab.view) || ["asking", "input", "failed"].includes(phase)) return "needs";
  if (phase === "working" || phase === "delegating") return "working";
  const worker = tab.view.role === "worker";
  if (!worker && unseenDone(tab)) return "unread";
  return worker && phase === "done" ? "done" : "idle";
}

const answerChoice = (label, choice, about = null) => ({ label, about, choice });

function pendingAsk(tab) {
  const view = tab.view;
  const menu = !tab.answering && tab.menu;
  if (!menu) {
    if (waitingKindOf(view) !== "needs_input") return null;
    const { kind } = askOf(view);
    const detail = String(view.ask_detail ?? "").trim() || null;
    return { kind, text: kind === "question" ? detail : null, command: kind === "approval" ? detail : null, choices: null, typed: false };
  }
  const kind = menuKind(menu) === "approval" && menuIsQuestion(tab) ? "question" : menuKind(menu);
  const free = freeOption(menu);
  const options = () => menu.options.filter((o) => o !== free).map((o) => answerChoice(o.head, { option: o.n }, o.more || null));
  if (kind === "approval") {
    const lines = menu.question.split("\n");
    const middle = lines.slice(1, -1).filter((l) => !/^Tip:/.test(l)).join("\n");
    const { accept, decline, always } = approvalChoices(menu);
    const choices = [
      accept && answerChoice("Approve", "approve"),
      decline && answerChoice("Decline", "decline"),
      always && answerChoice(/session/i.test(always.label) ? "Always allow this session" : "Always allow", "always", always.label),
    ].filter(Boolean);
    return { kind, text: (menu.box && middle) || null, command: menu.box || middle || String(view.ask_detail ?? "").trim() || null, choices, typed: !!decline };
  }
  if (kind === "plan") return { kind, text: menu.question || null, command: null, choices: options(), typed: !!free };
  const oneTap = !multiSelect(menu) && !questionTabs(tab);
  const question = menu.question.split("\n").filter((l) => !TAB_ROW.test(l)).join("\n").trim();
  return { kind, text: String(view.ask_detail ?? "").trim() || question || null, command: null, choices: oneTap ? options() : null, typed: oneTap && !!free };
}

const STEP_KINDS = new Set(["tool", "thinking", "text"]);
const stepOf = (item, cwd) => (item ? stepLine(item, cwd).label || null : null);
const AGENT_STATES = { running: "working", done: "done", failed: "failed", cancelled: "stopped", stopped: "stopped" };

/** The model each of a thread's delegate_task calls started a worker thread on, by the worker's tab
 * id, from the calls' results. Kept on the thread until its items change. */
function delegatedModels(tab) {
  const t = threadOf(tab);
  if (t.delegatedModels?.items === t.items && t.delegatedModels.length === t.items.length) return t.delegatedModels.models;
  const calls = new Set();
  const models = new Map();
  for (const i of t.items) {
    if (i.kind === "tool" && /(^|__)delegate_task$/.test(i.name)) calls.add(i.id);
    else if (i.kind === "result" && calls.has(i.tool)) {
      const tab = /"tabId"\s*:\s*"([^"]+)"/.exec(i.text ?? "")?.[1];
      const model = /"model"\s*:\s*"([^"]+)"/.exec(i.text ?? "")?.[1];
      if (tab && model) models.set(tab, model);
    }
  }
  t.delegatedModels = { items: t.items, length: t.items.length, models };
  return models;
}

function threadWorker(w, started = new Map()) {
  const t = threadOf(w);
  const thread = threadState(w);
  const phase = phaseOf(w);
  const ask = pendingAsk(w);
  const state =
    phase === "failed" && !ask ? "failed"
      : thread === "needs" ? "needs"
      : thread === "working" ? "working"
      : phase === "done" ? "done"
      : w.view.running ? "working"
      : "stopped";
  const last = state === "done" ? t.items.findLast((i) => i.kind === "text") : t.items.findLast((i) => STEP_KINDS.has(i.kind));
  const since = state === "working" ? unixOf(lastPrompt(w)?.at) * 1000 : state === "needs" ? (w.view.updated ?? 0) * 1000 : 0;
  return {
    kind: "thread",
    key: w.key,
    tab: w,
    agent: null,
    title: threadTitle(w),
    state,
    step: stepOf(last, w.view.cwd),
    ask,
    model: modelOf(w)?.name ?? t.items.findLast((i) => i.kind === "model")?.name ?? started.get(w.view.id) ?? null,
    elapsedMs: w.view.stats?.active_ms ?? null,
    since: since || null,
  };
}

function agentWorker(tab, agent) {
  const state = AGENT_STATES[agent.state] ?? "stopped";
  const started = startedAt(tab, agent);
  const ended = Date.parse(agent.updated ?? "");
  const ran = state !== "working" && Number.isFinite(started) && Number.isFinite(ended) ? Math.max(0, ended - started) : null;
  return {
    kind: agent.id.startsWith("task-") ? "task" : "subagent",
    key: `${tab.key}:${agent.id}`,
    tab: null,
    agent,
    title: agent.name,
    state,
    step: stepOf((agent.steps ?? []).findLast((s) => s.kind !== "result"), tab.view.cwd),
    ask: null,
    model: agent.model ?? null,
    elapsedMs: ran,
    since: state === "working" && Number.isFinite(started) ? started : null,
  };
}

function workersOf(tab) {
  const own = `${tab.machine}:${tab.view.id}`;
  const threads = tabs.filter((t) => t !== tab && spawnerKey(t) === own).sort((a, b) => a.view.opened - b.view.opened);
  const started = threads.length ? delegatedModels(tab) : new Map();
  return [...threads.map((w) => threadWorker(w, started)), ...agentsOf(tab).filter((a) => !a.parent).map((a) => agentWorker(tab, a))];
}

function workersSummary(rows) {
  const count = (state) => rows.filter((r) => r.state === state).length;
  const starts = rows.filter((r) => r.state === "working" && r.since).map((r) => r.since);
  return { total: rows.length, done: count("done"), working: count("working"), needs: count("needs"), since: starts.length ? Math.min(...starts) : null };
}

function needsYouItem(tab) {
  const view = tab.view;
  const item = (kind, extra = {}) => ({ tab, kind, title: threadTitle(tab), project: projectName(tab), machine: machineName(tab.machine), since: (view.updated ?? 0) * 1000 || null, ask: null, limit: null, ...extra });
  const waiting = waitingKindOf(view);
  if (waiting === "needs_input" || (waiting === "failed" && waitingLookOf(tab, waiting) === "approval")) {
    const ask = pendingAsk(tab) ?? { kind: "approval", text: null, command: String(view.ask_detail ?? "").trim() || null, choices: null, typed: false };
    return item(ask.kind, { ask });
  }
  if (waiting === "failed" && view.resume_at) {
    const switchTo = switchTargets(tab).map((a) => ({ id: a.id, name: a.name }));
    return item("limit", { limit: { resumesAt: view.resume_at * 1000, account: accountName(tab.machine, view.account), held: !!view.held, switchTo } });
  }
  if (waiting === "failed") return item("failed");
  if (view.role !== "worker" && !isGlobalOrchestratorTab(tab) && unseenDone(tab)) return item("review", { since: lastActivity(tab) * 1000 || null });
  return null;
}

const needsYouItems = () => tabs.map(needsYouItem).filter((i) => i && (i.ask || !snoozedNow(i.tab))).sort((a, b) => (b.since ?? 0) - (a.since ?? 0));

/** The Captains' reports in the global orchestrator's thread since the user last saw it there. */
function unseenReports() {
  const global = tabs.find(isGlobalOrchestratorTab);
  if (!global || global === active) return 0;
  const seen = seenAt[threadKey(global)] ?? 0;
  return threadOf(global).items.filter((i) => i.kind === "user" && unixOf(i.at) > seen).reduce((n, i) => n + reportBlocks(i.text).length, 0);
}

/** The `which` window ("five_hour", "seven_day") of `account` on `machine` while it still holds. */
function usageWindowOf(machine, account, which = "five_hour") {
  const window = machines.get(machine)?.usage?.[account]?.[which];
  return window && window.resets_at > Date.now() / 1000 ? window : null;
}
function usageNow() {
  const pick = (machine, account) => {
    const window = usageWindowOf(machine, account);
    if (!window) return null;
    const week = usageWindowOf(machine, account, "seven_day");
    return {
      percent: window.used_percentage,
      resetsAt: window.resets_at * 1000,
      account: accountName(machine, account),
      provider: PROVIDERS[accountOf(machine, account)?.provider] ?? PROVIDERS.claude,
      week: week && { percent: week.used_percentage, resetsAt: week.resets_at * 1000 },
    };
  };
  const own = active && pick(active.machine, active.view.account ?? DEFAULT_ACCOUNT);
  if (own) return own;
  const all = [...machines].flatMap(([machine, m]) => Object.keys(m.usage ?? {}).map((account) => pick(machine, account)).filter(Boolean));
  return all.sort((a, b) => b.percent - a.percent)[0] ?? null;
}

function machineCounts() {
  const listed = accountMachines();
  const views = (listed.length ? listed : [...machines.values()].map((m) => m.view)).filter((v) => !v.upstash_box);
  return { machinesOnline: views.filter((v) => v.online).length, machinesTotal: views.length };
}

const REVIEWS_EVERY_MS = 5 * 60 * 1000;
let reviewCount = null;
let reviewsAt = 0;
/** Counts the open pull requests that ask the user's review in the projects of open threads, at
 * most once per REVIEWS_EVERY_MS. Each machine runs `gh` for it, so it never runs faster. */
async function fetchReviews() {
  if (Date.now() - reviewsAt < REVIEWS_EVERY_MS) return;
  const scope = [...openProjects()];
  if (!scope.length) return;
  reviewsAt = Date.now();
  const answers = await Promise.allSettled(scope.map(([machine, paths]) => callOn(machine, { cmd: "pull_requests", paths: [...paths], state: "open", involvement: "reviewing", limit: 100 })));
  const found = answers.filter((a) => a.status === "fulfilled");
  if (!found.length) return;
  const count = new Set(found.flatMap((a) => a.value.rows.map((r) => r.url))).size;
  if (count === reviewCount) return;
  reviewCount = count;
  modelChanged();
}
visibleInterval(fetchReviews, REVIEWS_EVERY_MS);

function navCounts() {
  return {
    needsYou: needsYouItems().filter((i) => i.kind !== "review").length,
    orchestrator: unseenReports(),
    reviews: reviewCount,
    usage: usageNow(),
    ...machineCounts(),
  };
}

const PICKS = { approve: "accept", decline: "decline", always: "always" };
async function answerAsk(tab, choice) {
  const menu = !tab.answering && terminalMenu(tab);
  if (!menu) return false;
  tab.menu = menu;
  const ask = pendingAsk(tab);
  if (typeof choice === "object" && "text" in choice) return !!ask?.typed && answerWith(tab, choice.text);
  if (!ask?.choices?.some((c) => JSON.stringify(c.choice) === JSON.stringify(choice))) return false;
  const option = typeof choice === "string" ? approvalChoices(menu)[PICKS[choice]] : menu.options[choice.option - 1];
  const sent = chooseOption(tab, option.n, option.label);
  if (!sent) return false;
  await sent;
  return true;
}

const workersKey = (tab) => JSON.stringify(workersOf(tab).map((r) => [r.key, r.state, r.step, r.model, r.ask && [r.ask.kind, r.ask.command, r.ask.choices?.length ?? null]]));

const modelListeners = new Set();
function onModelChange(fn) {
  modelListeners.add(fn);
  return () => modelListeners.delete(fn);
}
let drawnWorkers = "";
function modelChanged() {
  const key = active ? workersKey(active) : "";
  if (key !== drawnWorkers) {
    drawnWorkers = key;
    if (active && guiOn() && !terminalOnly(active) && !threadOf(active).unsupported) renderThread();
  }
  if (!reviewsAt && tabs.length) fetchReviews();
  for (const fn of modelListeners) fn();
}

const GLYPH_LOOKS = { working: "working", needs: "needs", unread: "unread", done: "done", idle: "idle", failed: "failed", stopped: "idle" };
const glyphHtml = (state, size = 10) => `<span class="status-glyph" data-glyph="${GLYPH_LOOKS[state] ?? "idle"}" style="--glyph-size: ${size}px" aria-hidden="true"></span>`;
