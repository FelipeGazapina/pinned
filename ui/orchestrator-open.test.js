import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (file) => readFileSync(new URL(file, import.meta.url), "utf8");
const sidebar = read("./gui-sidebar.js");
const slice = (from, to) => sidebar.slice(sidebar.indexOf(from), sidebar.indexOf(to));

const RULE = "─".repeat(100);
// The end of a Captain's answer on 2026-11-10, as claude 2.1.322 left it on screen while idle.
const CAPTAIN_SCREEN = [
  "  Both are in Settings › Archive and can be brought back.",
  "  - Not archived: the restart didn't reopen the rest of the old worker tabs (19–20, 23, 25–30, 32–34,",
  "  36–40), so there was nothing left to archive.",
  "  - Skipped: none. No tab had uncommitted work or was still running.",
  "",
  RULE,
  "❯",
  RULE,
  "  ⏵⏵ bypass permissions on (shift+tab to cycle)",
];

/** The sidebar's two ways into an orchestrator, with `activate` reading every tab's screen for
 * claude's menu as the page's render does. `fail` makes activate throw that instead. */
function load({ fail } = {}) {
  const tabs = [
    { key: "mac:tab-18", machine: "mac", view: { id: "tab-18", orchestrator_global: true }, screen: [RULE, "❯", RULE] },
    { key: "mac:tab-5", machine: "mac", view: { id: "tab-5", role: "orchestrator", running: true }, screen: CAPTAIN_SCREEN },
  ];
  const app = {
    tabs,
    told: [],
    logged: [],
    active: null,
    console: { error: (...args) => app.logged.push(args) },
    globalHostMachine: () => "mac",
    isGlobalOrchestratorTab: (tab) => tab?.view?.orchestrator_global === true,
    narrow: () => false,
    toggleSidebar() {},
    tell: (html) => app.told.push(html),
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    callOn: async (_machine, call) => (call.cmd === "open_orchestrator" ? "tab-5" : null),
    startTab: async () => {},
    activate(tab) {
      if (fail) throw fail;
      for (const t of tabs) t.menu = app.menuOn(t.screen);
      app.active = tab;
    },
    async revealTab(machine, id) {
      const tab = tabs.find((t) => t.key === `${machine}:${id}`);
      app.activate(tab);
      return tab;
    },
  };
  app.globalThis = app;
  vm.createContext(app);
  vm.runInContext(read("./terminal-menu.js"), app);
  vm.runInContext(
    `${slice("async function openGlobalChat", "function renderSidebar")}\n${slice("async function openOrchestrator", "/** A new session in the thread's folder")}\nObject.assign(globalThis, { openGlobalChat, openOrchestrator });`,
    app,
  );
  return app;
}

test("the orchestrator opens while a Captain's screen shows a › in its answer", async () => {
  const app = load();
  await app.openGlobalChat();
  assert.deepEqual(app.told, []);
  assert.equal(app.active?.key, "mac:tab-18");
});

test("the Captain opens from its project while its own screen shows a › in its answer", async () => {
  const app = load();
  await app.openOrchestrator("mac", "/work/client-apps");
  assert.deepEqual(app.told, []);
  assert.equal(app.active?.key, "mac:tab-5");
});

test("a failed open logs the error its toast names, with its stack", async () => {
  const fail = new TypeError("Cannot read properties of undefined (reading 'length')");
  const app = load({ fail });
  await app.openGlobalChat();
  await app.openOrchestrator("mac", "/work/client-apps");
  assert.deepEqual(app.told, [
    "Couldn't open the orchestrator: TypeError: Cannot read properties of undefined (reading &#39;length&#39;)",
    "Couldn't open the Captain: TypeError: Cannot read properties of undefined (reading &#39;length&#39;)",
  ]);
  assert.equal(app.logged.length, 2);
  for (const args of app.logged) assert.equal(args.at(-1), fail);
});
