// The New request screen of the global Orchestrator on desktop. The user types a message; the app splits
// it into one card per #project paragraph (`splitRequest`), previews where each goes from what the
// Captains rail already knows (`captainFacts`), and offers a close open request to reply in instead
// (`closeTo`). Send is one `new_user_requests` call. No model is called and nothing is asked for the preview.
//
// Only a host that says it does not know `new_user_requests` is an older host: then the whole text,
// unchanged, goes to today's single Orchestrator tab as a normal prompt, with no banner, and the draft
// stays until that send succeeds. Any other error (a timeout, a closed socket) keeps the draft and
// shows the error, because the host may already have made the threads; Send then retries with the same
// idempotency keys, which the host de-duplicates.
//
// A draft is `{ text, titles, joins, keys, renaming, sending, redraw }`. This screen has one (`nr`) and
// All activity's footer has another; every function below takes the draft it works on, `nr` by default,
// so both send through the one path. Loaded after gui-requests.js, which installs a stub
// `openNewRequest` for this file to replace.

const NR_TITLE_MAX = REQUEST_TITLE_MAX;

/** An empty draft. `redraw` shows the draft's screen again. */
function newRequestDraft(redraw) {
  return {
    text: "",
    /** Item index -> the title the user renamed it to. */
    titles: {},
    /** Item index -> the id of the close match the user chose to reply in. */
    joins: new Map(),
    /** Item index -> its idempotency key. */
    keys: [],
    renaming: null,
    sending: false,
    redraw,
  };
}

const nr = Object.assign(newRequestDraft(() => renderNewRequest()), {
  open: false,
  /** The key of the tab open when the screen opened: choosing another tab closes the screen. */
  from: null,
  box: Object.assign(document.createElement("textarea"), { className: "nr-text", rows: 4, placeholder: "Describe it. Add #project and @machine to route a paragraph." }),
  onClear: () => {
    nr.box.value = "";
    renderNewRequestMirror();
  },
});

