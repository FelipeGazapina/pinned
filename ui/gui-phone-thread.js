// A thread on a phone (768px and below), as Phone-Thread, Phone-Workers, Phone-Approval and
// Phone-Actions draw it: the header's back button and second line, the Workers pill under it, the
// combined model chip in the composer, and three bottom sheets: the Workers sheet, the sheet for the
// open thread's own ask, and the thread's actions. Every answer and action goes through the
// functions the desktop's controls call.
//
// phoneChipLabel(model, effort, mode) -> "Opus 5.5 · High · Auto"
//   The composer chip's words: the model, the reasoning effort while claude runs, and the access
//   mode or Plan. A part the thread has no value for is left out.
//
// threadActionsOf(tab) -> { primary, tools, actions }
//   The actions sheet's three groups, each a list of { id, label, hint, tone, disabled, pressed,
//   run } or, for a row that opens a list of its own, { ..., items: () => [rowMenuItems entries] }.
//
// workersSheetRows(rows, first) -> { cards, rest }
//   The Workers sheet's waiting workers, `first` (a row key) leading, and everything else in order.
//
// openWorkersSheet(tab, key), openThreadActions(tab), syncPhoneAsk()
//   Open the sheets; syncPhoneAsk runs after each composer render and opens, repaints or closes the
//   ask sheet for the open thread.

const onPhone = () => narrow(768);
const PHONE_SVG = {
  back: `<path d="m15 6-6 6 6 6"></path>`,
  down: `<path d="m6 9 6 6 6-6"></path>`,
  right: `<path d="m9 6 6 6-6 6"></path>`,
  close: `<path d="M6 6l12 12"></path><path d="M18 6 6 18"></path>`,
  shield: `<path d="M12 3 5 6v6c0 4.5 3 7.7 7 9 4-1.3 7-4.5 7-9V6z"></path>`,
  mic: `<rect x="9" y="3" width="6" height="11" rx="3"></rect><path d="M5 11a7 7 0 0 0 14 0"></path><path d="M12 18v3"></path>`,
};
const phoneSvg = (name, size, cls = "", width = 2) =>
  `<svg class="${cls}" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${width}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${PHONE_SVG[name]}</svg>`;

/** The composer chip's words. */
function phoneChipLabel(model, effort, mode) {
  const access = mode === "plan" ? "Plan" : MODES[mode]?.label;
  return [model, effort, access].filter(Boolean).join(" · ");
}

const ASK_SAYS = { approval: "Needs your approval", question: "Has a question", plan: "Has a plan to approve" };

/** The Workers sheet's waiting workers, the one asked for first, then the rest in their order. */
function workersSheetRows(rows, first = null) {
  const cards = rows.filter((r) => r.state === "needs" && r.tab).sort((a, b) => (b.key === first) - (a.key === first));
  return { cards, rest: rows.filter((r) => !cards.includes(r)) };
}

/** The line and lines changed since the thread's base, from the diff the panel last read, else its
 * latest turn's. */
function diffCounts(t) {
  const patch = t.gitDiff?.data?.patch;
  if (patch) {
    const lines = patch.split("\n");
    return { added: lines.filter((l) => /^\+(?!\+\+ )/.test(l)).length, removed: lines.filter((l) => /^-(?!-- )/.test(l)).length };
  }
  const files = t.history?.at(-1)?.files;
  return files?.length ? { added: sum(files, "added"), removed: sum(files, "removed") } : null;
}

const PR_ROLLUP_WORDS = { passing: "Passing", failing: "Failing", pending: "Pending" };
const PHONE_LISTED = new Set(["pin", "pin-off", "circle-check", "clock", "user-round", "archive", "trash"]);

