/** The Way of working page: how the orchestrator, the Captains and each project's Captain split
 * work, pick models and ask for input. It edits a draft of the account's tastes and saves what
 * changed; the logic lives in taste.js. */

/** `saved` is the backend's last answer and `draft` the page's edits over it. `projects` holds
 * every online machine's project list, each project with its `machine`, null until it arrives.
 * `skills` holds the skills those machines list, by name. `flash` holds the brief lines the last
 * edit changed, drawn highlighted by the next render only. `note` is the note being typed, and
 * `adding` whether the add-skill form is open. */
const tasteUi = { saved: null, draft: null, scope: { kind: "captains" }, projects: null, skills: new Map(), error: null, saving: false, toast: false, flash: new Set(), note: "", adding: false, chatting: false };
let tasteToastTimer = 0;

const TASTE_FIELD_LABELS = {
  strategy: "Strategy",
  ladder: "Model ladder",
  hard_model: "Hard work model",
  parallel: "Workers at once",
  autonomy: "Autonomy",
  review_diffs: "Review each diff",
  second_opinion: "Second opinion",
  skills: "Skills",
  context: "Context",
  tier: "Model tier",
  default_effort: "Default effort",
  max_effort: "Most effort",
  budget_cents: "Budget per worker",
  time_limit_min: "Time limit per worker",
  box_budget_cents: "Box budget",
  box_size: "Default box size",
};
const TASTE_TIER_ABOUT = { low: "Haiku and flash models", mid: "Sonnet, GLM and the like", high: "Opus and pro models" };

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
  const online = onlineMachines();
  const [lists, listings] = await Promise.all([
    Promise.all(online.map((m) => callOn(m, { cmd: "list_projects" }).catch(() => []))),
    Promise.all(online.map((m) => callOn(m, { cmd: "skills" }).catch(() => null))),
  ]);
  tasteUi.projects = lists.flatMap((list, i) => list.map((p) => ({ ...p, machine: online[i] })));
  tasteUi.skills = new Map();
  for (const listing of listings) for (const c of listing?.copies ?? []) if (!tasteUi.skills.has(c.key)) tasteUi.skills.set(c.key, c);
  tasteRender();
}

/** Opens a thread whose session adjusts the current tab's scope by talking: in the project's
 * folder on a machine that has it, else on the orchestrator's machine without a project. */
