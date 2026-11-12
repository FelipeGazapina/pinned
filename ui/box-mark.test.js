import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const model = read("redesign-model.js");
const sidebar = read("gui-sidebar.js");
const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));
const between = (src, start, end) => src.slice(src.indexOf(start), src.indexOf(end, src.indexOf(start)));

const MARK = `<span class="box-mark" role="img" aria-label="Running in an Upstash Box" title="Running in an Upstash Box"></span>`;

/** The model's two helpers and the sidebar's three row builders, over the machines the hub listed. */
function page(machines) {
  const row = () => ({ className: "", dataset: {}, classList: { toggle() {}, contains: () => false }, setAttribute(name, value) { this[name] = value; }, tabIndex: 0 });
  const context = {
    machines: new Map(machines),
    document: { createElement: row },
    selected: new Set(),
    titling: new Set(),
    active: null,
    ui: {},
    sideMarks: { snoozed: {}, settled: {} },
    LOCAL: "local",
    isOrchestrator: (tab) => tab.view.role === "orchestrator",
    threadTitle: (tab) => tab.view.title,
    glyphHtml: () => "",
    threadState: () => "idle",
    toneRow() {},
    hasDraft: () => false,
    isPinned: () => false,
    phaseOf: () => "idle",
    statusSlot: () => null,
    ago: () => "1m",
    latestUserAt: () => 0,
    unseenDone: () => false,
    titleHtml: (tab) => tab.view.title,
    pinMark: () => "",
    roleMark: () => "",
    terminalPulse: () => "",
    rowActs: () => "",
    rowPrBadge: () => "",
    prWords: () => "",
    threadOf: () => ({ workspace: null }),
    projectIcon: () => "",
    projectName: () => "project",
    wakeLabel: () => "",
    icon: () => "",
    esc: (text) => String(text),
    middle: (text) => text,
    activeLabel: () => "",
    threadKey: (tab) => tab.key,
    workersOf: () => [],
    workersSummary: () => ({ total: 0 }),
  };
  vm.createContext(context);
  vm.runInContext([
    line(model, "const runsInBox"),
    line(model, "const boxMarkHtml"),
    between(sidebar, "function rowShell", "/** What gh last said"),
    "function stampHtml() { return ''; }",
    between(sidebar, "function cardRow", "/** The pull requests a thread linked"),
    "Object.assign(globalThis, { runsInBox, boxMarkHtml, cardRow, childRow, slimRow });",
  ].join("\n"), context);
  return context;
}

const thread = (machine, extra = {}) => ({ key: `${machine}:t`, machine, view: { title: "A thread", ...extra } });
const MACHINES = [
  ["local", { view: { name: "This Mac", upstash_box: null } }],
  ["mini", { view: { name: "mac-mini" } }],
  ["box-1", { view: { name: "Box · site", upstash_box: "b1" } }],
];

test("a session on a machine whose upstash_box is set shows the Upstash mark with its label", () => {
  const p = page(MACHINES);
  assert.equal(p.runsInBox(thread("box-1")), true);
  assert.equal(p.boxMarkHtml(thread("box-1")), MARK);
});

test("a session on one of the user's own machines shows no mark", () => {
  const p = page(MACHINES);
  for (const machine of ["local", "mini"]) {
    assert.equal(p.runsInBox(thread(machine)), false);
    assert.equal(p.boxMarkHtml(thread(machine)), "");
  }
});

test("a session on a machine the page does not know, or none at all, shows no mark; the name does not decide", () => {
  const p = page([...MACHINES, ["odd", { view: { name: "Box · looks like one" } }], ["bare", {}]]);
  for (const tab of [thread("gone"), thread("bare"), thread("odd"), thread(undefined), null, undefined]) assert.equal(p.boxMarkHtml(tab), "");
});

test("a worker's child row in a box carries the mark, its Captain's card and a settled row too, and a local worker's carries none", () => {
  const p = page(MACHINES);
  const worker = thread("box-1", { role: "worker" });
  const captain = thread("box-1", { role: "orchestrator" });
  assert.ok(p.childRow(worker).innerHTML.includes(MARK));
  assert.ok(p.cardRow(captain).innerHTML.includes(MARK));
  assert.ok(p.slimRow(worker, "unsettle").innerHTML.includes(MARK));
  assert.ok(!p.childRow(thread("local", { role: "worker" })).innerHTML.includes("box-mark"));
  assert.ok(!p.cardRow(thread("mini")).innerHTML.includes("box-mark"));
  assert.ok(!p.slimRow(thread("local"), "unsettle").innerHTML.includes("box-mark"));
});

test("a box row says so in its accessible name", () => {
  const p = page(MACHINES);
  assert.equal(p.childRow(thread("box-1", { role: "worker" }))["aria-label"], "A thread, worker, running in an Upstash Box");
  assert.equal(p.childRow(thread("local", { role: "worker" }))["aria-label"], "A thread, worker");
});
