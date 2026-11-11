import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const gui = read("gui.js");
const agents = read("gui-agents.js");
const sidebar = read("gui-sidebar.js");
const composer = read("gui-composer.js");
const chat = read("global-chat.js");
const between = (src, start, end) => {
  const from = src.indexOf(start);
  const to = src.indexOf(end, from);
  assert.ok(from >= 0 && to > from, `missing slice ${start}`);
  return src.slice(from, to);
};
const line = (src, start) => between(src, start, "\n");
/** A top-level `function` or a multi-line `const` that ends at the first line holding only its close. */
const decl = (src, start) => {
  const from = src.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  const close = src.slice(from).search(/\n\}+;?\n/);
  return src.slice(from, from + close + src.slice(from + close).indexOf("\n", 1) + 1);
};

const NOW = Date.parse("2026-11-11T18:10:00Z");
const S = NOW / 1000;

/** A terminal holding `lines` as its screen, as xterm's buffer reads. */
const term = (lines) => ({ rows: lines.length, buffer: { active: { baseY: 0, getLine: (i) => ({ translateToString: () => lines[i] ?? "", isWrapped: false, getCell: () => ({ isBgDefault: () => true }) }) } } });
const PUSH_MENU = [
  "> ",
  "────────────────────────────────────────",
  " Bash command",
  " Tip: auto mode handles these prompts for you",
  " Pushes one commit to a new remote branch. Nothing on main changes.",
  "╌".repeat(40),
  " git push origin readme-tidy",
  "╌".repeat(40),
  " Do you want to proceed?",
  " ❯ 1. Yes",
  "   2. Yes, and don't ask again for git push commands",
  "   3. No",
  " Esc to cancel · Tab to amend",
];
const QUESTION_SET = [
  "────────────────────────────────────────",
  "←  ☐ Color  ☐ Sizes  ✔ Submit  →",
  "",
  "Which color?",
  "",
  "❯ 1. Red",
  "     Warm",
  "  2. Blue",
  "     Cool",
  "  3. Type something.",
  "",
  "Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
];

/** The page's real rules for phases, asks and answers, around `world`, with the page's I/O stubbed. */
function page(world) {
  const calls = { composer: [], sidebar: 0, thread: 0, writes: [], asked: [] };
  const context = {
    console,
    Date: class extends Date {
      constructor(...a) {
        super(...(a.length ? a : [NOW]));
      }
      static now() {
        return NOW;
      }
    },
    tabs: world.tabs,
    active: world.active ?? null,
    seenAt: world.seenAt ?? {},
    sideMarks: { snoozed: world.snoozed ?? {} },
    machines: world.machines ?? new Map(),
    DEFAULT_ACCOUNT: "claude",
    statusOf: (tab) => tab.view.status ?? "idle",
    threadOf: (tab) => tab.thread,
    threadKey: (tab) => tab.key,
    threadTitle: (tab) => tab.title,
    projectName: (tab) => tab.project ?? "pinned",
    machineName: (id) => ({ mini: "mac-mini", book: "macbook" })[id] ?? id,
    accountName: (machine, id) => ({ claude: "Claude Code", work: "Work" })[id] ?? id,
    accountMachines: () => world.machineRows ?? [],
    switchTargets: (tab) => (tab.view.resume_at ? [{ id: "work", name: "Work" }] : []),
    isGlobalOrchestratorTab: (tab) => !!tab.view.orchestrator_global,
    localIdOf: (id) => id,
    modelOf: () => null,
    guiOn: () => true,
    terminalOnly: () => false,
    thoughtLine: (text) => text,
    toolIcon: () => "terminal",
    toolLabel: (step) => `Ran ${step.input.command ?? step.input.pattern}`,
    renderComposer: (tab) => calls.composer.push(tab.key),
    renderSidebar: () => calls.sidebar++,
    renderThread: () => calls.thread++,
    showComposerNote: () => calls.composer.push("note"),
    visibleInterval: () => {},
    openProjects: () => new Map([["mini", new Set(["/w/pinned"])], ["book", new Set(["/w/site"])]]),
    callOn: async (machine, call) => {
      if (call.cmd === "write_input") return calls.writes.push([`${machine}:${call.tab}`, call.data]), null;
      calls.asked.push([machine, call]);
      return world.answer(machine, call);
    },
  };
  vm.createContext(context);
  vm.runInContext(
    [
      read("terminal-menu.js"),
      line(chat, "const REPORT_HEADER"),
      decl(chat, "function reportBlocks"),
      decl(chat, "function waitingKindOf"),
      decl(chat, "function askOf"),
      between(gui, "function hookPhase", "const drawnOf"),
      between(agents, "const agentsOf", "function agentSteps"),
      decl(agents, "function stepLine"),
      line(sidebar, "const unixOf"),
      line(sidebar, "const lastPrompt"),
      line(sidebar, "const nowS"),
      line(sidebar, "const canSnooze"),
      decl(sidebar, "const raisedHand"),
      line(sidebar, "const snoozedNow"),
      line(sidebar, "const menuIsQuestion"),
      decl(sidebar, "function waitingLookOf"),
      decl(composer, "function menuKind"),
      line(composer, "const declineOption"),
      line(composer, "const freeOption"),
      line(composer, "const ARROW_DOWN"),
      line(composer, "const ARROW_UP"),
      line(composer, "const writeTo"),
      decl(composer, "const arrowsTo"),
      decl(composer, "async function until"),
      decl(composer, "function redrawAsk"),
      decl(composer, "function chooseOption"),
      decl(composer, "async function answerWith"),
      line(composer, "const TAB_ROW"),
      decl(composer, "function questionTabsOf"),
      decl(composer, "function questionTabs(tab)"),
      line(composer, "const multiSelect"),
      decl(composer, "function approvalChoices"),
      read("redesign-model.js"),
    ].join("\n"),
    context,
  );
  const run = (code, args = {}) => {
    Object.assign(context, { args });
    return vm.runInContext(code, context);
  };
  return { run, calls, context };
}