function threadActionsOf(tab) {
  const t = threadOf(tab);
  const ws = t.workspace;
  const worker = tab.view.role === "worker";
  const pr = prOf(tab);
  const primary = [];
  if (ws && worker) {
    const diff = diffCounts(t);
    primary.push({ id: "diff", label: diff ? `Open diff +${diff.added} -${diff.removed}` : "Open diff", tone: "primary", run: () => document.getElementById("open-diff").click() });
  } else if (ws) {
    const quick = quickAction(t.git, ws, !!pr);
    primary.push({ id: "git", label: quick.label, tone: "primary", disabled: !!quick.disabled, hint: quick.hint ?? "", run: () => gitAction.querySelector(".main").click() });
  }
  const known = !worker && ws && tab.view.running && projectScripts(tab);
  const script = known && !known.loading && primaryScript(known.scripts);
  if (script) primary.push({ id: "script", label: `Run ${script.name}`, run: () => runScript(tab, script) });
  if (pr) primary.push({ id: "pr", label: `PR #${pr.number}`, run: () => openPr(tab, pr.url) });

  const diff = diffCounts(t);
  const port = (t.ports ?? []).find((p) => p.own)?.port;
  const shells = scriptShells.filter((s) => s.tab === tab).length;
  const rollup = pr && prRollupState(prCache.get(`${tab.key}:${pr.url}`)?.data ?? {});
  const saved = hasProject(tab) ? memoryOf(tab).hits?.length : null;
  const device = panelOf(tab).surfaces.find((s) => s.kind === "device");
  const tools = [
    { id: "diff", label: "Diff", hint: diff ? `+${diff.added} -${diff.removed}` : "", disabled: ws?.repo === false, run: () => openSurface("diff") },
    { id: "browser", label: "Browser", hint: port ? `:${port}` : "", run: () => openSurface("browser") },
    { id: "files", label: "Files", hint: "Tree", run: () => openFilesSurface() },
    { id: "terminal", label: "Terminal", hint: ui.drawer ? "Shown" : `${shells + 1} open`, pressed: !!ui.drawer, run: () => toggleDrawer() },
    { id: "pr", label: pr ? `PR #${pr.number}` : "Pull request", hint: rollup ? PR_ROLLUP_WORDS[rollup] : "", disabled: !pr, run: () => openSurface("pr") },
    { id: "memory", label: "Memory", hint: saved ? `${saved} saved` : "", disabled: !hasProject(tab), run: () => addSurface(tab, { id: "memory", kind: "memory" }) },
    { id: "device", label: "Device", hint: device ? surfaceTitle(tab, device) : "Simulator", run: () => addSurface(tab, device ?? { id: `device:${Date.now().toString(36)}`, kind: "device" }) },
    { id: "details", label: "Details", hint: "Branch, cost", run: () => toggleDetails(true) },
  ];

  const menu = rowMenuItems(tab);
  const by = (...icons) => menu.find((i) => icons.includes(i.icon));
  const actions = [];
  const workers = workersOf(tab);
  if (workers.length) {
    const s = workersSummary(workers);
    actions.push({ id: "workers", label: "Workers", hint: s.needs ? `${s.needs} needs you` : `${s.done} of ${s.total} done`, run: () => openWorkersSheet(tab) });
  }
  if (worker && parentOf(tab)) actions.push({ id: "parent", label: `Open ${threadTitle(parentOf(tab))}`, hint: "Captain", run: () => activate(parentOf(tab)) });
  const panel = panelOf(tab);
  if (panel.surfaces.length) actions.push({ id: "panel", label: panel.open ? "Hide side panel" : "Show side panel", hint: `${panel.surfaces.length} open`, run: () => togglePanel() });
  const pin = by("pin", "pin-off");
  actions.push({ id: "pin", label: pin.label, run: pin.run });
  const settle = by("circle-check");
  actions.push({ id: "settle", label: settled(tab) ? "Un-settle" : "Settle", hint: settled(tab) ? "" : "Swipe left on the row", run: settle.run });
  const snooze = by("clock");
  actions.push(snooze.children
    ? { id: "snooze", label: "Snooze", hint: "Swipe right on the row", disabled: !!snooze.disabled, items: snooze.children }
    : { id: "snooze", label: snooze.label, run: snooze.run });
  const more = menu.filter((i) => !PHONE_LISTED.has(i.icon)).map((i) => (i.icon === "pencil" ? { ...i, run: () => startRename(tab, "header") } : i));
  actions.push({ id: "more", label: "Rename, mark unread, copy", hint: "More", items: () => more });
  const move = by("user-round");
  if (move) actions.push({ id: "account", label: "Move to account", hint: accountName(tab.machine, tab.view.account ?? DEFAULT_ACCOUNT), items: move.children });
  actions.push({ id: "archive", label: "Archive or delete", tone: "danger", items: () => [by("archive"), by("trash")].filter(Boolean) });
  return { primary, tools, actions };
}

/** An empty clock that sheetTickers fills, so a row's markup stays the same from one second to the
 * next and a repaint never replaces the row under a tap. */
const sheetTicker = (since) => `<span class="ticker" data-since="${since}"></span>`;
/** The workers being answered from the Workers sheet, whose buttons wait meanwhile. */
const sheetAnswering = new Set();

/** A waiting worker's card: what it asks, and Decline and Approve where one tap answers it, else Open. */
function workerCardHtml(row) {
  const ask = row.ask;
  const offers = (choice) => !!ask?.choices?.some((c) => c.choice === choice);
  const busy = sheetAnswering.has(row.key) ? " disabled" : "";
  const id = esc(row.key);
  const says = `${ASK_SAYS[ask?.kind] ?? ASK_SAYS.question}${row.since ? ` · waiting ${sheetTicker(row.since)}` : ""}`;
  const model = modelLabel(row.model);
  const what = ask?.kind === "approval" && ask.command ? `<pre>${esc(ask.command)}</pre>` : ask?.text ? `<p class="ask-text">${esc(ask.text)}</p>` : "";
  const answers = offers("approve") || offers("decline")
    ? `${offers("decline") ? `<button type="button" class="outline" data-ws-answer="decline" data-ws-row="${id}"${busy}>Decline</button>` : ""}${offers("approve") ? `<button type="button" class="primary" data-ws-answer="approve" data-ws-row="${id}"${busy}>Approve</button>` : ""}`
    : `<button type="button" class="outline" data-ws-open="${id}">Open</button>`;
  return `<div class="ws-card" data-ws-card="${id}"><div class="ws-card-head">${glyphHtml("needs", 12)}<span class="ws-text"><span class="ws-title">${esc(row.title)}</span><span class="ws-says">${says}</span></span>${model ? `<span class="ws-model">${esc(model)}</span>` : ""}</div>
    ${what}<div class="ws-answers">${answers}</div></div>`;
}
/** Any other worker: its state, its latest step and its time, opening its thread, or for an agent
 * its row in the Workers block, where its actions are. An agent another agent started sits
 * indented under it. */
