// The New request screen of the global Orchestrator on desktop. The user types a message; the app splits
// it into one card per #project paragraph (`splitRequest`), previews where each goes from what the
// Captains rail already knows (`captainFacts`), and offers a close open request to reply in instead
// (`closeTo`). Send is one `new_user_requests` call. No model is called and nothing is asked for the preview.
//
// An older host answers `new_user_requests` with an error: then the whole text, unchanged, goes to
// today's single Orchestrator tab as a normal prompt, with no banner, and the draft stays until that
// send succeeds. Each item keeps its idempotency key with the draft, so a retry reuses the keys.
// Loaded after gui-requests.js, which installs a stub `openNewRequest` for this file to replace.

const NR_TITLE_MAX = REQUEST_TITLE_MAX;

const nr = {
  open: false,
  text: "",
  /** Item index -> the title the user renamed it to. */
  titles: {},
  /** Item index -> the id of the close match the user chose to reply in. */
  joins: new Map(),
  /** Item index -> its idempotency key. */
  keys: [],
  renaming: null,
  sending: false,
  /** The key of the tab open when the screen opened: choosing another tab closes the screen. */
  from: null,
  box: Object.assign(document.createElement("textarea"), { className: "nr-text", rows: 4, placeholder: "Describe it. Add #project and @machine to route a paragraph." }),
};

const newRequestKey = () => (typeof crypto?.randomUUID === "function" ? crypto.randomUUID() : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`);
/** The key of item `i`, made once and kept until the draft is sent or cleared. */
const newRequestKeyFor = (i) => (nr.keys[i] ??= newRequestKey());

/** The draft's items: the split of the text, with the titles the user renamed. */
function newRequestItems() {
  return splitRequest(nr.text, composerTagCatalog()).map((item, i) => ({ ...item, title: nr.titles[i] ?? item.title }));
}

/** The open request each item looks like, or null. */
function newRequestMatches(items) {
  const open = requestGlances().filter((g) => g.closed_at == null);
  return items.map((item) => closeTo(item.text, open));
}

/** The close match the user chose for each item, or null. */
const newRequestJoined = (matches) => matches.map((match, i) => (match && nr.joins.get(i) === match.id ? match : null));

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
function newRequestSendLabel() {
  const items = newRequestItems();
  const fresh = items.length - newRequestJoined(newRequestMatches(items)).filter(Boolean).length;
  return fresh === 0 && items.length ? "Send" : fresh >= 2 ? `Open ${fresh} threads` : "Open thread";
}

/** One card per item. It shows only with 2 or more items or a close match: one plain item sends straight through. */
function newRequestCardsHtml() {
  const items = newRequestItems();
  const matches = newRequestMatches(items);
  if (items.length < 2 && !matches.some(Boolean)) return "";
  const joined = newRequestJoined(matches);
  const fresh = items.length - joined.filter(Boolean).length;
  const cards = items.map((item, i) => {
    const title =
      nr.renaming === i
        ? `<input class="nr-title-input" type="text" data-nr-title="${i}" value="${esc(item.title)}" maxlength="${NR_TITLE_MAX}" aria-label="Thread title">`
        : `<span class="nr-title">${esc(item.title)}</span>`;
    const chip = item.project ? `<span class="nr-project">#${esc(tagSlug(item.project))}</span>` : "";
    const route = joined[i] ? "Goes into that thread as a reply" : newRequestPreview(item);
    const match = matches[i]
      ? `<span class="nr-close">Looks like <b>${esc(matches[i].title)}</b>, add it there? <button type="button" class="nr-join" data-nr-join="${i}" aria-pressed="${!!joined[i]}">${joined[i] ? "Adding there" : "Add it there"}</button></span>`
      : "";
    return `<div class="nr-card" data-nr-card="${i}" data-joined="${!!joined[i]}"><span class="dot" aria-hidden="true"></span><div class="nr-body"><span class="nr-top">${title}${chip}</span><span class="nr-route">${esc(route)}</span>${match}</div><button type="button" class="nr-rename" data-nr-rename="${i}">Rename</button></div>`;
  });
  const head = fresh ? `This opens ${plural(fresh, "thread")}` : "Nothing new opens";
  return `<section class="nr-threads" aria-label="Threads this request opens"><div class="nr-threads-head"><span class="nr-count">${head}</span>${fresh ? `<span class="nr-new">New</span>` : ""}<span class="spacer"></span><span class="nr-note">one per paragraph, each with its own replies</span></div>${cards.join("")}</section>`;
}

