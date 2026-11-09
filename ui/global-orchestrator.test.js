import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

// Objects built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => JSON.parse(JSON.stringify(value));

// The pure half: merge, writes and notes, driven with literal machines. The consts the file keeps
// stay in its script scope, so a bridge hands its API out.
const pure = () => {
  const context = {};
  vm.createContext(context);
  vm.runInContext(`${read("global-orchestrator.js")}\nthis.api = { mergeGlobalSettings, globalSettingsWrites, globalSettingsNotes, isOlderPinned };`, context);
  return context.api;
};

const macMini = (over = {}) => ({ id: "mm", name: "Mac mini", online: true, settings: null, update: false, error: null, ...over });

test("the account's view takes the host's fields and names the host machine", () => {
  const { mergeGlobalSettings } = pure();
  const view = here(mergeGlobalSettings([
    macMini({ settings: { name: "Admiral", host: true, busy: "refuse", reports: "tool_only", group_reports: false, read_aloud: true } }),
    { id: "mb", name: "MacBook", online: true, settings: { name: "Orchestrator", host: false, busy: "queue", reports: "auto", group_reports: true, read_aloud: false }, update: false, error: null },
  ]));
  assert.deepEqual(view, { name: "Admiral", host: "mm", busy: "refuse", reports: "tool_only", group_reports: false, read_aloud: true });
});

test("with no host the first machine that answered speaks for the account", () => {
  const { mergeGlobalSettings } = pure();
  const view = here(mergeGlobalSettings([
    { id: "mb", name: "MacBook", online: true, settings: { name: "Comandante", host: false }, update: false, error: null },
    macMini({ settings: { name: "Admiral", host: false } }),
  ]));
  assert.deepEqual(view, { name: "Comandante", host: null, busy: "queue", reports: "auto", group_reports: true, read_aloud: false });
});

test("machines that never answered leave the defaults standing", () => {
  const { mergeGlobalSettings } = pure();
  const view = here(mergeGlobalSettings([macMini({ online: false }), { id: "mb", name: "MacBook", online: true, update: true }]));
  assert.deepEqual(view, { name: "Orchestrator", host: null, busy: "queue", reports: "auto", group_reports: true, read_aloud: false });
});

test("a save writes every online machine, the host flag only to the chosen one", () => {
  const { globalSettingsWrites } = pure();
  const writes = here(globalSettingsWrites(
    { name: "Admiral", host: "mm", busy: "refuse", reports: "auto", group_reports: false, read_aloud: true },
    [
      macMini(),
      { id: "mb", name: "MacBook", online: true, settings: null, update: false, error: null },
      { id: "vps", name: "Linux VPS", online: false, settings: null, update: false, error: null },
      { id: "old", name: "Old Mac", online: true, settings: null, update: true, error: null },
    ],
  ));
  assert.deepEqual(writes, [
    { machine: "mm", settings: { name: "Admiral", host: true, busy: "refuse", reports: "auto", group_reports: false, read_aloud: true } },
    { machine: "mb", settings: { name: "Admiral", host: false, busy: "refuse", reports: "auto", group_reports: false, read_aloud: true } },
  ]);
});

test("the notes name every machine the save could not reach, each in its own words", () => {
  const { globalSettingsNotes } = pure();
  const notes = here(globalSettingsNotes([
    macMini(),
    { id: "old", name: "Old Mac", online: true, update: true, error: null },
    { id: "vps", name: "Linux VPS", online: false, update: false, error: null },
    { id: "mb", name: "MacBook", online: true, update: false, error: "timed out" },
  ]));
  assert.deepEqual(notes, ["Old Mac needs to update pinned", "Linux VPS is offline; it updates when it reconnects", "MacBook: timed out"]);
});

test("an older node's unknown command and an older hub's refusal both read as needing an update", () => {
  const { isOlderPinned } = pure();
  assert.equal(isOlderPinned("this machine runs pinned 0.8.0, which can't answer that: unknown variant GlobalOrchestratorSettings"), true);
  assert.equal(isOlderPinned("this hub runs pinned 1.2.0, which can't answer that yet: the global orchestrator's relay is not on the hub"), true);
  assert.equal(isOlderPinned("connection closed"), false);
  assert.equal(isOlderPinned(null), false);
});