function workerSheetRowHtml(row) {
  const time = row.state === "done" ? "Done" : row.state === "working" && row.since ? sheetTicker(row.since) : row.elapsedMs != null ? duration(row.elapsedMs) : "";
  const opens = row.tab ? `data-ws-open="${esc(row.key)}"` : `data-ws-agent="${esc(row.key)}"`;
  const depth = row.depth ? ` data-depth="${row.depth}" style="--depth: ${row.depth}"` : "";
  return `<button type="button" class="ws-row" data-state="${row.state}" ${opens}${depth}>${glyphHtml(row.state, 12)}<span class="ws-text"><span class="ws-title">${esc(row.title || "Agent")}</span>${row.step ? `<span class="ws-step">${esc(row.step)}</span>` : ""}</span>${time ? `<span class="ws-time">${time}</span>` : ""}${phoneSvg("right", 16, "chev")}</button>`;
}
const stoppable = (rows) => rows.filter((r) => r.state === "working" && (r.tab || r.agent?.id.startsWith("task-")));
const stopLabel = (n) => (n === 1 ? "Stop the running one" : `Stop the ${n} running`);

// -- The header ------------------------------------------------------------------------------

const chatBack = Object.assign(document.createElement("button"), { type: "button", id: "chat-back", className: "icon", innerHTML: phoneSvg("back", 22) });
chatBack.setAttribute("aria-label", "Back to threads");
document.getElementById("chat-sidebar-toggle").after(chatBack);
const headSub = Object.assign(document.createElement("span"), { id: "head-sub" });
crumbEl.after(headSub);

/** The header's second line: where the thread is, its role, and its time and cost. A worker names
 * the thread that started it; a Captain shows its time, its cost being its workers' too. */
function phoneSubLine(tab) {
  if (!tab) return "";
  if (isGlobalOrchestratorTab(tab)) {
    const host = globalHostMachine();
    return host ? `Runs on ${machineName(host)}` : "";
  }
  const parent = tab.view.role === "worker" && parentOf(tab);
  if (parent) return `Worker of ${threadTitle(parent)}`;
  const stats = headStatsLabel(tab).split(" · ").filter(Boolean);
  if (isOrchestrator(tab)) return [projectName(tab), "Captain", stats[0]].filter(Boolean).join(" · ");
  return [projectName(tab), tab.view.role === "worker" ? "Worker" : "", ...stats].filter(Boolean).join(" · ");
}
function renderHeadSub() {
  const tab = active;
  const text = guiOn() && onPhone() ? phoneSubLine(tab) : "";
  if (headSub.textContent !== text) headSub.textContent = text;
  const parent = tab?.view.role === "worker" && parentOf(tab);
  chatBack.setAttribute("aria-label", subagentShown() ? `Back to ${threadTitle(tab)}` : parent ? `Back to ${threadTitle(parent)}` : "Back to threads");
}
new MutationObserver(renderHeadSub).observe(crumbEl, { childList: true });
visibleInterval(renderHeadSub, 1000);

chatBack.onclick = () => {
  // The crumb that leaves a subagent's conversation is hidden on a phone, so Back does.
  if (subagentShown()) return closeSubagent();
  const parent = active?.view.role === "worker" && parentOf(active);
  if (parent) return activate(parent);
  if (typeof phoneGo === "function") return phoneGo("threads");
  toggleSidebar(true);
};

// -- The Workers pill ------------------------------------------------------------------------

const workersPill = Object.assign(document.createElement("button"), { type: "button", id: "workers-pill", hidden: true });
document.getElementById("chat-head").after(workersPill);
function renderWorkersPill() {
  const tab = active;
  const rows = tab && guiOn() && onPhone() && !terminalOnly(tab) ? workersNow(tab) : [];
  if (workersPill.hidden !== !rows.length) {
    // The pill takes its height from the thread, which keeps to its end if it was there.
    const atEnd = threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight <= 40;
    workersPill.hidden = !rows.length;
    if (atEnd) threadView.scrollTop = threadView.scrollHeight;
  }
  if (!rows.length) return;
  const s = workersSummary(rows);
  const glyph = s.working ? "working" : s.needs ? "needs" : "done";
  setHtml(workersPill, `${glyphHtml(glyph, 12)}<span class="count">${s.total} ${s.total === 1 ? "worker" : "workers"}</span>${s.working ? `<span class="working">${s.working} working</span>` : ""}${s.needs ? `<span class="needs">${s.needs} needs you</span>` : ""}<span class="spacer"></span>${phoneSvg("down", 16, "chev")}`);
  workersPill.setAttribute("aria-label", `Workers: ${s.total}, ${s.working} working, ${s.needs} need you`);
}
onModelChange(renderWorkersPill);
workersPill.onclick = () => active && openWorkersSheet(active);

