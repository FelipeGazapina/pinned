/** The Reviewer's section of the Way of working page (U7a): its rules, model, rounds and each
 * project's switch, which only the user sets, with the hub's `set_reviewer`. gui-taste.js draws
 * what this returns and runs these acts. The section keeps its own draft and its own Save, which
 * sends `set_reviewer` and never `set_taste`; the two review toggles are knobs of the shown
 * scope, so they edit the page's draft and wait for the page's Save. */

const REVIEWER_ROUNDS = { min: 1, max: 5 };
const REVIEWER_SWITCHES = [
  { id: "auto", label: "Auto", about: "pinned decides by the change." },
  { id: "on", label: "On", about: "Every worker's change is reviewed." },
  { id: "off", label: "Off", about: "No review for this project." },
];

/** `scope` is "" for the account or a project's repo. `draft` is the edited Reviewer, null until
 * an edit. `text` and `replaces` are the rule being typed. */
const reviewerUi = { scope: "", draft: null, text: "", replaces: "", error: null, saving: false };

const reviewerSaved = () => tasteUi.saved?.reviewer ?? null;
const reviewerShown = () => reviewerUi.draft ?? reviewerSaved();
const reviewerProject = (r, repo) => r.projects?.[repo] ?? { switch: "auto", rules: [] };
const reviewerRulesOf = (r, repo) => (repo ? reviewerProject(r, repo).rules : r.rules);
const reviewerDirty = () => reviewerUi.draft != null && !tasteSame(reviewerSaved(), reviewerUi.draft);

/** The `set_reviewer` patches that turn `saved` into `draft`: the account's fields first, then one
 * per project that changed. */
function reviewerCalls(saved, draft) {
  const calls = [];
  const patch = {};
  if (!tasteSame(saved.model ?? null, draft.model ?? null)) patch.model = draft.model ?? null;
  if (saved.rounds !== draft.rounds) patch.rounds = draft.rounds;
  if (!tasteSame(saved.rules, draft.rules)) patch.rules = draft.rules;
  if (Object.keys(patch).length) calls.push({ patch });
  for (const repo of new Set([...Object.keys(saved.projects ?? {}), ...Object.keys(draft.projects ?? {})])) {
    const was = reviewerProject(saved, repo);
    const now = reviewerProject(draft, repo);
    const project = { project: repo };
    if (was.switch !== now.switch) project.switch = now.switch;
    if (!tasteSame(was.rules, now.rules)) project.rules = now.rules;
    if (Object.keys(project).length > 1) calls.push({ patch: project });
  }
  return calls;
}

function reviewerEdit(next) {
  reviewerUi.draft = next;
  reviewerUi.error = null;
  tasteUi.toast = false;
  tasteRender();
}

/** `rules` as the scope's own list in a copy of the shown Reviewer. */
function reviewerWithRules(rules) {
  const r = reviewerShown();
  const repo = reviewerUi.scope;
  return repo ? { ...r, projects: { ...r.projects, [repo]: { ...reviewerProject(r, repo), rules } } } : { ...r, rules };
}

/** The next rule id: A or P, then one past the highest number the list has used. */
const reviewerNextId = (rules, prefix) => `${prefix}${Math.max(0, ...rules.map((rule) => Number(rule.id.slice(1)) || 0)) + 1}`;

function reviewerRuleHtml(rule) {
  const id = esc(rule.id);
  return `<li><span class="taste-chip">${id}</span><span class="taste-rule-text">${esc(rule.text)}</span>${rule.replaces ? `<span class="taste-chip">replaces ${esc(rule.replaces)}</span>` : ""}
    <button type="button" class="taste-icon" data-act="rdrop" data-rule="${id}" aria-label="Remove rule ${id}">${icon("trash-2")}</button></li>`;
}

function reviewerModelHtml(r) {
  const catalog = tasteCatalog(claudeModels(), OPENCODE_GO_MODELS);
  const names = new Map(catalog.flatMap((group) => group.models.map((m) => [m.slug, m.name])));
  const key = r.model ? tasteRefKey(r.model) : "";
  const listed = r.model && !r.model.provider && names.has(r.model.model);
  const current = r.model && !listed ? `<option value="${esc(key)}" selected>${esc(tasteRefLabel(r.model, names))}</option>` : "";
  return `<div class="taste-field"><label class="taste-label"><span>Reviewer model</span>
    <select data-reviewer-model><option value=""${r.model ? "" : " selected"}>The next model down the ladder</option>${current}${tasteOptionsHtml(catalog, { selected: key })}</select></label></div>`;
}

