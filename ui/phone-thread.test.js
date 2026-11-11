import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui-phone-thread.js", import.meta.url), "utf8");
const slice = src.slice(src.indexOf("const onPhone"), src.indexOf("// -- The header"));

const MENU = [
  { label: "New thread in site", icon: "message-square-plus", run: () => "new" },
  { label: "Open the site Captain", icon: "bot", run: () => "captain" },
  { label: "Pin thread", icon: "pin", run: () => "pin" },
  { label: "Settle thread", icon: "circle-check", run: () => "settle" },
  { label: "Snooze", icon: "clock", children: () => [{ label: "1 hour" }] },
  { label: "Rename thread", icon: "pencil", run: () => "rename in the row" },
  { label: "Regenerate title", icon: "refresh-cw", run: () => "title" },
  { label: "Mark unread", icon: "mail-open", run: () => "unread" },
  { label: "Copy", icon: "copy", children: () => [{ label: "Path" }] },
  { label: "Move to account", icon: "user-round", children: () => [{ label: "Work" }] },
  { label: "Archive thread", icon: "archive", run: () => "archive" },
  { label: "Delete", icon: "trash", danger: true, run: () => "delete" },
];

function load({ workers = [], parent = null } = {}) {
  const renamed = [];
  const context = {
    narrow: () => true,
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    icon: (name) => `<svg data-icon="${name}"></svg>`,
    glyphHtml: (state) => `<span data-glyph="${state}"></span>`,
    modelLabel: (name) => name,
    durationLabel: (s) => `${Math.round(s / 60)}m`,
    duration: (ms) => `${ms}ms`,
    sum: (files, key) => files.reduce((n, f) => n + f[key], 0),
    MODES: { manual: { label: "Supervised" }, accept: { label: "Auto-accept edits" }, auto: { label: "Auto" }, bypass: { label: "Full access" }, plan: { label: "Plan" } },
    threadOf: (tab) => tab.thread,
    prOf: () => ({ number: 12, url: "https://github.com/example/site/pull/12" }),
    quickAction: () => ({ label: "Push", action: "push" }),
    projectScripts: () => ({ scripts: [{ id: "dev", name: "dev" }] }),
    primaryScript: (scripts) => scripts[0],
    scriptShells: [{ tab: null }],
    prRollupState: (d) => d.rollup ?? null,
    prCache: new Map([["t:https://github.com/example/site/pull/12", { data: { rollup: "passing" } }]]),
    hasProject: () => true,
    memoryOf: () => ({ hits: [1, 2, 3] }),
    panelOf: () => ({ surfaces: [] }),
    surfaceTitle: () => "",
    ui: { drawer: false },
    rowMenuItems: () => MENU,
    workersOf: () => workers,
    workersSummary: (rows) => ({ total: rows.length, done: rows.filter((r) => r.state === "done").length, needs: rows.filter((r) => r.state === "needs").length }),
    parentOf: () => parent,
    threadTitle: (tab) => tab.title,
    settled: () => false,
    accountName: () => "Claude Code",
    DEFAULT_ACCOUNT: "default",
    startRename: (tab, where) => renamed.push(where),
  };
  vm.createContext(context);
  vm.runInContext(`${slice}\nObject.assign(globalThis, { phoneChipLabel, threadActionsOf, workersSheetRows, workerCardHtml });`, context);
  return { ...context, renamed };
}

const thread = (role = null) => ({
  key: "t",
  machine: "mini",
  view: { role, running: true, account: "default" },
  thread: { workspace: { repo: true, branch: "fix/hero" }, items: [], ports: [{ port: 3000, own: true }], gitDiff: { data: { patch: "--- a/x\n+++ b/x\n+one\n+two\n-three\n" } }, history: [{ files: [{ added: 18, removed: 31 }] }] },
});
const labels = (list) => [...list].map((a) => (a.hint ? `${a.label} · ${a.hint}` : a.label));

test("the composer chip names the model, the effort and the mode, leaving out what the thread lacks", () => {
  const { phoneChipLabel } = load();
  assert.equal(phoneChipLabel("Opus 5.5", "High", "auto"), "Opus 5.5 · High · Auto");
  assert.equal(phoneChipLabel("Sonnet 5.5", "Extra High Fast", "accept"), "Sonnet 5.5 · Extra High Fast · Auto-accept edits");
  assert.equal(phoneChipLabel("Haiku 5.5", "Low", "plan"), "Haiku 5.5 · Low · Plan");
  assert.equal(phoneChipLabel("Default", null, null), "Default");
  assert.equal(phoneChipLabel("Opus 5.5", null, "bypass"), "Opus 5.5 · Full access");
});