// A worker thread's row in the timeline opens that thread on a phone, as the board's rows do; a
// task or subagent row, which has no thread, still opens in place.
threadView.addEventListener("click", (e) => {
  if (!onPhone()) return;
  const row = e.target.closest(".worker-row");
  const worker = row && e.target.closest(".worker-toggle") && tabByKey(row.dataset.worker);
  if (!worker) return;
  e.stopPropagation();
  activate(worker);
}, true);

// -- Sheets ----------------------------------------------------------------------------------

const sheetTickers = () => {
  for (const ticker of document.querySelectorAll(".sheet-panel .ticker[data-since]")) ticker.textContent = durationLabel(Math.max(0, Date.now() - Number(ticker.dataset.since)) / 1000);
};
visibleInterval(sheetTickers, 1000);

// -- The Workers sheet -----------------------------------------------------------------------

let workersSheet = null;

function paintWorkersSheet() {
  if (!workersSheet) return;
  const tab = tabByKey(workersSheet.tab);
  if (!tab || tab !== active || !onPhone()) return workersSheet.sheet.close();
  const rows = workersNow(tab);
  if (!rows.length) return workersSheet.sheet.close();
  const s = workersSummary(rows);
  const { cards, rest } = workersSheetRows(rows, workersSheet.first);
  workersSheet.drawn = new Map(rows.map((r) => [r.key, r]));
  const el = workersSheet.sheet.el;
  setHtml(el.querySelector(".ws-count"), `${s.done} of ${s.total} done`);
  setHtml(el.querySelector(".ws-list"), [...cards.map(workerCardHtml), ...rest.map(workerSheetRowHtml)].join(""));
  sheetTickers();
  const stop = el.querySelector("[data-ws=stop]");
  const running = stoppable(rows).length;
  stop.hidden = !running;
  if (running) stop.textContent = stopLabel(running);
  el.querySelector("[data-ws=message]").disabled = !rows.some((r) => r.tab?.view.running);
}

function openWorkersSheet(tab, first = null) {
  const content = `<div class="ws-head"><h2>Workers</h2><span class="ws-count"></span><button type="button" class="ws-close" aria-label="Close">${phoneSvg("close", 20)}</button></div>
    <div class="ws-list"></div>
    <div class="ws-foot"><button type="button" class="outline" data-ws="message">Message all</button><button type="button" class="outline danger" data-ws="stop"></button></div>
    <form class="ws-say" hidden><input type="text" enterkeyhint="send" aria-label="Message every worker" placeholder="Message every worker"><button type="submit" class="primary">Send</button></form>`;
  let off = null;
  const sheet = openSheet({ label: "Workers", content, className: "workers-sheet", onClose: () => {
    off?.();
    if (workersSheet?.sheet === sheet) workersSheet = null;
  } });
  workersSheet = { tab: tab.key, first, sheet, drawn: new Map() };
  off = onModelChange(paintWorkersSheet);
  paintWorkersSheet();
  if (first) sheet.el.querySelector(`[data-ws-card="${CSS.escape(first)}"]`)?.scrollIntoView({ block: "nearest" });
  sheet.el.addEventListener("click", (e) => onWorkersSheetClick(e, tab));
  sheet.el.querySelector(".ws-say").addEventListener("submit", (e) => {
    e.preventDefault();
    messageAllWorkers(tab, e.currentTarget.querySelector("input"));
  });
}

/** What "Message all" did: how many workers got the message and how many it skipped, each waiting on
 * the user (a message would answer its ask). */
function messageAllSaid(sent, skipped, failed) {
  const workers = (n) => `${n} ${n === 1 ? "worker" : "workers"}`;
  const waiting = skipped ? `Skipped ${workers(skipped)} waiting on you.` : "";
  if (!sent && !failed) return `Nothing sent. ${waiting}`;
  return [sent ? `Sent to ${workers(sent)}.` : "", failed ? `Couldn't reach ${workers(failed)}; your message is still here.` : "", waiting].filter(Boolean).join(" ");
}

/** Sends the sheet's message to every running worker that is not waiting on the user. The text stays
 * in its field unless every send went out. */
async function messageAllWorkers(tab, input) {
  const text = input.value.trim();
  const running = workersOf(tab).filter((r) => r.tab?.view.running);
  if (!text || !running.length) return;
  const targets = running.filter((r) => !waitsOnUser(r.tab));
  const sent = await Promise.all(targets.map((r) => sayToWorker(r.key, text, true)));
  const ok = sent.filter(Boolean).length;
  if (ok && ok === targets.length) {
    input.value = "";
    input.closest("form").hidden = true;
  }
  tell(esc(messageAllSaid(ok, running.length - targets.length, targets.length - ok)), 5000);
}