// The page half: a small document whose elements the section binds to, and the calls it makes.
function page({ settings = {}, older = false, offline = false } = {}) {
  const elements = {};
  const element = (id, extra = {}) => (elements[id] ??= {
    id,
    value: "",
    checked: false,
    disabled: false,
    textContent: "",
    innerHTML: "",
    outerHTML: "",
    dataset: {},
    classList: { toggle() {} },
    querySelectorAll: () => [],
    ...extra,
  });
  const calls = [];
  const events = [];
  const activated = [];
  const surfaces = [];
  const told = [];
  const pages = [];
  const machines = new Map([
    ["mm", { view: { id: "mm", name: "Mac mini", online: true } }],
    ["mb", { view: { id: "mb", name: "MacBook", online: true } }],
    ["vps", { view: { id: "vps", name: "Linux VPS", online: false } }],
  ]);
  const listeners = {};
  const context = {
    machines,
    LOCAL: "local",
    machineName: (id) => ({ mm: "Mac mini", mb: "MacBook", vps: "Linux VPS", local: "This Mac" })[id] ?? "machine",
    callOn: async (machine, call) => {
      calls.push([machine, call]);
      if (call.cmd === "global_orchestrator_settings") {
        if (machine === "mb" && older) throw new Error("this machine runs pinned 0.8.0, which can't answer that: unknown variant GlobalOrchestratorSettings");
        return { name: "Orchestrator", host: false, busy: "queue", reports: "auto", group_reports: true, read_aloud: false, ...settings[machine] };
      }
      if (call.cmd === "set_global_orchestrator_settings") return { ...call.settings };
      if (call.cmd === "open_global_orchestrator") return "g-1";
      return null;
    },
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    console: { warn: () => {} },
    document: {
      getElementById: (id) => element(id),
      dispatchEvent: (e) => events.push(e),
      hidden: false,
    },
    CustomEvent: class {
      constructor(type, init) {
        this.type = type;
        this.detail = init?.detail;
      }
    },
    SHELL_PAGES: {},
    pageView: { addEventListener: (type, fn) => (listeners[type] = fn) },
    viewListeners: [],
    setInterval: () => 0,
    shellPage: () => "global-projects",
    setPage: (html) => pages.push(html),
    headHtml: (title, rest = "") => `<h1>${title}</h1>${rest}`,
    openPage: (name) => pages.push(`open:${name}`),
    revealTab: async (machine, id) => {
      const tab = { machine, view: { id } };
      activated.push(tab);
      return tab;
    },
    activate: (tab) => activated.push(tab),
    addSurface: (tab, surface) => surfaces.push([tab, surface]),
    tell: (message) => told.push(message),
  };
  vm.createContext(context);
  vm.runInContext(`${read("global-orchestrator.js")}\n${read("composer-tags.js")}\n${read("gui-global-orchestrator.js")}`, context);
  return { context, element, elements, calls, events, activated, surfaces, told, pages, click: (e) => listeners.click(e) };
}

