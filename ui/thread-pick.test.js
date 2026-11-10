import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(name, import.meta.url), "utf8");
function between(src, startAt, endAt) {
  const start = src.indexOf(startAt);
  const end = src.indexOf(endAt, start + startAt.length);
  if (start < 0 || end < 0) throw new Error(`missing slice ${startAt}`);
  return src.slice(start, end);
}
const main = read("./main.js");
const gui = read("./gui.js");
const composer = read("./gui-composer.js");
const shell = read("./gui-shell.js");
const sidebar = read("./gui-sidebar.js");
const timeline = read("./gui-timeline.js");
const settings = read("./settings.js");

/** What a phone's and a desktop's browser answer the media queries the app asks. */
const SCREENS = {
  phone: { "(pointer: coarse)": true, "(any-pointer: fine)": false, "(max-width: 768px)": true },
  desktop: { "(pointer: coarse)": false, "(any-pointer: fine)": true, "(max-width: 768px)": false },
};

function pageOn(screen) {
  const context = {
    console,
    tabs: [],
    active: null,
    ui: { sidebarOpen: true },
    focused: null,
    matchMedia: (query) => ({ matches: SCREENS[screen][query] ?? false }),
    picker: { hidden: true },
    prompt: { focus: () => (context.focused = "prompt") },
    document: { body: { dataset: { mode: "gui", sidebar: "open" } } },
    pickerOpen: () => false,
    settingsOpen: () => false,
    showTermText() {},
    refit() {},
    render() {},
    markSeen() {},
  };
  vm.createContext(context);
  return context;
}

const tabOf = (machine, id, view = {}) => ({ key: `${machine}:${id}`, machine, view: { id, cwd: "/work/alpha", ...view }, term: { focus() {} } });

function switching(screen) {
  const context = pageOn(screen);
  vm.runInContext(
    `const guiOn = () => document.body.dataset.mode === "gui";
${between(main, "function activate(tab)", "// The window's minimum height")}
${between(composer, "function touchComposer", "function platformEnterMod")}`,
    context,
  );
  return context;
}

test("picking a thread on a phone leaves the keyboard down", () => {
  const page = switching("phone");
  page.activate(tabOf("box", "tab-1"));
  assert.equal(page.active.key, "box:tab-1");
  assert.equal(page.focused, null);
});

test("picking a thread on a desktop puts the focus in its prompt", () => {
  const page = switching("desktop");
  page.activate(tabOf("box", "tab-1"));
  assert.equal(page.focused, "prompt");
});

function palette(screen) {
  const context = pageOn(screen);
  context.activate = (tab) => (context.active = tab);
  for (const name of ["threadLead", "threadTrail", "threadSubHtml", "latestUserAt", "prTerms", "projectName", "esc"]) context[name] = () => "";
  context.prTerms = () => [];
  context.threadTitle = (tab) => tab.view.id;
  context.threadOf = () => ({});
  context.palette = {};
  vm.runInContext(
    `${between(gui, "const narrow =", "\n")}
${between(sidebar, "function toggleSidebar", 'document.getElementById("sidebar-toggle")')}
${between(shell, "function threadItem", "const openProjectIn")}`,
    context,
  );
  return context;
}

test("a thread picked in the palette on a phone is not left under the sidebar", () => {
  const page = palette("phone");
  vm.runInContext("threadItem", page)(tabOf("box", "tab-1")).run();
  assert.equal(page.active.key, "box:tab-1");
  assert.equal(page.document.body.dataset.sidebar, "closed");
});

test("a thread picked in the palette on a desktop keeps the sidebar open", () => {
  const page = palette("desktop");
  vm.runInContext("threadItem", page)(tabOf("box", "tab-1")).run();
  assert.equal(page.active.key, "box:tab-1");
  assert.equal(page.document.body.dataset.sidebar, "open");
});

