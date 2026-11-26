import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

// Objects built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => JSON.parse(JSON.stringify(value));

const taste = () => {
  const context = {};
  vm.createContext(context);
  vm.runInContext(
    `${read("taste.js")}\nthis.api = { TASTE_PRESETS, tasteProfile, tasteResolve, tasteBrief, tasteSetField, tasteResetField, tasteBoxSize, tasteApplyProfile, tasteSaveCalls, tasteRebase, tasteSetProfiles, tasteCost, tasteSpeed, tasteModelTier, tasteCatalog, tasteProjectTabs, placementMachines, placementAdd, placementRemove, placementMove, placementRows, placementLoad, tasteProposal, tasteValueText };`,
    context,
  );
  return context.api;
};

// Rust writes this file from its own presets and briefs, and a Rust test holds it to them.
const fixture = JSON.parse(read("taste.fixture.json"));
const SITE = "github.com/acme/site";
const site = { kind: "project", repo: SITE };
const captains = { kind: "captains" };
const opus = { model: "claude-opus-5-5" };
/** Rust's `Tastes::default()`: the balanced preset everywhere and no project patches. */
const defaults = (projects = {}) => ({ orchestrator: fixture.presets.balanced, captains: fixture.presets.balanced, projects });

test("each brief is the text Rust writes for the same taste", () => {
  const { tasteBrief } = taste();
  assert.equal(fixture.briefs.length, 8);
  for (const { taste: t, brief } of fixture.briefs) assert.equal(tasteBrief(t), brief);
});

test("the presets are Rust's presets", () => {
  const { TASTE_PRESETS } = taste();
  assert.deepEqual(here(TASTE_PRESETS), fixture.presets);
});

test("each preset profiles as itself, a changed knob makes it custom and context alone does not", () => {
  const { tasteProfile } = taste();
  assert.deepEqual(["economical", "balanced", "aggressive"].map((p) => tasteProfile(fixture.presets[p])), ["economical", "balanced", "aggressive"]);
  const noted = { ...fixture.presets.economical, context: ["Ship on Fridays."], skills: [{ name: "tdd", enabled: true }] };
  assert.equal(tasteProfile(noted), "economical");
  assert.equal(tasteProfile({ ...noted, parallel: 4 }), "custom");
  assert.equal(tasteProfile({ ...noted, ladder: [{ model: "deepseek-v4.1-flash" }, { model: "glm-5.3" }] }), "custom");
  const reordered = { ...fixture.presets.aggressive, ladder: [{ provider: undefined, model: "claude-opus-5-5" }], hard_model: { model: "claude-opus-5-5" } };
  assert.equal(tasteProfile(reordered), "aggressive", "a provider left undefined is no provider");
});

test("a project keeps only what differs from the Captains, merges later sets and drops an empty patch", () => {
  const { tasteSetField } = taste();
  let tastes = tasteSetField(defaults(), site, "parallel", 4);
  tastes = tasteSetField(tastes, site, "autonomy", "decide_and_tell");
  assert.deepEqual(here(tastes.projects), { [SITE]: { parallel: 4 } });
  tastes = tasteSetField(tastes, site, "second_opinion", true);
  assert.deepEqual(here(tastes.projects), { [SITE]: { parallel: 4, second_opinion: true } });
  tastes = tasteSetField(tasteSetField(tastes, site, "parallel", 5), site, "second_opinion", false);
  assert.deepEqual(here(tastes.projects), {});
  assert.deepEqual(here(tastes.captains), fixture.presets.balanced, "a project edit leaves the Captains alone");
});

test("a project resolves as the Captains with its patch on top", () => {
  const { tasteResolve } = taste();
  const tastes = { ...defaults({ [SITE]: { parallel: 5, autonomy: "report_at_end" } }), captains: fixture.presets.economical };
  assert.deepEqual(here(tasteResolve(tastes, site)), {
    strategy: "cheap_with_guidance",
    ladder: [{ model: "deepseek-v4.1-flash" }, { model: "glm-5.3" }, opus],
    hard_model: opus,
    parallel: 5,
    autonomy: "report_at_end",
    review_diffs: true,
    second_opinion: false,
    skills: [],
    context: [],
  });
  assert.equal(tasteResolve(tastes, { kind: "project", repo: "github.com/acme/api" }).parallel, 2, "a project with no patch is the Captains'");
  assert.equal(tasteResolve(tastes, { kind: "orchestrator" }).parallel, 5);
});

test("a Captains edit leaves each project's own fields alone, as Rust's set does", () => {
  const { tasteSetField, tasteApplyProfile, tasteResolve } = taste();
  const start = defaults({ [SITE]: { parallel: 4, review_diffs: false, autonomy: "report_at_end" } });
  let tastes = start;
  for (const parallel of [4, 5, 6]) tastes = tasteSetField(tastes, captains, "parallel", parallel);
  assert.equal(tastes.captains.parallel, 6);
  assert.equal(tasteResolve(tastes, site).parallel, 4, "stepping the Captains past the project's value kept the project's");
  tastes = tasteSetField(tasteSetField(tastes, captains, "review_diffs", false), captains, "review_diffs", true);
  tastes = tasteApplyProfile(tasteApplyProfile(tastes, captains, "aggressive"), captains, "balanced");
  assert.deepEqual(here(tastes.projects), here(start.projects));
});

