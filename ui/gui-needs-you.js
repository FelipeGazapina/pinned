/** The Needs you page: one card for everything that waits on the user, on every machine, answered in
 * place. Threads come from needsYouItems() (redesign-model.js), agents' requests to act as the user
 * from agentConfirmations (agent-surface.js), and threads whose last reply asked something from
 * offerOf (global-chat.js). The page repaints on onModelChange while it shows and drops its listener
 * and clock once it does not. */

/** What each card kind reads as: the word on its head, its status dot, and its place in the list,
 * asks first, then what stopped, then what only wants a look. */
const NEEDS_YOU_KINDS = {
  shown: { label: "Agent request", glyph: "needs", rank: -1 },
  agent: { label: "Agent request", glyph: "needs", rank: 0 },
  approval: { label: "Approval", glyph: "needs", rank: 0 },
  question: { label: "Question", glyph: "needs", rank: 0 },
  plan: { label: "Plan", glyph: "needs", rank: 0 },
  limit: { label: "Usage limit", glyph: "failed", rank: 1 },
  failed: { label: "Failed", glyph: "failed", rank: 2 },
  sleep: { label: "Stopped", glyph: "failed", rank: 2 },
  review: { label: "Finished", glyph: "unread", rank: 3 },
  offer: { label: "Asked", glyph: "idle", rank: 4 },
};

/** The last answer's first line, or the pull request it opened, for a finished thread's card. */
function needsYouFinished(tab) {
  const pr = prOf(tab) ?? tab.view.pr;
  if (pr?.number) return `#${pr.number} ready to review`;
  return threadOf(tab).items.findLast((i) => i.kind === "text")?.text.trim().split("\n")[0] || null;
}

/** Every card the page draws, in order: { key, kind, title, where, machine, since, tab, ask, limit,
 * line, confirmation }. `line` is the card's one line of text: an error, a summary, a question. */
function needsYouCards() {
  const threadCard = (item) => {
    const view = item.tab.view;
    const line =
      item.kind === "failed" ? String(view.message ?? "").trim() || null
        : item.kind === "sleep" ? item.sleep.text
        : item.kind === "review" ? needsYouFinished(item.tab)
        : item.kind === "question" ? item.ask.text
        : null;
    return { key: `thread:${item.tab.key}`, kind: item.kind, title: item.title, where: `${item.project} · ${item.machine}`, machine: item.tab.machine, since: item.since, tab: item.tab, ask: item.ask, limit: item.limit, line, confirmation: null, ...(item.sleep ? { sleep: item.sleep } : {}) };
  };
  const threads = needsYouItems().map(threadCard);
  const listed = new Set(threads.map((c) => c.tab));
  const now = nowS();
  const offers = tabs
    .filter((tab) => !listed.has(tab))
    .map((tab) => ({ tab, offer: offerOf(tab.view, now) }))
    .filter((o) => o.offer)
    .map(({ tab, offer }) => ({ key: `thread:${tab.key}`, kind: "offer", title: threadTitle(tab), where: `${projectName(tab)} · ${machineName(tab.machine)}`, machine: tab.machine, since: (tab.view.updated ?? 0) * 1000 || null, tab, ask: null, limit: null, line: offer, confirmation: null }));
  const agents = agentConfirmations.map((c) => ({
    key: `agent:${c.id}`,
    kind: "agent",
    title: `${agentActionTitle(c.action, c.args, c.machine)}${c.target ? ` on ${agentMachineName(c.target) || c.target}` : ""}`,
    where: `${agentWords(c.role)} on ${c.machine_name || "removed machine"}`,
    machine: c.machine,
    since: c.at ? c.at * 1000 : null,
    tab: null,
    ask: null,
    limit: null,
    line: `Asked by ${agentQuoted(c)}`,
    confirmation: c,
  }));
  // What an approved request handed the user to see once, such as a pairing command.
  const shown = agentShown ? [{ key: "agent-shown", kind: "shown", title: agentShown.action, where: "", machine: null, since: null, tab: null, ask: null, limit: null, line: null, confirmation: null }] : [];
  return [...shown, ...agents, ...threads, ...offers].sort((a, b) => NEEDS_YOU_KINDS[a.kind].rank - NEEDS_YOU_KINDS[b.kind].rank || (b.since ?? 0) - (a.since ?? 0));
}