function onWorkersSheetClick(e, tab) {
  if (e.target.closest(".ws-close")) return workersSheet?.sheet.close();
  const open = e.target.closest("[data-ws-open]");
  if (open) {
    const worker = tabByKey(open.dataset.wsOpen);
    workersSheet?.sheet.close();
    return worker && activate(worker);
  }
  const agent = e.target.closest("[data-ws-agent]");
  if (agent) {
    workersSheet?.sheet.close();
    threadOf(tab).open.add(`worker:${agent.dataset.wsAgent}`);
    rerender();
    return threadView.querySelector(`.worker-row[data-worker="${CSS.escape(agent.dataset.wsAgent)}"]`)?.scrollIntoView({ block: "center" });
  }
  const answer = e.target.closest("[data-ws-answer]");
  if (answer) return answerWorker(workersSheet?.drawn.get(answer.dataset.wsRow), answer.dataset.wsAnswer, sheetAnswering, paintWorkersSheet);
  const act = e.target.closest("[data-ws]")?.dataset.ws;
  if (act === "message") {
    const form = workersSheet?.sheet.el.querySelector(".ws-say");
    if (!form) return;
    form.hidden = !form.hidden;
    if (!form.hidden) form.querySelector("input").focus();
  } else if (act === "stop") {
    for (const row of stoppable(workersOf(tab))) {
      if (row.tab) stop(row.tab);
      else cancelDelegated(row.agent.id);
    }
  }
}

// -- The ask sheet ---------------------------------------------------------------------------

const askCard = Object.assign(document.createElement("div"), { id: "ask-card", hidden: true });
document.getElementById("approval").after(askCard);
let askSheet = null;
let askDismissed = "";
const askKeyOf = (tab) => (tab?.menu && !tab.answering ? JSON.stringify([tab.key, tab.menu.question, tab.menu.options.map((o) => o.label)]) : "");

/** Where the open thread stands among the threads that wait on an answer: its place and the count. */
function askPlace(tab) {
  const waiting = needsYouItems().filter((i) => i.ask).map((i) => i.tab);
  const at = waiting.indexOf(tab);
  return at < 0 ? { at: 0, count: waiting.length + 1, next: waiting[0] ?? null } : { at, count: waiting.length, next: waiting[(at + 1) % waiting.length] };
}

