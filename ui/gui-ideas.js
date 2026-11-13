/** The Ideas page (0.18.2): things the user might do, kept on the hub. The page, the sidebar's "N open"
 * and More's hint all read `ideasUi.rows`, which is `list_ideas`. The hub's settings push carries no
 * ideas, so it stays small however many there are. The hub sends `ideas_changed` after an idea is saved
 * or deleted, and the app reads again then and once after it connects; a settings push reads nothing. */

const IDEA_STATUSES = [["open", "Open"], ["done", "Done"], ["dropped", "Dropped"], ["all", "All"]];
const IDEA_INTRO = "Things you might do. Agents read them only when asked; facts you want agents to always know go in memory instead.";
const IDEA_BY = { global_orchestrator: "the Orchestrator", captain: "a Captain", worker: "a Worker", taste_chat: "the way-of-working chat", workflow: "a workflow", session: "a session" };
const IDEA_SAVE_AFTER_MS = 700;
const IDEA_REFRESH_AFTER_MS = 300;

const ideasUi = {
  rows: null, // the account's ideas, most recently changed first; null until the hub answers
  error: null,
  status: "open",
  project: "",
  query: "",
  editing: null, // the id of the idea open in the editor
  editor: null, // the editor's card
  base: null, // the title and text the editor showed when it opened, or as last saved from it: what "typed" is measured against
  saving: null, // "saving" | "saved" | `{ error }`, for the open editor
  queue: Promise.resolve(true), // the editor's saves, one after another
  failed: null, // what the last failed save sent, so a blur does not send it again
  opening: 0,
  adding: false,
  loading: false,
  again: false,
  timer: 0,
  refreshTimer: 0,
  filterHtml: null, // the project filter's options as last written
  cards: new Map(), // id -> { html, node }: a card's element is kept while what it shows is the same
};

const ideasHub = () => typeof hub !== "undefined" && !!hub;
const ideaOpenCount = () => (ideasUi.rows ? ideasUi.rows.filter((i) => i.status === "open").length : 0);
/** The sidebar's and More's line: "12 open", or nothing while there are none or the hub has not answered. */
const ideasOpenText = () => (ideaOpenCount() ? `${ideaOpenCount()} open` : "");
const ideaProjectName = (repo) => (repo ? repo.split("/").filter(Boolean).pop() : "—");
const ideaWho = (idea) => (idea.created_by?.kind === "agent" ? `by ${IDEA_BY[idea.created_by.role] ?? "an agent"}` : "");
const ideaFold = (text) => String(text ?? "").normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();

/** The ideas the filters let through, in the order they are kept. */
function ideasShown() {
  const { status, project, query } = ideasUi;
  const words = ideaFold(query).split(/\s+/).filter(Boolean);
  return (ideasUi.rows ?? []).filter((i) => {
    if (i.id === ideasUi.editing) return true; // an idea being edited does not vanish under the filters
    if (status !== "all" && i.status !== status) return false;
    if (project && i.project !== project) return false;
    const hay = ideaFold(`${i.title} ${i.text} ${ideaProjectName(i.project)}`);
    return words.every((w) => hay.includes(w));
  });
}

const ideasCall = (cmd, args = {}) => connectorCall(cmd, args);

async function ideasLoad() {
  if (!ideasHub()) return;
  if (ideasUi.loading) {
    ideasUi.again = true;
    return;
  }
  ideasUi.loading = true;
  try {
    ideasUi.rows = await ideasCall("list_ideas");
    ideasUi.error = null;
  } catch (e) {
    ideasUi.error = String(e?.message ?? e);
  }
  ideasUi.loading = false;
  if (ideasUi.again) {
    ideasUi.again = false;
    return ideasLoad();
  }
  ideasChanged();
}

/** The hub says ideas changed (`ideas_changed`): read again after a moment, so a burst of signals reads once. */
function ideasRefresh() {
  clearTimeout(ideasUi.refreshTimer);
  ideasUi.refreshTimer = setTimeout(ideasLoad, IDEA_REFRESH_AFTER_MS);
}