let ids = 0;
/** A tab on `machine` with a TabView of `view` and transcript `items`. */
function tab(title, view = {}, { machine = "mini", items = [], agents = [], screen = null, project } = {}) {
  const id = view.id ?? `tab-${++ids}`;
  return {
    key: `${machine}:${id}`,
    machine,
    title,
    project,
    term: term(screen ?? ["> "]),
    menu: null,
    view: { id, running: true, opened: ids, updated: S - 600, ...view },
    thread: { items, agents, open: new Set() },
  };
}
const user = (at, text) => ({ kind: "user", at, text });
const waiting = { status: "needs-input", interrupted: true, message: "Claude needs your permission to use Bash" };
/** Reads every tab's menu off its screen, as renderGui does on each render. */
const readMenus = (p) => p.run("for (const t of tabs) t.menu = terminalMenu(t)");

test("threadState takes needs, then working, then unread, then done, then idle", () => {
  const busy = tab("Release", { id: "cap", status: "done", role: "orchestrator" });
  const child = tab("CI", { status: "working", role: "worker", spawned_by: "cap" });
  const cases = {
    asking: tab("Asks", waiting),
    failedWithWorkers: tab("Failed", { id: "f", status: "failed" }),
    failedChild: tab("Its worker", { status: "working", role: "worker", spawned_by: "f" }),
    busy,
    child,
    unread: tab("Hero", { status: "done", updated: S - 60 }),
    workerDone: tab("Changelog", { status: "done", role: "worker", spawned_by: "elsewhere", updated: S - 60 }),
    seen: tab("Updater", { status: "done", updated: S - 7200 }),
    open: tab("Open", { status: "done", updated: S - 60 }),
    never: tab("Draft", { status: undefined, running: false }),
  };
  const p = page({ tabs: Object.values(cases), active: cases.open, seenAt: { [cases.seen.key]: S - 7200, [cases.busy.key]: 0 } });
  p.context.cases = cases;
  const states = p.run("Object.fromEntries(Object.entries(cases).map(([k, t]) => [k, threadState(t)]))");
  assert.deepEqual({ ...states }, {
    asking: "needs",
    failedWithWorkers: "needs",
    failedChild: "working",
    busy: "working",
    child: "working",
    unread: "unread",
    workerDone: "done",
    seen: "idle",
    open: "idle",
    never: "idle",
  });
});

