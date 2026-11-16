/** The team's Calls page: what the bots ask the person, answered in place, then what works on its
 * own and what was settled today. The data and the answer logic are in gui-team-store.js. */

/** The cards the page draws: the open calls that ask me, and the ones answered here or by someone else while it was open. */
function tcCardCalls() {
  const mine = tcCallsForMe();
  for (const call of mine) tcStore.seen.add(call.id);
  const ids = new Set(mine.map((c) => c.id));
  const kept = tcStore.calls.filter((c) => !ids.has(c.id) && (tcStore.answers.has(c.id) || tcStore.seen.has(c.id)));
  return [...mine, ...kept].sort((a, b) => b.created_at - a.created_at);
}
const tcWaiting = () => tcCallsForMe().filter((c) => !["done", "taken"].includes(tcStore.answers.get(c.id)?.state)).length;

function tcHeadline(n) {
  if (!n) return "No calls. The bots are managing on their own";
  return n === 1 ? "1 call waiting for you" : `${n} calls waiting for you`;
}

function tcCardHtml(call) {
  const bot = tcBotName(call.workflow);
  const mine = tcStore.answers.get(call.id);
  const waiting = tcOpen(call) && tcAsksMe(call) && !["done", "taken"].includes(mine?.state);
  const too = tcCalledToo(call);
  const controls = tcCanAnswer(call)
    ? `${tcActionsHtml(call, { disabled: mine?.state === "sending", extra: call.run ? tcLinkHtml("run", call.run, "See what it did", "tc-seerun") : "" })}${too ? `<div class="tc-note">${esc(too)} The first to answer decides.</div>` : ""}`
    : "";
  return `<article class="tc-card" data-call="${esc(call.id)}" data-state="${waiting ? "waiting" : "quiet"}" aria-label="${esc(`Call from ${bot}`)}">
    ${tcBotMark(bot, "tc-mark-lg")}
    <div class="tc-card-body">
      <div class="tc-meta">${tcLinkHtml("bot", call.workflow, bot, "tc-botname")}<span class="tc-dim">called</span>${tcAskedChips(call)}<span class="tc-dim">${esc(tcAgo(call.created_at))}</span></div>
      ${tcQuestionHtml(call, "p", "tc-question")}
      ${controls}${tcResultHtml(call)}
    </div>
  </article>`;
}

/** The runs working now, each with its latest log line when the hub keeps one. */
function tcWorkingHtml() {
  const runs = tcStore.runs.filter((r) => r.state === "working").sort((a, b) => b.started_at - a.started_at);
  if (!runs.length) return `<p class="tc-empty">Nothing is running on its own right now.</p>`;
  return runs
    .map((run) => {
      const bot = tcBotName(run.workflow);
      const last = tcLastLine(run.id);
      const inner = `${tcBotMark(bot)}<span class="tc-line"><span class="tc-line-main">${esc(bot)} is working</span>${last ? `<span class="tc-line-sub">${esc(last)}</span>` : ""}</span><span class="tc-time">${esc(tcAgo(run.started_at))}</span>`;
      return tcPageOpens("team-run") ? `<button type="button" class="tc-item tc-link" data-tc-open="run" data-value="${esc(run.id)}">${inner}</button>` : `<div class="tc-item">${inner}</div>`;
    })
    .join("");
}