/** Repaints the page, the sidebar's row and More's hint. */
function ideasChanged() {
  if (shellPage() === "ideas") drawIdeas();
  modelChanged();
}

function ideaCardHtml(idea) {
  const by = ideaWho(idea);
  const meta = [ideaProjectName(idea.project), ago(idea.created_at), by].filter(Boolean).join(" · ");
  const state = idea.status === "open" ? "" : `<span class="idea-state">${idea.status === "done" ? "Done" : "Dropped"}</span>`;
  const text = idea.text.trim().split("\n")[0];
  return `<article class="idea-card" data-id="${esc(idea.id)}" data-status="${idea.status}">
    <button type="button" class="idea-open" data-idea="edit"><span class="idea-head"><span class="idea-title">${esc(idea.title)}</span>${state}<span class="idea-meta">${esc(meta)}</span></span>${text ? `<span class="idea-text">${esc(text)}</span>` : ""}</button>
    <button type="button" class="icon idea-more" data-idea="menu" aria-label="${esc(`Actions for ${idea.title}`)}" aria-haspopup="menu">${icon("ellipsis")}</button></article>`;
}

function ideaEditorEl(idea) {
  const el = document.createElement("article");
  el.className = "idea-card idea-editor";
  el.dataset.id = idea.id;
  el.innerHTML = `<input type="text" class="idea-edit-title" name="title" value="${esc(idea.title)}" aria-label="Title" autocomplete="off">
    <textarea class="idea-edit-text" name="text" rows="6" aria-label="Text" placeholder="Details, links, why it matters…">${esc(idea.text)}</textarea>
    <div class="idea-edit-foot"><span class="idea-saving" role="status"></span><button type="button" class="idea-close" data-idea="close">Done</button></div>`;
  return el;
}

function ideasProjectFilterHtml() {
  const repos = [...new Set((ideasUi.rows ?? []).map((i) => i.project).filter(Boolean))].sort();
  if (ideasUi.project && !repos.includes(ideasUi.project)) repos.push(ideasUi.project);
  return `<option value="">All projects</option>${repos.map((r) => `<option value="${esc(r)}"${r === ideasUi.project ? " selected" : ""}>${esc(ideaProjectName(r))}</option>`).join("")}`;
}

function showIdeas() {
  setPage(`${headHtml("Ideas")}<div class="page-body"><div class="ideas-wrap">
    <form class="ideas-filters" role="search" onsubmit="return false">
      <select name="project" aria-label="Project"></select>
      <select name="status" aria-label="Status">${IDEA_STATUSES.map(([v, l]) => `<option value="${v}"${v === ideasUi.status ? " selected" : ""}>${l}</option>`).join("")}</select>
      <input type="search" name="query" placeholder="Search ideas" aria-label="Search ideas" autocomplete="off" value="${esc(ideasUi.query)}">
    </form>
    <p class="ideas-intro">${esc(IDEA_INTRO)}</p>
    <form class="ideas-add" autocomplete="off"><input type="text" name="title" placeholder="Quick idea…" aria-label="Quick idea" enterkeyhint="done"><button class="ideas-save">Save</button></form>
    <p class="ideas-error" role="alert" hidden></p>
    <section class="ideas-list" aria-label="Ideas"></section>
    <p class="ideas-empty" role="status" hidden></p>
  </div></div>`);
  ideasUi.editor = null;
  ideasUi.editing = null;
  ideasUi.base = null;
  ideasUi.filterHtml = null;
  ideasUi.cards = new Map();
  drawIdeas();
  ideasLoad();
}

