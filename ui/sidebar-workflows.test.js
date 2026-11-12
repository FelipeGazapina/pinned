import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const indexSrc = read("index.html");
const sidebarSrc = read("gui-sidebar.js");

const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in gui-sidebar.js`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};

test("the Workflows row is the last shortcut in the sidebar, after Usage", () => {
  const nav = indexSrc.slice(indexSrc.indexOf('<nav id="side-nav"'), indexSrc.indexOf("</nav>", indexSrc.indexOf('<nav id="side-nav"')));
  const rows = [...nav.matchAll(/<button [^>]*data-nav="([^"]+)"[^>]*>/g)].map((m) => m[1]);
  assert.deepEqual(rows, ["orchestrator", "needs-you", "pull-requests", "usage", "workflows"]);
  const row = nav.match(/<button [^>]*data-nav="workflows"[^>]*>.*<\/button>/)[0];
  assert.match(row, /^<button type="button" class="nav-row" /);
  assert.match(row, /<span class="mark" data-side-icon="workflow"><\/span><span class="label">Workflows<\/span>/);
});

test("clicking the Workflows row opens Settings on the Workflows section and closes the list on a narrow screen", () => {
  const run = (narrowScreen) => {
    const calls = [];
    const context = {
      narrow: () => narrowScreen,
      toggleSidebar: (open) => calls.push(["sidebar", open]),
      openSettings: (name) => calls.push(["settings", name]),
    };
    vm.createContext(context);
    vm.runInContext(`${fn(sidebarSrc, "openWorkflows")}\nopenWorkflows();`, context);
    return calls;
  };
  assert.deepEqual(run(false), [["settings", "connectors"]]);
  assert.deepEqual(run(true), [["sidebar", false], ["settings", "connectors"]]);
});

/** drawNav over a page with no machines, so only the nav rows' active state is in play. */
function activeRows({ shell = null, settings = false, section = "general" }) {
  const part = () => ({
    textContent: "",
    hidden: false,
    style: {},
    attributes: {},
    classList: { names: new Set(), toggle(name, force) { force ? this.names.add(name) : this.names.delete(name); } },
    setAttribute() {},
    removeAttribute() {},
    querySelector() { return (this.child ??= part()); },
  });
  const rows = {};
  const context = {
    machines: new Map(),
    LOCAL: "local",
    active: null,
    globalSettings: { name: "Orchestrator" },
    askGlobalSettings() {},
    navCounts: () => ({ needsYou: 0, orchestrator: 0, reviews: 0, usage: null, machinesOnline: 0 }),
    shellPage: () => shell,
    settingsOpen: () => settings,
    navRow: (name) => (rows[name] ??= part()),
    isGlobalOrchestratorTab: () => false,
    globalHostMachine: () => null,
    machineName: () => "machine",
    document: { body: { dataset: { settingsPage: section } }, getElementById: () => part() },
    accountButton: part(),
    bindingLabel: () => null,
  };
  vm.createContext(context);
  vm.runInContext(`${line(sidebarSrc, "const setText")}\n${fn(sidebarSrc, "drawCount")}\n${fn(sidebarSrc, "drawNav")}\ndrawNav();`, context);
  const on = (name) => rows[name].classList.names.has("active");
  return { workflows: on("workflows"), usage: on("usage") };
}

test("the Workflows row is active while the Workflows section of Settings is open, and not on other sections", () => {
  assert.deepEqual(activeRows({ settings: true, section: "connectors" }), { workflows: true, usage: false });
  assert.deepEqual(activeRows({ settings: true, section: "general" }), { workflows: false, usage: false });
});

test("the Workflows row is not active once Settings closes, nor on a shell page", () => {
  assert.deepEqual(activeRows({ settings: false, section: "connectors" }), { workflows: false, usage: false });
  assert.deepEqual(activeRows({ shell: "usage", section: "connectors" }), { workflows: false, usage: true });
});
