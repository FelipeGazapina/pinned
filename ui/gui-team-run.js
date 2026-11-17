/** One run of a team's bot: who it waits for, what it logged, the calls it made and who answered
 * them, with the buttons to take it over, give it back or stop it. */

function tcRunUnits() {
  const run = tcStore.runs.find((r) => r.id === tcRunShown) ?? null;
  return { run, bot: run ? tcBotName(run.workflow) : "" };
}

/** What the hub says of a run that ended: its outcome. Its times and cost are in `tcRunFactsHtml`. */
const tcEndedSub = (run) => run.outcome || "";

/** "Less than a minute", "12 min", "1 h 5 min": a time the hub counts in seconds. */
function tcDuration(secs) {
  const minutes = Math.round(Math.max(0, Number(secs) || 0) / 60);
  if (minutes < 1) return "Less than a minute";
  if (minutes < 60) return `${minutes} min`;
  return minutes % 60 ? `${Math.floor(minutes / 60)} h ${minutes % 60} min` : `${minutes / 60} h`;
}

/** The open call of a run that stopped at its limit (`situation: "on_limit"`), while the run goes on. */
const tcLimitCall = (run) => (["working", "waiting", "taken_over"].includes(run.state) ? tcStore.calls.find((c) => c.run === run.id && tcOpen(c) && c.situation === "on_limit") ?? null : null);

/** What the run has used, each on its own line: active time, waiting time (which no limit counts) and its cost, the box's
 * (`cost_usd`) and the model's (`model_usd`) apart. An older hub sends no `model_usd`: its line stays the box's alone. */
function tcRunFactsHtml(run) {
  const fact = (key, value, note = "") => `<p class="tc-fact"><span class="tc-fact-key">${key}:</span> <span class="tc-fact-value">${esc(value)}</span>${note ? ` <span class="tc-fact-note">(${esc(note)})</span>` : ""}</p>`;
  const usd = (n) => `US$ ${Number(n || 0).toFixed(2)}`;
  const cost = run.model_usd == null ? fact("Box cost", usd(run.cost_usd), "model usage not included") : fact("Cost", `Box ${usd(run.cost_usd)} · Model ${usd(run.model_usd)}`);
  return `<div class="tc-facts" aria-label="Time and cost of this run">${fact("Active time", tcDuration(run.active_secs))}${run.waiting_secs ? fact("Waiting for people", tcDuration(run.waiting_secs), "not counted toward the time limit") : ""}${cost}</div>`;
}

/** "Waiting for someone in @design", "Ana took over this run", "Stopped by you". `sub` is a second line when the run has one. */
function tcRunStatus(run) {
  const bot = tcBotName(run.workflow);
  const open = tcStore.calls.find((c) => c.run === run.id && tcOpen(c));
  switch (run.state) {
    case "working":
      return tcLimitCall(run) ? { title: "Limit reached", sub: "" } : { title: `${bot} is working`, sub: "" };
    case "waiting": {
      const parts = (open?.asked ?? []).map((w) => (w.kind === "group" ? `someone in @${w.handle}` : w.kind === "person" ? (tcPerson(w.account) === "You" ? "you" : tcPerson(w.account)) : (tcOwner()?.name ?? "the owner")));
      const waiting = parts.length ? `Waiting for ${[...new Set(parts)].join(" or ")}` : "Waiting for a person";
      return tcLimitCall(run) ? { title: "Limit reached", sub: waiting } : { title: waiting, sub: "" };
    }
    case "taken_over": {
      const who = run.controller ? tcPerson(run.controller) : "";
      return { title: who === "You" ? "You took over this run" : `${who || "Someone"} took over this run`, sub: "" };
    }
    case "done":
      return { title: "Finished", sub: tcEndedSub(run) };
    case "stopped":
      return { title: tcStore.stopped.has(run.id) ? "Stopped by you" : "Stopped", sub: tcEndedSub(run) };
    case "gave_up":
      return { title: `Gave up: nobody answered in ${TC_GIVE_UP_HOURS} hours`, sub: tcEndedSub(run) };
    default:
      return { title: "Failed", sub: tcEndedSub(run) };
  }
}