/** Draws the filters and the cards. The card being edited stays the element it is, so typing in it survives. */
function drawIdeas() {
  const list = pageView.querySelector(".ideas-list");
  if (!list) return;
  if (ideasUi.editing && !ideasUi.rows?.some((i) => i.id === ideasUi.editing)) ideasUi.editing = ideasUi.editor = ideasUi.base = null; // deleted elsewhere
  const project = pageView.querySelector('.ideas-filters [name="project"]');
  const options = ideasProjectFilterHtml();
  // The options are kept as written, not read back: the browser serializes `selected` as `selected=""`.
  if (ideasUi.filterHtml !== options) project.innerHTML = ideasUi.filterHtml = options;
  const filters = pageView.querySelector(".ideas-filters");
  for (const [name, value] of [["project", ideasUi.project], ["status", ideasUi.status], ["query", ideasUi.query]]) if (filters.elements[name].value !== value && filters.elements[name] !== document.activeElement) filters.elements[name].value = value;
  const add = pageView.querySelector(".ideas-add");
  add.hidden = !ideasHub();
  const shown = ideasShown();
  const kept = new Map();
  const cards = shown.map((idea) => {
    if (idea.id === ideasUi.editing && ideasUi.editor) return ideasUi.editor;
    // A card whose content did not change stays the element it is: a click that began on it before this
    // draw, such as the one on a second idea that blurred the editor and so saved it, must still land.
    const html = ideaCardHtml(idea);
    const hit = ideasUi.cards.get(idea.id);
    if (hit?.html === html) {
      kept.set(idea.id, hit);
      return hit.node;
    }
    const holder = document.createElement("template");
    holder.innerHTML = html;
    const node = holder.content.firstElementChild;
    kept.set(idea.id, { html, node });
    return node;
  });
  ideasUi.cards = kept;
  placeCards(list, cards);
  const error = pageView.querySelector(".ideas-error");
  error.hidden = !ideasUi.error;
  error.textContent = ideasUi.error ?? "";
  const empty = pageView.querySelector(".ideas-empty");
  const none = !ideasHub() ? "Ideas are kept on your pinned hub. Sign in to the hub to use them."
    : ideasUi.rows === null ? (ideasUi.error ? "" : "Loading…")
    : shown.length ? ""
    : ideasUi.rows.length ? "No ideas match."
    : "No ideas yet. Type one above, or tell the Orchestrator “note this idea: …”.";
  empty.hidden = !none;
  empty.textContent = none;
  const saving = pageView.querySelector(".idea-saving");
  if (saving) {
    const state = ideasUi.saving;
    saving.textContent = state === "saving" ? "Saving…" : state === "saved" ? "Saved" : (state?.error ?? "");
    saving.dataset.state = state?.error ? "error" : (state ?? "");
  }
}

/** Puts `cards` in the list, touching only the elements that are out of place. The open editor is never
 * taken out and put back, which would drop its focus and caret: the other cards go around it. */
function placeCards(list, cards) {
  for (const child of [...list.children]) if (!cards.includes(child)) child.remove();
  const lay = (items, ref) => {
    for (const card of items) {
      if (card === ref) ref = ref.nextSibling;
      else list.insertBefore(card, ref);
    }
  };
  const editor = ideasUi.editor;
  const at = cards.indexOf(editor);
  if (at < 0 || editor.parentNode !== list) return lay(cards, list.firstChild);
  lay(cards.slice(0, at), list.firstChild);
  lay(cards.slice(at + 1), editor.nextSibling);
}

function ideaPut(idea) {
  const rows = ideasUi.rows ?? [];
  const at = rows.findIndex((i) => i.id === idea.id);
  ideasUi.rows = at < 0 ? [idea, ...rows] : rows.map((i) => (i.id === idea.id ? idea : i));
}

/** An edit as the hub takes it: it sets the title, text and project as sent, and the status when sent. */
const ideaEdit = (idea, changes = {}) => ({ id: idea.id, title: idea.title, text: idea.text, ...(idea.project ? { project: idea.project } : {}), ...changes });
const ideaRow = (id) => ideasUi.rows?.find((i) => i.id === id);

/** Sets the status of the idea as it is now, not as it was when its menu opened: an agent may have edited it since. */
async function ideaSetStatus(id, status) {
  const idea = ideaRow(id);
  if (!idea) return;
  try {
    ideaPut(await ideasCall("save_idea", ideaEdit(ideaRow(id) ?? idea, { status })));
    ideasUi.error = null;
  } catch (e) {
    ideasUi.error = String(e?.message ?? e);
  }
  ideasChanged();
}