function reviewerRoundsHtml(r) {
  return `<div class="taste-field"><label class="taste-label"><span>Rounds</span>
    <input type="number" class="taste-rounds" data-reviewer-rounds min="${REVIEWER_ROUNDS.min}" max="${REVIEWER_ROUNDS.max}" step="1" value="${r.rounds}" aria-label="Review rounds"></label>
    <span class="taste-quiet">Findings go back to the worker this many times before the Captain hears.</span></div>`;
}

function reviewerSwitchHtml(project) {
  const buttons = REVIEWER_SWITCHES.map((s) => `<button type="button" data-act="rswitch" data-value="${s.id}" aria-pressed="${s.id === project.switch}">${s.label}</button>`).join("");
  const about = REVIEWER_SWITCHES.find((s) => s.id === project.switch)?.about ?? "";
  return `<div class="taste-field"><div class="taste-label"><span>Review</span></div><div class="taste-seg" role="group" aria-label="Review this project's changes">${buttons}</div><span class="taste-quiet">${esc(about)}</span></div>`;
}

function reviewerRulesHtml(r, repo) {
  const rules = reviewerRulesOf(r, repo);
  const replaces = repo
    ? `<select data-reviewer-replaces aria-label="Account rule this one replaces"><option value=""${reviewerUi.replaces ? "" : " selected"}>Replaces none</option>${r.rules.map((rule) => `<option value="${esc(rule.id)}"${rule.id === reviewerUi.replaces ? " selected" : ""}>Replaces ${esc(rule.id)}</option>`).join("")}</select>`
    : "";
  return `<div class="taste-field"><div class="taste-label"><span>${repo ? "Project rules" : "Account rules"}</span></div>
    <ul class="taste-rules" aria-label="${repo ? "Project rules" : "Account rules"}">${rules.map(reviewerRuleHtml).join("")}</ul>
    <form class="taste-note-form" data-reviewer-form><input type="text" data-reviewer-text value="${esc(reviewerUi.text)}" placeholder="E.g. no unwrap in the hub" aria-label="New rule">${replaces}
      <button type="submit" class="primary">Add rule</button></form></div>`;
}

function reviewerFootHtml() {
  const error = reviewerUi.error ? `<span class="taste-error" role="alert">${esc(reviewerUi.error)}</span>` : "";
  if (!reviewerDirty()) return error ? `<div class="taste-inline">${error}</div>` : "";
  const busy = reviewerUi.saving ? " disabled" : "";
  return `<div class="taste-inline"><span class="taste-unsaved">Unsaved reviewer changes</span>${error}<span class="spacer"></span>
    <button type="button" class="outline" data-act="rdiscard"${busy}>Discard</button><button type="button" class="primary" data-act="rsave"${busy}>${reviewerUi.saving ? "Saving…" : "Save reviewer"}</button></div>`;
}

function tasteReviewerHtml(t) {
  const head = `<div class="taste-heading"><h2>Reviewer</h2><p>A second session reads each worker's change against your rules and sends what it finds back.</p></div>`;
  const toggles = `${tasteToggleHtml(t, "review_diffs", "Review each diff before accepting", "The Captain reads the worker's diff before merging it in.")}
    ${tasteToggleHtml(t, "second_opinion", "Second opinion on risky changes", "Another model reviews the change before it goes ahead.")}`;
  const r = reviewerShown();
  if (!r) return `<section class="taste-section taste-reviewer">${head}${toggles}<p class="taste-quiet">Update pinned on the hub to set the Reviewer's rules.</p></section>`;
  const repos = [...Object.keys(r.projects ?? {}), ...Object.keys(tasteUi.draft.projects ?? {}), ...(reviewerUi.scope ? [reviewerUi.scope] : [])];
  const tabs = [{ repo: "", label: "Account" }, ...tasteProjectTabs(tasteUi.projects ?? [], repos).map((p) => ({ repo: p.repo, label: p.name }))]
    .map((tab) => `<button type="button" data-act="rscope" data-repo="${esc(tab.repo)}"${tab.repo ? ` title="${esc(tab.repo)}"` : ""} aria-pressed="${tab.repo === reviewerUi.scope}">${esc(tab.label)}</button>`)
    .join("");
  const repo = reviewerUi.scope;
  const body = repo ? `${reviewerSwitchHtml(reviewerProject(r, repo))}${reviewerRulesHtml(r, repo)}` : `${reviewerModelHtml(r)}${reviewerRoundsHtml(r)}${reviewerRulesHtml(r, "")}`;
  return `<section class="taste-section taste-reviewer">${head}${toggles}
    <div class="taste-scopes"><div class="taste-seg" role="group" aria-label="Whose review rules">${tabs}</div></div>
    ${body}${reviewerFootHtml()}</section>`;
}

