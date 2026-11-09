/** The Way of working page: how the orchestrator, the Captains and each project's Captain split
 * work, pick models and ask for input. It edits a draft of the account's tastes and saves what
 * changed; the logic lives in taste.js. */

/** `saved` is the backend's last answer and `draft` the page's edits over it. `projects` holds
 * every online machine's project list, null until it arrives. `flash` holds the brief lines the
 * last edit changed, drawn highlighted by the next render only. `note` is the note being typed. */
const tasteUi = { saved: null, draft: null, scope: { kind: "captains" }, projects: null, error: null, saving: false, toast: false, flash: new Set(), note: "" };
let tasteToastTimer = 0;

const TASTE_FIELD_LABELS = {
  strategy: "Strategy",
  ladder: "Model ladder",
  hard_model: "Hard work model",
  parallel: "Parallel workers",
  autonomy: "Autonomy",
  review_diffs: "Review each diff",
  second_opinion: "Second opinion",
  context: "Context",
};

const tasteShown = () => tasteResolve(tasteUi.draft, tasteUi.scope);
const tasteDirty = () => tasteUi.saved != null && !tasteSame(tasteUi.saved, tasteUi.draft);
const tasteErrorText = (e) => (isOlderPinned(e) ? "Update pinned on the hub to change the way of working." : String(e?.message ?? e));

/** A model ref as one option or data value, and back: its id, and its provider on a second line. */
const tasteRefKey = (ref) => (ref.provider ? `${ref.model}\n${ref.provider}` : ref.model);
function tasteRefOf(key) {
  const [model, provider] = key.split("\n");
  return provider ? { provider, model } : { model };
}
const tasteRefLabel = (ref, names) => (ref.provider ? `${names.get(ref.model) ?? ref.model} through ${ref.provider}` : names.get(ref.model) ?? ref.model);

/** Reads the account's tastes. Unsaved edits are made again over them, so a save sends only those
 * and keeps what another device changed meanwhile. */
async function tasteFetch() {
  try {
    const got = await connectorCall("get_tastes");
    tasteUi.draft = tasteDirty() ? tasteRebase(tasteUi.saved, tasteUi.draft, got) : got;
    tasteUi.saved = got;
    tasteUi.error = null;
  } catch (e) {
    tasteUi.error = tasteErrorText(e);
  }
  tasteRender();
}

async function tasteFetchProjects() {
  const lists = await Promise.all(onlineMachines().map((m) => callOn(m, { cmd: "list_projects" }).catch(() => [])));
  tasteUi.projects = lists.flat();
  tasteRender();
}

/** Sends the draft's changes in order. Each answer becomes the saved tastes, so a failure part way
 * keeps what went through and the next save sends only the rest. */
async function tasteSave() {
  const sent = tasteUi.draft;
  const calls = tasteSaveCalls(tasteUi.saved, sent);
  if (!calls.length) return;
  tasteUi.saving = true;
  tasteUi.error = null;
  tasteRender();
  try {
    for (const { cmd, ...args } of calls) tasteUi.saved = await connectorCall(cmd, args);
  } catch (e) {
    tasteUi.error = tasteErrorText(e);
  }
  tasteUi.saving = false;
  if (!tasteUi.error) {
    if (tasteUi.draft === sent) tasteUi.draft = tasteUi.saved;
    tasteUi.toast = true;
    clearTimeout(tasteToastTimer);
    tasteToastTimer = setTimeout(() => {
      tasteUi.toast = false;
      pageView.querySelector(".taste-toast")?.remove();
    }, 2600);
  }
  tasteRender();
}

function tasteEdit(next) {
  const before = new Set(tasteBrief(tasteShown()).split("\n"));
  tasteUi.draft = next;
  tasteUi.flash = new Set(tasteBrief(tasteShown()).split("\n").filter((line) => !before.has(line)));
  tasteUi.toast = false;
  tasteRender();
}
const tasteEditField = (field, value) => tasteEdit(tasteSetField(tasteUi.draft, tasteUi.scope, field, value));

function tasteMoveRung(key, step) {
  const ladder = [...tasteShown().ladder];
  const i = ladder.findIndex((ref) => tasteRefKey(ref) === key);
  [ladder[i], ladder[i + step]] = [ladder[i + step], ladder[i]];
  tasteEditField("ladder", ladder);
}

