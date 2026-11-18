/** The worker profiles' section of the Way of working page (U7b): the account's and each
 * project's profiles, and the ones the Orchestrator proposed, which only the user keeps. They
 * are the user's own, so every write goes through the hub's `set_worker_profiles` and never
 * `set_taste`. The section keeps its own draft and its own Save. gui-taste.js draws what
 * `tasteProfilesHtml` returns and runs `TASTE_PROFILE_ACTS`; act names start with `wp` because
 * `pdrop` and `pmove` already belong to the placement policy.
 *
 * `scope` is "" for the account or a project's repo. `draft` is the edited worker_profiles, null
 * until an edit. `form` is the profile being added or edited: `index` is its place in the shown
 * list, -1 for a new one, and `tools` is the text in the tools field. `dismissed` holds the
 * proposed rows the user dropped, as JSON: the hub has no call that drops one, so the page hides it. */
const profilesUi = { scope: "", draft: null, form: null, dismissed: new Set(), error: null, saving: false };

const PROFILE_ACCESS = [
  { id: "edit", label: "Can edit", about: "Reads and changes files." },
  { id: "read_only", label: "Read only", about: "Reads and reports; changes nothing." },
];

const profilesSaved = () => {
  const w = tasteUi.saved?.worker_profiles;
  return w ? { account: [], projects: {}, proposed: [], ...w } : null;
};
const profilesShown = () => profilesUi.draft ?? profilesSaved();
const profilesListOf = (w, repo) => (repo ? w.projects?.[repo] ?? [] : w.account);
const profilesDirty = () => profilesUi.draft != null && !tasteSame(profilesSaved(), profilesUi.draft);

/** `list` as the scope's own list in a copy of `w`. */
const profilesWithList = (w, repo, list) => (repo ? { ...w, projects: { ...w.projects, [repo]: list } } : { ...w, account: list });

/** The `set_worker_profiles` calls that turn `saved` into `draft`: the account's list, then one per
 * project that changed. */
function profilesCalls(saved, draft) {
  const calls = [];
  if (!tasteSame(saved.account, draft.account)) calls.push({ project: null, profiles: draft.account });
  for (const repo of new Set([...Object.keys(saved.projects ?? {}), ...Object.keys(draft.projects ?? {})])) {
    const was = profilesListOf(saved, repo);
    const now = profilesListOf(draft, repo);
    if (!tasteSame(was, now)) calls.push({ project: repo, profiles: now });
  }
  return calls;
}

function profilesEdit(next) {
  profilesUi.draft = next;
  profilesUi.error = null;
  tasteUi.toast = false;
  tasteRender();
}

/** Sends the changes that turn the saved lists into `next`, in order. Each answer is the account's
 * tastes, kept as the saved ones with the page's draft following, so the page's own Save sees only
 * the taste edits. A failure part way keeps what went through. Resolves true when all went. */
async function profilesSend(next) {
  const calls = profilesCalls(profilesSaved(), next);
  profilesUi.saving = true;
  profilesUi.error = null;
  tasteRender();
  let ok = true;
  try {
    for (const args of calls) {
      tasteUi.saved = await connectorCall("set_worker_profiles", args);
      tasteUi.draft = { ...tasteUi.draft, worker_profiles: tasteUi.saved.worker_profiles };
    }
    profilesUi.draft = null;
    tasteUi.toast = true;
  } catch (e) {
    profilesUi.error = tasteErrorText(e);
    ok = false;
  }
  profilesUi.saving = false;
  tasteRender();
  return ok;
}

async function profilesApprove(b) {
  const row = profilesSaved().proposed[Number(b.dataset.index)];
  if (!row) return;
  const w = profilesShown();
  const repo = profilesUi.scope;
  const before = profilesUi.draft;
  if (await profilesSend(profilesWithList(w, repo, [...profilesListOf(w, repo), row]))) {
    profilesUi.dismissed.add(JSON.stringify(row));
    tasteRender();
  } else profilesUi.draft = before;
}

const profilesBlank = () => ({ name: "", skills: [], access: "edit", delivers: "", by: "user" });

function profilesOpenForm(index) {
  const profile = index < 0 ? profilesBlank() : profilesListOf(profilesShown(), profilesUi.scope)[index];
  if (!profile) return;
  profilesUi.form = { index, profile: { ...profile }, tools: (profile.tools ?? []).join(", "), error: null };
  tasteRender();
}

