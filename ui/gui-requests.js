// The Requests screen of the global Orchestrator on desktop: the column of requests beside the chat,
// a request thread's header and "Where this stands" card, and the composer footer. What it draws is
// derived by requests-model.js from the `RequestGlance` list the host sends; nothing here decides a
// request's state. Loaded after gui-orchestrator.js, which draws the chat these sit around.
//
// The glances ride on the host's anchor tab (`TabView.requests`), else on its newest request tab, so
// every tab of the host is scanned. With none carrying them the screen asks `user_requests`; an
// error from that call means an older host, and the chat stays as it was, with no banner.

const REQUEST_FILTERS = [["open", "Open"], ["needs", "Needs you"], ["done", "Done"]];
const REQUEST_STATE_WORD = { needs: "Needs you", working: "Working", done: "Done" };
/** The column's width; the Captains rail reads it to leave the lane room. */
const REQUESTS_WIDTH = 340;
/** How long a read of `user_requests` stands, and how long an old host is left alone after it errored. */
const REQUESTS_FRESH_MS = 25 * 1000;
const REQUESTS_RETRY_MS = 5 * 60 * 1000;
/** How long the answer to Mark done or Reopen stands in for the next tabs push, and how long a failed card read waits. */
const REQUEST_PATCH_MS = 15 * 1000;
const REQUEST_VIEW_RETRY_MS = 30 * 1000;
const REQUEST_SEEN_KEY = "pinned.requests.seen";

/** The replies seen on this device, by request id. Storage can be missing or blocked. */
function readRequestsSeen() {
  try {
    const seen = JSON.parse(localStorage.getItem(REQUEST_SEEN_KEY) ?? "{}");
    return seen && typeof seen === "object" ? seen : {};
  } catch {
    return {};
  }
}

const rq = {
  filter: "open",
  /** The answer of `user_requests`, and the host and time it came from, read while no tab carries the glances. */
  fetched: null,
  host: null,
  fetchedAt: 0,
  asking: false,
  /** Set when the host answered `user_requests` with an error: an older host. */
  old: null,
  /** Request id -> { sig, view, at }: the `user_request` reply behind the card. */
  views: new Map(),
  loading: new Set(),
  /** Request id -> { glance, until }: an action's answer, shown until the next tabs push agrees. */
  patch: new Map(),
  seen: readRequestsSeen(),
  /** The key of the request tab open last, where the Orchestrator row lands. */
  lastKey: null,
  drawn: "",
};

const requestHost = () => globalHostMachine();

/** The glances the host's tabs carry, or null while none does. */
function carriedGlances() {
  const host = requestHost();
  const carrier = tabs.find((t) => t.machine === host && Array.isArray(t.view.requests));
  return carrier ? carrier.view.requests : null;
}
const rawGlances = () => carriedGlances() ?? (rq.host === requestHost() ? rq.fetched : null);

/** Whether the Requests screens are in use: the host's glances are known. Otherwise today's single chat shows. */
const requestsOn = () => rawGlances() !== null;

/** `glance` with the answer of an action on it still standing, until a tabs push agrees. */
function patchedGlance(glance) {
  const patch = rq.patch.get(glance.id);
  if (!patch) return glance;
  if (patch.until < Date.now() || (glance.closed_at != null) === (patch.glance.closed_at != null)) {
    rq.patch.delete(glance.id);
    return glance;
  }
  return patch.glance;
}
const requestGlances = () => (rawGlances() ?? []).map(patchedGlance);

/** The tabs' views by machine, as requestPlans reads them. */
function requestTabViews() {
  const by = {};
  for (const tab of tabs) (by[tab.machine] ??= []).push(tab.view);
  return by;
}

/** What requestState reads besides the glance: the request's plans, and whether its own tab waits on input. */
function requestCtx(glance, tabViews) {
  return {
    planGlances: requestPlans(glance, tabViews).map((entry) => entry.plan),
    tabWaits: Object.values(tabViews).flat().some((v) => v.request === glance.id && v.status === "needs-input" && v.running === true && v.interrupted === true),
  };
}
const requestStateOf = (glance, tabViews = requestTabViews()) => requestState(glance, requestCtx(glance, tabViews));

/** The glance of the request a tab is, or null for any other tab. */
function requestOf(tab) {
  if (!tab?.view?.request) return null;
  const found = (typeof threadRequestGlance === "function" ? threadRequestGlance(tab) : null) ?? (rawGlances() ?? []).find((g) => g.id === tab.view.request);
  return found ? patchedGlance(found) : null;
}
const requestHere = (tab) => (requestsOn() ? requestOf(tab) : null);