test("a reset makes a project inherit the field again, and the orchestrator has nothing to reset", () => {
  const { tasteResetField } = taste();
  const tastes = defaults({ [SITE]: { parallel: 4, review_diffs: false } });
  const once = tasteResetField(tastes, site, "parallel");
  assert.deepEqual(here(once.projects), { [SITE]: { review_diffs: false } });
  assert.deepEqual(here(tasteResetField(once, site, "review_diffs").projects), {});
  assert.deepEqual(here(tasteResetField(tastes, { kind: "orchestrator" }, "parallel")), here(tastes));
});

test("a profile sets every knob and keeps skills and context, pruned on a project", () => {
  const { tasteApplyProfile } = taste();
  const noted = { ...fixture.presets.balanced, context: ["Deploys go through the VPS."] };
  const orchestrated = tasteApplyProfile({ ...defaults(), orchestrator: noted }, { kind: "orchestrator" }, "economical");
  assert.deepEqual(here(orchestrated.orchestrator), { ...fixture.presets.economical, context: ["Deploys go through the VPS."] });

  const aggressive = tasteApplyProfile(defaults({ [SITE]: { parallel: 4, context: ["Releases go out on Fridays."] } }), site, "aggressive");
  assert.deepEqual(here(aggressive.projects), {
    [SITE]: {
      parallel: 8,
      context: ["Releases go out on Fridays."],
      strategy: "strongest_first",
      ladder: [opus],
      autonomy: "report_at_end",
      review_diffs: false,
      second_opinion: true,
    },
  });
  const balanced = tasteApplyProfile(aggressive, site, "balanced");
  assert.deepEqual(here(balanced.projects), { [SITE]: { context: ["Releases go out on Fridays."] } });
  assert.deepEqual(here(tasteApplyProfile(balanced, site, "custom")), here(balanced));
});

test("an unchanged draft saves nothing", () => {
  const { tasteSaveCalls } = taste();
  const saved = defaults({ [SITE]: { parallel: 4 } });
  assert.deepEqual(here(tasteSaveCalls(saved, JSON.parse(JSON.stringify(saved)))), []);
});

test("a Captains change sends only the fields it changed and touches no project", () => {
  const { tasteSaveCalls, tasteSetField } = taste();
  const saved = defaults({ [SITE]: { parallel: 4, review_diffs: false } });
  const draft = tasteSetField(tasteSetField(saved, captains, "parallel", 4), captains, "second_opinion", true);
  assert.deepEqual(here(tasteSaveCalls(saved, draft)), [{ cmd: "set_taste", scope: { kind: "captains" }, patch: { parallel: 4, second_opinion: true } }]);
  const undone = tasteSetField(tasteSetField(saved, captains, "review_diffs", false), captains, "review_diffs", true);
  assert.deepEqual(here(tasteSaveCalls(saved, undone)), [], "an undone toggle saves nothing");
});

/** Rust's default escalation settings, which the fixture's presets leave out. */
const escalation = { escalate_breaking_change: true, escalate_process_failure: true, escalate_over_lines: 500, escalate_areas: ["Release and version bumps", "CI and deploy config", "Sign-in, tokens and secrets"] };
const escalating = (projects = {}) => ({ ...defaults(projects), orchestrator: { ...fixture.presets.balanced, ...escalation }, captains: { ...fixture.presets.balanced, ...escalation } });

test("a changed escalation setting saves with the Captains, and only what changed", () => {
  const { tasteSaveCalls, tasteSetField } = taste();
  const saved = escalating();
  let draft = tasteSetField(saved, captains, "escalate_over_lines", 750);
  draft = tasteSetField(draft, captains, "escalate_areas", []);
  draft = tasteSetField(tasteSetField(draft, captains, "escalate_breaking_change", false), captains, "escalate_breaking_change", true);
  assert.deepEqual(here(tasteSaveCalls(saved, draft)), [{ cmd: "set_taste", scope: { kind: "captains" }, patch: { escalate_over_lines: 750, escalate_areas: [] } }]);
});

test("a project overrides an escalation setting, and Reset makes it inherit again", () => {
  const { tasteSaveCalls, tasteSetField, tasteResetField, tasteResolve } = taste();
  const saved = escalating();
  const draft = tasteSetField(saved, site, "escalate_process_failure", false);
  assert.deepEqual(here(draft.projects), { [SITE]: { escalate_process_failure: false } });
  assert.equal(tasteResolve(draft, site).escalate_process_failure, false);
  assert.deepEqual(here(tasteSaveCalls(saved, draft)), [{ cmd: "set_taste", scope: site, patch: { escalate_process_failure: false } }]);
  const back = tasteResetField(draft, site, "escalate_process_failure");
  assert.deepEqual(here(back.projects), {});
  assert.equal(tasteResolve(back, site).escalate_process_failure, true);
  const stored = escalating({ [SITE]: { escalate_over_lines: 90 } });
  assert.deepEqual(here(tasteSaveCalls(stored, tasteResetField(stored, site, "escalate_over_lines"))), [{ cmd: "reset_taste_field", scope: site, field: "escalate_over_lines" }]);
});