const nrElement = (tag, className, html = "") => Object.assign(document.createElement(tag), { className, innerHTML: html });
const newRequestPanel = Object.assign(document.createElement("section"), { id: "new-request", hidden: true });
newRequestPanel.setAttribute("aria-label", "New request");
const nrCards = nrElement("div", "nr-cards");
const nrFoot = nrElement("div", "nr-foot");
{
  const box = nrElement("div", "nr-box");
  box.append(nr.box, nrFoot);
  const main = nrElement("div", "nr-main", `<h2>What do you need?</h2>`);
  main.append(box, nrCards);
  newRequestPanel.append(nrElement("div", "nr-head", `<button type="button" class="nr-back" data-nr-close>Requests</button><span class="nr-sep">/</span><h1>New request</h1>`), main);
  document.getElementById("chat").append(newRequestPanel);
}

/** Draws the screen, or takes it down: it closes when another tab is chosen. */
function renderNewRequest() {
  if (nr.open && nr.from !== (active?.key ?? null)) nr.open = false;
  newRequestPanel.hidden = !nr.open;
  document.getElementById("chat").toggleAttribute("data-new-request", nr.open);
  if (!nr.open) return;
  nrCards.innerHTML = newRequestCardsHtml();
  nrFoot.innerHTML = `<span class="nr-hint">#project and @machine, as today</span><span class="spacer"></span><button type="button" class="nr-send" data-nr-send${nr.text.trim() && !nr.sending ? "" : " disabled"}>${esc(newRequestSendLabel())}</button>`;
  if (nr.renaming !== null) nrCards.querySelector("[data-nr-title]")?.focus();
}

function setNewRequestText(text) {
  nr.text = String(text ?? "");
  if (nr.box.value !== nr.text) nr.box.value = nr.text;
  if (!nr.text.trim()) clearNewRequestDraft();
  renderNewRequest();
}

function clearNewRequestDraft() {
  Object.assign(nr, { text: "", titles: {}, keys: [], renaming: null });
  nr.joins.clear();
  nr.box.value = "";
}

function closeNewRequest() {
  nr.open = false;
  nr.renaming = null;
  renderNewRequest();
}

function openNewRequest() {
  nr.open = true;
  nr.from = active?.key ?? null;
  nr.box.value = nr.text;
  renderNewRequest();
  nr.box.focus();
}

/** One item as `new_user_requests` takes it. */
function newRequestWire(item, i) {
  const wire = { key: newRequestKeyFor(i), title: item.title || "New request", text: item.text };
  if (item.project) wire.project = item.project;
  if (item.machine) wire.machine = item.machine;
  return wire;
}

const newRequestError = (e) => esc(String(e?.message ?? e));

/** An older host: the whole text, as typed, goes to today's single Orchestrator tab. The draft stays when that fails too. */
async function sendToSingleChat(host) {
  try {
    const anchor = tabs.find((t) => t.machine === host && isGlobalOrchestratorTab(t) && !t.view.request) ?? (await revealTab(host, await callOn(host, { cmd: "open_global_orchestrator" })));
    await callOn(host, { cmd: "prompt", tab: anchor.view.id, text: nr.text });
    clearNewRequestDraft();
    closeNewRequest();
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

/** Sends the draft: one `new_user_requests` call for the new threads, and a Prompt in the matching
 * request's tab for each paragraph the user chose to add there. Returns whether it went through. */
async function sendNewRequest() {
  if (nr.sending || !nr.text.trim()) return false;
  nr.sending = true;
  renderNewRequest();
  try {
    const host = requestHost();
    const items = newRequestItems();
    const joined = newRequestJoined(newRequestMatches(items));
    const fresh = items.map((item, i) => ({ item, i })).filter(({ i }) => !joined[i]);
    let made = [];
    if (fresh.length) {
      try {
        made = await callOn(host, { cmd: "new_user_requests", items: fresh.map(({ item, i }) => newRequestWire(item, i)) });
        if (!Array.isArray(made)) throw new Error("not a list of requests");
      } catch {
        return await sendToSingleChat(host);
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
    clearNewRequestDraft();
    closeNewRequest();
    try {
      if (made[0]?.tab) await revealTab(host, made[0].tab);
      else if (land?.tab) activate(land.tab);
      else if (land) await revealTab(host, land.id);
    } catch {
      // The threads exist; the column shows them even when the first one could not be opened.
    }
    return true;
  } finally {
    nr.sending = false;
    renderNewRequest();
  }
}

nr.box.addEventListener("input", () => setNewRequestText(nr.box.value));

newRequestPanel.addEventListener("click", (e) => {
  if (e.target.closest("[data-nr-close]")) return closeNewRequest();
  const rename = e.target.closest("[data-nr-rename]");
  if (rename) {
    nr.renaming = Number(rename.dataset.nrRename);
    return renderNewRequest();
  }
  const join = e.target.closest("[data-nr-join]");
  if (join) {
    const i = Number(join.dataset.nrJoin);
    const match = newRequestMatches(newRequestItems())[i];
    if (!match) return;
    if (nr.joins.get(i) === match.id) nr.joins.delete(i);
    else nr.joins.set(i, match.id);
    return renderNewRequest();
  }
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