/** "run started at 3:00 PM", or with the day when it was not today. */
function tcStartedText(run) {
  const day = new Date(run.started_at * 1000).toLocaleDateString(tcLocale(), { month: "short", day: "numeric" });
  return run.started_at >= tcMidnight() ? `run started at ${tcClock(run.started_at)}` : `run started on ${day} at ${tcClock(run.started_at)}`;
}

/** Why the bot's own log is not drawn. The hub keeps a run's log in the run's own box, so an ended run whose log it cannot find lost it with the box. Any other failure is the hub's words as sent. */
function tcLogNote(run, failed) {
  if (!failed) return "The bot's own log could not be read, so this shows its calls and the answers.";
  if (/the team has no such run/i.test(failed) && ["done", "stopped", "gave_up", "failed"].includes(run.state)) return "The log went away with the run's box.";
  return `The bot's own log could not be read: ${failed}`;
}

/** The author of a log line: the bot, or a member when `who` is an account the team knows. */
function tcLogAuthor(who, bot) {
  if (!who || /^(bot|agent|assistant|claude)$/i.test(who)) return { name: bot, mark: tcBotMark(bot) };
  const member = tcStore.members.find((m) => m.account === who || m.email === who);
  if (member) return { name: tcPerson(member.account), mark: tcPersonMark(member.account) };
  return { name: who, mark: tcBotMark(who) };
}

function tcPeopleAsked(call) {
  const names = tcCallAccounts(call).map((a) => (tcPerson(a) === "You" ? "you" : tcPerson(a)));
  return names.length ? tcNameList(names) : tcNameList(call.asked.map(tcWhoLabel));
}

function tcEventHtml(entry, bot, run) {
  if (entry.kind === "log") {
    const author = tcLogAuthor(entry.who, bot);
    const head = entry.first ? `${esc(author.name)}${entry.at ? ", " : ""}` : "";
    return `<div class="tc-event">${author.mark}<div class="tc-event-body"><div class="tc-event-meta">${head}${entry.at ? esc(tcClock(entry.at)) : ""}</div><div class="tc-event-text">${esc(entry.text)}</div></div></div>`;
  }
  if (entry.kind === "answer" && tcClosedByRun(entry.call)) {
    return `<div class="tc-event">${tcBotMark(bot)}<div class="tc-event-body"><div class="tc-event-meta">Closed, ${esc(tcClock(entry.call.answer.at))}</div><div class="tc-event-text">${esc(tcClosedText(entry.call))}</div></div></div>`;
  }
  if (entry.kind === "answer") {
    const a = entry.call.answer;
    const said = tcAnswerText(entry.call);
    const who = tcPerson(a.by);
    return `<div class="tc-event">${tcPersonMark(a.by)}<div class="tc-event-body"><div class="tc-event-meta">${esc(who === "You" ? "You" : who)}, ${esc(tcClock(a.at))}</div><div class="tc-event-text">${esc(said.text)}</div></div></div>`;
  }
  const call = entry.call;
  const owner = tcOwner()?.name ?? "the owner";
  const open = tcOpen(call) && !tcRunEnded(call.run);
  const mine = tcStore.answers.get(call.id);
  const waiting = open && !["done", "taken"].includes(mine?.state);
  const typed = run?.state === "taken_over" && tcMe().account && run.controller === tcMe().account;
  const note = open ? `<div class="tc-note">Asked: ${esc(tcPeopleAsked(call))}. ${call.escalated ? `Nobody answered in ${TC_ESCALATE_MIN} minutes, so ${esc(owner)} was called too. If nobody answers in ${TC_GIVE_UP_HOURS} hours, the run gives up.` : `If nobody answers in ${TC_ESCALATE_MIN} minutes, I call ${esc(owner)} directly.`}</div>` : "";
  return `<div class="tc-event tc-call" data-call="${esc(call.id)}" data-state="${waiting ? "waiting" : "quiet"}">${tcBotMark(bot)}<div class="tc-event-body tc-stack"><div class="tc-event-meta">${esc(tcClock(call.created_at))}</div>
    ${tcQuestionHtml(call, "div", "tc-event-text")}${tcAskedChipsRow(call)}
    ${tcCanAnswer(call) ? tcActionsHtml(call, { typed, disabled: mine?.state === "sending" }) : ""}${note}${tcResultHtml(call)}${tcOpen(call) && !open ? `<div class="tc-result" data-state="other">Nobody answered.</div>` : ""}</div></div>`;
}
const tcAskedChipsRow = (call) => (call.asked.length ? `<div class="tc-chips">${tcAskedChips(call)}</div>` : "");