test("the escalation settings leave the profile as it was, and a profile leaves them alone", () => {
  const { tasteProfile, tasteApplyProfile } = taste();
  const changed = { ...fixture.presets.economical, ...escalation, escalate_breaking_change: false, escalate_over_lines: 42, escalate_areas: [] };
  assert.equal(tasteProfile(changed), "economical");
  const applied = tasteApplyProfile({ ...escalating(), captains: changed }, captains, "aggressive");
  assert.deepEqual([applied.captains.escalate_breaking_change, applied.captains.escalate_over_lines, here(applied.captains.escalate_areas)], [false, 42, []]);
});

/** Rust's default Manage only, which the fixture's presets leave out. */
const managing = (projects = {}) => ({ ...defaults(projects), orchestrator: { ...fixture.presets.balanced, manage_only: false }, captains: { ...fixture.presets.balanced, manage_only: false } });

test("manage_only saves with the Captains or a project, and a project's Reset makes it inherit again", () => {
  const { tasteSaveCalls, tasteSetField, tasteResetField, tasteResolve } = taste();
  const saved = managing();
  assert.deepEqual(here(tasteSaveCalls(saved, tasteSetField(saved, captains, "manage_only", true))), [{ cmd: "set_taste", scope: { kind: "captains" }, patch: { manage_only: true } }]);
  const draft = tasteSetField(saved, site, "manage_only", true);
  assert.deepEqual(here(draft.projects), { [SITE]: { manage_only: true } });
  assert.equal(tasteResolve(draft, site).manage_only, true);
  assert.deepEqual(here(tasteSaveCalls(saved, draft)), [{ cmd: "set_taste", scope: site, patch: { manage_only: true } }]);
  const stored = managing({ [SITE]: { manage_only: true } });
  const back = tasteResetField(stored, site, "manage_only");
  assert.equal(tasteResolve(back, site).manage_only, false);
  assert.deepEqual(here(tasteSaveCalls(stored, back)), [{ cmd: "reset_taste_field", scope: site, field: "manage_only" }]);
});

test("manage_only leaves the profile as it was, and a profile leaves it alone", () => {
  const { tasteProfile, tasteApplyProfile } = taste();
  for (const id of ["economical", "balanced", "aggressive"]) assert.equal(tasteProfile({ ...fixture.presets[id], manage_only: true }), id);
  const applied = tasteApplyProfile({ ...managing(), captains: { ...fixture.presets.economical, manage_only: true } }, captains, "aggressive");
  assert.deepEqual([tasteProfile(applied.captains), applied.captains.manage_only], ["aggressive", true]);
  const project = tasteApplyProfile(managing({ [SITE]: { manage_only: true } }), site, "aggressive");
  assert.equal(project.projects[SITE].manage_only, true, "a project's own stays");
});

test("a rebase makes unsaved edits again over the tastes another device saved", () => {
  const { tasteRebase, tasteSetField, tasteResetField } = taste();
  const saved = defaults({ [SITE]: { parallel: 4, review_diffs: false } });
  let draft = tasteSetField(saved, { kind: "orchestrator" }, "autonomy", "ask_first");
  draft = tasteResetField(draft, site, "review_diffs");
  const fresh = { ...defaults({ [SITE]: { parallel: 4, review_diffs: false, context: ["Releases go out on Fridays."] } }), captains: { ...fixture.presets.balanced, parallel: 6 } };
  const rebased = tasteRebase(saved, draft, fresh);
  assert.deepEqual(here(rebased.projects), { [SITE]: { parallel: 4, context: ["Releases go out on Fridays."] } });
  assert.equal(rebased.captains.parallel, 6);
  assert.equal(rebased.orchestrator.autonomy, "ask_first");
});

test("a project override and a project reset save as a reset, then one set of the changed fields", () => {
  const { tasteSaveCalls, tasteSetField, tasteResetField } = taste();
  const saved = defaults({ [SITE]: { parallel: 4, review_diffs: false } });
  let draft = tasteResetField(saved, site, "review_diffs");
  draft = tasteSetField(draft, site, "autonomy", "ask_first");
  draft = tasteSetField(draft, { kind: "orchestrator" }, "second_opinion", true);
  assert.deepEqual(here(tasteSaveCalls(saved, draft)), [
    { cmd: "set_taste", scope: { kind: "orchestrator" }, patch: { second_opinion: true } },
    { cmd: "reset_taste_field", scope: { kind: "project", repo: SITE }, field: "review_diffs" },
    { cmd: "set_taste", scope: { kind: "project", repo: SITE }, patch: { autonomy: "ask_first" } },
  ]);
});