/** Calls answered today, then each bot's runs that finished today without a call. */
function tcResolvedHtml() {
  const since = tcMidnight();
  const items = [];
  for (const call of tcStore.calls.filter((c) => c.answer && c.answer.at >= since).sort((a, b) => b.answer.at - a.answer.at)) {
    const a = call.answer;
    const who = tcPerson(a.by) || "Someone";
    const bot = tcBotName(call.workflow);
    const group = call.asked.find((w) => w.kind === "group");
    const to = group ? ` to @${group.handle}` : "";
    const said = a.kind?.kind;
    const text =
      tcClosedByRun(call) ? `${bot}'s call was closed: ${tcClosedText(call).replace(/^Closed: /, "")}.`
        : said === "pass_to" ? `${who} passed ${bot}'s call${to} to @${a.kind.handle}.`
        : said === "take_over" ? `${who} took over ${bot}'s run.`
        : said === "stop" ? `${who} stopped ${bot}'s run.`
        : `${who} answered ${bot}'s call${to}: “${[a.choice, a.text].filter(Boolean).join(": ")}”.`;
    items.push(`<div class="tc-item tc-plain">${tcPersonMark(a.by)}<span class="tc-line"><span class="tc-line-main">${esc(text)}</span></span><span class="tc-time">${esc(tcClock(a.at))}</span></div>`);
  }
  const called = new Set(tcStore.calls.map((c) => c.run));
  const done = new Map();
  for (const run of tcStore.runs) if (run.state === "done" && (run.ended_at ?? 0) >= since && !called.has(run.id)) done.set(run.workflow, [...(done.get(run.workflow) ?? []), run]);
  for (const [workflow, runs] of done) {
    const bot = tcBotName(workflow);
    items.push(`<div class="tc-item tc-plain">${tcBotMark(bot)}<span class="tc-line"><span class="tc-line-main">${esc(`${bot} finished ${tcPlural(runs.length, "run", "runs")} without needing anyone.`)}</span></span><span class="tc-time">since ${esc(tcClock(Math.min(...runs.map((r) => r.started_at))))}</span></div>`);
  }
  return items.length ? items.join("") : `<p class="tc-empty">Nothing was settled yet today.</p>`;
}

function tcCallsPageHtml() {
  if (!tcCurrent()) return `<p class="tc-empty">Create or join a team to see the calls its bots make.</p>`;
  const cards = tcCardCalls();
  const body = !tcStore.loaded && !cards.length ? `<p class="tc-empty" role="status">Loading…</p>` : cards.map(tcCardHtml).join("");
  return `<div class="tc-title"><h1>${esc(tcHeadline(tcWaiting()))}</h1><p>The bots work on their own. They call you or one of your groups only when they need a person.</p></div>
    <section class="tc-cards" aria-label="Calls waiting for you">${body}</section>
    <section class="tc-block" aria-labelledby="tc-working"><h2 id="tc-working">Working on their own now</h2>${tcWorkingHtml()}</section>
    <section class="tc-block" aria-labelledby="tc-resolved"><h2 id="tc-resolved">Resolved today</h2>${tcResolvedHtml()}</section>`;
}

let tcCallsLive = null;
function tcStopCalls() {
  if (!tcCallsLive) return;
  tcCallsLive.off();
  clearInterval(tcCallsLive.timer);
  tcCallsLive = null;
}
function tcDrawCalls() {
  if (shellPage() !== "team-calls") return tcStopCalls();
  const el = pageView.querySelector(".tc-wrap");
  if (el) tcPaint(el, tcCallsPageHtml());
}
function tcShowCalls() {
  tcStore.seen.clear();
  for (const [id, answer] of tcStore.answers) if (answer.state !== "sending") tcStore.answers.delete(id);
  setPage(`${headHtml("Calls")}<div class="page-body"><div class="tc-wrap" data-tc-page="calls"></div></div>`);
  if (!tcCallsLive) tcCallsLive = { off: tcOnChange(tcDrawCalls), timer: setInterval(tcDrawCalls, 30000) };
  tcDrawCalls();
  tcRefresh();
}

const tcFindCall = (id) => tcStore.calls.find((c) => c.id === id);
pageView.addEventListener("click", (e) => {
  if (shellPage() !== "team-calls") return;
  const link = e.target.closest("[data-tc-open]");
  if (link) return tcOpenTarget(link);
  tcHandleClick(e, tcFindCall);
});
pageView.addEventListener("submit", (e) => shellPage() === "team-calls" && tcHandleSubmit(e, tcFindCall));
pageView.addEventListener("input", (e) => shellPage() === "team-calls" && tcHandleInput(e));

SHELL_PAGES["team-calls"] = { show: tcShowCalls };
