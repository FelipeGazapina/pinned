import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const sidebarSrc = readFileSync(new URL("./gui-sidebar.js", import.meta.url), "utf8");
const between = (start, end) => {
  const from = sidebarSrc.indexOf(start);
  assert.ok(from >= 0, `gui-sidebar.js has ${start}`);
  return sidebarSrc.slice(from, sidebarSrc.indexOf(end, from));
};

/** The sidebar's renders over a list whose rows are the keys in `order`, on a phone whose screen is
 * `screen`. The page's body attribute observer is kept, so a test fires it as the browser does when
 * the screen changes. */
function sidebar({ screen }) {
  const observers = [];
  const body = { dataset: { phoneScreen: screen } };
  const threadList = {
    children: [],
    replaceChildren(...rows) {
      this.children = rows;
    },
    querySelectorAll(selector) {
      return selector === ".thread-row" ? this.children : selector === "[data-since]" ? this.children.map((row) => row.since) : [];
    },
    insertAdjacentHTML() {},
  };
  const state = { order: ["m:1", "m:2"], modelChanges: 0 };
  const tabs = () => state.order.map((key) => ({ key, machine: "m" }));
  const context = {
    document: { activeElement: null, body, getElementById: () => ({ hidden: false, dataset: {} }), querySelectorAll: () => [] },
    MutationObserver: class {
      constructor(fn) {
        observers.push(fn);
      }
      observe() {}
    },
    rowDrag: null,
    snoozeLimitStops() {},
    get tabs() {
      return tabs();
    },
    firstSeen: new Map(),
    nowS: () => 117,
    selected: new Set(),
    tabByKey: (key) => tabs().find((t) => t.key === key),
    noteReturns() {},
    projectGroups: () => [],
    projectsWaiting: false,
    ui: { project: "", search: "", sidebarOpen: screen === "threads" },
    isGlobalOrchestratorTab: () => false,
    projectListsPending: () => false,
    projectNameMark: (name) => name,
    icon: () => "",
    projectFilter: { innerHTML: "", setAttribute() {} },
    threadList,
    projectCount: () => 1,
    MOD_LABEL: "⌘",
    machines: new Map([["m", {}]]),
    LOCAL: "local",
    threadsSeen: {},
    lastSeen: () => "",
    machineName: () => "",
    machineNote() {},
    noteThreadsSeen() {},
    offlineMachines: () => [],
    emptyListText: () => "",
    renderSyncAlert() {},
    drawSidebarToggle() {},
    scheduleWake() {},
    modelChanged: () => state.modelChanges++,
    shelfRows: (inScope) => inScope.map((tab) => ({ dataset: { tab: tab.key }, since: { dataset: { since: "100" }, textContent: "", closest: () => null } })),
    searchRows: () => [],
  };
  vm.createContext(context);
  vm.runInContext(
    [
      between("function rowTabs", "function markThreadSeen"),
      between("const durationLabel", "\n"),
      between("let linkDownAt", "function noteHubLink"),
      between("function tickDurations", "visibleInterval("),
      between("function renderSidebar", "function sidebarSummary"),
    ].join("\n"),
    context,
  );
  const shown = () => threadList.children.map((row) => row.dataset.tab);
  const durations = () => threadList.children.map((row) => row.since.textContent);
  const show = (next) => {
    body.dataset.phoneScreen = next;
    for (const fn of observers) fn();
  };
  return { context, state, shown, durations, show };
}

test("a render while the phone shows a thread leaves the hidden list alone, and the list is current once it shows", () => {
  const s = sidebar({ screen: "threads" });
  s.context.renderSidebar();
  assert.deepEqual(s.shown(), ["m:1", "m:2"]);
  s.show("thread");
  s.state.order = ["m:3", "m:1", "m:2"];
  s.context.renderSidebar();
  assert.deepEqual(s.shown(), ["m:1", "m:2"]);
  assert.equal(s.state.modelChanges, 2, "the other surfaces still hear of every change");
  s.show("threads");
  assert.deepEqual(s.shown(), ["m:3", "m:1", "m:2"]);
  assert.deepEqual(s.durations(), ["17s", "17s", "17s"], "the rows drawn as the list comes back show their timers at once");
});

test("a list read while hidden, as stepping or settling the open thread does, gets the current rows", () => {
  const s = sidebar({ screen: "orchestrator" });
  s.context.renderSidebar();
  assert.deepEqual(s.shown(), []);
  s.state.order = ["m:2", "m:1"];
  s.context.renderSidebar();
  assert.deepEqual([...s.context.rowTabs()].map((t) => t.key), ["m:2", "m:1"]);
});

test("on a wider screen the open sidebar draws on every render and a closed one waits until it opens", () => {
  const s = sidebar({ screen: undefined });
  s.context.ui.sidebarOpen = true;
  s.context.renderSidebar();
  assert.deepEqual(s.shown(), ["m:1", "m:2"]);
  s.context.ui.sidebarOpen = false;
  s.state.order = ["m:2"];
  s.context.renderSidebar();
  assert.deepEqual(s.shown(), ["m:1", "m:2"]);
  s.context.ui.sidebarOpen = true;
  s.show(undefined);
  assert.deepEqual(s.shown(), ["m:2"]);
});

test("renaming from the header menu while the list is hidden edits the title in the header", () => {
  const edited = [];
  const header = { replaced: null, replaceWith(input) { this.replaced = input; } };
  const context = {
    document: {
      body: { dataset: {} },
      querySelector: (selector) => (selector === "#chat-head .crumb button.title" ? header : null),
      createElement: () => ({ setAttribute() {} }),
    },
    ui: { sidebarOpen: false },
    threadTitle: (tab) => tab.title,
    editTitle: (tab, input) => edited.push([tab.key, input]),
    renderSidebar() {},
    threadList: { querySelector: () => null },
  };
  vm.createContext(context);
  vm.runInContext([between("const threadListShown", "\n"), between("function startRename", "/** Enter or blur")].join("\n"), context);
  context.startRename({ key: "m:1", title: "Fix the build" });
  assert.equal(edited.length, 1);
  assert.equal(edited[0][0], "m:1");
  assert.equal(edited[0][1], header.replaced);
  assert.equal(header.replaced.value, "Fix the build");
  assert.equal(context.ui.renaming, undefined, "no row is left waiting for an input nobody wired");
});
