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
    `${read("taste.js")}\nthis.api = { TASTE_PRESETS, tasteProfile, tasteResolve, tasteBrief, tasteSetField, tasteResetField, tasteApplyProfile, tasteSaveCalls, tasteRebase, tasteCost, tasteSpeed, tasteModelTier, tasteCatalog, tasteProjectTabs };`,
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