const nyPlural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
/** "4 threads on 3 machines", with the agents' requests after it. An offer blocks nobody, so it is
 * not counted. */
function needsYouSummary(cards) {
  const threads = cards.filter((c) => c.tab && c.kind !== "offer");
  const machinesOf = new Set(threads.map((c) => c.machine));
  const agents = cards.filter((c) => c.kind === "agent").length;
  return [threads.length && `${nyPlural(threads.length, "thread")} on ${nyPlural(machinesOf.size, "machine")}`, agents && nyPlural(agents, "agent request")].filter(Boolean).join(" · ");
}

/** What an answer from a card came to, by card key: `sig` is the ask it answered, `state` is
 * "sending", "sent" or "missed". A new ask on the same thread has a new `sig` and draws afresh. */
const needsYouAnswers = new Map();
/** The typed answer in a card's field, by card key, so a repaint never loses it. */
const needsYouTyped = new Map();
const needsYouSig = (card) => `${card.kind}:${card.since}`;

const nyButton = (act, label, cls, attrs = "") => `<button type="button" class="${cls}" data-ny="${act}"${attrs}>${esc(label)}</button>`;
const nyOpen = () => nyButton("open", "Open", "ny-link");

function needsYouHeadHtml(card) {
  const kind = NEEDS_YOU_KINDS[card.kind];
  const limit = card.limit;
  const when = limit ? `<span class="ny-when">${limit.held ? "Resets" : "Continues"} at ${esc(clock.format(limit.resumesAt))}</span>` : card.since ? `<span class="ticker ny-when" data-since="${card.since}"></span>` : "";
  return `<div class="ny-head">${glyphHtml(kind.glyph)}<span class="ny-kind">${kind.label}</span><span class="ny-where">${esc(card.where)}</span>${boxMarkHtml(card.tab)}${when}</div>`;
}

/** The card's controls. */
function needsYouActionsHtml(card) {
  const { ask } = card;
  const choice = (c) => ask?.choices?.some((x) => x.choice === c);
  const row = (html) => (html ? `<div class="ny-actions">${html}</div>` : "");
  switch (card.kind) {
    case "agent": {
      const busy = agentDeciding.has(card.confirmation.id) ? " disabled" : "";
      return row(`${nyButton("deny", "Deny", "ny-secondary", ` data-agent="deny"${busy}`)}${nyButton("approve", "Approve", "ny-primary", ` data-agent="approve"${busy}`)}`);
    }
    case "approval":
      return row(`${choice("decline") ? nyButton("decline", "Decline", "ny-secondary") : ""}${choice("approve") ? nyButton("approve", "Approve", "ny-primary") : ""}${nyOpen()}`);
    case "question": {
      if (!ask?.choices) return row(nyOpen());
      const options = ask.choices.map((c) => `<button type="button" class="ny-option" data-ny="option" data-option="${c.choice.option}"><span class="ny-label">${esc(c.label)}</span>${c.about ? `<span class="ny-about">${esc(c.about)}</span>` : ""}</button>`);
      const typed = ask.typed
        ? `<form class="ny-typed" data-ny-form><input type="text" name="answer" placeholder="Type or dictate another answer" aria-label="Type or dictate another answer" autocomplete="off" enterkeyhint="send"><button type="submit" class="ny-send">Send</button></form>`
        : "";
      return `<div class="ny-options">${options.join("")}${typed}</div>`;
    }
    case "plan":
      return row(`${ask?.choices?.length ? nyButton("plan", "Approve plan", "ny-primary") : ""}${nyOpen()}`);
    case "limit":
      return row(`${card.limit.switchTo.map((a) => nyButton("switch", `Continue under ${a.name}`, "ny-secondary ny-plain", ` data-account="${esc(a.id)}"`)).join("")}${nyButton("snooze", "Snooze", "ny-quiet")}`);
    case "failed":
      return row(nyOpen());
    case "sleep":
      return row(`${nyButton("continue", "Continue", "ny-primary")}${nyOpen()}`);
    default:
      return "";
  }
}

const NEEDS_YOU_NOTES = { sending: "Sending…", sent: "Answered. The thread goes on.", missed: "The question changed on its screen. Open the thread to answer it." };

