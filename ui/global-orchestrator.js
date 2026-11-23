// The global orchestrator's account-wide settings, as pure logic: the merge of every machine's
// answer into the one view Felipe edits, the host choice, and the writes a save sends. Kept here
// so node --test can drive it without the page; gui-global-orchestrator.js renders and wires it.

/** The names the settings offer. Anything else is custom text, which the node cleans. */
const GO_NAME_PRESETS = ["Orchestrator", "Admiral", "Commander"];

/** CONTRACTS §2's defaults, for machines that never answered. */
const GO_DEFAULT_SETTINGS = { name: "Orchestrator", host: false, busy: "queue", reports: "auto", group_reports: true, read_aloud: false, auto_merge: false, approval_profile: "" };

/** The approval profile the node uses when none is saved: global_orchestrator.rs's
 * DEFAULT_APPROVAL_PROFILE, which a Rust test keeps equal to this line. */
const GO_DEFAULT_APPROVAL_PROFILE = "Approval means deciding, for each pull request that you or your workers opened for the current request, whether it is merged automatically or left for a human review.\n\nMerge automatically only when that pull request's checks pass and nothing is unresolved: no failing or pending check, no open review thread, no conflict, and no question waiting on the user.\n\nPrefer GitHub's own auto-merge (`gh pr merge --auto`), so the repository's required checks and branch rules still decide. Never bypass branch protection.\n\nThese always need a human review: releases, deploys, database migrations, auth, secrets or credentials, payments or billing, wire-contract or API breaks, anything the repo's AGENTS.md or CLAUDE.md reserves for the owner, and any change you are unsure about.";

/** global_orchestrator.rs's APPROVAL_PROFILE_MAX: the node keeps no more of a profile than this. */
const GO_APPROVAL_PROFILE_MAX = 4000;

/** The busy policy and the report policy, each value with the words the section shows. */
const GO_BUSY_CHOICES = [
  ["queue", "Queue the message"],
  ["refuse", "Refuse and retry later"],
];
const GO_REPORT_CHOICES = [
  ["auto", "Automatic, at end of turn"],
  ["tool_only", "Only when the Captain calls"],
];

/** An older node answers an unknown command and an older hub refuses the call; both wordings
 * appear in the error text, so either one means "this machine needs an update". */
const isOlderPinned = (error) => /unknown variant|can't answer/.test(String(error ?? ""));

/** One account-wide view out of every machine's answer. `machines` holds one entry per machine:
 * `{ id, name, online, settings, update, error }`, with `settings` present only when the machine
 * answered, `update` when it is online but too old, and `error` for any other read failure. The
 * shared fields come from the host when it answered, else from the first machine that did; `host`
 * becomes that machine's id, or null when no machine claims it. */
function mergeGlobalSettings(machines) {
  const answered = machines.filter((m) => m.settings);
  const host = machines.find((m) => m.settings?.host === true) ?? null;
  const base = host?.settings ? host : answered[0] ?? null;
  return { ...GO_DEFAULT_SETTINGS, ...(base?.settings ?? {}), host: host?.id ?? null };
}

/** How the drift note words a machine's own value of each shared field. */
const GO_DRIFT_WORDS = {
  name: (v) => `the name ${v}`,
  busy: (v) => `"${GO_BUSY_CHOICES.find(([c]) => c === v)?.[1] ?? v}" when a Captain is busy`,
  reports: (v) => `reports "${GO_REPORT_CHOICES.find(([c]) => c === v)?.[1] ?? v}"`,
  group_reports: (v) => `Group consecutive reports ${v ? "on" : "off"}`,
  read_aloud: (v) => `Read summaries aloud ${v ? "on" : "off"}`,
  auto_merge: (v) => `Auto-merge ${v ? "on" : "off"}`,
  approval_profile: () => "a different approval profile",
};

/** The machines whose own settings differ from the account's view, which mergeGlobalSettings takes
 * from the host (else the first machine that answered). A save that missed a machine, or an agent's
 * change approved on one machine, leaves it so, and an agent that starts there follows its own
 * values. Each entry names the machine and the values it holds instead. A field missing from a
 * machine's answer, as from an older pinned, is not compared. */