test("the section renders one account-wide setting and the machines that cannot take it", async () => {
  const { context } = page({ settings: { mm: { name: "Admiral", host: true, busy: "refuse", reports: "tool_only", group_reports: false, read_aloud: true } }, older: true });
  const html = await context.globalOrchestratorSettingsHtml();
  assert.match(html, /<h2 class="looks-title">Global orchestrator<\/h2>/);
  assert.match(html, /value="Admiral"/);
  assert.match(html, /class="chip go-preset active" data-name="Admiral"/);
  assert.match(html, /class="chip go-host active" data-machine="mm"/);
  assert.match(html, /data-machine="mb" disabled/);
  assert.match(html, /value="refuse" selected/);
  assert.match(html, /value="tool_only" selected/);
  assert.match(html, /id="go-read-aloud" checked/);
  assert.doesNotMatch(html, /id="go-group-reports" checked/);
  assert.match(html, /Admiral's memory/);
  assert.match(html, /MacBook needs to update pinned/);
  assert.match(html, /Linux VPS is offline; it updates when it reconnects/);
});

test("saving writes the shared fields everywhere and tells the page through the event", async () => {
  const { context, element, calls, events } = page({ settings: { mm: { name: "Admiral", host: true } } });
  await context.globalOrchestratorSettingsHtml();
  const presets = [element("go-preset-comandante")];
  element("global-orchestrator-settings").querySelectorAll = (sel) => (sel === ".go-preset" ? presets : []);
  context.bindGlobalOrchestratorSettings();
  element("go-preset-comandante").dataset.name = "Comandante";
  element("go-preset-comandante").onclick();
  element("go-name").value = "Comandante";
  element("go-busy").value = "queue";
  element("go-reports").value = "tool_only";
  element("go-group-reports").checked = false;
  element("go-read-aloud").checked = true;
  await element("go-save").onclick();
  const writes = here(calls.filter(([, call]) => call.cmd === "set_global_orchestrator_settings"));
  assert.deepEqual(writes, [
    ["mm", { cmd: "set_global_orchestrator_settings", settings: { name: "Comandante", host: true, busy: "queue", reports: "tool_only", group_reports: false, read_aloud: true } }],
    ["mb", { cmd: "set_global_orchestrator_settings", settings: { name: "Comandante", host: false, busy: "queue", reports: "tool_only", group_reports: false, read_aloud: true } }],
  ]);
  assert.equal(events.length, 1);
  assert.equal(events[0].type, "global-orchestrator-settings");
  assert.deepEqual(here(events[0].detail), { name: "Comandante", host: true, busy: "queue", reports: "tool_only", group_reports: false, read_aloud: true });
  assert.equal(element("go-status").textContent, "Saved");
  assert.equal(element("go-error").textContent, "");
});

test("saving refuses a host that cannot take the write, without sending any", async () => {
  const { context, element, calls } = page({ older: true });
  await context.globalOrchestratorSettingsHtml();
  const hosts = [Object.assign(element("go-host-mb"), { dataset: { machine: "mb" } })];
  element("global-orchestrator-settings").querySelectorAll = (sel) => (sel === ".go-host" ? hosts : []);
  context.bindGlobalOrchestratorSettings();
  hosts[0].onclick();
  await element("go-save").onclick();
  assert.equal(element("go-error").textContent, "Choose which online machine the orchestrator runs on.");
  assert.equal(calls.filter(([, call]) => call.cmd === "set_global_orchestrator_settings").length, 0);
});

test("the memory opens on the host's global tab, as the panel of that tab", async () => {
  const { context, activated, surfaces, told } = page({ settings: { mm: { host: true } } });
  await context.globalOrchestratorSettingsHtml();
  await context.openGlobalMemory();
  assert.deepEqual(told, []);
  assert.equal(activated.length, 1);
  assert.equal(activated[0].machine, "mm");
  assert.deepEqual(here(surfaces), [[{ machine: "mm", view: { id: "g-1" } }, { id: "memory", kind: "memory" }]]);
});

test("without a host the memory says to pick one first", async () => {
  const { context, told, activated, surfaces } = page();
  await context.globalOrchestratorSettingsHtml();
  await context.openGlobalMemory();
  assert.deepEqual(activated, []);
  assert.deepEqual(surfaces, []);
  assert.match(told[0], /Choose which machine Orchestrator runs on in the settings first/);
});

// The projects page: rows out of the catalog, and a machine's chip opening its Captain.
function projectsPage() {
  const captain = { machine: "mm", view: { id: "cap-1", role: "orchestrator" } };
  const worker = (machine, running) => ({ machine, view: { role: "worker", running } });
  const groups = [{ name: "site", tabs: [captain, worker("mm", true), worker("mm", true), worker("mm", false), worker("local", true)], checkouts: [] }];
  const catalog = {
    projects: [
      {
        slug: "site",
        name: "site",
        repo: "github.com/felipe/site",
        machines: [
          { id: "mm", slug: "mac-mini", name: "Mac mini", online: true, captain: "working", workers: 1 },
          { id: "local", slug: "this-mac", name: "This Mac", online: true, captain: "none", workers: 0 },
        ],
      },
      { slug: "infra", name: "infra", repo: "", machines: [{ id: "vps", slug: "linux-vps", name: "Linux VPS", online: false, captain: "none", workers: 0 }] },
    ],
    machines: [
      { slug: "mac-mini", name: "Mac mini", online: true },
      { slug: "this-mac", name: "This Mac", online: true },
      { slug: "linux-vps", name: "Linux VPS", online: false },
    ],
    name: "Orchestrator",
  };
  const harness = page();
  Object.assign(harness.context, {
    machines: new Map(),
    tagCatalog: () => catalog,
    projectGroups: () => groups,
    machineDirectory: () => ({ localId: "local", localName: "This Mac", names: {}, online: {} }),
    phaseOf: () => "working",
    projectRepos: new Map(),
  });
  return { ...harness, captain, catalog, click: harness.click };
}

test("the projects page lists every project with its machines, Captain and running workers", () => {
  const { context, pages } = projectsPage();
  context.openGlobalProjects();
  context.renderGlobalProjectsPage();
  const html = pages.at(-1);
  assert.match(html, /<h1>Projects<\/h1>/);
  assert.match(html, /2 projects · 3 machines/);
  assert.match(html, /<span class="gp-name">site<\/span>/);
  assert.match(html, /<span class="gp-state working">Working<\/span>/);
  assert.match(html, /<span class="gp-repo">github.com\/felipe\/site<\/span>/);
  assert.match(html, /data-project="site" data-machine="mm">Mac mini<span class="gp-captain working">Working<\/span><span class="gp-count">2 workers<\/span><\/button>/);
  assert.match(html, /data-project="site" data-machine="local" disabled title="No Captain on this machine">This Mac<span class="gp-count">1 worker<\/span><\/button>/);
  assert.match(html, /<span class="gp-workers">3 workers<\/span>/);
  assert.match(html, /<span class="gp-repo">no remote<\/span>/);
  assert.match(html, /<span class="gp-state none">No Captain<\/span><span class="gp-repo">no remote<\/span><\/div>\s*<div class="gp-side"><button[^>]*>Linux VPS<\/button><span class="gp-workers">—<\/span>/);
});

test("a project's machine opens its Captain tab when one is open", () => {
  const { context, activated, captain, click } = projectsPage();
  context.renderGlobalProjectsPage();
  const chip = { dataset: { project: "site", machine: "mm" }, disabled: false };
  click({ target: { closest: (sel) => (sel === ".gp-machine" ? chip : null) } });
  assert.deepEqual(activated, [captain]);
  const none = { dataset: { project: "site", machine: "local" }, disabled: true };
  click({ target: { closest: (sel) => (sel === ".gp-machine" ? none : null) } });
  assert.deepEqual(activated, [captain]);
});