test("cost and speed for each preset, and an empty ladder costs only its extras", () => {
  const { tasteCost, tasteSpeed } = taste();
  const { economical, balanced, aggressive } = fixture.presets;
  assert.deepEqual([economical, balanced, aggressive].map((t) => tasteCost(t)), ["$0.29–0.60", "$0.70–1.46", "$1.33–2.76"]);
  assert.deepEqual([economical, balanced, aggressive].map(tasteSpeed), [1, 2, 3]);
  assert.equal(tasteCost({ ...economical, parallel: 1 }), "$0.27–0.56");
  assert.equal(tasteCost({ ...economical, ladder: [], second_opinion: true }), "$0.14–0.29");
  assert.equal(tasteCost(economical, () => "high"), "$1.10–2.29", "the tier lookup comes from the caller");
});

test("a model's tier is the cost its machine lists, and a guess from words of its id for any other", () => {
  const { tasteModelTier, tasteCost } = taste();
  const ids = ["deepseek-v4.1-flash", "claude-haiku-5-5", "o4-mini", "glm-5.3", "gpt-6-luna", "claude-opus-5-5", "kimi-k3.1-pro", "Qwen3.8-MAX", "minimax-m3", "claude-opus-fast"];
  assert.deepEqual(ids.map((id) => tasteModelTier(id)), ["low", "low", "low", "mid", "mid", "high", "high", "high", "mid", "high"], "an older node sends no list");
  const listed = [
    { slug: "claude-fable-5-3", name: "Claude Fable 5.3", cost: "mid" },
    { slug: "claude-haiku-6", name: "Claude Haiku 6", cost: "low" },
    { slug: "claude-opus-5-5", name: "Claude Opus 5.5", cost: "mid" },
    { slug: "claude-sonnet-5-5", name: "Claude Sonnet 5.5" },
  ];
  assert.deepEqual(["claude-fable-5-3", "claude-haiku-6", "claude-opus-5-5"].map((id) => tasteModelTier(id, listed)), ["mid", "low", "mid"], "the list wins over the words of the id");
  assert.equal(tasteModelTier("claude-sonnet-5-5", listed), "mid", "a listed model with no cost falls back to its id");
  assert.equal(tasteModelTier("glm-5.3-flash", listed), "low", "a model the list lacks falls back to its id");
  const { economical } = fixture.presets;
  const priced = (cost) => tasteCost(economical, (id) => tasteModelTier(id, [{ slug: id, cost }]));
  for (const tier of ["low", "mid", "high"]) assert.equal(priced(tier), tasteCost(economical, () => tier), `the cost estimate follows the listed tier ${tier}`);
});

test("the catalog offers current Claude models, then OpenCode Go's that Claude does not list", () => {
  const { tasteCatalog } = taste();
  const claude = [
    { slug: "claude-opus-5-5", name: "Claude Opus 5.5", efforts: [] },
    { slug: "claude-haiku-5-5", name: "Claude Haiku 5.5" },
    { slug: "claude-opus-5", name: "Claude Opus 5", legacy: true },
  ];
  const opencode = [
    { slug: "glm-5.3", name: "GLM-5.3", provider: "opencode-go", legacy: true },
    { slug: "claude-haiku-5-5", name: "Claude Haiku 5.5", provider: "opencode-go", legacy: true },
  ];
  assert.deepEqual(here(tasteCatalog(claude, opencode)), [
    { label: "Claude", models: [{ slug: "claude-opus-5-5", name: "Claude Opus 5.5" }, { slug: "claude-haiku-5-5", name: "Claude Haiku 5.5" }] },
    { label: "OpenCode Go", models: [{ slug: "glm-5.3", name: "GLM-5.3" }] },
  ]);
});

test("project tabs: one per repo, named by the machine, plus repos only the tastes know", () => {
  const { tasteProjectTabs } = taste();
  const views = [
    { path: "/Users/f/site", name: "site", repo: SITE },
    { path: "/Users/f/site-2", name: "site copy", repo: SITE },
    { path: "/Users/f/notes", name: "notes" },
    { path: "/Users/f/api", name: "api", repo: "github.com/acme/api" },
  ];
  assert.deepEqual(here(tasteProjectTabs(views, ["github.com/acme/infra", SITE])), [
    { repo: "github.com/acme/api", name: "api" },
    { repo: "github.com/acme/infra", name: "infra" },
    { repo: SITE, name: "site" },
  ]);
});

const policy = (over = {}) => ({ prefer: [], avoid: [], heavy: "worktree", ...over });

