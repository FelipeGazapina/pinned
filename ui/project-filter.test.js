import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import { sessionOn } from "./project-groups.js";

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
const longName = `studio "alpha" & beta`;
const group = {
  key: "repo:pinned",
  name: longName,
  tab: { machine: "studio" },
  checkouts: [
    { machine: "studio", path: "/src/pinned", thread: true },
    { machine: "local", path: "/Users/me/pinned", thread: false },
  ],
};

function labelMarks(html) {
  return [...html.matchAll(/<span class="label"[^>]*>[^<]*<\/span>/g)].map((match) => match[0]);
}

test("the project filter label keeps the full name as its title", () => {
  const start = sidebarSrc.indexOf("function renderSidebar");
  const end = sidebarSrc.indexOf("function shelfRows");
  const whereStart = sidebarSrc.indexOf("function projectWhere");
  const whereEnd = sidebarSrc.indexOf("const projectFilter = document.getElementById");
  const projectFilter = { innerHTML: "", setAttribute() {} };
  const context = {
    document: {
      activeElement: null,
      getElementById: () => ({ hidden: false, dataset: {} }),
    },
    rowDrag: null,
    snoozeLimitStops() {},
    tabs: [],
    firstSeen: new Set(),
    nowS: () => 0,
    selected: new Set(),
    tabByKey: () => null,
    noteReturns() {},
    projectGroups: () => [group],
    ui: { project: "repo:pinned", search: "" },
    projectListsPending: () => false,
    PinnedProjects: { sessionOn },
    machineDirectory: () => directory,
    projectIcon: () => "",
    icon: () => "",
    projectFilter,
    threadList: { replaceChildren() {}, querySelectorAll: () => [] },
    projectCount: () => 1,
    MOD_LABEL: "⌘",
    tickDurations() {},
    scheduleWake() {},
    shelfRows: () => [{}],
    searchRows: () => [],
  };
  vm.createContext(context);
  vm.runInContext(`${escFn}\n${sidebarSrc.slice(whereStart, whereEnd)}\n${sidebarSrc.slice(start, end)}`, context);
  context.renderSidebar();
  const html = projectFilter.innerHTML;
  const label = html.match(/<span class="label"[^>]*>[^<]*<\/span>/)?.[0];
  assert.equal(
    label,
    `<span class="label" title="studio &#34;alpha&#34; &#38; beta">studio &#34;alpha&#34; &#38; beta</span>`,
  );
  const where = html.match(/<span class="where"[^>]*>[^<]*<\/span>/)?.[0];
  assert.equal(where, `<span class="where" title="studio-box +1 (This Mac)">studio-box +1</span>`);
});

test("a project menu row keeps the full name and the machine as titles", () => {
  const start = sidebarSrc.indexOf("function projectWhere");
  const end = sidebarSrc.indexOf("function closeProjectMenu");
  const projectFilter = { setAttribute() {} };
  const projectMenu = {
    hidden: true,
    style: {},
    input: { value: "", focus() {} },
    list: { innerHTML: "" },
    set innerHTML(_value) {},
    querySelector(selector) {
      if (selector === "input") return this.input;
      if (selector === ".list") return this.list;
      return null;
    },
  };
  const context = {
    document: {
      getElementById: () => projectFilter,
      createElement: () => projectMenu,
      body: { append() {} },
      querySelector: () => ({ getBoundingClientRect: () => ({ left: 8, bottom: 40 }) }),
    },
    innerWidth: 1280,
    refreshProjectLists() {},
    projectGroups: () => [group],
    ui: { project: "repo:pinned" },
    PinnedProjects: { sessionOn },
    machineDirectory: () => directory,
    projectIcon: () => "",
    icon: () => "",
    redrawProjectMenu: () => {},
  };
  vm.createContext(context);
  vm.runInContext(`${escFn}\n${sidebarSrc.slice(start, end)}\nglobalThis.openProjectMenu = openProjectMenu;`, context);
  context.openProjectMenu();
  assert.deepEqual(labelMarks(projectMenu.list.innerHTML), [
    `<span class="label" title="All projects">All projects</span>`,
    `<span class="label" title="studio &#34;alpha&#34; &#38; beta">studio &#34;alpha&#34; &#38; beta</span>`,
  ]);
  const where = projectMenu.list.innerHTML.match(/<span class="where"[^>]*>[^<]*<\/span>/)?.[0];
  assert.equal(where, `<span class="where" title="studio-box +1 (This Mac)">studio-box +1</span>`);
});
