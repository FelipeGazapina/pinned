/** The worker profiles' section of the Way of working page (U7b): the account's profiles on the
 * Orchestrator and Captains (all) tabs, and a project's own on its tab, which only the user keeps,
 * with the hub's `set_worker_profiles` through the page's Save. gui-taste.js draws what this
 * returns and runs these acts. Proposed profiles are not shown. */

/** The open add or edit form, or null: the list it writes (`repo`, null for the account's), the
 * name of the profile it edits (null for a new one), what was typed, kept across renders, and
 * whether Done found no name. */
let tasteProfileForm = null;

/** The repo whose list the tab edits, or null for the account's. */
const tasteProfileRepo = () => (tasteUi.scope.kind === "project" ? tasteUi.scope.repo : null);
const tasteProfileList = (repo) => (repo == null ? tasteUi.draft.worker_profiles.account : tasteUi.draft.worker_profiles.projects?.[repo] ?? []);

function tasteProfileRowHtml(p, i, names, open) {
  const name = esc(p.name);
  const model = p.model ? esc(tasteRefLabel(p.model, names)) : "none";
  return `<li><span class="taste-skill-text"><span class="taste-skill-name"><b>${name}</b><span class="taste-chip">${p.access === "read_only" ? "Read-only" : "Can edit"}</span></span>
      <span class="taste-skill-about">Model: ${model} · Effort: ${esc(p.effort ?? "none")}</span>${p.delivers ? `<span class="taste-skill-about">Delivers: ${esc(p.delivers)}</span>` : ""}</span>
    <button type="button" class="taste-icon" data-act="wpedit" data-wp="${i}" aria-label="Edit ${name}"${open ? " disabled" : ""}>${icon("pencil")}</button>
    <button type="button" class="taste-icon" data-act="wpremove" data-wp="${i}" aria-label="Remove ${name}"${open ? " disabled" : ""}>${icon("trash-2")}</button></li>`;
}

function tasteProfileFormHtml(f, catalog, names) {
  const model = f.model ? tasteRefOf(f.model) : null;
  const kept = model && (model.provider || !names.has(model.model)) ? `<option value="${esc(f.model)}" selected>${esc(tasteRefLabel(model, names))}</option>` : "";
  // A stored effort the page does not list stays as its own option, as a model outside the catalog does.
  const listed = !f.effort || TASTE_EFFORTS.includes(f.effort) ? TASTE_EFFORTS : [...TASTE_EFFORTS, f.effort];
  const efforts = ["", ...listed].map((e) => `<option value="${esc(e)}"${e === f.effort ? " selected" : ""}>${esc(e) || "None"}</option>`).join("");
  const access = [["edit", "Can edit"], ["read_only", "Read-only"]].map(([id, label]) => `<option value="${id}"${id === f.access ? " selected" : ""}>${label}</option>`).join("");
  return `<form class="taste-profile-form" data-wp-form aria-label="${f.editing == null ? "New profile" : `Edit ${esc(f.editing)}`}">
    <label class="taste-label">Name <input type="text" data-wp-field="name" value="${esc(f.name)}" placeholder="E.g. scout" required${f.invalid ? ` aria-invalid="true" aria-describedby="wp-name-error"` : ""}></label>
    ${f.invalid ? `<p class="taste-error" id="wp-name-error" role="alert">Name the profile.</p>` : ""}
    <div class="taste-limit-pair"><label class="taste-label">Model <select data-wp-field="model"><option value=""${f.model ? "" : " selected"}>None</option>${kept}${tasteOptionsHtml(catalog, { selected: f.model })}</select></label>
      <label class="taste-label">Effort <select data-wp-field="effort">${efforts}</select></label>
      <label class="taste-label">Access <select data-wp-field="access">${access}</select></label></div>
    <label class="taste-label">Prompt <textarea data-wp-field="prompt" rows="4" placeholder="Who the worker is. It goes ahead of the task.">${esc(f.prompt)}</textarea></label>
    <label class="taste-label">What it delivers <input type="text" data-wp-field="delivers" value="${esc(f.delivers)}" placeholder="E.g. A list of the files that need changes"></label>
    <label class="taste-label">Tools <input type="text" data-wp-field="tools" value="${esc(f.tools)}" placeholder="E.g. Read, Grep, Bash"></label>
    <span class="taste-skill-actions"><button type="submit" class="primary">Done</button><button type="button" class="outline" data-act="wpcancel">Cancel</button></span></form>`;
}