const ASK_TITLES = { command: "Allow this command?", edit: "Allow this edit?", other: "Allow this?" };
function askSheetHtml(tab) {
  const menu = tab.menu;
  const kind = menuKind(menu) === "approval" && menuIsQuestion(tab) ? "question" : menuKind(menu);
  const place = askPlace(tab);
  const count = place.count > 1
    ? `<button type="button" class="ask-count" data-ask="next" aria-label="Show the next thread that waits">${place.at + 1} of ${place.count}</button>`
    : `<span class="ask-count">1 of 1</span>`;
  const row = (attrs, label, end = "right", cls = "") => `<button type="button" class="ask-row${cls}" ${attrs}><span class="label">${esc(label)}</span>${end ? phoneSvg(end, end === "mic" ? 18 : 16, "end", end === "mic" ? 1.75 : 2) : ""}</button>`;
  const terminal = row(`data-ask="terminal"`, "Show in terminal", "right", " quiet");
  if (kind === "approval") {
    const lines = menu.question.split("\n");
    const what = lines[0] ?? "";
    const title = /command/i.test(what) ? ASK_TITLES.command : /file|edit/i.test(what) ? ASK_TITLES.edit : ASK_TITLES.other;
    const middle = lines.slice(1, -1).filter((l) => !/^Tip:/.test(l));
    const detail = menu.box || middle.join("\n");
    const about = [menu.box ? middle.join("\n") : "", /^Do you want to proceed\?$/.test(lines.at(-1)) ? "" : lines.at(-1)].filter(Boolean).join("\n");
    const { accept, decline, always, rest } = approvalChoices(menu);
    const answers = `${decline ? `<button type="button" class="outline" data-approval="decline">Decline</button>` : ""}${accept ? `<button type="button" class="primary" data-approval="accept">Approve</button>` : ""}`;
    const rows = [
      always ? row(`data-approval="always"`, always.head) : "",
      ...rest.map((o) => row(`data-option="${o.n}"`, o.head)),
      decline ? row(`data-ask="type"`, "Decline and say what to do instead", "mic") : row(`data-ask="cancel"`, "Cancel"),
      decline ? row(`data-ask="cancel"`, "Cancel", "", " quiet") : "",
      terminal,
    ];
    return `<div class="ask-head">${phoneSvg("shield", 20, "lead", 1.75)}<h2>${title}</h2>${count}</div>
      ${detail ? `<pre tabindex="0">${esc(detail)}</pre>` : ""}${about ? `<p class="ask-about">${esc(about)}</p>` : ""}
      ${answers ? `<div class="ask-answers">${answers}</div>` : ""}<div class="ask-rows">${rows.join("")}</div>`;
  }
  const t = threadOf(tab);
  const options = (o) => {
    const on = t.picking === o.n || ticked(o);
    const label = o.head.replace(/^\[[ ✔]\]\s*/, "").replace(/\s✔$/, "");
    return `<button type="button" class="ask-option${on ? " picked" : ""}" data-option="${o.n}"${o.chosen ? " data-chosen" : ""}><span class="text"><span class="label">${esc(label)}</span>${o.more ? `<span class="about">${esc(o.more)}</span>` : ""}</span>${on ? icon("check", "tick") : ""}</button>`;
  };
  const free = freeOption(menu);
  if (kind === "plan") {
    return `<div class="ask-head">${icon("list-checks", "lead")}<h2>Plan ready</h2>${count}</div>
      ${menu.question ? `<p class="ask-about">${esc(menu.question)}</p>` : ""}
      <div class="ask-options">${menu.options.filter((o) => o !== free).map(options).join("")}</div>
      <div class="ask-rows">${free ? row(`data-ask="type"`, "Refine it with a message", "mic") : ""}${row(`data-ask="new-thread"`, "Implement in a new thread")}${terminal}</div>`;
  }
  const tabs = questionTabs(tab);
  const review = tabs && tabs.at >= tabs.headers.length;
  const lines = menu.question.split("\n").filter((l) => !TAB_ROW.test(l));
  const own = !tabs && lines[0]?.match(/^[☐☒✔]\s+(.+)$/);
  const header = review ? "Review your answers" : tabs ? tabs.headers[tabs.at] : own ? own[1] : "Question";
  const question = (own ? lines.slice(1) : review ? lines.filter((l) => !/^Review your answers$/.test(l)) : lines).join("\n").trim();
  const multi = multiSelect(menu);
  const steps = tabs || multi;
  const last = !tabs || tabs.at >= tabs.headers.length - 1;
  const step = steps
    ? `<div class="ask-answers">${tabs?.at > 0 ? `<button type="button" class="outline" data-q="previous">Previous</button>` : ""}<button type="button" class="primary" data-q="${last ? "submit" : "next"}">${last ? "Submit" : "Next question"}</button></div>`
    : "";
  return `<div class="ask-head">${icon("message-circle-question", "lead")}<h2>${esc(header)}</h2>${tabs && !review ? `<span class="ask-count">${tabs.at + 1} of ${tabs.headers.length}</span>` : count}</div>
    ${question ? `<p class="ask-question">${esc(question)}</p>` : ""}${multi ? `<p class="ask-about">Select one or more options.</p>` : ""}
    <div class="ask-options">${menu.options.filter((o) => o !== free).map(options).join("")}</div>${step}
    <div class="ask-rows">${free ? row(`data-ask="type"`, "Type your own answer", "mic") : ""}${row(`data-ask="cancel"`, "Dismiss without answering")}${terminal}</div>`;
}

function askCardHtml(tab) {
  const menu = tab.menu;
  const kind = menuKind(menu) === "approval" && menuIsQuestion(tab) ? "question" : menuKind(menu);
  const ask = pendingAsk(tab);
  const what = kind === "approval" ? (ask?.command ? `Allow ${ask.command.split("\n")[0].trim()}?` : "Allow this?") : kind === "plan" ? "Plan ready" : ask?.text?.split("\n")[0] || "Question";
  return `<span class="dot" aria-hidden="true"></span><span class="text">${esc(what)}</span><button type="button" class="primary" data-ask="review">Review</button>`;
}

function closeAskSheet() {
  const shown = askSheet;
  askSheet = null;
  shown?.sheet.close();
}

/** The sheets belong to the open thread's screen, the Orchestrator's included. An ask's sheet also
 * waits while the palette is up, which it would cover. */
const onThreadScreen = () => typeof phoneScreen === "function" && ["thread", "orchestrator"].includes(phoneScreen());
const askSheetMayShow = () => onThreadScreen() && !(typeof paletteOpen === "function" && paletteOpen());

/** Off a thread's screen its sheets close; back on it, its ask's sheet opens again. */
function syncPhoneSheets() {
  syncPhoneAsk();
  if (!onThreadScreen()) openedSheet?.close();
}
let sheetsScreen;
new MutationObserver(() => {
  if (document.body.dataset.phoneScreen === sheetsScreen) return;
  sheetsScreen = document.body.dataset.phoneScreen;
  syncPhoneSheets();
}).observe(document.body, { attributeFilter: ["data-phone-screen"] });