test("a plain thread's sheet offers its git action, its script and its PR, the eight tools, and the row actions", () => {
  const sheet = load().threadActionsOf(thread());
  assert.deepEqual(labels(sheet.primary), ["Push", "Run dev", "PR #12"]);
  assert.deepEqual(labels(sheet.tools), ["Diff · +2 -1", "Browser · :3000", "Files · Tree", "Terminal · 1 open", "PR #12 · Passing", "Memory · 3 saved", "Device · Simulator", "Details · Branch, cost"]);
  assert.deepEqual(labels(sheet.actions), ["Pin thread", "Settle · Swipe left on the row", "Snooze · Swipe right on the row", "Rename, mark unread, copy · More", "Move to account · Claude Code", "Archive or delete"]);
});

test("the sheet's sub-lists are the row menu's own entries, and rename edits the header's title", () => {
  const app = load();
  const actions = app.threadActionsOf(thread()).actions;
  const more = [...actions.find((a) => a.id === "more").items()];
  assert.deepEqual(more.map((i) => i.label), ["New thread in site", "Open the site Captain", "Rename thread", "Regenerate title", "Mark unread", "Copy"]);
  more.find((i) => i.icon === "pencil").run();
  assert.deepEqual(app.renamed, ["header"]);
  assert.deepEqual([...actions.find((a) => a.id === "archive").items()].map((i) => i.label), ["Archive thread", "Delete"]);
  assert.deepEqual([...actions.find((a) => a.id === "snooze").items()].map((i) => i.label), ["1 hour"]);
  assert.equal(actions.find((a) => a.id === "pin").run(), "pin");
});

test("a Captain's sheet leads its actions with its workers", () => {
  const workers = [{ state: "working" }, { state: "needs" }, { state: "done" }];
  const sheet = load({ workers }).threadActionsOf(thread("orchestrator"));
  assert.deepEqual(labels(sheet.primary), ["Push", "Run dev", "PR #12"]);
  assert.deepEqual(labels(sheet.actions).slice(0, 2), ["Workers · 1 needs you", "Pin thread"]);
});

test("a worker's sheet opens its diff where a thread has its git action, has no script, and offers its Captain", () => {
  const worker = thread("worker");
  worker.thread.gitDiff = null;
  const sheet = load({ parent: { title: "Release v0.17.21" } }).threadActionsOf(worker);
  assert.deepEqual(labels(sheet.primary), ["Open diff +18 -31", "PR #12"]);
  assert.equal(sheet.tools[0].hint, "+18 -31");
  assert.deepEqual(labels(sheet.actions).slice(0, 2), ["Open Release v0.17.21 · Captain", "Pin thread"]);
});

const approval = (choices) => ({ kind: "approval", text: null, command: "git push origin readme-tidy", choices: choices && choices.map((choice) => ({ label: choice, about: null, choice })), typed: false });
const text = (html) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();

test("the Workers sheet answers a one-tap ask in place and offers Open for one it can't", () => {
  const app = load();
  const oneTap = app.workerCardHtml({ key: "w1", title: "Tidy the README", model: "Haiku 5.5", since: 1794420480000, ask: approval(["approve", "decline", "always"]) });
  assert.equal(text(oneTap), "Tidy the README Needs your approval · waiting Haiku 5.5 git push origin readme-tidy Decline Approve");
  assert.match(oneTap, /waiting <span class="ticker" data-since="1794420480000"><\/span>/, "the clock is left to the ticker");
  assert.match(oneTap, /data-ws-answer="decline" data-ws-row="w1"/);
  assert.match(oneTap, /data-ws-answer="approve" data-ws-row="w1"/);
  const openOnly = app.workerCardHtml({ key: "w2", title: "Pick a color", model: null, since: null, ask: { kind: "question", text: "Which color?", command: null, choices: null, typed: false } });
  assert.equal(text(openOnly), "Pick a color Has a question Which color? Open");
  assert.match(openOnly, /data-ws-open="w2"/);
  assert.doesNotMatch(openOnly, /data-ws-answer/);
});