function tasteProfilesHtml(_t) {
  const section = (body, actions = "") => `<section class="taste-section taste-profiles"><div class="taste-heading-row"><div class="taste-heading"><h2>Worker profiles</h2>
    <p>Setups a Captain can start a worker with.</p></div>${actions}</div>${body}</section>`;
  if (tasteUi.draft.worker_profiles == null) return section(`<p class="taste-quiet">Update pinned on the hub to set worker profiles.</p>`);
  const repo = tasteProfileRepo();
  const list = tasteProfileList(repo);
  const catalog = tasteCatalog(claudeModels(), goModels());
  const names = new Map(catalog.flatMap((group) => group.models.map((m) => [m.slug, m.name])));
  const open = tasteProfileForm?.repo === repo;
  const hint = repo == null ? "" : `<p class="taste-quiet">The account's profiles, under Captains (all), apply here too. A profile here wins over the account's profile of the same name.</p>`;
  const empty = list.length ? "" : `<p class="taste-quiet">${repo == null ? "No profiles yet." : "This project has no profiles of its own."}</p>`;
  const add = `<span class="taste-heading-actions"><button type="button" class="outline" data-act="wpadd"${open ? " disabled" : ""}>Add profile</button></span>`;
  return section(`${hint}<ul class="taste-skills">${list.map((p, i) => tasteProfileRowHtml(p, i, names, open)).join("")}</ul>${empty}${open ? tasteProfileFormHtml(tasteProfileForm, catalog, names) : ""}`, add);
}

/** Focus on the form's Name, with the form in view. */
function tasteProfileFocusName() {
  pageView.querySelector("[data-wp-form]")?.scrollIntoView({ block: "nearest" });
  pageView.querySelector('[data-wp-field="name"]')?.focus({ preventScroll: true });
}

/** Closes the form, and gives focus to the Edit button of row `at`, or to Add profile without one. */
function tasteProfileClose(at) {
  pageView.querySelector(at >= 0 ? `[data-act="wpedit"][data-wp="${at}"]` : '[data-act="wpadd"]')?.focus();
}

/** Opens the form on the tab's list, empty or with row `at`. */
function tasteProfileOpen(at) {
  const repo = tasteProfileRepo();
  const p = at == null ? null : tasteProfileList(repo)[at];
  tasteProfileForm = { repo, editing: p?.name ?? null, name: p?.name ?? "", model: p?.model ? tasteRefKey(p.model) : "", effort: p?.effort ?? "", access: p?.access ?? "edit", prompt: p?.prompt ?? "", delivers: p?.delivers ?? "", tools: (p?.tools ?? []).join(", "), invalid: false };
  tasteRender();
  tasteProfileFocusName();
}

/** Done: the profile replaces the one the form edits, or joins the end of its list. Its skills
 * and author stay; a new one is the user's, with none. Without a name the form stays open and
 * says so; the hub's own checks answer at Save. */
function tasteProfileDone() {
  const f = tasteProfileForm;
  if (!f.name.trim()) {
    f.invalid = true;
    tasteRender();
    return tasteProfileFocusName();
  }
  const list = [...tasteProfileList(f.repo)];
  const at = f.editing == null ? -1 : list.findIndex((p) => p.name === f.editing);
  const profile = {
    ...(list[at] ?? { skills: [], by: "user" }),
    name: f.name.trim(),
    model: f.model ? tasteRefOf(f.model) : undefined,
    effort: f.effort || undefined,
    access: f.access,
    prompt: f.prompt,
    delivers: f.delivers.trim(),
    tools: f.tools.split(",").map((tool) => tool.trim()).filter(Boolean),
  };
  if (at < 0) list.push(profile);
  else list[at] = profile;
  tasteProfileForm = null;
  tasteEdit(tasteSetProfiles(tasteUi.draft, f.repo, list));
  tasteProfileClose(at);
}

const TASTE_PROFILE_ACTS = {
  wpadd: () => tasteProfileOpen(null),
  wpedit: (b) => tasteProfileOpen(Number(b.dataset.wp)),
  wpremove: (b) => tasteEdit(tasteSetProfiles(tasteUi.draft, tasteProfileRepo(), tasteProfileList(tasteProfileRepo()).filter((_, i) => i !== Number(b.dataset.wp)))),
  wpcancel: () => {
    const { repo, editing } = tasteProfileForm;
    tasteProfileForm = null;
    tasteRender();
    tasteProfileClose(editing == null ? -1 : tasteProfileList(repo).findIndex((p) => p.name === editing));
  },
};

pageView.addEventListener("input", (e) => {
  if (document.body.dataset.page !== "taste" || !tasteProfileForm || !e.target.matches("[data-wp-field]")) return;
  tasteProfileForm[e.target.dataset.wpField] = e.target.value;
  if (e.target.dataset.wpField === "name") tasteProfileForm.invalid = false;
});
pageView.addEventListener("submit", (e) => {
  if (document.body.dataset.page !== "taste" || !e.target.matches("[data-wp-form]")) return;
  e.preventDefault();
  tasteProfileDone();
});