function globalSettingsDrift(machines) {
  const answered = machines.filter((m) => m.settings);
  const host = machines.find((m) => m.settings?.host === true) ?? null;
  const base = host?.settings ? host : answered[0] ?? null;
  if (!base) return [];
  const want = { ...GO_DEFAULT_SETTINGS, ...base.settings };
  return answered
    .filter((m) => m !== base)
    .map((m) => ({ id: m.id, name: m.name, base: base.name, values: Object.keys(GO_DRIFT_WORDS).filter((k) => k in m.settings && m.settings[k] !== want[k]).map((k) => GO_DRIFT_WORDS[k](m.settings[k])) }))
    .filter((d) => d.values.length);
}

/** The writes a save sends: the shared fields to every online machine that can take them, with
 * `host` true only on the chosen machine. Offline and too-old machines are skipped; the section
 * tells Felipe about them instead of failing the save. */
function globalSettingsWrites(view, machines) {
  // The default is sent as "", so the node keeps following the default when it changes.
  const profile = view.approval_profile ?? "";
  const approval_profile = profile.trim() === GO_DEFAULT_APPROVAL_PROFILE ? "" : profile;
  return machines
    .filter((m) => m.online && !m.update)
    .map((m) => ({
      machine: m.id,
      settings: {
        name: view.name,
        host: m.id === view.host,
        busy: view.busy,
        reports: view.reports,
        group_reports: view.group_reports !== false,
        read_aloud: !!view.read_aloud,
        auto_merge: !!view.auto_merge,
        approval_profile,
      },
    }));
}

/** Whether a save from `before` to `after` tells the running Captains and the orchestrator, as
 * global_orchestrator.rs's approval_notice decides: auto-merge changed, or the profile in effect
 * changed while it stayed on. An empty profile is the default. */
function approvalChangeTold(before, after) {
  const profile = (s) => s.approval_profile || GO_DEFAULT_APPROVAL_PROFILE;
  if (!!before.auto_merge !== !!after.auto_merge) return true;
  return !!after.auto_merge && profile(before) !== profile(after);
}

const workersLabel = (n) => `${n} worker${n === 1 ? "" : "s"}`;

/** How the projects page and the Captains rail name a `captainState`, and the rail a machine that is away. */
const CAPTAIN_STATE_LABELS = { working: "Working", idle: "Idle", none: "No Captain", offline: "Offline" };

/** The rail's second line for one project, out of what `captainFacts` gathers: the machine, then
 * what its state counts (running workers, the last report, threads at work, when the machine was
 * last seen), then how many of its threads wait on the user. `reported` and `seen` come worded. */
function captainRailSub({ state, machine, workers = 0, working = 0, needs = [], reported = null, seen = null }) {
  const parts = [machine.name];
  if (state === "working" && workers) parts.push(workersLabel(workers));
  if (state === "idle" && reported) parts.push(`reported ${reported}`);
  if (state === "none" && working) parts.push(`${working} thread${working === 1 ? "" : "s"} working`);
  if (state === "offline" && seen) parts.push(seen);
  if (needs.length) parts.push(`${needs.length} needs you`);
  return parts.join(" · ");
}

/** What a report card says about its Captain right now: counts of the project's threads at work
 * and waiting on the user, or, with neither, the Captain's own state after its machine. */
function reportCaptainState({ state, working = 0, needs = [] }) {
  const chips = [working && { tone: "working", text: `${working} working` }, needs.length && { tone: "needs", text: `${needs.length} needs you` }].filter(Boolean);
  const now = { idle: "idle now", working: "working now", offline: "offline" }[state];
  return { chips, where: chips.length || !now ? "" : ` · ${now}` };
}

/** One line per machine the save could not reach, in the machine's own words: too old, offline, or
 * failed. Machines that took the settings stay quiet. */
function globalSettingsNotes(machines) {
  return machines
    .filter((m) => m.update || !m.online || m.error)
    .map((m) =>
      m.update ? `${m.name} needs to update pinned`
      : !m.online ? `${m.name} is offline; once it is back, this page shows whether it matches`
      : `${m.name}: ${m.error}`,
    );
}