/** On a project tab, whether the field is the project's own or the Captains', and the way back. */
function tasteTagHtml(field) {
  if (tasteUi.scope.kind !== "project") return "";
  if (tasteUi.draft.projects?.[tasteUi.scope.repo]?.[field] == null) return `<span class="taste-tag">inherited</span>`;
  const back = `Inherit ${TASTE_FIELD_LABELS[field].toLowerCase()} from Captains (all)`;
  return `<span class="taste-tag changed">changed here</span><button type="button" class="taste-reset" data-act="reset" data-field="${field}" aria-label="${back}" title="${back}">Reset</button>`;
}
const tasteLabelHtml = (field) => `<div class="taste-label"><span>${TASTE_FIELD_LABELS[field]}</span>${tasteTagHtml(field)}</div>`;

function tasteOptionsHtml(catalog, { selected = "", taken = new Set() } = {}) {
  return catalog
    .map((group) => `<optgroup label="${group.label}">${group.models
      .map((m) => {
        const key = tasteRefKey({ model: m.slug });
        return `<option value="${esc(key)}" title="${esc(m.slug)}"${key === selected ? " selected" : ""}${taken.has(m.slug) ? " disabled" : ""}>${esc(m.name)}</option>`;
      })
      .join("")}</optgroup>`)
    .join("");
}

function tasteModelsHtml(t, catalog, names) {
  const strategies = TASTE_STRATEGIES.map((s) => `<button type="button" class="taste-option" data-act="set" data-field="strategy" data-value="${s.id}" aria-pressed="${s.id === t.strategy}">
    <span class="dot"></span><span><b>${s.label}</b><span>${s.about}</span></span></button>`).join("");
  const rungs = t.ladder.map((ref, i) => {
    const key = esc(tasteRefKey(ref));
    const label = esc(tasteRefLabel(ref, names));
    return `<li><span class="pos">${i + 1}</span><span class="name" title="${esc(tasteModelName(ref))}">${label}</span>
      <button type="button" class="taste-icon" data-act="up" data-rung="${key}" aria-label="Move ${label} up"${i === 0 ? " disabled" : ""}>${icon("chevron-up")}</button>
      <button type="button" class="taste-icon" data-act="down" data-rung="${key}" aria-label="Move ${label} down"${i === t.ladder.length - 1 ? " disabled" : ""}>${icon("chevron-down")}</button>
      <button type="button" class="taste-icon" data-act="remove" data-rung="${key}" aria-label="Remove ${label} from the ladder"${t.ladder.length === 1 ? " disabled" : ""}>${icon("trash-2")}</button></li>`;
  }).join("");
  const taken = new Set(t.ladder.map((ref) => ref.model));
  const full = catalog.every((group) => group.models.every((m) => taken.has(m.slug)));
  const hard = t.hard_model;
  const hardKey = hard ? tasteRefKey(hard) : "";
  const listed = hard && !hard.provider && names.has(hard.model);
  const current = !hard ? `<option value="" selected disabled>The top rung of the ladder</option>` : listed ? "" : `<option value="${esc(hardKey)}" selected>${esc(tasteRefLabel(hard, names))}</option>`;
  return `<section class="taste-section"><h2>Models</h2>
    <div class="taste-field">${tasteLabelHtml("strategy")}<div class="taste-options" role="group" aria-label="Strategy">${strategies}</div></div>
    <div class="taste-field">${tasteLabelHtml("ladder")}<ol class="taste-ladder">${rungs}</ol>
      <div class="taste-inline"><label class="taste-label">Add model <select data-add-model><option value="" selected disabled>Choose</option>${tasteOptionsHtml(catalog, { taken })}</select></label>
      ${full ? `<span class="taste-quiet">Every model is already on the ladder.</span>` : ""}</div></div>
    <div class="taste-field taste-inline"><label class="taste-label">Hard work goes straight to <select data-hard-model>${current}${tasteOptionsHtml(catalog, { selected: hardKey })}</select></label>${tasteTagHtml("hard_model")}</div>
  </section>`;
}

function tastePaceHtml(t) {
  const autonomies = TASTE_AUTONOMIES.map((a) => `<button type="button" data-act="set" data-field="autonomy" data-value="${a.id}" aria-pressed="${a.id === t.autonomy}">${a.label}</button>`).join("");
  const toggle = (field, title, about) => `<div class="taste-switch-row">
    <button type="button" class="taste-switch" role="switch" aria-checked="${t[field]}" aria-label="${title}" data-act="toggle" data-field="${field}"><span></span></button>
    <span class="taste-switch-text"><b>${title}</b><span>${about}</span></span><span class="taste-tags">${tasteTagHtml(field)}</span></div>`;
  return `<section class="taste-section"><h2>Pace</h2>
    <div class="taste-field taste-inline"><div class="taste-label"><span>Parallel workers</span>${tasteTagHtml("parallel")}</div>
      <span class="taste-stepper"><button type="button" data-act="parallel" data-step="-1" aria-label="Fewer parallel workers"${t.parallel <= 1 ? " disabled" : ""}>${icon("minus")}</button>
      <output aria-live="polite">${t.parallel}</output>
      <button type="button" data-act="parallel" data-step="1" aria-label="More parallel workers"${t.parallel >= TASTE_MAX_PARALLEL ? " disabled" : ""}>${icon("plus")}</button></span></div>
    <div class="taste-field">${tasteLabelHtml("autonomy")}<div class="taste-seg" role="group" aria-label="Autonomy">${autonomies}</div></div>
    ${toggle("review_diffs", "Review each diff before accepting", "The Captain reads the worker's diff before merging it in.")}
    ${toggle("second_opinion", "Second opinion on risky changes", "Another model reviews the change before it goes ahead.")}
  </section>`;
}