/** The Main board's Captain: three worker threads, a failed and a cancelled delegated task, and a
 * running CLI subagent with a nested one under it. */
function captainWorld() {
  const captain = tab("Release v0.17.21", { id: "cap", status: "done", role: "orchestrator", stats: { active_ms: 4_320_000 } }, {
    agents: [
      { id: "task-9", name: "Port the updater", kind: "Cursor", model: "composer-2.5", state: "failed", started: "2026-11-11T18:00:00Z", updated: "2026-11-11T18:01:30Z", steps: [{ kind: "text", text: "Build failed: missing crate" }] },
      { id: "task-8", name: "Bump the docs", kind: "Codex", model: null, state: "cancelled", started: "2026-11-11T18:02:00Z", updated: "2026-11-11T18:02:20Z", steps: [] },
      { id: "a1b2", name: "Explore the release scripts", kind: "Explore", model: "haiku", state: "running", started: "2026-11-11T18:05:00Z", updated: "2026-11-11T18:06:00Z", steps: [{ kind: "tool", id: "s1", name: "Grep", input: { pattern: "release" } }, { kind: "result", tool: "s1" }] },
      { id: "a3", parent: "a1b2", name: "Nested", state: "running", steps: [] },
    ],
  });
  const ci = tab("Run the CI on main", { status: "working", role: "worker", spawned_by: "cap", stats: { active_ms: 250_000 } }, {
    items: [user("2026-11-11T18:06:00Z", "Run the CI on main."), { kind: "model", name: "Sonnet 5.5" }, { kind: "tool", name: "Bash", input: { command: "cargo test -p pinned-server" } }],
  });
  const readme = tab("Tidy the README", { ...waiting, role: "worker", spawned_by: "cap", ask_detail: "git push origin readme-tidy", updated: S - 120 }, {
    items: [user("2026-11-11T18:03:00Z", "Tidy the README."), { kind: "text", text: "The README is tidied." }, { kind: "tool", name: "Bash", input: { command: "git push origin readme-tidy" } }],
    screen: PUSH_MENU,
  });
  const changelog = tab("Changelog for v0.17.21", { status: "done", role: "worker", spawned_by: "cap", stats: { active_ms: 160_000 } }, {
    items: [user("2026-11-11T17:58:00Z", "Write the changelog."), { kind: "text", text: "Wrote CHANGELOG.md with 14 entries and opened #231\n\nIt lists every merged PR." }, { kind: "pr", number: 231 }],
  });
  const stranger = tab("Elsewhere", { status: "working", spawned_by: "cap" }, { machine: "book" });
  return { captain, ci, readme, changelog, stranger };
}