/** The form's profile as the list stores it: tools split on commas, an unset model or effort left out. */
function profilesFormProfile(form) {
  const { model, effort, ...rest } = form.profile;
  return {
    ...rest,
    name: rest.name.trim(),
    tools: form.tools.split(",").map((t) => t.trim()).filter(Boolean),
    delivers: rest.delivers.trim(),
    ...(model ? { model } : {}),
    ...(effort ? { effort } : {}),
  };
}

function profilesSubmitForm() {
  const form = profilesUi.form;
  if (!form) return;
  const profile = profilesFormProfile(form);
  const list = profilesListOf(profilesShown(), profilesUi.scope);
  if (!profile.name) form.error = "Every profile needs a name.";
  else if (list.some((p, i) => i !== form.index && p.name.trim().toLowerCase() === profile.name.toLowerCase())) form.error = `Two profiles are named ${profile.name}.`;
  if (form.error) return tasteRender();
  profilesUi.form = null;
  profilesEdit(profilesWithList(profilesShown(), profilesUi.scope, form.index < 0 ? [...list, profile] : list.map((p, i) => (i === form.index ? profile : p))));
}

function profilesFormEdit(change) {
  if (!profilesUi.form) return;
  profilesUi.form = { ...profilesUi.form, profile: { ...profilesUi.form.profile, ...change }, error: null };
}

function profileModelHtml(profile) {
  const catalog = tasteCatalog(claudeModels(), OPENCODE_GO_MODELS);
  const names = new Map(catalog.flatMap((group) => group.models.map((m) => [m.slug, m.name])));
  const key = profile.model ? tasteRefKey(profile.model) : "";
  const listed = profile.model && !profile.model.provider && names.has(profile.model.model);
  const current = profile.model && !listed ? `<option value="${esc(key)}" selected>${esc(tasteRefLabel(profile.model, names))}</option>` : "";
  return `<label class="taste-label"><span>Model</span><select data-profile-model><option value=""${profile.model ? "" : " selected"}>The next model down the ladder</option>${current}${tasteOptionsHtml(catalog, { selected: key })}</select></label>`;
}

function profileEffortHtml(profile) {
  const options = TASTE_EFFORTS.map((e) => `<option value="${e}"${e === profile.effort ? " selected" : ""}>${e}</option>`).join("");
  return `<label class="taste-label"><span>Effort</span><select data-profile-effort><option value=""${profile.effort ? "" : " selected"}>Not set</option>${options}</select></label>`;
}

/** The skills a profile may pick: those the machines list, and any it already holds. Skills come
 * from the machines' own listing, the one the Skills section above uses. */
function profileSkillsHtml(profile) {
  const held = new Set(profile.skills.map((s) => s.name));
  const names = [...new Set([...tasteUi.skills.keys(), ...held])].sort();
  if (!tasteUi.skills.size) {
    const text = profile.skills.length ? profile.skills.map((s) => `<code>${esc(s.name)}</code>`).join(" ") : "none";
    return `<div class="taste-field"><div class="taste-label"><span>Skills</span></div><p class="taste-quiet">${text}. No online machine listed its skills to this page, so they can't be picked here.</p></div>`;
  }
  const buttons = names.map((name) => `<button type="button" data-act="wpskill" data-skill="${esc(name)}" aria-pressed="${held.has(name)}"${tasteUi.skills.has(name) ? "" : ` title="Not on an online machine"`}>${esc(name)}</button>`).join("");
  return `<div class="taste-field"><div class="taste-label"><span>Skills</span></div><div class="taste-seg taste-wrap" role="group" aria-label="Skills this profile carries">${buttons}</div></div>`;
}

function profilesFormHtml(form) {
  const p = form.profile;
  const access = PROFILE_ACCESS.map((a) => `<button type="button" data-act="wpaccess" data-value="${a.id}" aria-pressed="${a.id === p.access}">${a.label}</button>`).join("");
  const about = PROFILE_ACCESS.find((a) => a.id === p.access)?.about ?? "";
  const error = form.error ? `<span class="taste-error" role="alert">${esc(form.error)}</span>` : "";
  return `<form class="taste-profile-form" data-profile-form>
    <label class="taste-label"><span>Name</span><input type="text" data-profile-name value="${esc(p.name)}" placeholder="E.g. scout" aria-label="Profile name"></label>
    <label class="taste-label"><span>What it delivers</span><input type="text" data-profile-delivers value="${esc(p.delivers)}" placeholder="One line, e.g. a report of what it found" aria-label="What the worker delivers"></label>
    <div class="taste-field"><div class="taste-label"><span>Access</span></div><div class="taste-seg" role="group" aria-label="Access">${access}</div><span class="taste-quiet">${esc(about)}</span></div>
    <label class="taste-label"><span>Tools</span><input type="text" data-profile-tools value="${esc(form.tools)}" placeholder="Read, Grep, Bash" aria-label="Tools, separated by commas"></label>
    ${profileSkillsHtml(p)}
    <div class="taste-limit-pair">${profileModelHtml(p)}${profileEffortHtml(p)}</div>
    <div class="taste-inline">${error}<span class="spacer"></span><button type="button" class="outline" data-act="wpcancel">Cancel</button><button type="submit" class="primary">${form.index < 0 ? "Add profile" : "Keep changes"}</button></div></form>`;
}