function needsYouCardHtml(card, answer = null) {
  const attrs = `class="ny-card" data-key="${esc(card.key)}" data-kind="${card.kind}"${card.confirmation ? ` data-confirmation="${esc(card.confirmation.id)}"` : ""}`;
  const label = `${NEEDS_YOU_KINDS[card.kind].label}: ${card.title}`;
  if (card.kind === "shown") return `<article ${attrs} aria-label="${esc(label)}">${agentShownHtml()}</article>`;
  if (card.kind === "review" || card.kind === "offer") {
    const sub = card.kind === "review" ? ["Finished", card.line].filter(Boolean).join(" · ") : `Asked · ${card.line}`;
    return `<article ${attrs} aria-label="${esc(`${label}, ${card.where}`)}">${glyphHtml(NEEDS_YOU_KINDS[card.kind].glyph)}<span class="ny-texts"><span class="ny-name">${esc(card.title)}</span><span class="ny-sub">${esc(sub)}</span></span>${boxMarkHtml(card.tab)}${nyOpen()}</article>`;
  }
  const body =
    card.kind === "approval" && card.ask?.command ? `<pre>${esc(card.ask.command)}</pre>`
      : card.kind === "agent" && Object.keys(card.confirmation.args ?? {}).length ? `<pre>${esc(JSON.stringify(card.confirmation.args, null, 2))}</pre>`
      : card.line && card.line !== card.title ? `<p class="ny-line">${esc(card.line)}</p>`
      : "";
  const unsupervised = card.sleep?.delegated ? `<p class="ny-line">${esc(`${nyPlural(card.sleep.delegated, "delegated task")} still running with nobody to supervise ${card.sleep.delegated === 1 ? "it" : "them"}.`)}</p>` : "";
  const agentLine = unsupervised + (card.kind === "agent" ? `<p class="ny-line">${esc(card.line)}</p>` : "");
  const actions = answer?.state === "sent" ? "" : `<fieldset class="ny-fields"${answer?.state === "sending" ? " disabled" : ""}>${needsYouActionsHtml(card)}</fieldset>`;
  const note = answer ? `<p class="ny-note" role="status">${NEEDS_YOU_NOTES[answer.state]}</p>` : "";
  return `<article ${attrs} aria-label="${esc(label)}">${needsYouHeadHtml(card)}<div class="ny-name">${esc(card.title)}</div>${body}${agentLine}${actions}${note}</article>`;
}

/** The cards drawn last, by key, for the clicks. */
let needsYouShown = new Map();
const needsYouDrawn = new WeakMap();
let needsYouLive = null;

function stopNeedsYou() {
  if (!needsYouLive) return;
  needsYouLive.off();
  clearInterval(needsYouLive.timer);
  needsYouLive = null;
}

function tickNeedsYou() {
  if (shellPage() !== "needs-you") return stopNeedsYou();
  for (const el of pageView.querySelectorAll(".ny-list .ticker[data-since]")) {
    const text = ago(Number(el.dataset.since) / 1000);
    if (el.textContent !== text) el.textContent = text;
  }
}

/** Draws the cards, replacing only those whose markup changed, so a field being typed in and the
 * button under a press survive a repaint. */
function drawNeedsYou() {
  if (shellPage() !== "needs-you") return stopNeedsYou();
  const cards = needsYouCards();
  needsYouShown = new Map(cards.map((c) => [c.key, c]));
  for (const [key, answer] of needsYouAnswers) if (needsYouShown.get(key) ? needsYouSig(needsYouShown.get(key)) !== answer.sig : true) needsYouAnswers.delete(key);
  for (const key of needsYouTyped.keys()) if (!needsYouShown.has(key)) needsYouTyped.delete(key);
  const summary = needsYouSummary(cards);
  for (const el of pageView.querySelectorAll(".ny-count")) if (el.textContent !== summary) el.textContent = summary;
  const list = pageView.querySelector(".ny-list");
  const drawn = new Map([...list.children].map((el) => [el.dataset.key, el]));
  const focused = document.activeElement;
  const next = cards.map((card) => {
    const html = needsYouCardHtml(card, needsYouAnswers.get(card.key));
    const old = drawn.get(card.key);
    if (old && needsYouDrawn.get(old) === html) return old;
    const holder = document.createElement("template");
    holder.innerHTML = html;
    const el = holder.content.firstElementChild;
    needsYouDrawn.set(el, html);
    const field = el.querySelector("[data-ny-form] input");
    if (field) field.value = needsYouTyped.get(card.key) ?? "";
    if (old?.contains(focused)) {
      const same = focused.matches("input") ? field : el.querySelector(`[data-ny="${focused.dataset?.ny}"]${focused.dataset?.option ? `[data-option="${focused.dataset.option}"]` : ""}`);
      queueMicrotask(() => (same ?? el.querySelector("button:not(:disabled), input"))?.focus());
    }
    return el;
  });
  if (next.length !== list.children.length || next.some((el, i) => list.children[i] !== el)) list.replaceChildren(...next);
  pageView.querySelector(".ny-empty").hidden = cards.length > 0;
  tickNeedsYou();
}