test("a machine joins Prefer or Avoid once, and never both", () => {
  const { placementAdd } = taste();
  const start = policy({ prefer: ["mini"], avoid: ["book"] });
  assert.deepEqual(here(placementAdd(start, "prefer", "vps")), policy({ prefer: ["mini", "vps"], avoid: ["book"] }));
  assert.deepEqual(here(placementAdd(start, "avoid", "mini")), start, "a preferred machine is not avoided as well");
  assert.deepEqual(here(placementAdd(start, "prefer", "book")), start, "an avoided machine is not preferred as well");
  assert.deepEqual(here(placementAdd(start, "prefer", "mini")), start, "a machine is not listed twice");
  assert.deepEqual(here(placementAdd(start, "prefer", "")), start);
});

test("Prefer keeps its order, and a move at an end changes nothing", () => {
  const { placementMove, placementRemove } = taste();
  const start = policy({ prefer: ["a", "b", "c"], avoid: ["z"] });
  assert.deepEqual(here(placementMove(start, "c", -1)).prefer, ["a", "c", "b"]);
  assert.deepEqual(here(placementMove(start, "a", 1)).prefer, ["b", "a", "c"]);
  assert.deepEqual(here(placementMove(start, "a", -1)), start);
  assert.deepEqual(here(placementMove(start, "c", 1)), start);
  assert.deepEqual(here(placementRemove(start, "prefer", "b")), policy({ prefer: ["a", "c"], avoid: ["z"] }));
});

test("a listed machine shows by name, and one the account lost shows struck out once the machines are known", () => {
  const { placementMachines, placementRows, placementLoad } = taste();
  const machines = placementMachines([
    { id: "m1", name: "mac-mini", online: true, state: { workers: 2, workers_at_once: 5 } },
    { id: "m2", name: "lypes-vps", online: false },
    { id: "box", name: "a box", online: true, upstash_box: {} },
    { name: "no id" },
  ]);
  assert.deepEqual(here(machines.map((m) => m.id)), ["m1", "m2"], "boxes and unidentified views are not machines");
  const rows = placementRows(["m2", "m9", "m1"], machines);
  assert.deepEqual(here(rows.map(({ name, online, gone }) => ({ name, online, gone }))), [
    { name: "lypes-vps", online: false, gone: false },
    { name: "m9", online: false, gone: true },
    { name: "mac-mini", online: true, gone: false },
  ]);
  assert.deepEqual(here(placementRows(["m9"], [])).map((m) => m.gone), [false], "with no machine listed yet, nothing is called removed");
  assert.deepEqual(rows.map(placementLoad), ["", "", "2 of 5 workers"]);
});

test("a changed policy saves in the Captains' call, beside their other changes, and an unchanged one saves nothing", () => {
  const { tasteSaveCalls, tasteSetField } = taste();
  const saved = { ...defaults(), placement: policy({ prefer: ["mini"] }) };
  assert.deepEqual(here(tasteSaveCalls(saved, JSON.parse(JSON.stringify(saved)))), []);
  const moved = { ...saved, placement: policy({ prefer: ["mini", "vps"], avoid: ["book"], heavy: "all_workers" }) };
  assert.deepEqual(here(tasteSaveCalls(saved, moved)), [{ cmd: "set_taste", scope: captains, patch: { placement: moved.placement } }]);
  const both = tasteSetField(moved, captains, "parallel", 4);
  assert.deepEqual(here(tasteSaveCalls(saved, both)), [{ cmd: "set_taste", scope: captains, patch: { parallel: 4, placement: moved.placement } }]);
  assert.deepEqual(here(tasteSaveCalls(defaults(), defaults())), [], "a hub that sends no policy saves none");
});

test("a rebase keeps an unsaved policy over what another device saved", () => {
  const { tasteRebase, tasteSetField } = taste();
  const saved = { ...defaults(), placement: policy() };
  const draft = { ...saved, placement: policy({ avoid: ["book"] }) };
  const fresh = { ...tasteSetField(saved, captains, "parallel", 6), placement: policy({ prefer: ["mini"] }) };
  const rebased = tasteRebase(saved, draft, fresh);
  assert.deepEqual(here(rebased.placement), policy({ avoid: ["book"] }));
  assert.equal(rebased.captains.parallel, 6);
});

test("the box size is the project's, else the account's, else small; the account can clear it", () => {
  const { tasteBoxSize, tasteSetField, tasteResetField, tasteSaveCalls } = taste();
  const captains = { kind: "captains" };
  let tastes = defaults({ [SITE]: { parallel: 4 } });
  tastes = { ...tastes, captains: { ...tastes.captains, box_size: null } };
  assert.deepEqual(here(tasteBoxSize(tastes, site)), { size: "small", from: "built_in" });
  const account = tasteSetField(tastes, captains, "box_size", "medium");
  assert.deepEqual(here(tasteBoxSize(account, site)), { size: "medium", from: "account" });
  const own = tasteSetField(account, site, "box_size", "large");
  assert.deepEqual(here(tasteBoxSize(own, site)), { size: "large", from: "project" });
  assert.deepEqual(here(tasteBoxSize(own, captains)), { size: "medium", from: "account" });
  assert.deepEqual(here(tasteBoxSize(tasteSetField(account, site, "box_size", "medium"), site)), { size: "medium", from: "account" }, "a project set to the account's size inherits it");
  assert.deepEqual(here(tasteSaveCalls(tastes, account)), [{ cmd: "set_taste", scope: captains, patch: { box_size: "medium" } }]);
  assert.deepEqual(here(tasteSaveCalls(account, tasteResetField(account, captains, "box_size"))), [{ cmd: "reset_taste_field", scope: captains, field: "box_size" }]);
});