function syncPhoneAsk() {
  const tab = active;
  const key = guiOn() && onPhone() && askSheetMayShow() && tab && !terminalOnly(tab) ? askKeyOf(tab) : "";
  if (!key) {
    askCard.hidden = true;
    if (askSheet) closeAskSheet();
    if (!tab?.menu) askDismissed = "";
    return;
  }
  askCard.dataset.kind = menuKind(tab.menu);
  setHtml(askCard, askCardHtml(tab));
  askCard.hidden = key !== askDismissed;
  if (key === askDismissed) return askSheet && closeAskSheet();
  const html = askSheetHtml(tab);
  if (askSheet?.key !== key && askSheet) closeAskSheet();
  if (!askSheet) {
    const tone = menuKind(tab.menu) === "approval" ? "approval" : null;
    const sheet = openSheet({ label: menuKind(tab.menu) === "approval" ? "Command approval" : "Question", content: `<div class="ask-body"></div>`, tone, className: "ask-sheet", onClose: () => {
      if (askSheet?.sheet !== sheet) return;
      askSheet = null;
      askDismissed = key;
      syncPhoneAsk();
    } });
    askSheet = { key, sheet };
    sheet.el.addEventListener("click", onAskClick);
  }
  setHtml(askSheet.sheet.el.querySelector(".ask-body"), html);
}

/** Answers from the ask sheet as the approval card does. */
function onAskClick(e) {
  const tab = active;
  const menu = tab?.menu;
  if (!menu) return;
  const answer = e.target.closest("[data-approval]");
  if ((answer || e.target.closest("[data-option]")) && menuKind(menu) !== "question" && !drawnMenuShown(tab)) return;
  if (answer) {
    const choice = approvalChoices(menu)[answer.dataset.approval];
    if (answer.dataset.approval === "decline" && prompt.value.trim()) return send();
    return choice && chooseOption(tab, choice.n, choice.label);
  }
  const option = e.target.closest("[data-option]");
  if (option) {
    if (menuKind(menu) === "question") return pickOption(tab, Number(option.dataset.option));
    return chooseOption(tab, Number(option.dataset.option), menu.options[option.dataset.option - 1]?.label);
  }
  const q = e.target.closest("[data-q]");
  if (q) return questionStep(tab, q.dataset.q);
  const act = e.target.closest("[data-ask]")?.dataset.ask;
  if (act === "cancel") return cancelMenu(tab);
  if (act === "new-thread") return implementInNewThread(tab);
  if (act === "next") return askPlace(tab).next && activate(askPlace(tab).next);
  const key = askSheet?.key;
  if (act === "type" || act === "terminal") {
    askDismissed = key;
    closeAskSheet();
    syncPhoneAsk();
    if (act === "terminal") return toggleDrawer(true);
    prompt.focus();
  }
}

askCard.addEventListener("click", (e) => {
  if (!e.target.closest("[data-ask=review], .text, .dot")) return;
  askDismissed = "";
  syncPhoneAsk();
});
/** Past phone width the sheets, which are phone screens, close rather than stay up as modals over
 * the desktop. */
function phoneWidthChanged() {
  if (!onPhone()) openedSheet?.close();
  if (!guiOn()) return;
  renderHeadSub();
  renderWorkersPill();
  if (active) renderComposer(active);
}
matchMedia("(max-width: 768px)").addEventListener("change", phoneWidthChanged);

// -- The thread actions sheet ----------------------------------------------------------------

function actionButtonHtml(a, cls) {
  const attrs = `${a.disabled ? " disabled" : ""}${a.pressed ? ` aria-pressed="true"` : ""}${a.hint && cls === "act-primary" ? ` title="${esc(a.hint)}"` : ""}`;
  if (cls === "act-tile") return `<button type="button" class="act-tile" data-act="${a.id}"${attrs}><span class="label">${esc(a.label)}</span><span class="hint">${esc(a.hint || " ")}</span></button>`;
  if (cls === "act-primary") return `<button type="button" class="act-primary${a.tone === "primary" ? " primary" : " outline"}" data-act="${a.id}"${attrs}><span>${esc(a.label)}</span></button>`;
  return `<button type="button" class="act-row${a.tone === "danger" ? " danger" : ""}" data-act="${a.id}"${attrs}${a.items ? ` aria-haspopup="menu"` : ""}><span class="label">${esc(a.label)}</span>${a.hint ? `<span class="hint">${esc(a.hint)}</span>` : ""}</button>`;
}

/** A list of rowMenuItems entries, as the sheet shows one under its title with a way back. */
function itemsHtml(title, items) {
  const rows = items.map((item, i) => `<button type="button" class="act-row${item.danger ? " danger" : ""}" data-item="${i}"${item.disabled ? " disabled" : ""}${item.checked ? ` aria-pressed="true"` : ""}><span class="label">${esc(item.label)}</span>${item.hint ? `<span class="hint">${esc(item.hint)}</span>` : ""}${item.checked ? icon("check", "end") : item.children ? phoneSvg("right", 16, "end") : ""}</button>`);
  return `<button type="button" class="act-back" data-back>${phoneSvg("back", 18)}<span>${esc(title)}</span></button><div class="act-list">${rows.join("")}</div>`;
}