test("a Workers sheet row's markup does not change as its clock runs, so a repaint keeps the row under a tap", () => {
  const clock = { now: 1794420540000 };
  const app = {
    Date: { now: () => clock.now },
    esc: (t) => String(t ?? ""),
    glyphHtml: () => "",
    durationLabel: (s) => `${Math.round(s / 60)}m`,
    duration: (ms) => `${ms}ms`,
    phoneSvg: () => "",
  };
  vm.createContext(app);
  vm.runInContext(`${src.slice(src.indexOf("const sheetTicker"), src.indexOf("/** The workers being answered"))}${src.slice(src.indexOf("/** Any other worker:"), src.indexOf("const stoppable"))}\nglobalThis.workerSheetRowHtml = workerSheetRowHtml;`, app);
  const row = { key: "ci", state: "working", since: 1794420480000, title: "Run the CI on main", step: "cargo test", tab: {} };
  const first = app.workerSheetRowHtml(row);
  clock.now += 60000;
  assert.equal(app.workerSheetRowHtml(row), first);
  assert.match(first, /<span class="ws-time"><span class="ticker" data-since="1794420480000"><\/span><\/span>/);
  const nested = app.workerSheetRowHtml({ key: "cap:task-2", state: "working", since: null, elapsedMs: null, title: "Port the Windows half", step: null, tab: null, depth: 1 });
  assert.match(nested, /^<button type="button" class="ws-row" data-state="working" data-ws-agent="cap:task-2" data-depth="1" style="--depth: 1">/, "a nested agent sits indented and opens its row in the block");
});

test("the Workers sheet puts the waiting workers first, the reviewed one leading", () => {
  const rows = [
    { key: "ci", state: "working", tab: {} },
    { key: "a", state: "needs", tab: {} },
    { key: "b", state: "needs", tab: {} },
    { key: "task", state: "needs", tab: null },
    { key: "log", state: "done", tab: {} },
  ];
  const { workersSheetRows } = load();
  const shown = (first) => {
    const { cards, rest } = workersSheetRows(rows, first);
    return [cards.map((r) => r.key), rest.map((r) => r.key)];
  };
  assert.deepEqual(shown(null), [["a", "b"], ["ci", "task", "log"]]);
  assert.deepEqual(shown("b"), [["b", "a"], ["ci", "task", "log"]]);
});

test("Message all skips the workers waiting on you, says how many, and keeps the text unless every send went out", async () => {
  const rows = [
    { key: "ci", tab: { view: { running: true } } },
    { key: "readme", tab: { view: { running: true }, waiting: true } },
    { key: "docs", tab: { view: { running: true }, waiting: true } },
    { key: "old", tab: { view: { running: false } } },
    { key: "task", tab: null },
  ];
  const sent = [];
  const told = [];
  let works = true;
  const context = {
    workersOf: () => rows,
    waitsOnUser: (tab) => !!tab.waiting,
    sayToWorker: async (key, text, quiet) => (sent.push([key, text, quiet]), works),
    tell: (html) => told.push(html),
    esc: (text) => text,
  };
  vm.createContext(context);
  vm.runInContext(`${src.slice(src.indexOf("function messageAllSaid"), src.indexOf("function onWorkersSheetClick"))}\nglobalThis.messageAllWorkers = messageAllWorkers;`, context);
  const form = { hidden: false };
  const input = { value: " Rebase on main ", closest: () => form };
  works = false;
  await context.messageAllWorkers({}, input);
  assert.deepEqual(sent, [["ci", "Rebase on main", true]], "only the running worker that waits on nothing gets it");
  assert.deepEqual([input.value, form.hidden], [" Rebase on main ", false], "a failed send keeps the text");
  works = true;
  await context.messageAllWorkers({}, input);
  assert.deepEqual([input.value, form.hidden], ["", true]);
  assert.deepEqual(told, [
    "Couldn't reach 1 worker; your message is still here. Skipped 2 workers waiting on you.",
    "Sent to 1 worker. Skipped 2 workers waiting on you.",
  ]);
  rows.splice(0, 1);
  input.value = "Rebase on main";
  await context.messageAllWorkers({}, input);
  assert.equal(told.at(-1), "Nothing sent. Skipped 2 workers waiting on you.");
  assert.equal(input.value, "Rebase on main");
});

test("a sheet closes when the window grows past phone width, and stays while it is a phone", () => {
  let width = 390;
  const closed = [];
  const context = {
    narrow: (px) => width <= px,
    guiOn: () => true,
    renderHeadSub: () => {},
    renderWorkersPill: () => {},
    renderComposer: () => {},
    active: null,
    openedSheet: { close: () => closed.push(width) },
  };
  vm.createContext(context);
  vm.runInContext(`const onPhone = () => narrow(768);\n${src.slice(src.indexOf("function phoneWidthChanged"), src.indexOf('matchMedia("(max-width: 768px)")'))}`, context);
  vm.runInContext("phoneWidthChanged()", context);
  assert.deepEqual(closed, []);
  width = 1024;
  vm.runInContext("phoneWidthChanged()", context);
  assert.deepEqual(closed, [1024]);
});