async function ideaDelete(idea) {
  if (!(await confirmAction(`Delete “${esc(idea.title)}”?`, "The idea is removed for good.", "Delete", true))) return;
  try {
    await ideasCall("delete_idea", { id: idea.id });
    ideasUi.rows = ideasUi.rows.filter((i) => i.id !== idea.id);
    if (ideasUi.editing === idea.id) ideasUi.editing = ideasUi.editor = ideasUi.base = null;
    ideasUi.error = null;
  } catch (e) {
    ideasUi.error = String(e?.message ?? e);
  }
  ideasChanged();
}

async function ideaAdd(title) {
  title = title.trim();
  if (!title || ideasUi.adding) return false;
  ideasUi.adding = true;
  let saved = false;
  try {
    const idea = await ideasCall("save_idea", { title });
    ideaPut(idea);
    // A new idea is open and has no project: show it, whatever the filters were.
    if (ideasUi.status !== "all") ideasUi.status = "open";
    ideasUi.project = "";
    ideasUi.query = "";
    ideasUi.error = null;
    saved = true;
  } catch (e) {
    ideasUi.error = String(e?.message ?? e);
  }
  ideasUi.adding = false;
  ideasChanged();
  return saved;
}

/** Opens the idea in the editor in place of its card. The idea being edited, if any, is saved and closed
 * first; if that save fails it stays open, with its error, and this one does not open. */
async function ideaOpenEditor(id) {
  if (ideasUi.editing === id) return;
  const turn = ++ideasUi.opening;
  if (ideasUi.editing && !(await ideaCloseEditor())) return;
  const idea = ideaRow(id);
  if (!idea || turn !== ideasUi.opening) return;
  ideasUi.editing = id;
  ideasUi.editor = ideaEditorEl(idea);
  ideasUi.base = { title: idea.title, text: idea.text };
  ideasUi.saving = null;
  ideasUi.failed = null;
  drawIdeas();
  ideasUi.editor.querySelector(".idea-edit-text").focus();
}

/** What the editor holds that the user changed since it opened (or since its last save), as the hub would keep it. */
function ideaTyped(el) {
  const title = el.querySelector('[name="title"]').value.trim();
  const text = el.querySelector('[name="text"]').value;
  const { base } = ideasUi;
  return { title, text, titleChanged: title !== base.title, textChanged: text.trim() !== base.text };
}

/** Saves what the user typed in the editor, one save at a time: a change, the blur that follows it and
 * Done each ask, and the first sends it. Resolves to false when the draft is not saved. */
function ideaSaveDraft(retry = false) {
  clearTimeout(ideasUi.timer);
  return (ideasUi.queue = ideasUi.queue.then(() => ideaSaveNow(retry)));
}

async function ideaSaveNow(retry) {
  const el = ideasUi.editor;
  const id = ideasUi.editing;
  if (!el || !ideaRow(id)) return true;
  const typed = ideaTyped(el);
  if (!typed.titleChanged && !typed.textChanged) {
    if (ideasUi.saving?.error) ideasUi.saving = null;
    return true;
  }
  if (!typed.title) {
    ideasUi.saving = { error: "A title is needed" };
    drawIdeas();
    return false;
  }
  const sig = JSON.stringify([typed.title, typed.text]);
  if (!retry && ideasUi.failed === sig) return false; // a blur does not send again what just failed
  // The field the user did not change keeps what the hub holds now, which an agent may have edited.
  const row = ideaRow(id);
  ideasUi.saving = "saving";
  drawIdeas();
  let ok = false;
  try {
    ideaPut(await ideasCall("save_idea", ideaEdit(row, { ...(typed.titleChanged ? { title: typed.title } : {}), ...(typed.textChanged ? { text: typed.text } : {}) })));
    if (ideasUi.editor === el) {
      if (typed.titleChanged) ideasUi.base.title = typed.title;
      if (typed.textChanged) ideasUi.base.text = typed.text.trim();
      const now = ideaTyped(el);
      ideasUi.saving = now.titleChanged || now.textChanged ? null : "saved";
    }
    ideasUi.failed = null;
    ok = true;
  } catch (e) {
    ideasUi.failed = sig;
    if (ideasUi.editor === el) ideasUi.saving = { error: String(e?.message ?? e) };
  }
  if (ideasUi.editor === el) drawIdeas();
  modelChanged();
  return ok;
}