test("an agent's change of the Captains reads as each field's label, its value now and the proposed one, in the page's words", () => {
  const { tasteProposal } = taste();
  const proposal = tasteProposal(defaults(), "set_taste", { scope: captains, patch: { parallel: 3, autonomy: "ask_first" } });
  assert.deepEqual(here(proposal), {
    scope: "Captains (all)",
    rows: [
      { field: "parallel", label: "Workers at once", now: "5", next: "3" },
      { field: "autonomy", label: "Autonomy", now: "Decide and tell", next: "Ask first" },
    ],
  });
  assert.equal(tasteProposal(defaults(), "set_taste", { scope: { kind: "orchestrator" }, patch: {} }).scope, "the Orchestrator");
});

test("a project's change shows the value it has now, its own or the Captains' it inherits", () => {
  const { tasteProposal } = taste();
  const asked = { scope: site, patch: { parallel: 3 } };
  assert.deepEqual(here(tasteProposal(defaults(), "set_taste", asked)), { scope: "Captain site", rows: [{ field: "parallel", label: "Workers at once", now: "5", next: "3" }] });
  assert.equal(tasteProposal(defaults({ [SITE]: { parallel: 4 } }), "set_taste", asked).rows[0].now, "4");
});

test("a reset reads as what the project inherits from the Captains, or the built-in default for the Captains", () => {
  const { tasteProposal } = taste();
  const tastes = defaults({ [SITE]: { parallel: 4 } });
  assert.deepEqual(here(tasteProposal(tastes, "reset_taste_field", { scope: site, field: "parallel" }).rows), [{ field: "parallel", label: "Workers at once", now: "4", next: "Inherits 5 from Captains (all)" }]);
  assert.deepEqual(here(tasteProposal(tastes, "reset_taste_field", { scope: captains, field: "parallel" }).rows), [{ field: "parallel", label: "Workers at once", now: "5", next: "Built-in default" }]);
});

test("a field the page has no words for shows by its JSON name and value, and without tastes the value now is unknown", () => {
  const { tasteProposal } = taste();
  assert.deepEqual(here(tasteProposal(defaults(), "set_taste", { scope: captains, patch: { bogus: { deep: [1] } } }).rows), [{ field: "bogus", label: "bogus", now: "Not set", next: '{"deep":[1]}' }]);
  assert.deepEqual(here(tasteProposal(defaults(), "set_taste", { scope: captains, patch: { toString: 1 } }).rows), [{ field: "toString", label: "toString", now: "Not set", next: "1" }], "a key inherited by every object is still only a name");
  assert.deepEqual(here(tasteProposal(null, "set_taste", { scope: captains, patch: { parallel: 3 } }).rows), [{ field: "parallel", label: "Workers at once", now: "unknown", next: "3" }]);
  assert.deepEqual(here(tasteProposal(null, "reset_taste_field", { scope: site, field: "parallel" }).rows), [{ field: "parallel", label: "Workers at once", now: "unknown", next: "Inherits from Captains (all)" }]);
});

test("args it can't read give no rows and never throw: no scope, an unknown scope, no patch, a patch that is no object, no field", () => {
  const { tasteProposal } = taste();
  const none = (action, args) => here(tasteProposal(defaults(), action, args));
  assert.deepEqual(none("set_taste", { patch: { parallel: 3 } }), { scope: null, rows: [] });
  assert.deepEqual(none("set_taste", { scope: { kind: "team" }, patch: { parallel: 3 } }), { scope: null, rows: [] });
  assert.deepEqual(none("set_taste", { scope: { kind: "project" }, patch: { parallel: 3 } }), { scope: null, rows: [] });
  assert.deepEqual(none("set_taste", null), { scope: null, rows: [] });
  assert.deepEqual(none("set_taste", { scope: captains }), { scope: "Captains (all)", rows: [] });
  assert.deepEqual(none("set_taste", { scope: captains, patch: "parallel" }), { scope: "Captains (all)", rows: [] });
  assert.deepEqual(none("set_taste", { scope: captains, patch: [3] }), { scope: "Captains (all)", rows: [] });
  assert.deepEqual(none("reset_taste_field", { scope: captains }), { scope: "Captains (all)", rows: [] });
});

