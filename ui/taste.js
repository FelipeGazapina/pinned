// The way of working, as pure logic: the presets, the profile a taste matches, a project's taste
// over the Captains', the brief a session receives, the draft edits the page makes and the calls a
// save sends. It mirrors crates/core/src/taste.rs; ui/taste.fixture.json holds both to the same
// presets and briefs. gui-taste.js renders and wires it.

/** The knobs a profile sets: every field but skills and context. */
const TASTE_KNOBS = ["strategy", "ladder", "hard_model", "parallel", "autonomy", "review_diffs", "second_opinion"];
const TASTE_FIELDS = [...TASTE_KNOBS, "skills", "context"];
const TASTE_MAX_PARALLEL = 8;

const TASTE_PRESETS = {
  economical: {
    strategy: "cheap_with_guidance",
    ladder: [{ model: "deepseek-v4.1-flash" }, { model: "glm-5.3" }, { model: "claude-opus-5-5" }],
    hard_model: { model: "claude-opus-5-5" },
    parallel: 2,
    autonomy: "ask_first",
    review_diffs: true,
    second_opinion: false,
    skills: [],
    context: [],
  },
  balanced: {
    strategy: "escalate_on_failure",
    ladder: [{ model: "glm-5.3" }, { model: "claude-sonnet-5-5" }, { model: "claude-opus-5-5" }],
    hard_model: { model: "claude-opus-5-5" },
    parallel: 3,
    autonomy: "decide_and_tell",
    review_diffs: true,
    second_opinion: false,
    skills: [],
    context: [],
  },
  aggressive: {
    strategy: "strongest_first",
    ladder: [{ model: "claude-opus-5-5" }],
    hard_model: { model: "claude-opus-5-5" },
    parallel: 6,
    autonomy: "report_at_end",
    review_diffs: false,
    second_opinion: true,
    skills: [],
    context: [],
  },
};

const TASTE_PROFILES = [
  { id: "economical", label: "Economical", about: "A cheap model with a detailed brief, 2 workers. Asks before a pull request." },
  { id: "balanced", label: "Balanced", about: "Starts cheap and moves up when a worker fails, 3 workers. Reviews each diff." },
  { id: "aggressive", label: "Aggressive", about: "The strongest model from the start, 6 workers, a second opinion on risky changes." },
];

/** Each strategy's words on the page, and its line of the brief given the ladder's first rung, the
 * rungs after it and the model hard work goes to. */
const TASTE_STRATEGIES = [
  {
    id: "cheap_with_guidance",
    label: "Cheap, with more context and guidance",
    about: "A cheap model, a rich brief and tight steps.",
    brief: (first, rest) =>
      `Start each worker on ${first} with a detailed brief that names the files, the contracts and a command that proves the result, and re-brief it narrower ${rest.length ? `before moving it up to ${rest[0]}` : "when it fails"}.`,
  },
  {
    id: "escalate_on_failure",
    label: "Start cheap, move up on failure",
    about: "When a worker fails twice, it moves one rung up the ladder.",
    brief: (first, rest) =>
      rest.length
        ? `Start each worker on ${first}, and when it fails twice move it one rung up the ladder: ${rest.join(", then ")}.`
        : `Start each worker on ${first}, and when it fails twice ask the user.`,
  },
  {
    id: "strongest_first",
    label: "Strongest first",
    about: "The hard-work model from the start, no rungs.",
    brief: (first, rest, top) => `Start each worker on ${top} directly, without trying a cheaper model first.`,
  },
];

const TASTE_AUTONOMIES = [
  { id: "ask_first", label: "Ask first", brief: "Ask the user before you open a pull request or delete data." },
  { id: "decide_and_tell", label: "Decide and tell", brief: "Decide on your own and tell the user what you decided." },
  { id: "report_at_end", label: "Report at the end", brief: "Tell the user only at the end, with the result." },
];

/** A rough cost per task in dollars, by the cost tier of the model a worker starts on. */
const TASTE_TIER_COST = { low: 0.45, mid: 0.85, high: 1.8 };

/** Whether two values hold the same data, whatever their key order. A key set to undefined counts
 * as absent, as serde leaves out a provider that is None. */