/** The number on the Orchestrator row: requests that need you, or null while the screens are off. */
function requestsBadge() {
  if (!requestsOn()) return null;
  const views = requestTabViews();
  return requestGlances().filter((g) => requestStateOf(g, views) === "needs").length;
}

/** Asks the host for its requests while no tab carries them. An error marks an older host. */
async function refreshRequests() {
  const host = requestHost();
  if (!host || rq.asking || carriedGlances() || machines.get(host)?.view?.online !== true) return;
  if (rq.old?.host === host && Date.now() - rq.old.at < REQUESTS_RETRY_MS) return;
  if (rq.host === host && rq.fetched && Date.now() - rq.fetchedAt < REQUESTS_FRESH_MS) return;
  rq.asking = true;
  try {
    const list = await callOn(host, { cmd: "user_requests" });
    if (!Array.isArray(list)) throw new Error("not a list of requests");
    Object.assign(rq, { fetched: list, host, fetchedAt: Date.now(), old: null });
  } catch {
    Object.assign(rq, { fetched: null, host, old: { host, at: Date.now() } });
  } finally {
    rq.asking = false;
  }
  renderRequests();
}

const requestLastWho = (glance, name) =>
  ({ user: "You", orchestrator: name, captain: glance.captains.length === 1 ? `Captain ${glance.captains[0].project}` : "Captain" })[glance.last_who] ?? "";
const requestLastLine = (glance, name) => (glance.last_text ? `${requestLastWho(glance, name)}: ${glance.last_text}` : "");
const requestWhen = (glance) => ago((glance.last_at ?? glance.opened_at) / 1000);
const requestProjectChip = (glance) => (glance.project ? `<span class="rq-project">#${esc(tagSlug(glance.project))}</span>` : "");

function requestRowHtml(glance, { state, steps, unread, current, name }) {
  return `<button type="button" class="rq-row" data-request="${esc(glance.id)}" data-state="${state}" aria-current="${current}"><span class="dot" aria-hidden="true"></span><span class="body">
    <span class="top"><span class="title">${esc(glance.title)}</span><span class="when">${esc(requestWhen(glance))}</span></span>
    <span class="last">${esc(requestLastLine(glance, name))}</span>
    <span class="meta">${requestProjectChip(glance)}<span class="state">${REQUEST_STATE_WORD[state]}</span>${steps.total ? `<span class="steps">${steps.done} of ${steps.total} steps</span>` : ""}<span class="spacer"></span>${unread ? `<span class="unread">${unread}</span>` : ""}</span></span></button>`;
}

/** The column: the heading with All activity and New request, the filters with their counts, and a row per request. */
function requestsColumnHtml({ glances, filter, seen, current, name, tabViews }) {
  const ctx = (glance) => requestCtx(glance, tabViews);
  const counts = requestCounts(glances, ctx);
  const filters = REQUEST_FILTERS.map(([id, label]) => `<button type="button" class="rq-filter" data-rq-filter="${id}" aria-pressed="${filter === id}">${label} <span class="n">${counts[id]}</span></button>`).join("");
  const rows = requestFilter(glances, filter, ctx).map((glance) =>
    requestRowHtml(glance, { state: requestState(glance, ctx(glance)), steps: requestProgress(requestPlans(glance, tabViews)), unread: requestUnread(glance, seen[glance.id]), current: glance.id === current, name }),
  );
  return `<div class="rq-head"><h1>Requests</h1><span class="spacer"></span><button type="button" class="rq-all" data-rq-all>All activity</button><button type="button" class="rq-new" data-rq-new>${icon("plus")}New request</button></div>
    <div class="rq-filters" role="group" aria-label="Filter requests">${filters}</div>
    <div class="rq-list">${rows.join("") || `<p class="rq-empty">Nothing here. A request you send shows up as a thread.</p>`}</div>`;
}

/** A request thread's header, or the New request screen's while it is open: its state dot, title, project and state, with Mark done or Reopen. Empty in any other tab. */
function requestCrumbHtml(tab) {
  const screen = typeof newRequestCrumbHtml === "function" ? newRequestCrumbHtml() : "";
  if (screen) return screen;
  const glance = requestHere(tab);
  if (!glance) return "";
  const state = requestStateOf(glance);
  const [action, label] = state === "done" ? ["reopen", "Reopen"] : ["done", "Mark done"];
  return `<button type="button" class="request-back" data-rq-back>Requests</button><span class="sep">/</span><span class="request-dot" data-state="${state}" aria-hidden="true"></span><h2 class="request-title">${esc(glance.title)}</h2><span class="request-state" data-state="${state}">${REQUEST_STATE_WORD[state]}</span>${requestProjectChip(glance)}<span class="orchestrator-actions"><button type="button" data-request-action="${action}" data-request-id="${esc(glance.id)}">${label}</button></span>`;
}