function tasteContextHtml(t) {
  const notes = t.context.map((note, i) => `<li><span>${esc(note)}</span><button type="button" class="taste-icon" data-act="unnote" data-note="${i}" aria-label="Remove note">${icon("trash-2")}</button></li>`).join("");
  return `<section class="taste-section"><div class="taste-heading"><div class="taste-title"><h2>Context</h2>${tasteTagHtml("context")}</div><p>Notes every turn carries.</p></div>
    <ul class="taste-notes">${notes}</ul>
    <form class="taste-note-form" data-note-form><input type="text" data-note-input value="${esc(tasteUi.note)}" placeholder="E.g. heavy builds always run on the Mac mini" aria-label="New note">
      <button type="submit" class="primary">Add</button></form>
  </section>`;
}

function tastePreviewHtml(t) {
  const lines = tasteBrief(t).split("\n").map((line) => `<div class="taste-line${tasteUi.flash.has(line) ? " flash" : ""}">${esc(line)}</div>`).join("");
  const speed = tasteSpeed(t);
  const dots = [0, 1, 2].map((i) => `<span${i < speed ? ' class="on"' : ""}></span>`).join("");
  return `<aside class="taste-preview"><h2>${tasteUi.scope.kind === "orchestrator" ? "What the orchestrator receives" : "What the Captain receives"}</h2>
    <div class="taste-box"><div class="taste-lines">${lines}</div>
      <div class="taste-meters"><div><span>Cost per task</span><span title="A rough estimate from each model's cost tier">${tasteCost(t)}</span></div>
      <div><span>Speed</span><span class="taste-speed" role="img" aria-label="Speed ${speed} of 3">${dots}</span></div></div></div>
    <p class="taste-hint">Changes with the fields on the left.</p></aside>`;
}

function tasteFootHtml() {
  const dirty = tasteDirty();
  if (!dirty && !tasteUi.error) return "";
  const busy = tasteUi.saving ? " disabled" : "";
  return `<footer class="taste-foot">${dirty ? `<span class="taste-unsaved">Unsaved changes</span>` : ""}${tasteUi.error ? `<span class="taste-error" role="alert">${esc(tasteUi.error)}</span>` : ""}
    <span class="spacer"></span>${dirty ? `<button type="button" class="outline" data-act="discard"${busy}>Discard</button><button type="button" class="primary" data-act="save"${busy}>${tasteUi.saving ? "Saving…" : "Save"}</button>` : ""}</footer>`;
}

/** A control's identity across renders, so the one that had focus gets it back. */
const tasteFocusKey = (el) => (el && pageView.contains(el) ? `${el.tagName}${JSON.stringify(el.dataset)}` : null);