test("workersOf lists a Captain's worker threads, delegated tasks and subagents with the board's fields", () => {
  const w = captainWorld();
  const p = page({ tabs: Object.values(w), active: w.captain });
  readMenus(p);
  p.context.captain = w.captain;
  const rows = p.run("workersOf(captain).map(({ tab, agent, ...r }) => ({ ...r, tab: tab?.title ?? null, agent: agent?.id ?? null }))");
  const approval = {
    kind: "approval",
    text: "Pushes one commit to a new remote branch. Nothing on main changes.",
    command: "git push origin readme-tidy",
    choices: [
      { label: "Approve", about: null, choice: "approve" },
      { label: "Decline", about: null, choice: "decline" },
      { label: "Always allow", about: "Yes, and don't ask again for git push commands", choice: "always" },
    ],
    typed: true,
  };
  assert.deepEqual(JSON.parse(JSON.stringify(rows)), [
    { kind: "thread", key: w.ci.key, title: "Run the CI on main", state: "working", step: "Ran cargo test -p pinned-server", ask: null, model: "Sonnet 5.5", elapsedMs: 250000, since: Date.parse("2026-11-11T18:06:00Z"), tab: "Run the CI on main", agent: null },
    { kind: "thread", key: w.readme.key, title: "Tidy the README", state: "needs", step: "Ran git push origin readme-tidy", ask: approval, model: null, elapsedMs: null, since: (S - 120) * 1000, tab: "Tidy the README", agent: null },
    { kind: "thread", key: w.changelog.key, title: "Changelog for v0.17.21", state: "done", step: "Wrote CHANGELOG.md with 14 entries and opened #231", ask: null, model: null, elapsedMs: 160000, since: null, tab: "Changelog for v0.17.21", agent: null },
    { kind: "task", key: "mini:cap:task-9", title: "Port the updater", state: "failed", step: "Build failed: missing crate", ask: null, model: "composer-2.5", elapsedMs: 90000, since: null, tab: null, agent: "task-9" },
    { kind: "task", key: "mini:cap:task-8", title: "Bump the docs", state: "stopped", step: null, ask: null, model: null, elapsedMs: 20000, since: null, tab: null, agent: "task-8" },
    { kind: "subagent", key: "mini:cap:a1b2", title: "Explore the release scripts", state: "working", step: "Ran release", ask: null, model: "haiku", elapsedMs: null, since: Date.parse("2026-11-11T18:05:00Z"), tab: null, agent: "a1b2" },
  ]);
  assert.deepEqual({ ...p.run("workersSummary(workersOf(captain))") }, { total: 6, done: 1, working: 2, needs: 1, since: Date.parse("2026-11-11T18:05:00Z") });
  assert.equal(p.run("threadState(captain)"), "working", "a Captain whose own turn ended is working while its workers run");
});

test("a worker whose menu is not read yet asks with its command and offers only Open; a question set offers only Open", () => {
  const w = captainWorld();
  const quiz = tab("Pick the sizes", { ...waiting, message: "Claude needs your input", ask_detail: "Which color?" }, { screen: QUESTION_SET });
  const p = page({ tabs: [...Object.values(w), quiz], active: w.captain });
  p.context.both = [w.readme, quiz];
  assert.deepEqual(JSON.parse(JSON.stringify(p.run("both.map(pendingAsk)"))), [
    { kind: "approval", text: null, command: "git push origin readme-tidy", choices: null, typed: false },
    { kind: "question", text: "Which color?", command: null, choices: null, typed: false },
  ]);
  readMenus(p);
  const set = p.run("pendingAsk(both[1])");
  assert.equal(set.kind, "question");
  assert.equal(set.choices, null, "several questions in a row cannot be answered in one tap");
});

