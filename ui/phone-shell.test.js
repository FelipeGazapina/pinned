import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("./gui-phone.js", import.meta.url), "utf8");
const between = (start, end) => {
  const from = source.indexOf(start);
  assert.ok(from >= 0, `gui-phone.js has ${start}`);
  return source.slice(from, source.indexOf(end, from));
};

/** The phone shell's state rules over a page of `width` pixels with these tabs, where the openers
 * do what the app's do to the state the shell reads: pages, the open thread and the thread list. */
function shell({ width = 390, coarse = true } = {}) {
  const thread = { key: "mini:1", view: {} };
  const orchestrator = { key: "mini:9", view: { orchestrator_global: true } };
  const attrs = new Set();
  const body = { dataset: { mode: "gui" }, toggleAttribute: (name, on) => (on ? attrs.add(name) : attrs.delete(name)) };
  const context = {
    document: { body, activeElement: null },
    matchMedia: (query) => ({ matches: query === "(pointer: coarse)" ? coarse : /max-width: (\d+)px/.test(query) && width <= Number(query.match(/(\d+)px/)[1]) }),
    narrow: (px) => width <= px,
    guiOn: () => body.dataset.mode === "gui",
    ui: { sidebarOpen: true },
    active: null,
    isGlobalOrchestratorTab: (tab) => !!tab.view.orchestrator_global,
    shellPage: () => (body.dataset.page && body.dataset.page !== "settings" ? body.dataset.page : null),
    settingsOpen: () => body.dataset.page === "settings",
    openPage: (name) => {
      context.ui.sidebarOpen = false;
      body.dataset.page = name;
    },
    activate: (tab) => {
      delete body.dataset.page;
      context.active = tab;
    },
    openGlobalChat: () => {
      context.ui.sidebarOpen = false;
      context.activate(orchestrator);
    },
    toggleSidebar: (open) => (context.ui.sidebarOpen = open),
    drawPhoneTabs: () => {},
    drawPhoneChips: () => {},
    drawMore: () => {},
    sidebarEl: { toggleAttribute: () => {} },
    phoneSearching: false,
  };
  vm.createContext(context);
  vm.runInContext(
    [
      between("const PHONE_MAX", "Object.assign(ICON_PATHS"),
      between("function phoneScreenOf", "/** Shows `screen`"),
      between("function phoneGo", "const phoneTabsEl"),
      between("let phoneLoaded", "// A phone opens"),
      "phoneLoaded = true;",
    ].join("\n"),
    context,
  );
  const shown = () => {
    vm.runInContext("phoneSync()", context);
    return { screen: body.dataset.phoneScreen ?? null, tabs: attrs.has("data-phone-tabs") };
  };
  /** A row tapped in the list: the thread opens and the list closes, as the sidebar's click does on a phone. */
  const pick = (tab = thread) => {
    context.activate(tab);
    context.toggleSidebar(false);
  };
  return { context, body, shown, pick, thread, go: (screen) => vm.runInContext(`phoneGo(${JSON.stringify(screen)})`, context) };
}

test("a phone opens on Threads with the tab bar, and a thread picked there is its own screen without it", () => {
  const s = shell();
  assert.deepEqual(s.shown(), { screen: "threads", tabs: true });
  s.pick();
  assert.deepEqual(s.shown(), { screen: "thread", tabs: false });
});

test("each tab leads to its screen and keeps the tab bar", () => {
  const s = shell();
  s.pick();
  s.go("needs-you");
  assert.deepEqual(s.shown(), { screen: "needs-you", tabs: true });
  s.go("orchestrator");
  assert.deepEqual(s.shown(), { screen: "orchestrator", tabs: true });
  s.go("more");
  assert.deepEqual(s.shown(), { screen: "more", tabs: true });
  s.go("threads");
  assert.deepEqual(s.shown(), { screen: "threads", tabs: true });
  assert.equal(s.body.dataset.page, undefined, "Threads closes the page it came from");
});

test("a page or Settings opened from More stays under the More tab", () => {
  const s = shell();
  s.go("more");
  s.context.openPage("usage");
  assert.deepEqual(s.shown(), { screen: "more", tabs: true });
  s.body.dataset.page = "settings";
  assert.deepEqual(s.shown(), { screen: "more", tabs: true });
});

test("the back from a thread, and from a Needs you card's thread, lands on Threads", () => {
  const s = shell();
  s.go("needs-you");
  s.context.activate(s.thread);
  assert.deepEqual(s.shown(), { screen: "thread", tabs: false }, "a card's Open shows the thread");
  s.go("threads");
  assert.deepEqual(s.shown(), { screen: "threads", tabs: true });
});

test("the open thread with nothing open is the Threads screen", () => {
  const s = shell();
  s.context.toggleSidebar(false);
  assert.deepEqual(s.shown(), { screen: "threads", tabs: true });
  s.go("thread");
  assert.deepEqual(s.shown(), { screen: "threads", tabs: true });
});

test("the tab bar hides while a touch keyboard is up, and stays for a mouse at phone width", () => {
  const s = shell();
  s.context.document.activeElement = { matches: (sel) => sel.startsWith("textarea") };
  assert.deepEqual(s.shown(), { screen: "threads", tabs: false });
  const mouse = shell({ coarse: false });
  mouse.context.document.activeElement = { matches: () => true };
  assert.deepEqual(mouse.shown(), { screen: "threads", tabs: true });
});

test("above 768px, and in the terminal view, there is no phone screen and no tab bar", () => {
  const wide = shell({ width: 769 });
  assert.deepEqual(wide.shown(), { screen: null, tabs: false });
  wide.pick();
  assert.deepEqual(wide.shown(), { screen: null, tabs: false });
  const edge = shell({ width: 768 });
  assert.deepEqual(edge.shown(), { screen: "threads", tabs: true });
  edge.body.dataset.mode = "terminal";
  assert.deepEqual(edge.shown(), { screen: null, tabs: false });
});