function tasteSame(a, b) {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) || Array.isArray(b)) return Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((x, i) => tasteSame(x, b[i]));
  const keys = (o) => Object.keys(o).filter((k) => o[k] !== undefined);
  return keys(a).length === keys(b).length && keys(a).every((k) => tasteSame(a[k], b[k]));
}

/** "economical", "balanced", "aggressive", or "custom" when the knobs match no preset. */
function tasteProfile(taste) {
  return TASTE_PROFILES.find(({ id }) => TASTE_KNOBS.every((k) => tasteSame(taste[k], TASTE_PRESETS[id][k])))?.id ?? "custom";
}

function tasteResolve(tastes, scope) {
  if (scope.kind === "orchestrator") return tastes.orchestrator;
  if (scope.kind === "captains") return tastes.captains;
  return { ...tastes.captains, ...tastes.projects?.[scope.repo] };
}

/** A model as the brief names it: "glm-5.3", or "glm-5.3 through opencode". */
const tasteModelName = (ref) => (ref.provider ? `${ref.model} through ${ref.provider}` : ref.model);

/** What a session of this scope is told, one line per knob: Rust's `Taste::brief`. */
function tasteBrief(taste) {
  const lines = [taste.parallel === 1 ? "Run one worker at a time." : `Run up to ${taste.parallel} workers at once.`];
  if (taste.ladder.length) {
    const [first, ...rest] = taste.ladder.map(tasteModelName);
    const top = tasteModelName(taste.hard_model ?? taste.ladder.at(-1));
    lines.push(TASTE_STRATEGIES.find((s) => s.id === taste.strategy).brief(first, rest, top));
    lines.push(`Send hard work, such as architecture calls or a bug whose cause is unknown, straight to ${top}.`);
  }
  lines.push(TASTE_AUTONOMIES.find((a) => a.id === taste.autonomy).brief);
  if (taste.review_diffs) lines.push("Review each worker's diff before you accept its work.");
  if (taste.second_opinion) lines.push("Have another model review a risky change before it goes ahead.");
  for (const skill of taste.skills) if (skill.enabled) lines.push(`Follow the ${skill.name} skill.`);
  for (const note of taste.context) if (note.trim()) lines.push(note.trim());
  return lines.join("\n");
}

/** The patch without the fields equal to `base`'s, which the scope inherits instead. */
const tastePrune = (patch, base) => Object.fromEntries(Object.entries(patch).filter(([field, value]) => value != null && !tasteSame(value, base[field])));

function tasteWithPatch(tastes, repo, patch) {
  const projects = { ...tastes.projects };
  if (Object.keys(patch).length) projects[repo] = patch;
  else delete projects[repo];
  return { ...tastes, projects };
}

/** The tastes with `fields` set on the scope, as Rust's `Tastes::set`: a project keeps only what
 * differs from the Captains, and a change to the Captains leaves every project's own fields alone. */
function tasteSetFields(tastes, scope, fields) {
  if (scope.kind === "project") return tasteWithPatch(tastes, scope.repo, tastePrune({ ...tastes.projects?.[scope.repo], ...fields }, tastes.captains));
  return { ...tastes, [scope.kind]: { ...tastes[scope.kind], ...fields } };
}

const tasteSetField = (tastes, scope, field, value) => tasteSetFields(tastes, scope, { [field]: value });

/** A project inherits the field again. The orchestrator and the Captains have nothing to inherit. */
function tasteResetField(tastes, scope, field) {
  if (scope.kind !== "project") return tastes;
  const { [field]: _, ...patch } = tastes.projects?.[scope.repo] ?? {};
  return tasteWithPatch(tastes, scope.repo, patch);
}

/** Sets every knob to the preset's and keeps skills and context. "custom" changes nothing. */
function tasteApplyProfile(tastes, scope, profile) {
  if (!TASTE_PRESETS[profile]) return tastes;
  const { skills, context, ...knobs } = TASTE_PRESETS[profile];
  return tasteSetFields(tastes, scope, knobs);
}

/** The fields of `now` that differ from `was`'s, as one patch. */
const tasteChanged = (was, now) => Object.fromEntries(TASTE_FIELDS.filter((field) => now[field] != null && !tasteSame(now[field], was[field])).map((field) => [field, now[field]]));

