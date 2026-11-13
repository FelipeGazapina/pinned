/** The Ideas page (0.18.2): things the user might do, kept on the hub. The page, the sidebar's "N open"
 * and More's hint all read `ideasUi.rows`, which is `list_ideas`. The hub's settings push carries no
 * ideas, so it stays small however many there are; each push only tells this script to read again. */

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
  saving: null, // "saving" | "saved" | an error, for the open editor
  adding: false,
  loading: false,
  again: false,
  timer: 0,
  refreshTimer: 0,
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

/** A settings push means something may have changed, an agent saving an idea among it. */
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

function ideaProjectOptions(current) {
  const repos = new Set([...(typeof projectRepos !== "undefined" ? projectRepos.values() : []), ...(ideasUi.rows ?? []).map((i) => i.project), current].filter(Boolean));
  return [...repos].sort().map((r) => `<option value="${esc(r)}"${r === current ? " selected" : ""}>${esc(ideaProjectName(r))}</option>`).join("");
}

function ideaEditorEl(idea) {
  const el = document.createElement("article");
  el.className = "idea-card idea-editor";
  el.dataset.id = idea.id;
  el.innerHTML = `<input type="text" class="idea-edit-title" name="title" value="${esc(idea.title)}" aria-label="Title" autocomplete="off">
    <textarea class="idea-edit-text" name="text" rows="6" aria-label="Text" placeholder="Details, links, why it matters…">${esc(idea.text)}</textarea>
    <div class="idea-edit-foot"><select name="project" aria-label="Project"><option value="">No project</option>${ideaProjectOptions(idea.project)}</select>
      <span class="idea-saving" role="status"></span><button type="button" class="idea-close" data-idea="close">Done</button></div>`;
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
  drawIdeas();
  ideasLoad();
}

/** Draws the filters and the cards. The card being edited stays the element it is, so typing in it survives. */
function drawIdeas() {
  const list = pageView.querySelector(".ideas-list");
  if (!list) return;
  const project = pageView.querySelector('.ideas-filters [name="project"]');
  const options = ideasProjectFilterHtml();
  if (project.innerHTML !== options) project.innerHTML = options;
  const filters = pageView.querySelector(".ideas-filters");
  for (const [name, value] of [["project", ideasUi.project], ["status", ideasUi.status], ["query", ideasUi.query]]) if (filters.elements[name].value !== value && filters.elements[name] !== document.activeElement) filters.elements[name].value = value;
  const add = pageView.querySelector(".ideas-add");
  add.hidden = !ideasHub();
  const shown = ideasShown();
  const cards = shown.map((idea) => {
    if (idea.id === ideasUi.editing && ideasUi.editor) return ideasUi.editor;
    const holder = document.createElement("template");
    holder.innerHTML = ideaCardHtml(idea);
    return holder.content.firstElementChild;
  });
  list.replaceChildren(...cards);
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
  if (saving) saving.textContent = ideasUi.saving === "saving" ? "Saving…" : ideasUi.saving === "saved" ? "Saved" : (ideasUi.saving ?? "");
}

function ideaPut(idea) {
  const rows = ideasUi.rows ?? [];
  const at = rows.findIndex((i) => i.id === idea.id);
  ideasUi.rows = at < 0 ? [idea, ...rows] : rows.map((i) => (i.id === idea.id ? idea : i));
}

/** An edit as the hub takes it: it sets the title, text and project as sent, and the status when sent. */
const ideaEdit = (idea, changes = {}) => ({ id: idea.id, title: idea.title, text: idea.text, ...(idea.project ? { project: idea.project } : {}), ...changes });

async function ideaSetStatus(idea, status) {
  try {
    ideaPut(await ideasCall("save_idea", ideaEdit(idea, { status })));
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
    if (ideasUi.editing === idea.id) ideasUi.editing = ideasUi.editor = null;
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

/** Opens `idea` in the editor in place of its card. */
function ideaOpenEditor(idea) {
  if (ideasUi.editing && ideasUi.editing !== idea.id) ideaCloseEditor();
  ideasUi.editing = idea.id;
  ideasUi.editor = ideaEditorEl(idea);
  ideasUi.saving = null;
  drawIdeas();
  ideasUi.editor.querySelector(".idea-edit-text").focus();
}

/** Saves the editor's fields as typed. An empty title is not an idea, so it waits. */
async function ideaSaveDraft() {
  clearTimeout(ideasUi.timer);
  const el = ideasUi.editor;
  const idea = ideasUi.rows?.find((i) => i.id === ideasUi.editing);
  if (!el || !idea) return;
  const title = el.querySelector('[name="title"]').value.trim();
  const text = el.querySelector('[name="text"]').value;
  const project = el.querySelector('[name="project"]').value || null;
  if (!title) {
    ideasUi.saving = "A title is needed";
    return drawIdeas();
  }
  if (title === idea.title && text.trim() === idea.text && project === (idea.project ?? null)) return;
  ideasUi.saving = "saving";
  drawIdeas();
  try {
    const saved = await ideasCall("save_idea", { id: idea.id, title, text, ...(project ? { project } : {}) });
    ideaPut(saved);
    ideasUi.saving = ideasUi.editor === el && el.querySelector('[name="title"]').value.trim() === title && el.querySelector('[name="text"]').value === text ? "saved" : null;
  } catch (e) {
    ideasUi.saving = String(e?.message ?? e);
  }
  if (ideasUi.editor === el) drawIdeas();
  modelChanged();
}

async function ideaCloseEditor() {
  await ideaSaveDraft();
  ideasUi.editing = ideasUi.editor = null;
  ideasUi.saving = null;
  if (ideasUi.rows) ideasUi.rows = [...ideasUi.rows].sort((a, b) => b.updated_at - a.updated_at);
  drawIdeas();
}

function ideaMenu(idea, at) {
  openSideMenu([
    { label: "Edit", icon: "pencil", run: () => ideaOpenEditor(idea) },
    ...(idea.status === "done" ? [] : [{ label: "Mark done", icon: "check", run: () => ideaSetStatus(idea, "done") }]),
    ...(idea.status === "dropped" ? [] : [{ label: "Drop", icon: "x", run: () => ideaSetStatus(idea, "dropped") }]),
    ...(idea.status === "open" ? [] : [{ label: "Reopen", icon: "rotate-ccw", run: () => ideaSetStatus(idea, "open") }]),
    { label: "Delete", icon: "trash", danger: true, sep: true, run: () => ideaDelete(idea) },
  ], at);
}

pageView.addEventListener("click", (e) => {
  if (shellPage() !== "ideas") return;
  const button = e.target.closest("[data-idea]");
  if (!button) return;
  const idea = ideasUi.rows?.find((i) => i.id === button.closest("[data-id]")?.dataset.id);
  if (button.dataset.idea === "close") return ideaCloseEditor();
  if (!idea) return;
  if (button.dataset.idea === "edit") ideaOpenEditor(idea);
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
