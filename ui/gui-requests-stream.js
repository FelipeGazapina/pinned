// All activity: every request of the global Orchestrator, grouped by day, newest first, with a 420px
// side panel that shows one request's "Where this stands" card and its latest items. It opens over the
// chat from the Requests column's "All activity" and needs no tab: the card is unit 7's (requestCardFrom)
// and the items come from `user_request`. Older requests are paged with `user_requests{before}`.
// Loaded after gui-requests.js, whose glances, state and Open thread it reuses.

const STREAM_ITEMS = 20;
const STREAM_END_PX = 48;
const STREAM_EARLIER = "Earlier conversation";
const STREAM_WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const STREAM_MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

const rqs = {
  open: false,
  /** The key of the tab All activity opened over: another tab closes it. */
  key: null,
  /** Requests paged in past the ones the host's tabs carry, and whether the host has no more. */
  older: [],
  end: false,
  paging: false,
  selected: null,
  /** Request id -> { view, at }: the `user_request` reply behind the side panel. */
  views: new Map(),
  loading: new Set(),
  drawn: "",
};

const streamPad = (n) => String(n).padStart(2, "0");
const streamTime = (ms) => {
  const d = new Date(ms);
  return `${streamPad(d.getHours())}:${streamPad(d.getMinutes())}`;
};
const streamDayStart = (ms) => {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
};

/** "Today", "Yesterday", the weekday inside the last week, else the date, by the local calendar. */
function requestDayLabel(ms, now) {
  const back = Math.round((streamDayStart(now) - streamDayStart(ms)) / 86400000);
  if (back <= 0) return "Today";
  if (back === 1) return "Yesterday";
  const d = new Date(ms);
  if (back < 7) return STREAM_WEEKDAYS[d.getDay()];
  const date = `${STREAM_MONTHS[d.getMonth()]} ${d.getDate()}`;
  return d.getFullYear() === new Date(now).getFullYear() ? date : `${date}, ${d.getFullYear()}`;
}

/** The glances grouped by the local day they opened, newest first: [{ label, glances }]. */
function requestDays(glances, now = Date.now()) {
  const days = [];
  for (const glance of [...glances].sort((a, b) => b.opened_at - a.opened_at)) {
    const start = streamDayStart(glance.opened_at);
    const last = days.at(-1);
    if (last?.start === start) last.glances.push(glance);
    else days.push({ start, label: requestDayLabel(glance.opened_at, now), glances: [glance] });
  }
  return days.map(({ label, glances: list }) => ({ label, glances: list }));
}

/** What the host's tabs carry plus what was paged in, once per id. */
function streamGlances() {
  const seen = new Set();
  return [...requestGlances(), ...rqs.older].filter((g) => !seen.has(g.id) && seen.add(g.id));
}

const streamReplies = (n) => (n > 0 ? `${n} ${n === 1 ? "reply" : "replies"}` : "No replies yet");
const streamTitle = (glance) => (glance.kind === "earlier" ? STREAM_EARLIER : glance.title);

function streamEntryHtml(glance, { tabViews, name, selected }) {
  const state = requestStateOf(glance, tabViews);
  const last = glance.kind === "earlier" ? "" : requestLastLine(glance, name);
  return `<div class="rqs-ask"><span class="text">${esc(streamTitle(glance))}</span><time>${streamTime(glance.opened_at)}</time></div>
    <button type="button" class="rqs-state" data-rqs-entry="${esc(glance.id)}" data-state="${state}" aria-expanded="${selected}"><span class="dot" aria-hidden="true"></span><span class="word">${REQUEST_STATE_WORD[state]}</span>${requestProjectChip(glance)}<span class="n">${streamReplies(glance.replies)}</span><span class="last">${esc(last)}</span>${icon("chevron-right")}</button>`;
}

const STREAM_WHO = { user: () => "You", orchestrator: (_item, name) => name, captain: (item) => (item.captain ? `Captain ${item.captain}` : "Captain") };

/** The side panel: the request's state, the card, its latest items and Open thread. */
function streamPanelHtml(glance, have, name, tabViews) {
  const state = requestStateOf(glance, tabViews);
  const items = (have?.view?.items ?? []).map((item) => {
    const who = STREAM_WHO[item.who]?.(item, name) ?? "";
    return `<div class="rqs-item"><div class="who"><span class="name" data-who="${esc(String(item.who))}">${esc(who)}</span><time>${streamTime(item.at)}</time></div><div class="text">${esc(item.text)}</div></div>`;
  });
  return `<aside class="rqs-panel" aria-label="Thread"><div class="rqs-phead"><span class="dot" data-state="${state}" aria-hidden="true"></span><h2>${esc(streamTitle(glance))}</h2><button type="button" data-rqs-panel-close aria-label="Close thread">${icon("x")}</button></div>
    <div class="rqs-pbody">${requestCardFrom(glance, have)}${items.join("")}</div>
    <div class="rqs-pfoot"><button type="button" data-rqs-open="${esc(glance.id)}">Open thread</button></div></aside>`;
}