/** Closes the editor after saving it. A draft that cannot be saved keeps the editor open, with the error beside Done. */
async function ideaCloseEditor() {
  if (!ideasUi.editing) return true;
  if (!(await ideaSaveDraft(true))) return false;
  ideasUi.editing = ideasUi.editor = ideasUi.base = null;
  ideasUi.saving = null;
  if (ideasUi.rows) ideasUi.rows = [...ideasUi.rows].sort((a, b) => b.updated_at - a.updated_at);
  drawIdeas();
  return true;
}

function ideaMenu(idea, at) {
  openSideMenu([
    { label: "Edit", icon: "pencil", run: () => ideaOpenEditor(idea.id) },
    ...(idea.status === "done" ? [] : [{ label: "Mark done", icon: "check", run: () => ideaSetStatus(idea.id, "done") }]),
    ...(idea.status === "dropped" ? [] : [{ label: "Drop", icon: "x", run: () => ideaSetStatus(idea.id, "dropped") }]),
    ...(idea.status === "open" ? [] : [{ label: "Reopen", icon: "rotate-ccw", run: () => ideaSetStatus(idea.id, "open") }]),
    { label: "Delete", icon: "trash", danger: true, sep: true, run: () => ideaDelete(idea) },
  ], at);
}

// Done and Save keep the focus where it is. On a phone the tab bar comes back the moment a field loses it,
// and covers the button the finger is on, so the press never became a click.
pageView.addEventListener("mousedown", (e) => {
  if (shellPage() === "ideas" && e.target.closest?.(".idea-close, .ideas-save")) e.preventDefault();
});
pageView.addEventListener("click", (e) => {
  if (shellPage() !== "ideas") return;
  const button = e.target.closest("[data-idea]");
  if (!button) return;
  const idea = ideasUi.rows?.find((i) => i.id === button.closest("[data-id]")?.dataset.id);
  if (button.dataset.idea === "close") return ideaCloseEditor();
  if (!idea) return;
  if (button.dataset.idea === "edit") ideaOpenEditor(idea.id);
  else if (button.dataset.idea === "menu") ideaMenu(idea, button);
});
pageView.addEventListener("input", (e) => {
  if (shellPage() !== "ideas") return;
  if (e.target.closest(".idea-editor")) {
    clearTimeout(ideasUi.timer);
    ideasUi.timer = setTimeout(ideaSaveDraft, IDEA_SAVE_AFTER_MS);
  } else if (e.target.name === "query") {
    ideasUi.query = e.target.value;
    drawIdeas();
  }
});
pageView.addEventListener("change", (e) => {
  if (shellPage() !== "ideas") return;
  if (e.target.closest(".idea-editor")) return ideaSaveDraft();
  if (e.target.name === "status" || e.target.name === "project") {
    ideasUi[e.target.name] = e.target.value;
    drawIdeas();
  }
});
pageView.addEventListener("focusout", (e) => {
  // Leaving the editor for another part of the page saves what is typed at once.
  if (shellPage() === "ideas" && e.target.closest?.(".idea-editor") && !e.relatedTarget?.closest?.(".idea-editor")) ideaSaveDraft();
});
pageView.addEventListener("keydown", (e) => {
  if (shellPage() !== "ideas" || e.key !== "Escape" || !e.target.closest?.(".idea-editor")) return;
  e.preventDefault();
  e.stopPropagation();
  ideaCloseEditor();
});
pageView.addEventListener("submit", async (e) => {
  const form = e.target.closest(".ideas-add");
  if (!form || shellPage() !== "ideas") return;
  e.preventDefault();
  if (await ideaAdd(form.elements.title.value)) {
    const field = pageView.querySelector(".ideas-add [name='title']");
    field.value = "";
    field.focus();
  }
});

SHELL_PAGES.ideas = { show: showIdeas };