function tasteRender() {
  if (document.body.dataset.page !== "taste") return;
  const toast = tasteUi.toast ? `<span class="taste-toast" role="status">${icon("check")}Saved. New sessions start with it.</span>` : "";
  const head = headHtml("Way of working", `<span class="spacer"></span>${toast}`);
  const sub = `<p class="taste-sub">How the orchestrator and the Captains split work, pick models and ask for your input.</p>`;
  if (!tasteUi.draft) {
    setPage(`${head}<div class="page-body"><div class="page-wrap taste">${sub}<p class="taste-state${tasteUi.error ? " error" : ""}">${esc(tasteUi.error ?? "Loading…")}</p></div></div>`);
    return;
  }
  const scope = tasteUi.scope;
  const t = tasteShown();
  const catalog = tasteCatalog(CLAUDE_MODELS, OPENCODE_GO_MODELS);
  const names = new Map(catalog.flatMap((group) => group.models.map((m) => [m.slug, m.name])));
  const repos = [...Object.keys(tasteUi.draft.projects ?? {}), ...(scope.kind === "project" ? [scope.repo] : [])];
  const tabs = [
    { kind: "orchestrator", label: "Orchestrator" },
    { kind: "captains", label: "Captains (all)" },
    ...tasteProjectTabs(tasteUi.projects ?? [], repos).map((p) => ({ kind: "project", repo: p.repo, label: `Captain ${p.name}` })),
  ].map((tab) => {
    const on = tab.kind === scope.kind && tab.repo === scope.repo;
    return `<button type="button" data-act="scope" data-kind="${tab.kind}"${tab.repo ? ` data-repo="${esc(tab.repo)}" title="${esc(tab.repo)}"` : ""} aria-pressed="${on}">${esc(tab.label)}</button>`;
  }).join("");
  const profile = tasteProfile(t);
  const cards = TASTE_PROFILES.map((p) => `<button type="button" class="taste-card${p.id === profile ? " active" : ""}" data-act="profile" data-profile="${p.id}" aria-pressed="${p.id === profile}"><b>${p.label}</b><span>${p.about}</span></button>`).join("");
  const custom = `<div class="taste-card custom${profile === "custom" ? " active" : ""}"><b>Custom</b><span>The fields below, your way. Shows when you change something.</span></div>`;
  const focused = tasteFocusKey(document.activeElement);
  setPage(`${head}<div class="page-body"><div class="page-wrap taste">${sub}
    <div class="taste-scopes"><div class="taste-seg" role="group" aria-label="Whose way of working">${tabs}</div>${scope.kind === "project" ? `<span class="taste-quiet">Inherits from Captains (all).</span>` : ""}</div>
    <div class="taste-split"><div class="taste-controls">
      <section class="taste-section"><div class="taste-heading"><h2>Profile</h2><p>Sets every field below. Changing a field makes it Custom.</p></div><div class="taste-cards">${cards}${custom}</div></section>
      ${tasteModelsHtml(t, catalog, names)}${tastePaceHtml(t)}${tasteContextHtml(t)}
    </div>${tastePreviewHtml(t)}</div>
  </div></div>${tasteFootHtml()}`);
  tasteUi.flash = new Set();
  const back = focused && [...pageView.querySelectorAll("button, select, input")].find((el) => tasteFocusKey(el) === focused);
  back?.focus({ preventScroll: true });
}

const TASTE_ACTS = {
  scope: (b) => {
    tasteUi.scope = b.dataset.kind === "project" ? { kind: "project", repo: b.dataset.repo } : { kind: b.dataset.kind };
    tasteRender();
  },
  profile: (b) => tasteEdit(tasteApplyProfile(tasteUi.draft, tasteUi.scope, b.dataset.profile)),
  set: (b) => tasteEditField(b.dataset.field, b.dataset.value),
  toggle: (b) => tasteEditField(b.dataset.field, !tasteShown()[b.dataset.field]),
  reset: (b) => tasteEdit(tasteResetField(tasteUi.draft, tasteUi.scope, b.dataset.field)),
  parallel: (b) => tasteEditField("parallel", tasteShown().parallel + Number(b.dataset.step)),
  up: (b) => tasteMoveRung(b.dataset.rung, -1),
  down: (b) => tasteMoveRung(b.dataset.rung, 1),
  remove: (b) => tasteEditField("ladder", tasteShown().ladder.filter((ref) => tasteRefKey(ref) !== b.dataset.rung)),
  unnote: (b) => tasteEditField("context", tasteShown().context.filter((_, i) => i !== Number(b.dataset.note))),
  discard: () => {
    tasteUi.draft = tasteUi.saved;
    tasteUi.error = null;
    tasteRender();
  },
  save: () => tasteSave(),
};

pageView.addEventListener("click", (e) => {
  if (document.body.dataset.page !== "taste") return;
  const b = e.target.closest("[data-act]");
  if (b && !b.disabled) TASTE_ACTS[b.dataset.act](b);
});
pageView.addEventListener("change", (e) => {
  if (document.body.dataset.page !== "taste" || !e.target.value) return;
  if (e.target.matches("[data-add-model]")) tasteEditField("ladder", [...tasteShown().ladder, tasteRefOf(e.target.value)]);
  else if (e.target.matches("[data-hard-model]")) tasteEditField("hard_model", tasteRefOf(e.target.value));
});
pageView.addEventListener("input", (e) => {
  if (document.body.dataset.page === "taste" && e.target.matches("[data-note-input]")) tasteUi.note = e.target.value;
});
pageView.addEventListener("submit", (e) => {
  if (document.body.dataset.page !== "taste" || !e.target.matches("[data-note-form]")) return;
  e.preventDefault();
  const note = tasteUi.note.trim();
  if (!note) return;
  tasteUi.note = "";
  tasteEditField("context", [...tasteShown().context, note]);
});

SHELL_PAGES.taste = {
  show() {
    if (!tasteUi.saved) tasteUi.error = null;
    tasteRender();
    tasteFetch();
    tasteFetchProjects();
  },
};