const newRequestKey = () => (typeof crypto?.randomUUID === "function" ? crypto.randomUUID() : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`);
/** The key of item `i`, made once and kept until the draft is sent or cleared. */
const newRequestKeyFor = (i, draft = nr) => (draft.keys[i] ??= newRequestKey());

/** The draft's items: the split of the text, with the titles the user renamed. */
function newRequestItems(draft = nr) {
  return splitRequest(draft.text, composerTagCatalog()).map((item, i) => ({ ...item, title: draft.titles[i] ?? item.title }));
}

/** The open request each item looks like, or null. */
function newRequestMatches(items) {
  const open = requestGlances().filter((g) => g.closed_at == null);
  return items.map((item) => closeTo(item.text, open));
}

/** The close match the user chose for each item, or null. */
const newRequestJoined = (matches, draft = nr) => matches.map((match, i) => (match && draft.joins.get(i) === match.id ? match : null));

/** Chooses the close match of item `i` to reply in, or lets it go. */
function toggleNewRequestJoin(draft, i) {
  const match = newRequestMatches(newRequestItems(draft))[i];
  if (!match) return;
  if (draft.joins.get(i) === match.id) draft.joins.delete(i);
  else draft.joins.set(i, match.id);
  draft.redraw();
}

/** Where an item goes, from the Captain facts the rail reads: the Captain, its machine and whether it is
 * working, free, missing or offline. A working Captain under the Queue policy leaves the message waiting. */
function newRequestPreview(item) {
  const catalog = composerTagCatalog();
  const first = composerParagraphs(item.text)[0]?.text ?? item.text;
  const route = routeParagraph(first, catalog);
  const upper = (text) => text.charAt(0).toUpperCase() + text.slice(1);
  if (route.kind !== "routed") return upper(route.text);
  const fact = captainFacts().find((f) => f.slug === tagSlug(route.project));
  if (!fact || fact.machine.name !== route.machine) return route.text;
  const goes = `Goes to Captain ${route.project} on ${route.machine}`;
  if (fact.state === "offline") return `${goes}, which is offline, so it waits until it is back`;
  if (fact.state === "none") return `${goes}. It has no Captain yet, so one starts.`;
  if (fact.state === "working") return `${goes}, working now, ${globalSettings.busy === "queue" ? "so it waits in the queue" : "and may be turned away until it is free"}`;
  return `${goes}, free now`;
}

const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** The Send button's words: how many threads it opens, or Send when every item is a reply. */
function newRequestSendLabel(draft = nr) {
  const items = newRequestItems(draft);
  const fresh = items.length - newRequestJoined(newRequestMatches(items), draft).filter(Boolean).length;
  return fresh === 0 && items.length ? "Send" : fresh >= 2 ? `Open ${fresh} threads` : "Open thread";
}

const NR_NOTE_ARROW = `<svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 12h12M12 6l6 6-6 6"></path></svg>`;

/** One card per item, then a dashed row under them for each item that looks like an open request. It
 * shows only with 2 or more items or a close match: one plain item sends straight through. `rename: false`
 * leaves out the Rename buttons, as in All activity's footer. */
function newRequestCardsHtml(draft = nr, { rename = true } = {}) {
  const items = newRequestItems(draft);
  const matches = newRequestMatches(items);
  if (items.length < 2 && !matches.some(Boolean)) return "";
  const joined = newRequestJoined(matches, draft);
  const fresh = items.length - joined.filter(Boolean).length;
  const cards = items.map((item, i) => {
    const title =
      draft.renaming === i
        ? `<input class="nr-title-input" type="text" data-nr-title="${i}" value="${esc(item.title)}" maxlength="${NR_TITLE_MAX}" aria-label="Thread title">`
        : `<span class="nr-title">${esc(item.title)}</span>`;
    const route = joined[i] ? "Goes into that thread as a reply" : newRequestPreview(item);
    return `<div class="nr-card" data-nr-card="${i}" data-joined="${!!joined[i]}"><span class="dot" aria-hidden="true"></span><div class="nr-body"><span class="nr-top">${title}</span><span class="nr-route">${esc(route)}</span></div>${rename ? `<button type="button" class="nr-rename" data-nr-rename="${i}">Rename</button>` : ""}</div>`;
  });
  const head = fresh ? `This opens ${plural(fresh, "thread")}` : "Nothing new opens";
  const threads = `<section class="nr-threads" aria-label="Threads this request opens"><div class="nr-threads-head"><span class="nr-count">${head}</span>${fresh ? `<span class="nr-new">New</span>` : ""}<span class="spacer"></span><span class="nr-note">one per paragraph, each with its own replies</span></div>${cards.join("")}<div class="nr-report">${NR_NOTE_ARROW}<span>A Captain's report lands in the thread of the request it answers, not in one long chat.</span></div></section>`;
  const close = matches
    .map((match, i) =>
      match
        ? `<section class="nr-close-row" aria-label="Open threads on the same subject"><span class="dot" aria-hidden="true"></span><span class="nr-close-text">Looks close to an open thread: <b>${esc(match.title)}</b></span><button type="button" class="nr-join" data-nr-join="${i}" aria-pressed="${!!joined[i]}">${joined[i] ? "Replying there. Undo" : "Reply there instead"}</button></section>`
        : "",
    )
    .join("");
  return threads + close;
}

const nrElement = (tag, className, html = "") => Object.assign(document.createElement(tag), { className, innerHTML: html });
const newRequestPanel = Object.assign(document.createElement("section"), { id: "new-request", hidden: true });
newRequestPanel.setAttribute("aria-label", "New request");
const nrCards = nrElement("div", "nr-cards");
const nrFoot = nrElement("div", "nr-foot");
/** The tags in the text box are painted by a copy of the text under the textarea, as the main composer does: `tagMirrorHtml`. */
const nrMirror = Object.assign(document.createElement("div"), { className: "nr-mirror", ariaHidden: "true" });
const nrEdit = nrElement("div", "nr-edit");
{
  nrEdit.append(nrMirror, nr.box);
  const box = nrElement("div", "nr-box");
  box.append(nrEdit, nrFoot);
  const main = nrElement("div", "nr-main", `<h2>What do you need?</h2>`);
  main.append(box, nrCards);
  newRequestPanel.append(main);
  document.getElementById("chat").append(newRequestPanel);
}

function renderNewRequestMirror() {
  const html = hasTags(nr.text) ? tagMirrorHtml(nr.text, composerTagCatalog(), esc) : "";
  nrMirror.innerHTML = html;
  nrEdit.dataset.tags = String(!!html);
  nrMirror.scrollTop = nr.box.scrollTop ?? 0;
}

/** The chat head's crumb while this screen is open: Requests, a slash and the title. The head's other
 * controls and the sidebar toggle stay. */
function newRequestCrumbHtml() {
  if (!nr.open || nr.from !== (active?.key ?? null)) return "";
  return `<button type="button" class="nr-back" data-nr-close>Requests</button><span class="nr-sep">/</span><h2 class="nr-crumb-title">New request</h2>`;
}
const redrawHead = () => typeof renderThread === "function" && renderThread();

/** Draws the screen, or takes it down: it closes when another tab is chosen. */
function renderNewRequest() {
  if (nr.open && nr.from !== (active?.key ?? null)) nr.open = false;
  newRequestPanel.hidden = !nr.open;
  document.getElementById("chat").toggleAttribute("data-new-request", nr.open);
  if (!nr.open) return;
  renderNewRequestMirror();
  nrCards.innerHTML = newRequestCardsHtml();
  nrFoot.innerHTML = `<span class="nr-hint">#project and @machine, as today</span><span class="spacer"></span><button type="button" class="nr-send" data-nr-send${nr.text.trim() && !nr.sending ? "" : " disabled"}>${esc(newRequestSendLabel())}</button>`;
  if (nr.renaming !== null) nrCards.querySelector("[data-nr-title]")?.focus();
}

function setNewRequestText(text) {
  nr.text = String(text ?? "");
  if (nr.box.value !== nr.text) nr.box.value = nr.text;
  if (!nr.text.trim()) clearNewRequestDraft();
  renderNewRequestMirror();
  renderNewRequest();
}

function clearDraft(draft) {
  Object.assign(draft, { text: "", titles: {}, keys: [], renaming: null });
  draft.joins.clear();
  draft.onClear?.();
}
const clearNewRequestDraft = () => clearDraft(nr);

function closeNewRequest() {
  const was = nr.open;
  nr.open = false;
  nr.renaming = null;
  renderNewRequest();
  if (was) redrawHead();
}

function openNewRequest() {
  nr.open = true;
  nr.from = active?.key ?? null;
  nr.box.value = nr.text;
  renderNewRequest();
  redrawHead();
  nr.box.focus();
}

/** One item as `new_user_requests` takes it. */
function newRequestWire(item, i, draft = nr) {
  const wire = { key: newRequestKeyFor(i, draft), title: item.title || "New request", text: item.text };
  if (item.project) wire.project = item.project;
  if (item.machine) wire.machine = item.machine;
  return wire;
}

const newRequestError = (e) => esc(String(e?.message ?? e));

/** Whether `e` says the host does not know the command: serde's "unknown variant `new_user_requests`", or
 * a hub relaying it. Any other error says nothing about what the host did. */
const newRequestHostIsOld = (e) => /\bunknown (variant|command|cmd)\b/i.test(String(e?.message ?? e));

/** An older host: the whole text, as typed, goes to today's single Orchestrator tab. The draft stays when that fails too. */
async function sendToSingleChat(host, draft = nr) {
  try {
    const anchor = tabs.find((t) => t.machine === host && isGlobalOrchestratorTab(t) && !t.view.request) ?? (await revealTab(host, await callOn(host, { cmd: "open_global_orchestrator" })));
    await callOn(host, { cmd: "prompt", tab: anchor.view.id, text: draft.text });
    clearDraft(draft);
    if (draft === nr) closeNewRequest();
    activate(anchor);
    return true;
  } catch (e) {
    tell(`Couldn't send the message: ${newRequestError(e)}`, 6000);
    return false;
  }
}

/** The tab to prompt for a request chosen as a close match, asking the host to open one for a request that rests with none. */
async function newRequestTabOf(host, glance) {
  const own = glance.tab && tabs.find((t) => t.machine === host && t.view.id === glance.tab);
  if (own) return { id: own.view.id, tab: own };
  const opened = await callOn(host, { cmd: "user_request_action", id: glance.id, action: "open" });
  if (!opened?.tab) throw new Error("the request has no thread to reply in");
  return { id: opened.tab, tab: null };
}

/** Sends `draft`: one `new_user_requests` call for the new threads, and a Prompt in the matching
 * request's tab for each paragraph the user chose to add there. Returns whether it went through. */
async function sendNewRequest(draft = nr) {
  if (draft.sending || !draft.text.trim()) return false;
  draft.sending = true;
  draft.redraw();
  try {
    const host = requestHost();
    const items = newRequestItems(draft);
    const joined = newRequestJoined(newRequestMatches(items), draft);
    const fresh = items.map((item, i) => ({ item, i })).filter(({ i }) => !joined[i]);
    let made = [];
    if (fresh.length) {
      try {
        made = await callOn(host, { cmd: "new_user_requests", items: fresh.map(({ item, i }) => newRequestWire(item, i, draft)) });
        if (!Array.isArray(made)) throw new Error("not a list of requests");
      } catch (e) {
        if (newRequestHostIsOld(e)) return await sendToSingleChat(host, draft);
        tell(`Couldn't open the request: ${newRequestError(e)}`, 6000);
        return false;
      }
    }
    let land = null;
    try {
      for (const [i, glance] of joined.entries()) {
        if (!glance) continue;
        const target = await newRequestTabOf(host, glance);
        await callOn(host, { cmd: "prompt", tab: target.id, text: items[i].text });
        land ??= target;
      }
    } catch (e) {
      tell(`Couldn't send to the request: ${newRequestError(e)}`, 6000);
      return false;
    }
    clearDraft(draft);
    if (draft === nr) closeNewRequest();
    try {
      if (made[0]?.tab) await revealTab(host, made[0].tab);
      else if (land?.tab) activate(land.tab);
      else if (land) await revealTab(host, land.id);
    } catch {
      // The threads exist; the column shows them even when the first one could not be opened.
    }
    return true;
  } finally {
    draft.sending = false;
    draft.redraw();
  }
}

nr.box.addEventListener("input", () => setNewRequestText(nr.box.value));
nr.box.addEventListener("scroll", () => {
  nrMirror.scrollTop = nr.box.scrollTop;
});
document.querySelector("#chat-head .crumb").addEventListener("click", (e) => {
  if (e.target.closest("[data-nr-close]")) closeNewRequest();
});

newRequestPanel.addEventListener("click", (e) => {
  if (e.target.closest("[data-nr-close]")) return closeNewRequest();
  const rename = e.target.closest("[data-nr-rename]");
  if (rename) {
    nr.renaming = Number(rename.dataset.nrRename);
    return renderNewRequest();
  }
  const join = e.target.closest("[data-nr-join]");
  if (join) return toggleNewRequestJoin(nr, Number(join.dataset.nrJoin));
  if (e.target.closest("[data-nr-send]")) sendNewRequest();
});

newRequestPanel.addEventListener("focusout", (e) => {
  const input = e.target.closest("[data-nr-title]");
  const i = input ? Number(input.dataset.nrTitle) : null;
  if (i === null || nr.renaming !== i) return;
  const title = String(input.value).replace(/\s+/g, " ").trim().slice(0, NR_TITLE_MAX);
  if (title) nr.titles[i] = title;
  else delete nr.titles[i];
  nr.renaming = null;
  renderNewRequest();
});

newRequestPanel.addEventListener("keydown", (e) => {
  if (e.target.closest?.("[data-nr-title]")) {
    if (e.key === "Enter") {
      e.preventDefault();
      e.target.blur();
    } else if (e.key === "Escape") {
      e.stopPropagation();
      nr.renaming = null;
      renderNewRequest();
    }
    return;
  }
  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
    e.preventDefault();
    sendNewRequest();
  }
});

onModelChange(renderNewRequest);