function openThreadActions(tab) {
  let groups = threadActionsOf(tab);
  const content = `${groups.primary.length ? `<div class="act-primaries">${groups.primary.map((a) => actionButtonHtml(a, "act-primary")).join("")}</div>` : ""}
    <div class="act-tools"><span class="act-heading">Open beside this thread</span><div class="act-grid">${groups.tools.map((a) => actionButtonHtml(a, "act-tile")).join("")}</div></div>
    <div class="act-list">${groups.actions.map((a) => actionButtonHtml(a, "act-row")).join("")}</div>`;
  const sheet = openSheet({ label: "Thread actions", content: `<div class="act-main">${content}</div><div class="act-sub" hidden></div>`, className: "actions-sheet" });
  const main = sheet.el.querySelector(".act-main");
  const sub = sheet.el.querySelector(".act-sub");
  const stack = [];
  const showItems = (title, items) => {
    stack.push({ title, items });
    sub.innerHTML = itemsHtml(title, items);
    main.hidden = true;
    sub.hidden = false;
    sub.querySelector("button:not([disabled])")?.focus({ preventScroll: true });
  };
  sheet.el.addEventListener("click", (e) => {
    if (e.target.closest("[data-back]")) {
      stack.pop();
      const back = stack.pop();
      if (back) return showItems(back.title, back.items);
      sub.hidden = true;
      main.hidden = false;
      return;
    }
    const pick = e.target.closest("[data-act], [data-item]");
    if (!pick || pick.disabled) return;
    const group = { "act-primary": groups.primary, "act-tile": groups.tools, "act-row": groups.actions }[pick.classList[0]];
    const entry = pick.dataset.act ? group?.find((a) => a.id === pick.dataset.act) : stack.at(-1).items[Number(pick.dataset.item)];
    const children = entry?.items ?? entry?.children;
    if (children) return showItems(entry.label, children());
    sheet.close();
    entry?.run?.();
  });
  readToolHints(tab).then(() => {
    if (!sheet.el.isConnected) return;
    groups = threadActionsOf(tab);
    setHtml(sheet.el.querySelector(".act-grid"), groups.tools.map((a) => actionButtonHtml(a, "act-tile")).join(""));
  });
  return sheet;
}

/** Reads what the tool tiles' hints say and the thread has not read yet: the diff's line counts, the
 * ports it listens on, the pull request's checks and the project's memories. */
function readToolHints(tab) {
  const t = threadOf(tab);
  const d = diffState(tab);
  const pr = prOf(tab);
  return Promise.allSettled([
    t.workspace?.repo && d.scope !== "turn" && !t.gitDiff ? fetchGitDiff(tab, d.scope, d.scope === "base" ? d.base : null) : null,
    t.portsAt ? null : fetchPorts(tab),
    pr && !prCache.get(`${tab.key}:${pr.url}`)?.data ? loadPr(tab, pr.url, true) : null,
    hasProject(tab) && !memoryOf(tab).hits ? refreshMemory(tab) : null,
  ]);
}

// -- The composer chip -----------------------------------------------------------------------

const composerChip = Object.assign(document.createElement("button"), { type: "button", id: "composer-chip", className: "pick" });
composer.querySelector(".tools .spacer").before(composerChip);
function renderComposerChip(tab) {
  const label = tab ? phoneChipLabel(modelName(tab), tab.view.running ? traitsLabel(tab) : null, tab.view.running ? tab.mode : null) : "Default";
  setHtml(composerChip, `<span class="name">${esc(label)}</span>${phoneSvg("down", 13, "chev")}`);
  composerChip.setAttribute("aria-label", `Model and modes: ${label}`);
}

/** The chip's menu: the model, then what the desktop's controls beside it pick, the context, the
 * dictation language and the stash. */
function chipEntries(tab) {
  const entries = [{ label: "Model", hint: modelName(tab), run: () => openModels(composerChip) }];
  if (tab.view.running && tab.traits) entries.push({ label: "Reasoning", hint: traitsLabel(tab), run: () => openMenu(composerChip, traitEntries(tab), "traits") });
  if (tab.view.running && tab.mode) {
    const plan = tab.mode === "plan";
    const access = plan ? threadOf(tab).access ?? "manual" : tab.mode;
    entries.push({ label: "Access", hint: MODES[access]?.label, run: () => openMenu(composerChip, accessEntries(tab), "access") }, { label: "Plan mode", checked: plan, run: () => togglePlan(tab) });
  }
  const context = !contextMeter.hidden && contextOf(tab);
  if (context) entries.push({ label: "Context", hint: contextMeter.getAttribute("aria-label") ?? "", run: () => contextMeter.click() });
  const lang = document.getElementById("composer-dictate-lang");
  entries.push({ label: "Dictation language", hint: lang.title.replace(/^Dictation language: ([^.]+)\..*$/, "$1"), run: () => lang.click() });
  if (stash.length) entries.push({ label: "Stashed prompts", hint: String(stash.length), run: () => document.getElementById("stash").click() });
  return entries;
}
composerChip.onclick = () => {
  if (!active) return;
  if (!composerMenu.hidden && composerMenu.dataset.kind === "chip") return (composerMenu.hidden = true);
  openMenu(composerChip, chipEntries(active), "chip");
};