test("needsYouItems lists what waits on the user, newest first, with each kind, and leaves a snoozed thread out until it asks again", () => {
  const approval = tab("Tidy the README", { ...waiting, ask_detail: "git push origin readme-tidy", updated: S - 120 }, { screen: PUSH_MENU });
  const question = tab("Hero image", { ...waiting, message: "Claude needs your input", ask_detail: "Which hero image should the launch page use?", updated: S - 60 }, { project: "site", machine: "book" });
  const limit = tab("Rate limit the webhook", { status: "failed", resume_at: S + 3000, account: "claude", updated: S - 90 }, { project: "api" });
  const failed = tab("Flaky test", { status: "failed", message: "API Error: 500", updated: S - 900 });
  const review = tab("Fix hero spacing on mobile", { status: "done", updated: S - 30 }, { project: "site", machine: "book" });
  const workerDone = tab("Changelog", { status: "done", role: "worker", spawned_by: "x", updated: S - 10 });
  const working = tab("Rate", { status: "working", updated: S - 5 });
  const snoozed = tab("Snoozed limit", { status: "failed", resume_at: S + 3000, updated: S - 40 });
  const asksAgain = tab("Snoozed, then asked", { ...waiting, updated: S - 20 });
  const p = page({ tabs: [approval, question, limit, failed, review, workerDone, working, snoozed, asksAgain], snoozed: { [snoozed.key]: { until: S + 3000, at: S - 30 }, [asksAgain.key]: { until: S + 3000, at: S - 30 } } });
  readMenus(p);
  const items = p.run("needsYouItems().map(({ tab, ask, ...i }) => ({ ...i, ask: ask && ask.kind + ' ' + (ask.command ?? ask.text) + ' ' + (ask.choices ? ask.choices.length : 'open') }))");
  assert.deepEqual(JSON.parse(JSON.stringify(items)), [
    { kind: "approval", title: "Snoozed, then asked", project: "pinned", machine: "mac-mini", since: (S - 20) * 1000, limit: null, ask: "approval null open" },
    { kind: "review", title: "Fix hero spacing on mobile", project: "site", machine: "macbook", since: (S - 30) * 1000, limit: null, ask: null },
    { kind: "question", title: "Hero image", project: "site", machine: "macbook", since: (S - 60) * 1000, limit: null, ask: "question Which hero image should the launch page use? open" },
    { kind: "limit", title: "Rate limit the webhook", project: "api", machine: "mac-mini", since: (S - 90) * 1000, limit: { resumesAt: (S + 3000) * 1000, account: "Claude Code", held: false, switchTo: [{ id: "work", name: "Work" }] }, ask: null },
    { kind: "approval", title: "Tidy the README", project: "pinned", machine: "mac-mini", since: (S - 120) * 1000, limit: null, ask: "approval git push origin readme-tidy 3" },
    { kind: "failed", title: "Flaky test", project: "pinned", machine: "mac-mini", since: (S - 900) * 1000, limit: null, ask: null },
  ]);
});

test("navCounts counts what waits, the unseen Captain reports, reviews once fetched, the usage window and the machines", async () => {
  const report = (machine, tabId, text) => `[pinned] Capitão pinned (mac-mini) [${machine} ${tabId}]: ${text}`;
  const global = tab("Orchestrator", { orchestrator_global: true, status: "done", updated: S - 30 }, {
    items: [
      user("2026-11-11T17:00:00Z", report("mini", "tab-1", "Old news")),
      user("2026-11-11T18:00:00Z", `${report("mini", "tab-1", "CI is green")}\n${report("book", "tab-7", "Hero fixed")}`),
      { kind: "text", text: "Noted." },
    ],
  });
  const asking = tab("Asks", { ...waiting, account: "work" });
  const review = tab("Done", { status: "done", updated: S - 20 });
  const machines = new Map([
    ["mini", { view: { online: true }, usage: { work: { five_hour: { used_percentage: 12, resets_at: S + 600 } }, claude: { five_hour: { used_percentage: 88, resets_at: S - 1 } } } }],
    ["book", { view: { online: true }, usage: { claude: { five_hour: { used_percentage: 23.5, resets_at: S + 1200 } } } }],
  ]);
  const p = page({
    tabs: [global, asking, review],
    active: asking,
    seenAt: { [global.key]: Date.parse("2026-11-11T17:30:00Z") / 1000 },
    machines,
    machineRows: [{ online: true }, { online: false }, { online: true, upstash_box: true }],
    answer: async (machine) => ({ rows: machine === "mini" ? [{ url: "u/1" }, { url: "u/2" }] : [{ url: "u/2" }, { url: "u/3" }] }),
  });
  assert.deepEqual(JSON.parse(JSON.stringify(p.run("navCounts()"))), {
    needsYou: 1,
    orchestrator: 2,
    reviews: null,
    usage: { percent: 12, resetsAt: (S + 600) * 1000, account: "Work" },
    machinesOnline: 1,
    machinesTotal: 2,
  });
  await p.run("fetchReviews()");
  assert.deepEqual(p.calls.asked.map(([m, c]) => [m, c.cmd, c.involvement, c.state, c.paths]).map((x) => JSON.parse(JSON.stringify(x))), [
    ["mini", "pull_requests", "reviewing", "open", ["/w/pinned"]],
    ["book", "pull_requests", "reviewing", "open", ["/w/site"]],
  ]);
  assert.equal(p.run("navCounts().reviews"), 3, "a pull request two machines list counts once");
  await p.run("fetchReviews()");
  assert.equal(p.calls.asked.length, 2, "a second fetch inside five minutes asks nobody");
  p.context.active = null;
  assert.deepEqual({ ...p.run("navCounts().usage") }, { percent: 23.5, resetsAt: (S + 1200) * 1000, account: "Claude Code" }, "with no thread open, the most used window that still holds");
});