/** The run's log lines and calls in time order; `log` is null when the hub does not keep one. */
function tcTimeline(run, log) {
  const calls = tcStore.calls.filter((c) => c.run === run.id);
  const questions = new Set(calls.map((c) => c.question));
  const entries = [
    ...(log ?? []).filter((l) => !questions.has(l.text)).map((l) => ({ at: l.at, kind: "log", who: l.who, text: l.text, order: 0 })),
    ...calls.map((c) => ({ at: c.created_at, kind: "call", call: c, order: 1 })),
    ...calls.filter((c) => c.answer).map((c) => ({ at: c.answer.at || c.created_at, kind: "answer", call: c, order: 2 })),
  ].sort((a, b) => a.at - b.at || a.order - b.order);
  let last = null;
  for (const e of entries) {
    e.first = e.kind !== "log" || last?.kind !== "log" || last.who !== e.who;
    last = e;
  }
  return entries;
}

/** The run's open limit call as a panel above its log: the question, the choices as plain buttons, who it waits for. */
function tcLimitHtml(call, run, bot) {
  const owner = tcOwner()?.name ?? "the owner";
  const mine = tcStore.answers.get(call.id);
  const typed = run.state === "taken_over" && tcMe().account && run.controller === tcMe().account;
  const note = `<div class="tc-note">Asked: ${esc(tcPeopleAsked(call))}. ${call.escalated ? `Nobody answered in ${TC_ESCALATE_MIN} minutes, so ${esc(owner)} was called too. If nobody answers in ${TC_GIVE_UP_HOURS} hours, the run gives up.` : `If nobody answers in ${TC_ESCALATE_MIN} minutes, I call ${esc(owner)} directly.`}</div>`;
  return `<section class="tc-limit" data-call="${esc(call.id)}" data-state="${tcCanAnswer(call) ? "waiting" : "quiet"}" aria-label="Limit reached">${tcBotMark(bot, "tc-mark-lg")}<div class="tc-limit-body">
    <div class="tc-limit-head">Limit reached</div>${tcQuestionHtml(call, "p", "tc-question")}${tcAskedChipsRow(call)}
    ${tcCanAnswer(call) ? tcActionsHtml(call, { typed, disabled: mine?.state === "sending" }) : ""}${note}${tcResultHtml(call)}</div></section>`;
}