/** The global orchestrator's thread view with one Captain report drawn, and a click on its Captain. */
function reportClick(tabs, { machineId = "box", tabId = "tab-old-1", account = null } = {}) {
  const context = pageOn("desktop");
  context.LOCAL = "local";
  context.account = account;
  context.tabs = tabs;
  context.active = tabs[0];
  context.revealed = [];
  context.told = [];
  context.esc = (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
  context.markdown = (text) => text;
  context.whenHtml = () => "";
  context.threadOf = (tab) => ({ workspace: { project: tab.view.cwd } });
  context.keyOf = (machine, id) => `${machine}:${id}`;
  context.tabByKey = (key) => context.tabs.find((tab) => tab.key === key) ?? null;
  context.activate = (tab) => (context.active = tab);
  context.revealTab = (machine, id) => {
    context.revealed.push(`${machine}:${id}`);
    const tab = context.tabByKey(context.keyOf(machine, id));
    if (!tab) return Promise.reject(new Error("that tab is not on this machine"));
    context.activate(tab);
    return Promise.resolve(tab);
  };
  context.tell = (html) => context.told.push(html);
  context.threadView = { addEventListener: (type, listener) => type === "click" && (context.onClick = listener) };
  vm.runInContext(
    `${between(gui, "const projectName =", "\n")}
${between(sidebar, "const isOrchestrator =", "\n")}
${between(timeline, "function reportRowHtml", "/** The prompt bubble")}
${between(settings, "const localIdOf =", "\n")}
${between(timeline, 'threadView.addEventListener("click"', 'threadView.addEventListener("toggle"')}`,
    context,
  );
  const html = vm.runInContext("reportRowHtml", context)({ project: "alpha", machineName: "sweep-node", machineId, tabId, text: "CI is green." }, "t0");
  const dataset = Object.fromEntries([...html.matchAll(/data-report-([a-z]+)="([^"]*)"/g)].map(([, name, value]) => [`report${name[0].toUpperCase()}${name.slice(1)}`, value]));
  const button = { dataset };
  context.onClick({ target: { closest: (selector) => (selector === "[data-report-tab]" ? button : null) } });
  return context;
}

test("a Captain report from before a restart opens that project's Captain under its new tab id", async () => {
  const global = tabOf("box", "tab-new-9", { orchestrator_global: true, cwd: "/home/.pinned/orchestrator" });
  const worker = tabOf("box", "tab-new-2", { role: "worker" });
  const beta = tabOf("box", "tab-new-3", { role: "orchestrator", cwd: "/work/beta" });
  const captain = tabOf("box", "tab-new-1", { role: "orchestrator" });
  const page = reportClick([global, worker, beta, captain]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(page.active.key, "box:tab-new-1");
  assert.deepEqual(page.revealed, []);
  assert.deepEqual(page.told, []);
});

test("in the desktop app, a report from a Captain on the same Mac opens its tab", async () => {
  const global = tabOf("local", "tab-1a-9", { orchestrator_global: true, cwd: "/home/.pinned/orchestrator" });
  const captain = tabOf("local", "tab-1a-1", { role: "orchestrator" });
  const page = reportClick([global, captain], { machineId: "mac-1", tabId: "tab-1a-1", account: { machine_id: "mac-1" } });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(page.active.key, "local:tab-1a-1");
  assert.deepEqual(page.told, []);
});

test("in the desktop app, a report from before a restart opens the same Mac's Captain under its new tab id", async () => {
  const global = tabOf("local", "tab-1a-9", { orchestrator_global: true, cwd: "/home/.pinned/orchestrator" });
  const captain = tabOf("local", "tab-1a-4", { role: "orchestrator" });
  const page = reportClick([global, captain], { machineId: "mac-1", tabId: "tab-1a-1", account: { machine_id: "mac-1" } });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(page.active.key, "local:tab-1a-4");
  assert.deepEqual(page.revealed, []);
  assert.deepEqual(page.told, []);
});

test("a Captain report whose tab is still open opens that tab", async () => {
  const global = tabOf("box", "tab-old-9", { orchestrator_global: true, cwd: "/home/.pinned/orchestrator" });
  const page = reportClick([global, tabOf("box", "tab-old-1", { role: "orchestrator" })]);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(page.active.key, "box:tab-old-1");
  assert.deepEqual(page.told, []);
});