/** The calls that turn `saved` into `draft`, in order: the orchestrator, the Captains, then each
 * project, whose dropped fields reset before its changed ones are set. Each sends only the fields
 * the draft changed, so a change made elsewhere to another field stands. */
function tasteSaveCalls(saved, draft) {
  const calls = [];
  for (const kind of ["orchestrator", "captains"]) {
    const patch = tasteChanged(saved[kind], draft[kind]);
    if (Object.keys(patch).length) calls.push({ cmd: "set_taste", scope: { kind }, patch });
  }
  const repos = [...new Set([...Object.keys(saved.projects ?? {}), ...Object.keys(draft.projects ?? {})])].sort();
  for (const repo of repos) {
    const scope = { kind: "project", repo };
    const was = saved.projects?.[repo] ?? {};
    const now = draft.projects?.[repo] ?? {};
    for (const field of TASTE_FIELDS) if (was[field] != null && now[field] == null) calls.push({ cmd: "reset_taste_field", scope, field });
    const patch = tasteChanged(was, now);
    if (Object.keys(patch).length) calls.push({ cmd: "set_taste", scope, patch });
  }
  return calls;
}

/** The draft's edits over `saved`, made again over `fresh`, the tastes as they are now: what a
 * reload keeps of unsaved edits while taking every change made elsewhere. */
function tasteRebase(saved, draft, fresh) {
  return tasteSaveCalls(saved, draft).reduce(
    (next, call) => (call.cmd === "reset_taste_field" ? tasteResetField(next, call.scope, call.field) : tasteSetFields(next, call.scope, call.patch)),
    fresh,
  );
}

/** A model's cost tier from hints in its id: Rust's `cost_of` in delegate/catalog.rs. */
function tasteModelTier(id) {
  const lower = String(id).toLowerCase();
  if (["haiku", "flash", "mini", "fast", "nano", "lite"].some((hint) => lower.includes(hint))) return "low";
  if (["opus", "max", "pro", "ultra", "thinking"].some((hint) => lower.includes(hint))) return "high";
  return "mid";
}

/** A rough cost per task, as "$0.27–0.56", from the tier of the model a worker starts on. */
function tasteCost(taste, tierOf = tasteModelTier) {
  const start = taste.strategy === "strongest_first" ? taste.hard_model ?? taste.ladder.at(-1) : taste.ladder[0];
  const base = taste.ladder.length ? TASTE_TIER_COST[tierOf(start.model)] : 0;
  const score = base + (taste.strategy === "escalate_on_failure" ? 0.2 : 0) + (taste.second_opinion ? 0.2 : 0) + (taste.parallel - 1) * 0.03;
  return `$${(score * 0.6).toFixed(2)}–${(score * 1.25).toFixed(2)}`;
}

/** Speed out of 3: more workers at once, and fewer stops on the way. */
function tasteSpeed(taste) {
  return Math.min(3, 1 + (taste.parallel >= 3 ? 1 : 0) + (taste.autonomy === "report_at_end" || taste.strategy === "strongest_first" ? 1 : 0));
}

/** The models the page offers, in two groups: Claude's current ones, then OpenCode Go's, without
 * the ones Claude already lists. Pass the composer's claudeModels() and OPENCODE_GO_MODELS. */
function tasteCatalog(claudeModels, opencodeModels) {
  const claude = claudeModels.filter((m) => !m.legacy);
  const listed = new Set(claude.map((m) => m.slug));
  const pick = ({ slug, name }) => ({ slug, name });
  return [
    { label: "Claude", models: claude.map(pick) },
    { label: "OpenCode Go", models: opencodeModels.filter((m) => !listed.has(m.slug)).map(pick) },
  ];
}

/** One tab per project, by repo: the machines' projects that have one, named as the machine names
 * them, and each of `repos` they lack, named by the repo, so an offline project's changes stay in reach. */
function tasteProjectTabs(projectViews, repos) {
  const names = new Map();
  for (const p of projectViews) if (p.repo && !names.has(p.repo)) names.set(p.repo, p.name);
  for (const repo of repos) if (!names.has(repo)) names.set(repo, repo.split("/").pop());
  return [...names].map(([repo, name]) => ({ repo, name })).sort((a, b) => a.name.localeCompare(b.name) || a.repo.localeCompare(b.repo));
}