test("answerAsk answers a worker from its Captain without touching the Captain's composer", async () => {
  const w = captainWorld();
  const p = page({ tabs: Object.values(w), active: w.captain });
  readMenus(p);
  p.context.worker = w.readme;
  assert.equal(await p.run("answerAsk(worker, 'decline')"), true);
  assert.deepEqual(p.calls.writes, [[w.readme.key, "\x1b[B\x1b[B\r"]], "arrows down to No, then Enter, into the worker's session");
  assert.deepEqual(p.calls.composer, [], "the open thread's composer is not redrawn");
  assert.equal(p.calls.sidebar, 1, "the lists that show the ask redraw");
  assert.equal(w.readme.menu, null);

  readMenus(p);
  assert.equal(await p.run("answerAsk(worker, 'always')"), true);
  assert.equal(await p.run("answerAsk(worker, 'approve')"), true);
  assert.deepEqual(p.calls.writes.slice(1), [[w.readme.key, "\x1b[B\r"], [w.readme.key, "\r"]]);
  assert.equal(await p.run("answerAsk(worker, { option: 7 })"), false, "a choice the ask does not offer is refused");
  assert.deepEqual(p.calls.composer, []);

  p.context.active = w.readme;
  assert.equal(await p.run("answerAsk(worker, 'approve')"), true);
  assert.deepEqual(p.calls.composer, [w.readme.key], "answered in its own open thread, its composer redraws");
});

test("answerAsk refuses a question set and a thread with no menu, and types nothing", async () => {
  const quiz = tab("Pick the sizes", { ...waiting, message: "Claude needs your input" }, { screen: QUESTION_SET });
  const quiet = tab("No menu", waiting);
  const p = page({ tabs: [quiz, quiet] });
  p.context.both = [quiz, quiet];
  assert.equal(await p.run("answerAsk(both[0], { option: 1 })"), false);
  assert.equal(await p.run("answerAsk(both[1], 'approve')"), false);
  assert.equal(await p.run("answerAsk(both[0], { text: 'Green' })"), false);
  assert.deepEqual(p.calls.writes, []);
});

test("workersKey changes when a worker's ask is answered, and modelChanged repaints the open Captain then", () => {
  const w = captainWorld();
  const p = page({ tabs: Object.values(w), active: w.captain });
  readMenus(p);
  p.context.captain = w.captain;
  const seen = [];
  p.context.seen = seen;
  p.run("onModelChange(() => seen.push(workersKey(captain)))");
  p.run("modelChanged()");
  assert.equal(p.calls.thread, 1);
  p.run("modelChanged()");
  assert.equal(p.calls.thread, 1, "nothing changed, nothing repaints");
  w.readme.menu = null;
  w.readme.view = { ...w.readme.view, status: "working", interrupted: false };
  p.run("modelChanged()");
  assert.equal(p.calls.thread, 2, "the worker's change repaints its parent");
  assert.equal(seen.length, 3);
  assert.notEqual(seen[1], seen[2]);
});

test("glyphHtml draws each state's dot at its size", () => {
  const p = page({ tabs: [] });
  assert.equal(p.run("glyphHtml('working', 8)"), `<span class="status-glyph" data-glyph="working" style="--glyph-size: 8px" aria-hidden="true"></span>`);
  assert.deepEqual(["needs", "unread", "done", "idle", "failed", "stopped", "bogus"].map((s) => p.run(`glyphHtml(${JSON.stringify(s)})`).match(/data-glyph="(\w+)" style="--glyph-size: (\d+)px"/).slice(1).join(" ")), ["needs 10", "unread 10", "done 10", "idle 10", "failed 10", "idle 10", "idle 10"]);
});