/** What `renderThread` redraws the card for: the glance, the card's reply and the state they give. */
const requestSig = (glance) => JSON.stringify([glance.title, glance.project, glance.closed_at, glance.needs, glance.needs_at, glance.last_user_at, glance.last_at, glance.replies]);
function requestCardKey(tab) {
  const glance = requestHere(tab);
  if (!glance) return null;
  const have = rq.views.get(glance.id);
  return [requestSig(glance), requestStateOf(glance), have?.sig ?? null, !!have?.view];
}

/** "Where this stands": what was asked, what is done and left, what needs the user, and the links. The
 * Needs you row always draws. Every field is text. */
function requestCardHtml(tab) {
  const glance = requestHere(tab);
  return glance ? requestCardFrom(glance, rq.views.get(glance.id)) : "";
}
/** The card of `glance` from `have`, a `{ view }` read of its `user_request` or undefined while it loads. */
function requestCardFrom(glance, have) {
  if (have && !have.view) return "";
  const stands = have?.view?.stands ?? {};
  const waiting = requestStateOf(glance) === "needs";
  const needs = waiting ? stands.needs || glance.needs || "Waiting on you." : "Nothing right now.";
  const row = (label, text, attr = "") => `<span class="rc-k">${label}</span><span class="rc-v"${attr}>${esc(text || "—")}</span>`;
  const links = (stands.links ?? []).map((link) => `<span class="rc-link">${esc(link)}</span>`).join("");
  const body = have
    ? `<div class="rc-grid">${row("You asked", stands.asked)}${row("Done", stands.done)}${row("Left", stands.left)}${row("Needs you", needs, ` data-needs="${waiting}"`)}</div>${links ? `<div class="rc-links">${links}</div>` : ""}`
    : `<p class="rc-loading" role="status">Loading…</p>`;
  return `<section class="request-card" aria-label="Where this request stands"><div class="rc-head"><span class="rc-title">Where this stands</span></div>${body}</section>`;
}

/** The composer footer in a request thread: the reply stays here, and a new subject is a new request. */
function requestFooterHtml(tab) {
  const glance = requestHere(tab);
  if (!glance) return "";
  return `<span class="request-foot">${requestProjectChip(glance)}<span>stays in this thread</span><button type="button" class="new-request" data-new-request>Another subject? New request</button></span>`;
}

/** Reads the card's `user_request` once per change of the request, and redraws the thread when it arrives. */
async function ensureRequestView(tab) {
  const glance = requestHere(tab);
  if (!glance || rq.loading.has(glance.id)) return;
  const sig = requestSig(glance);
  const have = rq.views.get(glance.id);
  if (have?.sig === sig && (have.view || Date.now() - have.at < REQUEST_VIEW_RETRY_MS)) return;
  rq.loading.add(glance.id);
  let view = null;
  try {
    view = await callOn(tab.machine, { cmd: "user_request", id: glance.id });
  } catch {
    // The card stays away rather than showing an error; it is read again after a pause.
  } finally {
    rq.loading.delete(glance.id);
  }
  rq.views.set(glance.id, { sig, view, at: Date.now() });
  if (active === tab) renderRequests();
}

function markRequestSeen(glance) {
  if ((rq.seen[glance.id] ?? 0) >= glance.replies) return;
  rq.seen[glance.id] = glance.replies;
  try {
    localStorage.setItem(REQUEST_SEEN_KEY, JSON.stringify(rq.seen));
  } catch {
    // Unread counts then last until the page closes.
  }
}

/** The board (ThreadPlan) gives the Plan panel the room the Requests column takes: a request tab with its Plan open has no column. */
function requestPlanOpen(tab) {
  if (!tab?.view?.request || typeof panelOf !== "function") return false;
  return !!panelOf(tab)?.open && shownSurface(tab)?.kind === "plan";
}

const requestsColumn = Object.assign(document.createElement("aside"), { id: "requests-column", hidden: true });
requestsColumn.setAttribute("aria-label", "Requests");
document.getElementById("chat-head").after(requestsColumn);

/** Draws the column beside a global tab on a wide screen, and redraws the thread's header and card when
 * what they read changed. */