async function tasteChat() {
  const scope = tasteUi.scope;
  const here = scope.kind === "project" ? (tasteUi.projects ?? []).find((p) => p.repo === scope.repo && machines.get(p.machine)?.view.online) : null;
  const machine = here?.machine ?? globalHostMachine();
  if (!machine) {
    tasteUi.error = "No machine is online to chat on.";
    return tasteRender();
  }
  tasteUi.chatting = true;
  tasteUi.error = null;
  tasteRender();
  try {
    const id = await callOn(machine, { cmd: "open_taste_chat", scope, ...(here ? { path: here.path } : {}) });
    tasteUi.chatting = false;
    await startTab(await revealTab(machine, id));
  } catch (e) {
    tasteUi.chatting = false;
    tasteUi.error = /unknown variant/.test(String(e)) ? `Update pinned on ${machineName(machine)} to chat about the way of working.` : String(e?.message ?? e);
    tasteRender();
  }
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
/** The account's placement policy changes as a whole: it belongs to no one scope. */
const tasteEditPlacement = (placement) => tasteEdit({ ...tasteUi.draft, placement });
const tasteEditField = (field, value) => tasteEdit(tasteSetField(tasteUi.draft, tasteUi.scope, field, value));

function tasteMoveRung(key, step) {
  const ladder = [...tasteShown().ladder];
  const i = ladder.findIndex((ref) => tasteRefKey(ref) === key);
  [ladder[i], ladder[i + step]] = [ladder[i + step], ladder[i]];
  tasteEditField("ladder", ladder);
}

/** On a project tab, whether the field is the project's own or the Captains', and the way back. */
function tasteTagHtml(field) {
  if (tasteUi.scope.kind !== "project" || tasteUi.draft.projects?.[tasteUi.scope.repo]?.[field] == null) return "";
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

/** A labelled − value + row. `down` and `up` are the values a click sets, null at an end. */
function tasteStepperHtml(field, shown, down, up, noun) {
  const step = (value, dir, what) => `<button type="button" data-act="step" data-field="${field}" data-value="${value ?? ""}" aria-label="${what} ${noun}"${value == null ? " disabled" : ""}>${icon(dir)}</button>`;
  return `<div class="taste-field taste-inline">${tasteLabelHtml(field)}
    <span class="taste-stepper">${step(down, "minus", "Less")}<output aria-live="polite">${esc(shown)}</output>${step(up, "plus", "More")}</span></div>`;
}

function tasteParallelHtml(t) {
  return tasteStepperHtml("parallel", String(t.parallel), t.parallel > 1 ? t.parallel - 1 : null, t.parallel < TASTE_MAX_PARALLEL ? t.parallel + 1 : null, "workers at once");
}

/** A stepper over `TASTE_STEPS[field]`, labelled by `label`. */
function tasteLadderHtml(t, field, label, noun) {
  const value = t[field];
  const near = (dir) => {
    const next = tasteStep(field, value, dir);
    return next === value ? null : next;
  };
  return tasteStepperHtml(field, label(value), near(-1), near(1), noun);
}

/** The default Upstash Box size: the effective size and where it comes from, and the choices. A
 * hub that predates it sends none, so there is nothing to show. A project row says what it
 * inherits and carries the Reset; the account's can go back to the built-in small. */
function tasteBoxSizeHtml() {
  const { draft, scope } = tasteUi;
  if (draft.captains.box_size === undefined) return "";
  const { size, from } = tasteBoxSize(draft, scope);
  const sizes = TASTE_BOX_SIZES.map((s) => `<button type="button" data-act="set" data-field="box_size" data-value="${s}" aria-pressed="${s === size}">${tasteBoxSizeName(s)}</button>`).join("");
  const own = scope.kind === "project" ? draft.projects?.[scope.repo]?.box_size != null : draft.captains.box_size != null;
  const tag = scope.kind === "project" ? tasteTagHtml("box_size") : own ? `<button type="button" class="taste-reset" data-act="reset" data-field="box_size" aria-label="Use the built-in size for the account" title="Use the built-in size for the account">Reset</button>` : "";
  const inherits = scope.kind === "project" && !own ? `Inherits ${tasteBoxSizeName(size)} from ${from === "account" ? "Captains (all)" : "the built-in default"}. ` : "";
  return `<div class="taste-field"><div class="taste-label"><span>${TASTE_FIELD_LABELS.box_size}</span>${tag}</div>
    <div class="taste-seg" role="group" aria-label="Default box size">${sizes}</div>
    <span class="taste-quiet"><output aria-live="polite">${tasteBoxSizeText(draft, scope)}</output> ${inherits}Boxes opened with no size named start at this size. A size named for one box wins.</span></div>`;
}

function tasteLimitsHtml(t) {
  const note = (text, rest = "") => `<section class="taste-section"><div class="taste-heading"><h2>Delegation limits</h2><p>${text}</p></div>${rest}</section>`;
  if (tasteUi.scope.kind === "orchestrator") return note("The orchestrator starts no workers itself. Each Captain follows the limits under Captains (all) or its project.");
  if (t.tier == null) return note("Update pinned on the hub to set the other delegation limits.", tasteParallelHtml(t));
  const tiers = TASTE_TIERS.map((tier) => `<button type="button" data-act="set" data-field="tier" data-value="${tier}" aria-pressed="${tier === t.tier}" title="${TASTE_TIER_ABOUT[tier]}">${tier[0].toUpperCase()}${tier.slice(1)}</button>`).join("");
  const efforts = (selected, none) => `${none ? `<option value=""${selected == null ? " selected" : ""}>${none}</option>` : ""}${TASTE_EFFORTS.map((e) => `<option value="${e}"${e === selected ? " selected" : ""}>${e}</option>`).join("")}`;
  return `<section class="taste-section taste-limits"><div class="taste-heading"><h2>Delegation limits</h2><p>pinned holds every worker to these. A call past them is refused, and a worker past its time or budget is stopped.</p></div>
    ${tasteParallelHtml(t)}
    <div class="taste-field">${tasteLabelHtml("tier")}<div class="taste-seg" role="group" aria-label="Model tier">${tiers}</div>
      <span class="taste-quiet">The costliest tier a worker's own model may be. ${TASTE_TIER_ABOUT[t.tier]} and below. The models a worker picks for its own subagents are not held to it.</span></div>
    <div class="taste-limit-pair">
      <div class="taste-field taste-inline"><label class="taste-label">${TASTE_FIELD_LABELS.default_effort} <select data-effort="default_effort">${efforts(t.default_effort, "The model's own")}</select></label>${tasteTagHtml("default_effort")}</div>
      <div class="taste-field taste-inline"><label class="taste-label">${TASTE_FIELD_LABELS.max_effort} <select data-effort="max_effort">${efforts(t.max_effort)}</select></label>${tasteTagHtml("max_effort")}</div>
    </div>
    ${tasteLadderHtml(t, "time_limit_min", tasteMinutes, "time per worker")}
    ${tasteLadderHtml(t, "budget_cents", tasteMoney, "budget per worker")}
    ${tasteLadderHtml(t, "box_budget_cents", tasteMoney, "budget per box")}
    ${tasteBoxSizeHtml()}
    <p class="taste-quiet">The budget counts what a Claude Code worker's tokens cost at list prices, its subagents' included. Cursor and Codex workers are held to time and tier only.</p>
  </section>`;
}

/** The account's machines as the policy can name them, by hub id. This device's own entry carries
 * the hub's view of it, which has the id the hub knows it by. */
const tasteMachines = () => placementMachines([...machines].map(([id, m]) => (id === LOCAL ? m.hubView : m.view && { ...m.view, id })));

function tastePlacementRowHtml(m, list, i, count) {
  const id = esc(m.id);
  const name = esc(m.name);
  const load = placementLoad(m);
  const chip = m.gone ? `<span class="taste-chip">removed</span>` : `<span class="taste-chip" data-state="${placementState(m)}">${placementState(m)}</span>`;
  const move = (dir, step) => `<button type="button" class="taste-icon" data-act="pmove" data-id="${id}" data-step="${step}" aria-label="Move ${name} ${dir}"${(step < 0 ? i === 0 : i === count - 1) ? " disabled" : ""}>${icon(`chevron-${dir}`)}</button>`;
  return `<li${m.gone ? ` class="gone"` : ""}>${list === "prefer" ? `<span class="pos">${i + 1}</span>` : ""}<span class="name" title="${id}">${m.gone ? `<s>${name}</s>` : name}</span>${chip}${!m.gone && load ? `<span class="taste-quiet mach-meta">${esc(load)}</span>` : ""}<span class="spacer"></span>${list === "prefer" ? move("up", -1) + move("down", 1) : ""}<button type="button" class="taste-icon" data-act="pdrop" data-list="${list}" data-id="${id}" aria-label="Remove ${name} from ${list === "prefer" ? "Prefer" : "Avoid"}">${icon("trash-2")}</button></li>`;
}

function tastePlacementListHtml(list, label, hint, policy, all) {
  const rows = placementRows(policy[list], all);
  const other = list === "prefer" ? "avoid" : "prefer";
  const free = all.filter((m) => !placementHas(policy, m.id));
  const locked = all.filter((m) => policy[other].includes(m.id));
  const options = `<option value="" selected disabled>${all.length ? "Choose a machine" : "No machine to choose"}</option>${free.map((m) => `<option value="${esc(m.id)}">${esc(m.name)}</option>`).join("")}${locked.map((m) => `<option value="${esc(m.id)}" disabled>${esc(m.name)} (in ${other === "prefer" ? "Prefer" : "Avoid"})</option>`).join("")}`;
  return `<div class="taste-field"><div class="taste-label"><span>${label}</span><span class="taste-quiet">${hint}</span></div>
    <${list === "prefer" ? "ol" : "ul"} class="taste-ladder taste-machines" aria-label="${label}">${rows.map((m, i) => tastePlacementRowHtml(m, list, i, rows.length)).join("")}</${list === "prefer" ? "ol" : "ul"}>
    <div class="taste-inline"><label class="taste-label">Add to ${label} <select data-place-add="${list}">${options}</select></label></div></div>`;
}

/** Where heavy workers run, for the whole account: Prefer, Avoid, and which work is heavy. A hub
 * that predates placement sends no policy, and a project tab points to the account's. */
function tastePlacementHtml() {
  const section = (body) => `<section class="taste-section taste-placement"><div class="taste-heading"><h2>Placement</h2><p>Where heavy workers run. A task that names a machine always runs there.</p></div>${body}</section>`;
  if (tasteUi.scope.kind === "project") return section(`<p class="taste-quiet">This is set for the whole account, under <button type="button" class="taste-link" data-act="scope" data-kind="captains">Captains (all)</button>.</p>`);
  const policy = tasteUi.draft.placement;
  if (policy == null) return section(`<p class="taste-quiet">Update pinned on the hub to choose where heavy work runs.</p>`);
  const all = tasteMachines();
  const heavy = PLACEMENT_HEAVY.map((h) => `<button type="button" data-act="pheavy" data-value="${h.id}" aria-pressed="${h.id === (policy.heavy ?? "worktree")}">${h.label}</button>`).join("");
  const active = policy.prefer.length || policy.avoid.length;
  return section(`${tastePlacementListHtml("prefer", "Prefer", "First that fits wins", policy, all)}${tastePlacementListHtml("avoid", "Avoid", "Never chosen by rank", policy, all)}
    <div class="taste-field"><div class="taste-label"><span>Heavy work</span></div><div class="taste-seg" role="group" aria-label="Heavy work">${heavy}</div></div>
    <div class="taste-policy"><p class="taste-quiet">${active ? "A heavy worker goes to the first preferred machine that fits, then to the least busy one. An avoided machine runs a worker only when it is the one that asked and nothing else fits." : "Nothing is preferred or avoided, so every worker runs on the machine it was asked on."}</p>
      <p class="taste-quiet">If no machine fits, the worker runs on the machine that asked, and the reason is shown.</p>
      <p class="taste-quiet">A machine counts as busy while its CPU is over 90% for 1 minute or it has under 2 GB of memory free.</p>
      <p class="taste-quiet">A machine with under 10 GB of disk free takes no heavy worker.</p></div>`);
}

function tastePaceHtml(t) {
  const autonomies = TASTE_AUTONOMIES.map((a) => `<button type="button" data-act="set" data-field="autonomy" data-value="${a.id}" aria-pressed="${a.id === t.autonomy}">${a.label}</button>`).join("");
  const toggle = (field, title, about) => `<div class="taste-switch-row">
    <button type="button" class="taste-switch" role="switch" aria-checked="${t[field]}" aria-label="${title}" data-act="toggle" data-field="${field}"><span></span></button>
    <span class="taste-switch-text"><b>${title}</b><span>${about}</span></span><span class="taste-tags">${tasteTagHtml(field)}</span></div>`;
  return `<section class="taste-section"><h2>Pace</h2>
    ${tasteUi.scope.kind === "orchestrator" ? tasteParallelHtml(t) : ""}
    <div class="taste-field">${tasteLabelHtml("autonomy")}<div class="taste-seg" role="group" aria-label="Autonomy">${autonomies}</div></div>
    ${toggle("review_diffs", "Review each diff before accepting", "The Captain reads the worker's diff before merging it in.")}
    ${toggle("second_opinion", "Second opinion on risky changes", "Another model reviews the change before it goes ahead.")}
  </section>`;
}

function tasteSkillsHtml(t) {
  const on = new Set(t.skills.map((s) => s.name));
  const rows = t.skills.map((s, i) => {
    const listed = tasteUi.skills.get(s.name);
    const origin = listed ? scopeLabel(listed.scope) : "Not on an online machine";
    const label = `Use the ${s.name} skill`;
    return `<li><span class="taste-skill-text"><span class="taste-skill-name"><code>${esc(s.name)}</code><span class="taste-chip">${esc(origin)}</span></span>${listed?.description ? `<span class="taste-skill-about">${esc(listed.description)}</span>` : ""}</span>
      <button type="button" class="taste-switch" role="switch" aria-checked="${s.enabled}" aria-label="${esc(label)}" data-act="skill" data-skill="${i}"><span></span></button></li>`;
  }).join("");
  const choices = [...tasteUi.skills.keys()].filter((name) => !on.has(name)).sort();
  const form = tasteUi.adding
    ? `<form class="taste-skill-form" data-skill-form><label class="taste-label">Skill <select data-skill-pick>${choices.length ? choices.map((name) => `<option value="${esc(name)}">${esc(name)}</option>`).join("") : `<option value="" disabled selected>No other skill on your machines</option>`}</select></label>
      <span class="taste-skill-actions"><button type="submit" class="primary"${choices.length ? "" : " disabled"}>Add</button><button type="button" class="outline" data-act="unadd">Cancel</button></span></form>`
    : "";
  return `<section class="taste-section"><div class="taste-heading-row"><div class="taste-heading"><div class="taste-title"><h2>Skills</h2>${tasteTagHtml("skills")}</div><p>Guides every turn carries.</p></div>
    <span class="taste-heading-actions"><button type="button" class="outline" data-act="adding">Add skill</button><button type="button" class="taste-link" data-act="chat">Create by chatting</button></span></div>
    <ul class="taste-skills">${rows}</ul>${form}
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
  return `<aside class="taste-preview"><h2>${tasteUi.scope.kind === "orchestrator" ? `What the ${esc(globalSettings.name)} receives` : "What the Captain receives"}</h2>
    <div class="taste-box"><div class="taste-lines">${lines}</div>
      <div class="taste-meters"><div><span>Cost per task</span><span title="A rough estimate from each model's cost tier">${tasteCost(t, (id) => tasteModelTier(id, claudeModels()))}</span></div>
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
  const chat = `<button type="button" class="outline taste-chat" data-act="chat"${tasteUi.chatting ? " disabled" : ""}>${icon("message-square")}${tasteUi.chatting ? "Opening…" : "Chat to adjust"}</button>`;
  const head = headHtml("Way of working", `<span class="spacer"></span>${toast}${tasteUi.draft ? chat : ""}`);
  const sub = `<p class="taste-sub">How the ${esc(globalSettings.name)} and the Captains split work, pick models and ask for your input.</p>`;
  if (!tasteUi.draft) {
    setPage(`${head}<div class="page-body"><div class="page-wrap taste">${sub}<p class="taste-state${tasteUi.error ? " error" : ""}">${esc(tasteUi.error ?? "Loading…")}</p></div></div>`);
    return;
  }
  const scope = tasteUi.scope;
  const t = tasteShown();
  const catalog = tasteCatalog(claudeModels(), OPENCODE_GO_MODELS);
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
      ${tasteModelsHtml(t, catalog, names)}${tastePaceHtml(t)}${tasteLimitsHtml(t)}${tastePlacementHtml()}${tasteSkillsHtml(t)}${tasteContextHtml(t)}
    </div>${tastePreviewHtml(t)}</div>
    ${delegationSectionHtml()}
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
  step: (b) => tasteEditField(b.dataset.field, Number(b.dataset.value)),
  up: (b) => tasteMoveRung(b.dataset.rung, -1),
  down: (b) => tasteMoveRung(b.dataset.rung, 1),
  remove: (b) => tasteEditField("ladder", tasteShown().ladder.filter((ref) => tasteRefKey(ref) !== b.dataset.rung)),
  unnote: (b) => tasteEditField("context", tasteShown().context.filter((_, i) => i !== Number(b.dataset.note))),
  skill: (b) => tasteEditField("skills", tasteShown().skills.map((s, i) => (i === Number(b.dataset.skill) ? { ...s, enabled: !s.enabled } : s))),
  adding: () => {
    tasteUi.adding = true;
    tasteRender();
  },
  unadd: () => {
    tasteUi.adding = false;
    tasteRender();
  },
  pmove: (b) => tasteEditPlacement(placementMove(tasteUi.draft.placement, b.dataset.id, Number(b.dataset.step))),
  pdrop: (b) => tasteEditPlacement(placementRemove(tasteUi.draft.placement, b.dataset.list, b.dataset.id)),
  pheavy: (b) => tasteEditPlacement({ ...tasteUi.draft.placement, heavy: b.dataset.value }),
  chat: () => tasteChat(),
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
/** An effort pick. Lowering the most effort below the default brings the default down with it. */
function tasteEditEffort(field, value) {
  const t = tasteShown();
  const max = field === "max_effort" ? value : t.max_effort;
  const fallen = t.default_effort && TASTE_EFFORTS.indexOf(t.default_effort) > TASTE_EFFORTS.indexOf(max);
  tasteEdit(tasteSetFields(tasteUi.draft, tasteUi.scope, fallen ? { [field]: value, default_effort: max } : { [field]: value }));
}

pageView.addEventListener("change", (e) => {
  if (document.body.dataset.page !== "taste") return;
  if (e.target.matches("[data-effort]")) return tasteEditEffort(e.target.dataset.effort, e.target.value || null);
  if (!e.target.value) return;
  if (e.target.matches("[data-place-add]")) tasteEditPlacement(placementAdd(tasteUi.draft.placement, e.target.dataset.placeAdd, e.target.value));
  else if (e.target.matches("[data-add-model]")) tasteEditField("ladder", [...tasteShown().ladder, tasteRefOf(e.target.value)]);
  else if (e.target.matches("[data-hard-model]")) tasteEditField("hard_model", tasteRefOf(e.target.value));
});
pageView.addEventListener("input", (e) => {
  if (document.body.dataset.page === "taste" && e.target.matches("[data-note-input]")) tasteUi.note = e.target.value;
});
pageView.addEventListener("submit", (e) => {
  if (document.body.dataset.page !== "taste" || !e.target.matches("[data-skill-form]")) return;
  e.preventDefault();
  const name = e.target.querySelector("[data-skill-pick]").value;
  if (!name) return;
  tasteUi.adding = false;
  tasteEditField("skills", [...tasteShown().skills, { name, enabled: true }]);
});
pageView.addEventListener("submit", (e) => {
  if (document.body.dataset.page !== "taste" || !e.target.matches("[data-note-form]")) return;
  e.preventDefault();
  const note = tasteUi.note.trim();
  if (!note) return;
  tasteUi.note = "";
  tasteEditField("context", [...tasteShown().context, note]);
});

/** A taste chat's session changed the tastes: the page reads them again, keeping unsaved edits. */
function tastesChanged() {
  if (document.body.dataset.page === "taste") tasteFetch();
}

SHELL_PAGES.taste = {
  show() {
    if (!tasteUi.saved) tasteUi.error = null;
    tasteRender();
    tasteFetch();
    tasteFetchProjects();
    delegationFetch();
  },
};
