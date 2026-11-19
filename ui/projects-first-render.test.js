import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import * as PinnedProjects from "./project-groups.js";

const sidebarSrc = readFileSync(new URL("./gui-sidebar.js", import.meta.url), "utf8");
const settingsSrc = readFileSync(new URL("./settings.js", import.meta.url), "utf8");
const escStart = settingsSrc.indexOf("const esc = ");
const escFn = settingsSrc.slice(escStart, settingsSrc.indexOf("\n", escStart));

const directory = {
  localId: "local",
  localName: "This Mac",
  names: { studio: "studio-box" },
  online: { studio: true, local: true },
};
const projectKey = "studio:/src/pinned";
const allProjects = `<span class="label" title="All projects">All projects</span>`;
const projectLabel = `<span class="label" title="pinned">pinned</span>`;
const projectWhere = `<span class="where" title="studio-box">studio-box</span>`;

function labelOf(html) {
  return html.match(/<span class="label"[^>]*>[^<]*<\/span>/)?.[0];
}

function whereOf(html) {
  return html.match(/<span class="where"[^>]*>[^<]*<\/span>/)?.[0];
}

/** The sidebar as a hub `hello` leaves it: studio is online with one project, and the filter is on that project.
 * `PinnedProjects` is left unset, as it is until project-groups-load.js runs. The listeners the sidebar registers
 * on `document` are kept, so a test can fire the signal the loader sends. */
function sidebar() {
  const start = sidebarSrc.indexOf("function renderSidebar");
  const end = sidebarSrc.indexOf("function sidebarSummary");
  const groupsStart = sidebarSrc.indexOf("function projectGroups(");
  const groupsEnd = sidebarSrc.indexOf("const projectFilter = document.getElementById");
  const listeners = {};
  const projectFilter = { innerHTML: "", setAttribute() {} };
  const context = {
    document: {
      activeElement: null,
      body: { dataset: {} },
      getElementById: () => ({ hidden: false, dataset: {} }),
      addEventListener: (type, fn) => (listeners[type] = fn),
    },
    MutationObserver: class {
      observe() {}
    },
    rowDrag: null,
    snoozeLimitStops() {},
    tabs: [],
    firstSeen: new Set(),
    nowS: () => 0,
    selected: new Set(),
    tabByKey: () => null,
    noteReturns() {},
    isGlobalOrchestratorTab: () => false,
    projectPathOf: (tab) => tab.path,
    projectName: (tab) => tab.name,
    projectRepos: new Map(),
    sideSettings: { grouping: true },
    sortedProjects: (list) => [...list],
    machines: new Map([["studio", { view: { id: "studio", online: true }, projects: [{ name: "pinned", path: "/src/pinned" }] }]]),
    ui: { project: projectKey, search: "", sidebarOpen: true },
    askGlobalSettings() {},
    projectListsPending: () => false,
    machineDirectory: () => directory,
    projectIcon: () => "",
    icon: () => "",
    guiOn: () => true,
    projectFilter,
    threadList: { replaceChildren() {}, querySelectorAll: () => [] },
    projectCount: () => 1,
    MOD_LABEL: "⌘",
    LOCAL: "local",
    threadsSeen: {},
    lastSeen: () => "",
    machineName: () => "",
    machineNote() {},
    noteThreadsSeen() {},
    offlineMachines: () => [],
    emptyListText: () => "No threads yet",
    renderSyncAlert() {},
    projectsOnManyMachines: () => new Set(),
    drawSidebarToggle() {},
    tickDurations() {},
    scheduleWake() {},
    modelChanged() {},
    shelfRows: () => [{ matches: () => false }],
    searchRows: () => [],
  };
  vm.createContext(context);
  vm.runInContext(
    `${escFn}\n${sidebarSrc.slice(groupsStart, groupsEnd)}\n${sidebarSrc.slice(start, end)}`,
    context,
  );
  return { context, projectFilter, listeners };
}

test("a sidebar drawn before project-groups-load.js runs does not throw and keeps the project filter", () => {
  const { context, projectFilter } = sidebar();
  assert.doesNotThrow(() => context.renderSidebar());
  assert.equal(labelOf(projectFilter.innerHTML), allProjects);
  assert.equal(context.ui.project, projectKey);
});

test("the loader's projects:loaded signal draws the project the filter kept", () => {
  const { context, projectFilter, listeners } = sidebar();
  context.renderSidebar();
  context.PinnedProjects = PinnedProjects;
  listeners["projects:loaded"]();
  assert.equal(labelOf(projectFilter.innerHTML), projectLabel);
  assert.equal(whereOf(projectFilter.innerHTML), projectWhere);
  assert.equal(context.ui.project, projectKey);
});

test("with project-groups loaded first, the sidebar draws the project as before", () => {
  const { context, projectFilter } = sidebar();
  context.PinnedProjects = PinnedProjects;
  context.renderSidebar();
  assert.equal(labelOf(projectFilter.innerHTML), projectLabel);
  assert.equal(whereOf(projectFilter.innerHTML), projectWhere);
  assert.equal(context.ui.project, projectKey);
});