function tcRunPageHtml() {
  if (!tcCurrent()) return `<p class="tc-empty">Create or join a team to see its runs.</p>`;
  const { run, bot } = tcRunUnits();
  if (!run) return tcStore.loaded ? `<p class="tc-empty" role="status">This run is not in the team's recent runs.</p>` : `<p class="tc-empty" role="status">Loading…</p>`;
  const status = tcRunStatus(run);
  const me = tcMe();
  const ended = ["done", "stopped", "gave_up", "failed"].includes(run.state);
  const taken = run.state === "taken_over";
  const mineRun = taken && !!me.account && run.controller === me.account;
  const canTake = run.state === "working" || run.state === "waiting";
  const log = tcStore.logs.get(run.id);
  const limit = tcLimitCall(run);
  const entries = tcTimeline(run, log).filter((e) => !(limit && e.kind === "call" && e.call.id === limit.id));
  const controller = run.controller ? tcPerson(run.controller) : "Someone";
  const banner = taken
    ? `<div class="tc-banner" role="status"><span>${mineRun ? "You are in charge. The bot stopped where it was and waits. You can answer its call in your own words, pass it on or stop the bot." : `${esc(controller)} is in charge. The bot stopped where it was and waits.`}</span>${mineRun || me.owner ? `<button type="button" class="tc-primary" data-tc-run="give_back">Give back to the bot</button>` : ""}</div>`
    : "";
  const error = tcStore.runError?.run === run.id ? `<p class="tc-result" data-state="error" role="alert">${esc(tcStore.runError.line)}</p>` : "";
  const noLog = log === null && tcStore.logs.has(run.id) ? `<p class="tc-note" data-log-note>${esc(tcLogNote(run, tcStore.logErrors.get(run.id) ?? ""))}</p>` : "";
  return `<div class="tc-runhead"><div class="tc-runhead-text"><div class="tc-crumb">${tcLinkHtml("bot", run.workflow, bot, "tc-botname")}, ${esc(tcStartedText(run))}</div><h1>${esc(status.title)}</h1>${status.sub ? `<p class="tc-sub">${esc(status.sub)}</p>` : ""}</div>
      <div class="tc-head-actions">${canTake ? `<button type="button" class="tc-secondary" data-tc-run="take_over">Take over</button>` : ""}${ended ? "" : `<button type="button" class="tc-danger" data-tc-run="stop_run">Stop the bot</button>`}</div></div>
    ${tcRunFactsHtml(run)}${error}${banner}${limit ? tcLimitHtml(limit, run, bot) : ""}
    <section class="tc-timeline" aria-label="What happened in this run">${entries.length ? entries.map((e) => tcEventHtml(e, bot, run)).join("") : `<p class="tc-empty">Nothing was logged yet.</p>`}${noLog}</section>
    <p class="tc-footer">Everything the bot and the people do in a run is kept here.</p>`;
}

let tcRunLive = null;
const tcLoadingLogs = new Set();
function tcStopRun() {
  if (!tcRunLive) return;
  tcRunLive.off();
  clearInterval(tcRunLive.timer);
  tcRunLive = null;
}
function tcDrawRun() {
  if (shellPage() !== "team-run") return tcStopRun();
  if (tcRunShown && tcStore.runs.some((r) => r.id === tcRunShown) && !tcStore.logs.has(tcRunShown) && !tcLoadingLogs.has(tcRunShown)) {
    tcLoadingLogs.add(tcRunShown);
    tcLoadLog(tcRunShown).finally(() => tcLoadingLogs.delete(tcRunShown));
  }
  const el = pageView.querySelector(".tc-wrap");
  if (el) tcPaint(el, tcRunPageHtml());
}
function tcShowRun() {
  for (const [id, answer] of tcStore.answers) if (answer.state !== "sending") tcStore.answers.delete(id);
  tcStore.runError = null;
  setPage(`${headHtml("Run")}<div class="page-body"><div class="tc-wrap" data-tc-page="run"></div></div>`);
  if (!tcRunLive) tcRunLive = { off: tcOnChange(tcDrawRun), timer: setInterval(tcDrawRun, 30000) };
  if (tcRunShown) tcStore.logs.delete(tcRunShown);
  tcDrawRun();
  tcRefresh();
}

pageView.addEventListener("click", (e) => {
  if (shellPage() !== "team-run") return;
  const link = e.target.closest("[data-tc-open]");
  if (link) return tcOpenTarget(link);
  const op = e.target.closest("[data-tc-run]");
  if (op && !op.disabled && tcRunShown) {
    tcStore.runError = null;
    tcRunOp(op.dataset.tcRun, tcRunShown);
    return;
  }
  tcHandleClick(e, tcFindCall);
});
pageView.addEventListener("submit", (e) => shellPage() === "team-run" && tcHandleSubmit(e, tcFindCall));
pageView.addEventListener("input", (e) => shellPage() === "team-run" && tcHandleInput(e));

SHELL_PAGES["team-run"] = { show: tcShowRun };