function renderRequests() {
  refreshRequests();
  const chat = document.getElementById("chat");
  const inThread = requestsOn() && guiOn() && !!active && isGlobalOrchestratorTab(active) && !shellPage();
  const on = inThread && !narrow(768) && !requestPlanOpen(active);
  const shown = chat.hasAttribute("data-requests");
  chat.toggleAttribute("data-requests", on);
  requestsColumn.hidden = !on;
  if (shown !== on && typeof renderRail === "function") renderRail();
  if (!on && requestsColumn.innerHTML) requestsColumn.innerHTML = "";
  if (!inThread) return;
  // The card reads its request whether or not the column is beside it: with the Plan open the column is gone, the card stays.
  const glance = requestOf(active);
  if (glance) {
    markRequestSeen(glance);
    rq.lastKey = active.key;
    ensureRequestView(active);
  }
  if (on) {
    const html = requestsColumnHtml({ glances: requestGlances(), filter: rq.filter, seen: rq.seen, current: glance?.id ?? null, name: globalOrchestratorName(), tabViews: requestTabViews() });
    if (requestsColumn.innerHTML !== html) requestsColumn.innerHTML = html;
  }
  const drawn = JSON.stringify([active.key, requestCardKey(active)]);
  if (drawn === rq.drawn) return;
  rq.drawn = drawn;
  if (glance) renderThread();
}

/** Selects a request's own tab, asking the host to open one for a request that rests with none. */
async function openRequest(id) {
  const glance = requestGlances().find((g) => g.id === id);
  if (!glance) return;
  const host = requestHost();
  const own = glance.tab && tabs.find((t) => t.machine === host && t.view.id === glance.tab);
  if (own) return activate(own);
  try {
    const opened = await callOn(host, { cmd: "user_request_action", id, action: "open" });
    if (opened?.tab) await revealTab(host, opened.tab);
  } catch (e) {
    tell(`Couldn't open the request: ${esc(String(e?.message ?? e))}`, 6000);
  }
}

/** Mark done and Reopen. The host's answer stands in for the glance until the next tabs push brings it. */
async function requestAction(id, action) {
  try {
    const glance = await callOn(requestHost(), { cmd: "user_request_action", id, action });
    if (glance?.id === id) rq.patch.set(id, { glance, until: Date.now() + REQUEST_PATCH_MS });
    renderRequests();
  } catch (e) {
    tell(`Couldn't ${action === "reopen" ? "reopen" : "mark done"} the request: ${esc(String(e?.message ?? e))}`, 6000);
  }
}

/** Where the Orchestrator row lands on `host`: the request tab open last, else the first request that
 * needs you, else the anchor. With the screens off, the first global tab, as before. */
function requestLandingTab(host) {
  const globals = tabs.filter((t) => t.machine === host && isGlobalOrchestratorTab(t));
  const anchor = globals.find((t) => !t.view.request) ?? globals[0] ?? null;
  if (!requestsOn()) return anchor;
  const last = globals.find((t) => t.key === rq.lastKey);
  if (last) return last;
  const views = requestTabViews();
  const needing = requestFilter(requestGlances(), "needs", (g) => requestCtx(g, views)).map((g) => globals.find((t) => t.view.request === g.id)).find(Boolean);
  return needing ?? anchor;
}

// Unit 10 fills in New request and unit 11 All activity; until they load these say so.
if (typeof openNewRequest === "undefined") globalThis.openNewRequest = () => tell("Starting a request from here isn't ready yet.", 3000);
if (typeof openAllActivity === "undefined") globalThis.openAllActivity = () => tell("All activity isn't ready yet.", 3000);

requestsColumn.addEventListener("click", (e) => {
  const filter = e.target.closest("[data-rq-filter]");
  if (filter) {
    if (REQUEST_FILTERS.some(([id]) => id === filter.dataset.rqFilter)) rq.filter = filter.dataset.rqFilter;
    return renderRequests();
  }
  if (e.target.closest("[data-rq-new]")) return openNewRequest();
  if (e.target.closest("[data-rq-all]")) return openAllActivity();
  const row = e.target.closest("[data-request]");
  if (row) openRequest(row.dataset.request);
});
document.querySelector("#chat-head .crumb").addEventListener("click", (e) => {
  if (e.target.closest("[data-rq-back]")) {
    if (requestPlanOpen(active) && typeof closePanel === "function") closePanel();
    return;
  }
  const button = e.target.closest("[data-request-action]");
  if (button && ["done", "reopen"].includes(button.dataset.requestAction)) requestAction(button.dataset.requestId, button.dataset.requestAction);
});
document.getElementById("composer-footer").addEventListener("click", (e) => {
  if (e.target.closest("[data-new-request]")) openNewRequest();
});

onModelChange(renderRequests);
visibleInterval(refreshRequests, 30 * 1000);