/** Sends the draft's changes in order. Each answer is the account's tastes, kept as the saved ones
 * with the page's draft following, so the page's own Save sees only the taste edits. A failure
 * part way keeps what went through. */
async function reviewerSave() {
  const sent = reviewerUi.draft;
  const calls = reviewerCalls(reviewerSaved(), sent);
  if (!calls.length) return;
  reviewerUi.saving = true;
  reviewerUi.error = null;
  tasteRender();
  try {
    for (const args of calls) {
      tasteUi.saved = await connectorCall("set_reviewer", args);
      tasteUi.draft = { ...tasteUi.draft, reviewer: tasteUi.saved.reviewer };
    }
    if (reviewerUi.draft === sent) reviewerUi.draft = null;
    tasteUi.toast = true;
  } catch (e) {
    reviewerUi.error = tasteErrorText(e);
  }
  reviewerUi.saving = false;
  tasteRender();
}

function reviewerAddRule() {
  const text = reviewerUi.text.trim();
  if (!text) return;
  const repo = reviewerUi.scope;
  const rules = reviewerRulesOf(reviewerShown(), repo);
  const replaces = repo && reviewerShown().rules.some((rule) => rule.id === reviewerUi.replaces) ? reviewerUi.replaces : null;
  const rule = { id: reviewerNextId(rules, repo ? "P" : "A"), text, ...(replaces ? { replaces } : {}) };
  reviewerUi.text = "";
  reviewerUi.replaces = "";
  reviewerEdit(reviewerWithRules([...rules, rule]));
}

/** A rounds entry that is a whole number from 1 to 5, or null. */
function reviewerRoundsOf(value) {
  const n = Number(value);
  return value.trim() !== "" && Number.isInteger(n) && n >= REVIEWER_ROUNDS.min && n <= REVIEWER_ROUNDS.max ? n : null;
}

const TASTE_REVIEWER_ACTS = {
  rscope: (b) => {
    reviewerUi.scope = b.dataset.repo ?? "";
    reviewerUi.text = "";
    reviewerUi.replaces = "";
    tasteRender();
  },
  rswitch: (b) => {
    const r = reviewerShown();
    const repo = reviewerUi.scope;
    reviewerEdit({ ...r, projects: { ...r.projects, [repo]: { ...reviewerProject(r, repo), switch: b.dataset.value } } });
  },
  rdrop: (b) => reviewerEdit(reviewerWithRules(reviewerRulesOf(reviewerShown(), reviewerUi.scope).filter((rule) => rule.id !== b.dataset.rule))),
  rdiscard: () => {
    reviewerUi.draft = null;
    reviewerUi.error = null;
    tasteRender();
  },
  rsave: () => reviewerSave(),
};

pageView.addEventListener("change", (e) => {
  if (document.body.dataset.page !== "taste") return;
  if (e.target.matches("[data-reviewer-rounds]")) {
    const rounds = reviewerRoundsOf(e.target.value);
    if (rounds == null) {
      reviewerUi.error = `Rounds go from ${REVIEWER_ROUNDS.min} to ${REVIEWER_ROUNDS.max}.`;
      tasteRender();
    } else reviewerEdit({ ...reviewerShown(), rounds });
  } else if (e.target.matches("[data-reviewer-model]")) reviewerEdit({ ...reviewerShown(), model: e.target.value ? tasteRefOf(e.target.value) : null });
  else if (e.target.matches("[data-reviewer-replaces]")) reviewerUi.replaces = e.target.value;
});
pageView.addEventListener("input", (e) => {
  if (document.body.dataset.page === "taste" && e.target.matches("[data-reviewer-text]")) reviewerUi.text = e.target.value;
});
pageView.addEventListener("submit", (e) => {
  if (document.body.dataset.page !== "taste" || !e.target.matches("[data-reviewer-form]")) return;
  e.preventDefault();
  reviewerAddRule();
});
