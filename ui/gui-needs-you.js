/** The Needs you page: one card for everything that waits on the user, on every machine, answered in
 * place. Threads come from needsYouItems() (redesign-model.js), agents' requests to act as the user
 * from agentConfirmations (agent-surface.js), and threads whose last reply asked something from
 * offerOf (global-chat.js). A project with two or more decisions (approvals, questions, plans) draws
 * them as one group at its first decision's place. A tap in a group picks an answer and sends nothing;
 * the group's one button sends every pick at once, each through needsYouAnswer to its own thread. The
 * page repaints on onModelChange while it shows and drops its listener and clock once it does not. */

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
  stopped: { label: "Stopped on error", glyph: "failed", rank: 2 },
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
 * line, confirmation }, and a thread's `project`. `line` is the card's one line of text: an error, a
 * summary, a question. */
function needsYouCards() {
  const threadCard = (item) => {
    const view = item.tab.view;
    const line =
      item.kind === "failed" ? String(view.message ?? "").trim() || null
        : item.kind === "stopped" ? item.error
        : item.kind === "sleep" ? item.sleep.text
        : item.kind === "review" ? needsYouFinished(item.tab)
        : item.kind === "question" ? item.ask.text
        : null;
    return { key: `thread:${item.tab.key}`, kind: item.kind, title: item.title, project: item.project, where: `${item.project} · ${item.machine}`, machine: item.tab.machine, since: item.since, tab: item.tab, ask: item.ask, limit: item.limit, line, confirmation: null, ...(item.sleep ? { sleep: item.sleep } : {}) };
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
  return [...shown, ...agents, ...threads.filter((card) => needsYouDismissed.get(card.key) !== card.since), ...offers].sort((a, b) => NEEDS_YOU_KINDS[a.kind].rank - NEEDS_YOU_KINDS[b.kind].rank || (b.since ?? 0) - (a.since ?? 0));
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
/** The stopped-on-error cards dismissed here, by card key, with the `since` each was dismissed at, so a
 * newer error on the same thread draws again. Kept for this page's life only. */
const needsYouDismissed = new Map();
const needsYouSig = (card) => `${card.kind}:${card.since}`;
/** The answer picked on a grouped card for its group's send, by card key: { choice, label, sig, ask },
 * with the card's needsYouSig and ask at pick time. */
const needsYouPicks = new Map();
/** Whether a pick still answers the card as drawn: the same ask, asked at the same time. */
const needsYouPickHolds = (card, pick) => needsYouSig(card) === pick.sig && askSig(card.ask) === askSig(pick.ask);

/** The cards a project's group gathers: the decisions on its threads. */
const NEEDS_YOU_DECISIONS = new Set(["approval", "question", "plan"]);

/** The cards in draw order, with each project's two or more decisions as one group, { key, project,
 * cards }, at its first decision's place. Every other card stays on its own where it was. */
function needsYouGroups(cards) {
  const decisions = new Map();
  for (const card of cards) if (NEEDS_YOU_DECISIONS.has(card.kind)) decisions.set(card.project, [...(decisions.get(card.project) ?? []), card]);
  return cards.flatMap((card) => {
    const mates = NEEDS_YOU_DECISIONS.has(card.kind) ? decisions.get(card.project) : [card];
    if (mates.length < 2) return [card];
    return mates[0] === card ? [{ key: `group:${card.project}`, project: card.project, cards: mates }] : [];
  });
}

/** The cards drawn last, and their groups, by key, for the clicks. */
let needsYouShown = new Map();
/** The keys of the cards drawn last inside a group, where a tap picks instead of sending. */
let needsYouGrouped = new Set();

/** Lays the cards out for a draw and keeps them for the clicks. Drops each answer and typed text
 * whose card is gone, and each pick whose card is gone, left its group or asks something new. */
function needsYouLayout(cards) {
  const layout = needsYouGroups(cards);
  const groups = layout.filter((entry) => entry.cards);
  needsYouShown = new Map([...cards, ...groups].map((c) => [c.key, c]));
  needsYouGrouped = new Set(groups.flatMap((g) => g.cards.map((c) => c.key)));
  for (const [key, answer] of needsYouAnswers) if (needsYouShown.get(key) ? needsYouSig(needsYouShown.get(key)) !== answer.sig : true) needsYouAnswers.delete(key);
  for (const key of needsYouTyped.keys()) if (!needsYouShown.has(key)) needsYouTyped.delete(key);
  for (const [key, pick] of needsYouPicks) if (!needsYouGrouped.has(key) || !needsYouPickHolds(needsYouShown.get(key), pick)) needsYouPicks.delete(key);
  return layout;
}

const nyButton = (act, label, cls, attrs = "") => `<button type="button" class="${cls}" data-ny="${act}"${attrs}>${esc(label)}</button>`;
const nyOpen = () => nyButton("open", "Open", "ny-link");

function needsYouHeadHtml(card) {
  const kind = NEEDS_YOU_KINDS[card.kind];
  const limit = card.limit;
  const when = limit ? `<span class="ny-when">${limit.held ? "Resets" : "Continues"} at ${esc(clock.format(limit.resumesAt))}</span>` : card.since ? `<span class="ticker ny-when" data-since="${card.since}"></span>` : "";
  return `<div class="ny-head">${glyphHtml(kind.glyph)}<span class="ny-kind">${kind.label}</span><span class="ny-where">${esc(card.where)}</span>${boxMarkHtml(card.tab)}${when}</div>`;
}

/** The card's controls. In a group, `grouped` is { pick }: each answer says whether it is the pick,
 * and the typed answer is picked, not sent. */
function needsYouActionsHtml(card, grouped = null) {
  const { ask } = card;
  const choice = (c) => ask?.choices?.some((x) => x.choice === c);
  const pressed = (c) => (grouped ? ` aria-pressed="${JSON.stringify(c) === JSON.stringify(grouped.pick?.choice)}"` : "");
  const row = (html) => (html ? `<div class="ny-actions">${html}</div>` : "");
  switch (card.kind) {
    case "agent": {
      const busy = agentDeciding.has(card.confirmation.id) ? " disabled" : "";
      return row(`${nyButton("deny", "Deny", "ny-secondary", ` data-agent="deny"${busy}`)}${nyButton("approve", "Approve", "ny-primary", ` data-agent="approve"${busy}`)}`);
    }
    case "approval":
      return row(`${choice("decline") ? nyButton("decline", "Decline", "ny-secondary", pressed("decline")) : ""}${choice("approve") ? nyButton("approve", "Approve", "ny-primary", pressed("approve")) : ""}${nyOpen()}`);
    case "question": {
      if (!ask?.choices) return row(nyOpen());
      const options = ask.choices.map((c) => `<button type="button" class="ny-option" data-ny="option" data-option="${c.choice.option}"${pressed(c.choice)}><span class="ny-label">${esc(c.label)}</span>${c.about ? `<span class="ny-about">${esc(c.about)}</span>` : ""}</button>`);
      const typed = ask.typed
        ? `<form class="ny-typed" data-ny-form><input type="text" name="answer" placeholder="Type or dictate another answer" aria-label="Type or dictate another answer" autocomplete="off" enterkeyhint="${grouped ? "done" : "send"}"><button type="submit" class="ny-send">${grouped ? "Pick" : "Send"}</button></form>`
        : "";
      return `<div class="ny-options">${options.join("")}${typed}</div>`;
    }
    case "plan":
      return row(`${ask?.choices?.length ? nyButton("plan", "Approve plan", "ny-primary", pressed(ask.choices[0].choice)) : ""}${nyOpen()}`);
    case "limit":
      return row(`${card.limit.switchTo.map((a) => nyButton("switch", `Continue under ${a.name}`, "ny-secondary ny-plain", ` data-account="${esc(a.id)}"`)).join("")}${nyButton("snooze", "Snooze", "ny-quiet")}`);
    case "failed":
      return row(nyOpen());
    case "stopped":
      return row(`${nyButton("continue", "Try again", "ny-primary")}${nyButton("dismiss", "Dismiss", "ny-secondary")}`);
    case "sleep":
      return row(`${nyButton("continue", "Continue", "ny-primary")}${nyOpen()}`);
    default:
      return "";
  }
}

const NEEDS_YOU_NOTES = { sending: "Sending…", sent: "Answered. The thread goes on.", missed: "The question changed on its screen. Open the thread to answer it." };

/** A card's markup. `grouped` is { pick } for a card in a group: its pick shows with a Clear. */
function needsYouCardHtml(card, answer = null, grouped = null) {
  const attrs = `class="ny-card" data-key="${esc(card.key)}" data-kind="${card.kind}"${card.confirmation ? ` data-confirmation="${esc(card.confirmation.id)}"` : ""}`;
  const label = `${NEEDS_YOU_KINDS[card.kind].label}: ${card.title}`;
  if (card.kind === "shown") return `<article ${attrs} aria-label="${esc(label)}">${agentShownHtml()}</article>`;
  if (card.kind === "review" || card.kind === "offer") {
    const sub = card.kind === "review" ? ["Finished", card.line].filter(Boolean).join(" · ") : `Asked · ${card.line}`;
    return `<article ${attrs} aria-label="${esc(`${label}, ${card.where}`)}">${glyphHtml(NEEDS_YOU_KINDS[card.kind].glyph)}<span class="ny-texts"><span class="ny-name">${esc(card.title)}</span><span class="ny-sub">${esc(sub)}</span></span>${boxMarkHtml(card.tab)}${nyOpen()}</article>`;
  }
  const body =
    card.kind === "approval" && card.ask?.command ? `<pre>${esc(card.ask.command)}</pre>`
      : card.kind === "agent" && Object.keys(card.confirmation.args ?? {}).length && !AGENT_TITLE_ONLY.has(card.confirmation.action) ? `<pre>${esc(JSON.stringify(card.confirmation.args, null, 2))}</pre>`
      : card.kind !== "agent" && card.line && card.line !== card.title ? `<p class="ny-line">${esc(card.line)}</p>`
      : "";
  const unsupervised = card.sleep?.delegated ? `<p class="ny-line">${esc(`${nyPlural(card.sleep.delegated, "delegated task")} still running with nobody to supervise ${card.sleep.delegated === 1 ? "it" : "them"}.`)}</p>` : "";
  const agentLine = unsupervised + (card.kind === "agent" ? `<p class="ny-line">${esc(card.line)}</p>` : "");
  const actions = answer?.state === "sent" ? "" : `<fieldset class="ny-fields"${answer?.state === "sending" ? " disabled" : ""}>${needsYouActionsHtml(card, grouped)}</fieldset>`;
  const pick = grouped?.pick ? `<div class="ny-pick"><span class="ny-picked">Picked: ${esc(grouped.pick.label)}</span>${nyButton("clear", "Clear", "ny-quiet")}</div>` : "";
  const note = answer ? `<p class="ny-note" role="status">${NEEDS_YOU_NOTES[answer.state]}</p>` : "";
  return `<article ${attrs} aria-label="${esc(label)}">${needsYouHeadHtml(card)}<div class="ny-name">${esc(card.title)}</div>${body}${agentLine}${actions}${pick}${note}</article>`;
}

/** The group's cards with a pick to send, never one already sending or sent. */
const needsYouPicked = (group) => group.cards.filter((c) => needsYouPicks.has(c.key) && !["sending", "sent"].includes(needsYouAnswers.get(c.key)?.state));
/** What the group's head and button read: how many decisions wait, and how many picks it sends. */
function needsYouGroupTexts(group) {
  const picked = needsYouPicked(group).length;
  return { count: nyPlural(group.cards.length, "decision"), send: picked ? `Send ${nyPlural(picked, "answer")}` : "Send answers", off: !picked };
}
/** A grouped card's markup, with its answer and its pick. */
const needsYouGroupedHtml = (card) => needsYouCardHtml(card, needsYouAnswers.get(card.key), { pick: needsYouPicks.get(card.key) });

/** A group's markup: the project and its count, its cards, and the one button that sends their picks. */
function needsYouGroupHtml(group) {
  const texts = needsYouGroupTexts(group);
  return `<div class="ny-group" role="group" data-key="${esc(group.key)}" aria-label="${esc(group.project)}"><div class="ny-group-head"><span class="ny-group-name">${esc(group.project)}</span><span class="ny-group-count">${texts.count}</span></div><div class="ny-group-cards">${group.cards.map(needsYouGroupedHtml).join("")}</div><button type="button" class="ny-primary ny-group-send" data-ny="send"${texts.off ? " disabled" : ""}>${texts.send}</button></div>`;
}

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

/** Draws the cards and groups, replacing only the cards whose markup changed and patching a group's
 * head and button in place, so a field being typed in and the button under a press survive a repaint. */
function drawNeedsYou() {
  if (shellPage() !== "needs-you") return stopNeedsYou();
  const cards = needsYouCards();
  const layout = needsYouLayout(cards);
  const summary = needsYouSummary(cards);
  for (const el of pageView.querySelectorAll(".ny-count")) if (el.textContent !== summary) el.textContent = summary;
  const list = pageView.querySelector(".ny-list");
  const drawn = new Map([...list.querySelectorAll("[data-key]")].map((el) => [el.dataset.key, el]));
  const focused = document.activeElement;
  const parse = (html) => {
    const holder = document.createElement("template");
    holder.innerHTML = html;
    return holder.content.firstElementChild;
  };
  const fill = (parent, next) => {
    if (next.length !== parent.children.length || next.some((el, i) => parent.children[i] !== el)) parent.replaceChildren(...next);
  };
  const cardEl = (card, html) => {
    const old = drawn.get(card.key);
    if (old && needsYouDrawn.get(old) === html) return old;
    const el = parse(html);
    needsYouDrawn.set(el, html);
    const field = el.querySelector("[data-ny-form] input");
    if (field) field.value = needsYouTyped.get(card.key) ?? "";
    if (old?.contains(focused)) {
      const same = focused.matches("input") ? field : el.querySelector(`[data-ny="${focused.dataset?.ny}"]${focused.dataset?.option ? `[data-option="${focused.dataset.option}"]` : ""}`);
      queueMicrotask(() => (same ?? el.querySelector("button:not(:disabled), input"))?.focus());
    }
    return el;
  };
  const groupEl = (group) => {
    const el = drawn.get(group.key) ?? parse(needsYouGroupHtml(group));
    const texts = needsYouGroupTexts(group);
    const [count, send] = [el.querySelector(".ny-group-count"), el.querySelector(".ny-group-send")];
    if (count.textContent !== texts.count) count.textContent = texts.count;
    if (send.textContent !== texts.send) send.textContent = texts.send;
    send.disabled = texts.off;
    fill(el.querySelector(".ny-group-cards"), group.cards.map((card) => cardEl(card, needsYouGroupedHtml(card))));
    return el;
  };
  fill(list, layout.map((entry) => (entry.cards ? groupEl(entry) : cardEl(entry, needsYouCardHtml(entry, needsYouAnswers.get(entry.key))))));
  pageView.querySelector(".ny-empty").hidden = cards.length > 0;
  tickNeedsYou();
}

/** The page's heading: it says Personal while a team is current, so the person's own asks say whose they are. */
const needsYouTeamCurrent = () => typeof teamsNow !== "undefined" && teamsNow !== null;
const needsYouTitle = () => (needsYouTeamCurrent() ? "Needs you · Personal" : "Needs you");
/** Sets both headings of the open page to the title, after a team switch. */
function paintNeedsYouTitle() {
  for (const h of pageView.querySelectorAll(".page-head h1, .ny-title h1")) h.textContent = needsYouTitle();
}
document.addEventListener("teams:changed", () => needsYouLive && paintNeedsYouTitle());

function showNeedsYou() {
  setPage(`${headHtml(needsYouTitle(), `<span class="range ny-count"></span>`)}<div class="page-body"><div class="ny-wrap">
    <div class="ny-title"><h1>${needsYouTitle()}</h1><span class="ny-count"></span></div>
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

/** Answers a lone card at once. A card drawn in a group keeps the answer as its pick, read as
 * `label`, for the group's send. */
function needsYouChoose(card, choice, label) {
  if (!needsYouGrouped.has(card.key)) return needsYouAnswer(card, choice);
  needsYouPicks.set(card.key, { choice, label, sig: needsYouSig(card), ask: card.ask });
  drawNeedsYou();
}

/** Sends each of the group's picks to its own thread at once, through needsYouAnswer with the card as
 * drawn, and skips a pick that no longer holds. A pick goes as its send starts, so a second press
 * sends nothing twice, and a card that misses can be picked again. */
function needsYouSend(group) {
  const picked = needsYouPicked(group).map((card) => [card, needsYouPicks.get(card.key)]);
  for (const [card] of picked) needsYouPicks.delete(card.key);
  return Promise.allSettled(picked.filter(([card, pick]) => needsYouPickHolds(card, pick)).map(([card, pick]) => needsYouAnswer(card, pick.choice)));
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
  dismiss: (card) => {
    needsYouDismissed.set(card.key, card.since);
    drawNeedsYou();
  },
  open: (card) => activate(card.tab),
  approve: (card) => needsYouChoose(card, "approve", "Approve"),
  decline: (card) => needsYouChoose(card, "decline", "Decline"),
  option: (card, b) => {
    const option = Number(b.dataset.option);
    return needsYouChoose(card, { option }, card.ask.choices.find((c) => c.choice.option === option)?.label);
  },
  plan: (card) => needsYouChoose(card, card.ask.choices[0].choice, "Approve plan"),
  clear: (card) => {
    needsYouPicks.delete(card.key);
    drawNeedsYou();
  },
  send: (group) => needsYouSend(group),
  // The typed answer's form submits rather than clicks.
  typed: (card, form) => {
    const text = form.answer.value.trim();
    if (text) return needsYouChoose(card, { text }, `“${text}”`);
  },
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
  if (card) NEEDS_YOU_ACTS.typed(card, form);
});

SHELL_PAGES["needs-you"] = { show: showNeedsYou };