test("each value reads as the page words it, and one it doesn't know as its JSON", () => {
  const { tasteValueText } = taste();
  const cases = [
    ["strategy", "strongest_first", "Strongest first"],
    ["strategy", "bogus", '"bogus"'],
    ["ladder", [{ model: "glm-5.3" }, { model: "kimi", provider: "opencode" }], "glm-5.3, kimi through opencode"],
    ["ladder", "glm-5.3", '"glm-5.3"'],
    ["hard_model", null, "The top rung of the ladder"],
    ["hard_model", { model: "claude-opus-5-5" }, "claude-opus-5-5"],
    ["parallel", 3, "3"],
    ["parallel", "3", '"3"'],
    ["tier", "mid", "Mid"],
    ["default_effort", null, "The model's own"],
    ["default_effort", "high", "high"],
    ["max_effort", "max", "max"],
    ["budget_cents", 250, "US$ 2.50"],
    ["budget_cents", 0, "No cap"],
    ["box_budget_cents", 500, "US$ 5.00"],
    ["time_limit_min", 90, "1 h 30 min"],
    ["box_size", null, "Small (built-in)"],
    ["box_size", "large", "Large"],
    ["skills", [{ name: "tdd", enabled: true }, { name: "docs", enabled: false }], "tdd, docs (off)"],
    ["skills", [], "None"],
    ["context", ["Ship on Fridays.", "Small PRs."], "Ship on Fridays. · Small PRs."],
    ["context", [], "None"],
    ["review_diffs", false, "Off"],
    ["manage_only", true, "On"],
    ["escalate_over_lines", 750, "750"],
    ["default_model", null, "The CLI's own"],
    ["default_model", { model: "claude-haiku-5-5" }, "claude-haiku-5-5"],
  ];
  for (const [field, value, words] of cases) assert.equal(tasteValueText(field, value), words, `${field}: ${JSON.stringify(value)}`);
});


const scout = { name: "scout", skills: [], tools: ["Read"], access: "read_only", delivers: "A map.", by: "user", prompt: "You map code." };
const fixer = { name: "fixer", skills: [{ name: "tdd", enabled: true }], tools: [], access: "edit", model: { model: "glm-5.3" }, effort: "high", delivers: "", by: "user", prompt: "" };
const profiled = (projects = {}) => ({ ...defaults(), worker_profiles: { account: [scout], projects, proposed: [{ ...fixer, name: "tester", by: "orchestrator" }] } });

test("an unchanged draft sends no set_worker_profiles, and a hub without profiles none at all", () => {
  const { tasteSaveCalls, tasteSetField } = taste();
  const saved = profiled({ [SITE]: [fixer] });
  assert.deepEqual(here(tasteSaveCalls(saved, JSON.parse(JSON.stringify(saved)))), []);
  assert.deepEqual(here(tasteSaveCalls(defaults(), tasteSetField(defaults(), captains, "parallel", 4))), [{ cmd: "set_taste", scope: captains, patch: { parallel: 4 } }]);
});

test("a changed account list saves after the set_taste calls, and a changed project list names its project", () => {
  const { tasteSaveCalls, tasteSetField, tasteSetProfiles } = taste();
  const saved = profiled({ [SITE]: [fixer] });
  let draft = tasteSetField(saved, captains, "parallel", 4);
  draft = tasteSetProfiles(draft, null, [scout, { ...fixer, name: "builder" }]);
  draft = tasteSetProfiles(draft, "github.com/acme/api", [scout]);
  draft = tasteSetProfiles(draft, SITE, []);
  assert.deepEqual(here(draft.worker_profiles.projects), { "github.com/acme/api": [scout] }, "an emptied project has no list");
  assert.deepEqual(here(tasteSaveCalls(saved, draft)), [
    { cmd: "set_taste", scope: captains, patch: { parallel: 4 } },
    { cmd: "set_worker_profiles", profiles: [scout, { ...fixer, name: "builder" }] },
    { cmd: "set_worker_profiles", project: "github.com/acme/api", profiles: [scout] },
    { cmd: "set_worker_profiles", project: SITE, profiles: [] },
  ]);
});

test("an emptied project list that was never saved sends nothing", () => {
  const { tasteSaveCalls, tasteSetProfiles } = taste();
  const saved = profiled();
  assert.deepEqual(here(tasteSaveCalls(saved, tasteSetProfiles(tasteSetProfiles(saved, SITE, [fixer]), SITE, []))), []);
  assert.deepEqual(here(tasteSaveCalls(saved, { ...saved, worker_profiles: { ...saved.worker_profiles, projects: { [SITE]: [] } } })), [], "a missing list and an empty one are the same");
});

test("a rebase keeps an unsaved profile edit over the profiles another device saved", () => {
  const { tasteRebase, tasteSetField, tasteSetProfiles } = taste();
  const saved = profiled();
  const draft = tasteSetProfiles(saved, null, [{ ...scout, prompt: "You map code, and only that." }]);
  const fresh = tasteSetProfiles(tasteSetField(saved, captains, "parallel", 6), SITE, [fixer]);
  const rebased = tasteRebase(saved, draft, fresh);
  assert.deepEqual(here(rebased.worker_profiles.account), [{ ...scout, prompt: "You map code, and only that." }]);
  assert.deepEqual(here(rebased.worker_profiles.projects), { [SITE]: [fixer] });
  assert.equal(rebased.captains.parallel, 6);
  assert.deepEqual(here(rebased.worker_profiles.proposed), here(saved.worker_profiles.proposed));
});