function showNeedsYou() {
  setPage(`${headHtml("Needs you", `<span class="range ny-count"></span>`)}<div class="page-body"><div class="ny-wrap">
    <div class="ny-title"><h1>Needs you</h1><span class="ny-count"></span></div>
    <section class="ny-list" aria-label="Threads that need you"></section>
    <p class="ny-empty" role="status" hidden>Nothing needs you right now.</p>
  </div></div>`);
  if (!needsYouLive) needsYouLive = { off: onModelChange(drawNeedsYou), timer: setInterval(tickNeedsYou, 15000) };
  drawNeedsYou();
}

/** Answers the card's thread without leaving the page, and says on the card how it went. */
async function needsYouAnswer(card, choice) {
  const sig = needsYouSig(card);
  needsYouAnswers.set(card.key, { sig, state: "sending" });
  drawNeedsYou();
  const sent = await answerAsk(card.tab, choice, card.ask).catch(() => false);
  if (needsYouAnswers.get(card.key)?.sig === sig) needsYouAnswers.set(card.key, { sig, state: sent ? "sent" : "missed" });
  if (sent) needsYouTyped.delete(card.key);
  drawNeedsYou();
}

/** What the harness types to a session that has work left, as sessions.rs's CONTINUE_PROMPT. */
const CONTINUE_PROMPT = "Continue from where you left off.";

/** Types the prompt as the user would; Enter follows apart, or the terminal takes it as part of a paste. */
async function needsYouContinue(card) {
  await typeInto(card.tab, CONTINUE_PROMPT);
  await new Promise((r) => setTimeout(r, 300));
  await typeInto(card.tab, "\r");
}

const NEEDS_YOU_ACTS = {
  continue: (card) => needsYouContinue(card),
  open: (card) => activate(card.tab),
  approve: (card) => needsYouAnswer(card, "approve"),
  decline: (card) => needsYouAnswer(card, "decline"),
  option: (card, b) => needsYouAnswer(card, { option: Number(b.dataset.option) }),
  plan: (card) => needsYouAnswer(card, card.ask.choices[0].choice),
  switch: (card, b) => switchAccount(card.tab, b.dataset.account),
  snooze: (card) => snooze(card.tab, card.limit.resumesAt / 1000),
};

pageView.addEventListener("click", (e) => {
  if (shellPage() !== "needs-you") return;
  if (e.target.closest("[data-agent]")) return agentSurfaceClick(e);
  const button = e.target.closest("[data-ny]");
  const card = button && needsYouShown.get(button.closest("[data-key]")?.dataset.key);
  if (card && !button.matches(":disabled")) NEEDS_YOU_ACTS[button.dataset.ny](card, button);
});
pageView.addEventListener("input", (e) => {
  const form = e.target.closest?.("[data-ny-form]");
  if (form && shellPage() === "needs-you") needsYouTyped.set(form.closest("[data-key]").dataset.key, e.target.value);
});
pageView.addEventListener("submit", (e) => {
  const form = e.target.closest("[data-ny-form]");
  if (!form || shellPage() !== "needs-you") return;
  e.preventDefault();
  const card = needsYouShown.get(form.closest("[data-key]").dataset.key);
  const text = form.answer.value.trim();
  if (card && text) needsYouAnswer(card, { text });
});

SHELL_PAGES["needs-you"] = { show: showNeedsYou };