function profileRowHtml(profile, index, names) {
  const access = PROFILE_ACCESS.find((a) => a.id === profile.access)?.label ?? profile.access;
  const chips = [access, profile.effort ? `${profile.effort} effort` : "", profile.model ? tasteRefLabel(profile.model, names) : ""].filter(Boolean).map((c) => `<span class="taste-chip">${esc(c)}</span>`).join("");
  const skills = (profile.skills ?? []).map((s) => `<code>${esc(s.name)}</code>`).join(" ");
  const tools = (profile.tools ?? []).length ? `Tools: ${profile.tools.map((t) => `<code>${esc(t)}</code>`).join(" ")}` : "";
  const detail = [profile.delivers ? esc(profile.delivers) : "", skills ? `Skills: ${skills}` : "", tools].filter(Boolean).join(" · ");
  return `<li><span class="taste-profile-text"><span class="taste-skill-name"><b>${esc(profile.name)}</b>${chips}</span>${detail ? `<span class="taste-skill-about">${detail}</span>` : ""}</span>
    <button type="button" class="taste-icon" data-act="wpedit" data-index="${index}" aria-label="Edit profile ${esc(profile.name)}">${icon("pencil")}</button>
    <button type="button" class="taste-icon" data-act="wpdrop" data-index="${index}" aria-label="Remove profile ${esc(profile.name)}">${icon("trash-2")}</button></li>`;
}

function profileProposedHtml(w, names, where) {
  const rows = w.proposed.map((profile, index) => ({ profile, index })).filter(({ profile }) => !profilesUi.dismissed.has(JSON.stringify(profile)));
  if (!rows.length) return "";
  const busy = profilesUi.saving ? " disabled" : "";
  const items = rows.map(({ profile, index }) => `<li><span class="taste-profile-text"><span class="taste-skill-name"><b>${esc(profile.name)}</b><span class="taste-chip">Created by the Orchestrator</span></span>${profile.delivers ? `<span class="taste-skill-about">${esc(profile.delivers)}</span>` : ""}</span>
    <button type="button" class="outline" data-act="wpdismiss" data-index="${index}">Discard</button><button type="button" class="primary" data-act="wpapprove" data-index="${index}"${busy}>Approve for ${esc(where)}</button></li>`).join("");
  return `<div class="taste-field"><div class="taste-label"><span>Proposed</span></div><ul class="taste-rules taste-profiles-list" aria-label="Proposed profiles">${items}</ul></div>`;
}

function profilesFootHtml() {
  const error = profilesUi.error ? `<span class="taste-error" role="alert">${esc(profilesUi.error)}</span>` : "";
  if (!profilesDirty()) return error ? `<div class="taste-inline">${error}</div>` : "";
  const busy = profilesUi.saving ? " disabled" : "";
  return `<div class="taste-inline"><span class="taste-unsaved">Unsaved profile changes</span>${error}<span class="spacer"></span>
    <button type="button" class="outline" data-act="wpdiscard"${busy}>Discard</button><button type="button" class="primary" data-act="wpsave"${busy}>${profilesUi.saving ? "Saving…" : "Save profiles"}</button></div>`;
}