/** Tastes from a hub that has the default model, unset everywhere. */
const modelled = (projects = {}) => ({ ...defaults(projects), orchestrator: { ...fixture.presets.balanced, default_model: null }, captains: { ...fixture.presets.balanced, default_model: null } });

test("a default model saves with the Captains, clears with a reset, and a project inherits it unless it sets its own", () => {
  const { tasteSaveCalls, tasteSetField, tasteResetField, tasteResolve, tasteProfile } = taste();
  const sonnet = { model: "claude-sonnet-5-5" };
  const haiku = { model: "claude-haiku-5-5" };
  const saved = modelled();
  const set = tasteSetField(saved, captains, "default_model", sonnet);
  assert.deepEqual(here(tasteSaveCalls(saved, set)), [{ cmd: "set_taste", scope: { kind: "captains" }, patch: { default_model: { model: "claude-sonnet-5-5" } } }]);
  assert.equal(tasteProfile(set.captains), "balanced", "no profile sets or compares it");
  for (const cleared of [tasteResetField(set, captains, "default_model"), tasteSetField(set, captains, "default_model", null)]) {
    assert.equal(cleared.captains.default_model, null);
    assert.deepEqual(here(tasteSaveCalls(set, cleared)), [{ cmd: "reset_taste_field", scope: { kind: "captains" }, field: "default_model" }]);
  }

  assert.deepEqual(here(tasteSetField(set, site, "default_model", sonnet).projects), {}, "a project set to the Captains' model inherits it");
  const own = tasteSetField(set, site, "default_model", haiku);
  assert.deepEqual(here(own.projects), { [SITE]: { default_model: haiku } });
  assert.deepEqual(here(tasteResolve(own, site).default_model), haiku);
  assert.deepEqual(here(tasteResolve(own, { kind: "project", repo: "github.com/acme/api" }).default_model), sonnet, "another project keeps the Captains'");
  assert.deepEqual(here(tasteSetField(own, site, "default_model", null).projects), {}, "null on a project inherits");
  assert.deepEqual(here(tasteSaveCalls(own, tasteResetField(own, site, "default_model"))), [{ cmd: "reset_taste_field", scope: site, field: "default_model" }]);
});

const remembering = (projects = {}) => ({ ...defaults(projects), orchestrator: { ...fixture.presets.balanced, claude_memory: true, worker_claude_memory: true }, captains: { ...fixture.presets.balanced, claude_memory: true, worker_claude_memory: true } });

test("the Claude Code memory switches save with their scope, and a project's Reset makes them inherit again", () => {
  const { tasteSaveCalls, tasteSetField, tasteResetField, tasteResolve } = taste();
  const saved = remembering();
  for (const field of ["claude_memory", "worker_claude_memory"]) {
    assert.deepEqual(here(tasteSaveCalls(saved, tasteSetField(saved, captains, field, false))), [{ cmd: "set_taste", scope: { kind: "captains" }, patch: { [field]: false } }]);
    const draft = tasteSetField(saved, site, field, false);
    assert.deepEqual(here(draft.projects), { [SITE]: { [field]: false } });
    assert.equal(tasteResolve(draft, site)[field], false);
    const stored = remembering({ [SITE]: { [field]: false } });
    const back = tasteResetField(stored, site, field);
    assert.equal(tasteResolve(back, site)[field], true);
    assert.deepEqual(here(tasteSaveCalls(stored, back)), [{ cmd: "reset_taste_field", scope: site, field }]);
  }
});

test("the Claude Code memory switches leave the profile as it was, and a profile leaves them alone", () => {
  const { tasteProfile, tasteApplyProfile } = taste();
  const off = { claude_memory: false, worker_claude_memory: false };
  for (const id of ["economical", "balanced", "aggressive"]) assert.equal(tasteProfile({ ...fixture.presets[id], ...off }), id);
  const applied = tasteApplyProfile({ ...remembering(), captains: { ...fixture.presets.economical, ...off } }, captains, "aggressive");
  assert.deepEqual([tasteProfile(applied.captains), applied.captains.claude_memory, applied.captains.worker_claude_memory], ["aggressive", false, false]);
  const project = tasteApplyProfile(remembering({ [SITE]: { claude_memory: false } }), site, "aggressive");
  assert.equal(project.projects[SITE].claude_memory, false, "a project's own stays");
});

test("an agent's change of the Claude Code memory switches reads as their labels and On or Off", () => {
  const { tasteProposal } = taste();
  const proposal = tasteProposal(remembering(), "set_taste", { scope: captains, patch: { claude_memory: false, worker_claude_memory: false } });
  assert.deepEqual(here(proposal.rows), [
    { field: "claude_memory", label: "Claude Code memory", now: "On", next: "Off" },
    { field: "worker_claude_memory", label: "Claude Code memory for workers", now: "On", next: "Off" },
  ]);
});