function requestsStreamHtml({ glances, now, name, tabViews, selected, views, end }) {
  const days = requestDays(glances, now).map(
    (day) => `<section class="rqs-day"><div class="rqs-dayhead"><span>${esc(day.label)}</span></div>${day.glances.map((g) => `<div class="rqs-entry">${streamEntryHtml(g, { tabViews, name, selected: g.id === selected })}</div>`).join("")}</section>`,
  );
  const picked = glances.find((g) => g.id === selected);
  return `<div class="rqs-main"><header class="rqs-head"><h1>All activity</h1><span class="spacer"></span><button type="button" data-rqs-close>Requests</button></header>
    <div class="rqs-list" data-rqs-list>${days.join("") || `<p class="rqs-empty">Nothing here yet.</p>`}${end && days.length ? `<p class="rqs-end">That is everything.</p>` : ""}</div>
    <div class="rqs-foot"><span>Each request opens its own thread</span><button type="button" data-rqs-new>${icon("plus")}New request</button></div></div>${picked ? streamPanelHtml(picked, views.get(picked.id), name, tabViews) : ""}`;
}

const requestsStream = Object.assign(document.createElement("section"), { id: "all-activity", hidden: true });
requestsStream.setAttribute("aria-label", "All activity");
document.getElementById("chat-head").after(requestsStream);

function closeAllActivity() {
  rqs.open = false;
  renderStream();
}

function renderStream() {
  if (rqs.open && (!requestsOn() || (active?.key ?? null) !== rqs.key || narrow(768) || shellPage())) rqs.open = false;
  requestsStream.hidden = !rqs.open;
  if (!rqs.open) {
    if (requestsStream.innerHTML) requestsStream.innerHTML = "";
    rqs.drawn = "";
    return;
  }
  const html = requestsStreamHtml({ glances: streamGlances(), now: Date.now(), name: globalOrchestratorName(), tabViews: requestTabViews(), selected: rqs.selected, views: rqs.views, end: rqs.end });
  if (html !== rqs.drawn) {
    const top = requestsStream.querySelector("[data-rqs-list]")?.scrollTop ?? 0;
    requestsStream.innerHTML = rqs.drawn = html;
    const list = requestsStream.querySelector("[data-rqs-list]");
    if (list) list.scrollTop = top;
  }
  const list = requestsStream.querySelector("[data-rqs-list]");
  if (list && list.scrollHeight <= list.clientHeight) streamOlder();
}

/** Opens All activity over the chat. An older host that answered `user_requests` with an error has none. */
function openAllActivity() {
  if (!requestsOn() || rq.old?.host === requestHost()) return;
  Object.assign(rqs, { open: true, key: active?.key ?? null, older: [], end: false, selected: null });
  renderStream();
}

/** The next page of older requests: those opened before the oldest one listed. An empty page, or an error, ends the list. */
async function streamOlder() {
  const all = streamGlances();
  if (rqs.paging || rqs.end || !all.length) return;
  const before = Math.min(...all.map((g) => g.opened_at));
  rqs.paging = true;
  try {
    const page = await callOn(requestHost(), { cmd: "user_requests", before });
    const known = new Set(all.map((g) => g.id));
    const fresh = Array.isArray(page) ? page.filter((g) => !known.has(g.id)) : [];
    if (fresh.length) rqs.older.push(...fresh);
    else rqs.end = true;
  } catch {
    rqs.end = true;
  } finally {
    rqs.paging = false;
  }
  renderStream();
}

/** Selects an entry and reads its `user_request` for the panel. A failed read leaves the card out. */
async function streamSelect(id) {
  rqs.selected = id;
  renderStream();
  if (rqs.loading.has(id)) return;
  rqs.loading.add(id);
  let view = null;
  try {
    view = await callOn(requestHost(), { cmd: "user_request", id, items: STREAM_ITEMS });
  } catch {
    // The card stays away rather than showing an error.
  } finally {
    rqs.loading.delete(id);
  }
  rqs.views.set(id, { view, at: Date.now() });
  renderStream();
}

async function streamOpenThread(id) {
  await openRequest(id);
  closeAllActivity();
}

requestsStream.addEventListener("click", (e) => {
  if (e.target.closest("[data-rqs-panel-close]")) {
    rqs.selected = null;
    return renderStream();
  }
  if (e.target.closest("[data-rqs-close]")) return closeAllActivity();
  if (e.target.closest("[data-rqs-new]")) {
    closeAllActivity();
    return openNewRequest();
  }
  const open = e.target.closest("[data-rqs-open]");
  if (open) return streamOpenThread(open.dataset.rqsOpen);
  const entry = e.target.closest("[data-rqs-entry]");
  if (entry) streamSelect(entry.dataset.rqsEntry);
});
requestsStream.addEventListener(
  "scroll",
  (e) => {
    const list = e.target;
    if (list.closest?.("[data-rqs-list]") && list.scrollTop + list.clientHeight >= list.scrollHeight - STREAM_END_PX) streamOlder();
  },
  true,
);

onModelChange(renderStream);