function tasteProfilesHtml(_t) {
  const head = `<div class="taste-heading"><h2>Profiles</h2><p>Named worker setups the Orchestrator picks from when it hands work out.</p></div>`;
  const w = profilesShown();
  if (!w) return `<section class="taste-section taste-profiles">${head}<p class="taste-quiet">Update pinned on the hub to set worker profiles.</p></section>`;
  const repo = profilesUi.scope;
  const repos = [...Object.keys(w.projects ?? {}), ...Object.keys(tasteUi.draft.projects ?? {}), ...(repo ? [repo] : [])];
  const projects = tasteProjectTabs(tasteUi.projects ?? [], repos);
  const tabs = [{ repo: "", label: "Account" }, ...projects.map((p) => ({ repo: p.repo, label: p.name }))]
    .map((tab) => `<button type="button" data-act="wpscope" data-repo="${esc(tab.repo)}"${tab.repo ? ` title="${esc(tab.repo)}"` : ""} aria-pressed="${tab.repo === repo}">${esc(tab.label)}</button>`)
    .join("");
  const names = new Map(tasteCatalog(claudeModels(), OPENCODE_GO_MODELS).flatMap((group) => group.models.map((m) => [m.slug, m.name])));
  const where = repo ? projects.find((p) => p.repo === repo)?.name ?? repo : "the account";
  const list = profilesListOf(w, repo);
  const inherited = repo && w.account.length ? `<span class="taste-quiet">The account's ${w.account.length === 1 ? "profile also applies" : "profiles also apply"} here: ${w.account.map((p) => esc(p.name)).join(", ")}.</span>` : "";
  const rows = list.map((p, i) => profileRowHtml(p, i, names)).join("");
  const form = profilesUi.form ? profilesFormHtml(profilesUi.form) : `<div class="taste-inline"><button type="button" class="outline" data-act="wpadd">Add profile</button></div>`;
  return `<section class="taste-section taste-profiles">${head}
    <div class="taste-scopes"><div class="taste-seg" role="group" aria-label="Whose profiles">${tabs}</div>${inherited}</div>
    ${profileProposedHtml(w, names, where)}
    <ul class="taste-rules taste-profiles-list" aria-label="${repo ? "Project profiles" : "Account profiles"}">${rows}</ul>${form}${profilesFootHtml()}</section>`;
}

const TASTE_PROFILE_ACTS = {
  wpscope: (b) => {
    profilesUi.scope = b.dataset.repo ?? "";
    profilesUi.form = null;
    tasteRender();
  },
  wpadd: () => profilesOpenForm(-1),
  wpedit: (b) => profilesOpenForm(Number(b.dataset.index)),
  wpcancel: () => {
    profilesUi.form = null;
    tasteRender();
  },
  wpdrop: (b) => {
    profilesUi.form = null;
    const w = profilesShown();
    profilesEdit(profilesWithList(w, profilesUi.scope, profilesListOf(w, profilesUi.scope).filter((_, i) => i !== Number(b.dataset.index))));
  },
  wpaccess: (b) => {
    profilesFormEdit({ access: b.dataset.value });
    tasteRender();
  },
  wpskill: (b) => {
    const held = profilesUi.form.profile.skills;
    const name = b.dataset.skill;
    profilesFormEdit({ skills: held.some((s) => s.name === name) ? held.filter((s) => s.name !== name) : [...held, { name, enabled: true }] });
    tasteRender();
  },
  wpapprove: (b) => profilesApprove(b),
  wpdismiss: (b) => {
    const row = profilesSaved().proposed[Number(b.dataset.index)];
    if (row) profilesUi.dismissed.add(JSON.stringify(row));
    tasteRender();
  },
  wpdiscard: () => {
    profilesUi.draft = null;
    profilesUi.form = null;
    profilesUi.error = null;
    tasteRender();
  },
  wpsave: () => profilesUi.draft && profilesSend(profilesUi.draft),
};

pageView.addEventListener("change", (e) => {
  if (document.body.dataset.page !== "taste") return;
  if (e.target.matches("[data-profile-model]")) profilesFormEdit({ model: e.target.value ? tasteRefOf(e.target.value) : null });
  else if (e.target.matches("[data-profile-effort]")) profilesFormEdit({ effort: e.target.value });
});
pageView.addEventListener("input", (e) => {
  if (document.body.dataset.page !== "taste") return;
  if (e.target.matches("[data-profile-name]")) profilesFormEdit({ name: e.target.value });
  else if (e.target.matches("[data-profile-delivers]")) profilesFormEdit({ delivers: e.target.value });
  else if (e.target.matches("[data-profile-tools]") && profilesUi.form) profilesUi.form.tools = e.target.value;
});
pageView.addEventListener("submit", (e) => {
  if (document.body.dataset.page !== "taste" || !e.target.matches("[data-profile-form]")) return;
  e.preventDefault();
  profilesSubmitForm();
});
