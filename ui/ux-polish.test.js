import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const sidebarSrc = read("gui-sidebar.js");
const settingsSrc = read("settings.js");
const line = (src, from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};

function sidebar(extra = {}) {
  const toggle = { innerHTML: "", title: "", attrs: {}, setAttribute(name, value) { this.attrs[name] = value; } };
  const calls = [];
  const context = {
    tabs: [],
    agentConfirmations: [],
    waitingKindOf: (view) => view.waits ?? null,
    phaseOf: (tab) => tab.phase,
    icon: (name) => `<i ${name}>`,
    MOD_LABEL: "⌘",
    narrow: () => true,
    shellPage: () => null,
    ui: { sidebarOpen: false },
    toggleSidebar: (...args) => calls.push(args),
    projectKeyOf: (tab) => tab.project,
    document: { getElementById: () => toggle },
    Date,
    ...extra,
  };
  vm.createContext(context);
  vm.runInContext(
    [line(settingsSrc, "const esc = "), fn(sidebarSrc, "sidebarSummary"), fn(sidebarSrc, "sidebarToggleHtml"), line(sidebarSrc, "const sidebarToggleLabel"), "let sidebarTouched = false;\nlet phoneDrawerUntil = 0;", fn(sidebarSrc, "drawSidebarToggle"),
      line(sidebarSrc, "const projectMachinesHtml"), line(sidebarSrc, "const remoteMark"), fn(sidebarSrc, "projectsOnManyMachines"), "this.remoteMark = remoteMark; this.projectMachinesHtml = projectMachinesHtml;"].join("\n"),
    context,
  );
  return { context, toggle, calls };
}
const tab = (phase, waits = null, project = "p", machine = "m1") => ({ phase, machine, project, view: { waits } });

test("the sidebar toggle shows how many things wait and a dot while a thread works", () => {
  const { context, toggle } = sidebar();
  context.tabs.push(tab("working"), tab("asking", "needs_input"), tab("failed", "failed"), tab("done"));
  context.agentConfirmations.push({}, {});
  context.drawSidebarToggle();
  assert.equal(toggle.innerHTML, `<span class="glyph"><i panel-left><span class="running" aria-hidden="true"></span></span><span class="count">4</span>`);
  assert.equal(toggle.title, "Toggle sidebar (⌘B), 4 waiting on you, something is working");
  assert.equal(toggle.attrs["aria-label"], toggle.title);
});

test("with nothing waiting or working the toggle is the bare icon", () => {
  const { context, toggle } = sidebar();
  context.tabs.push(tab("done"), tab("idle"));
  context.drawSidebarToggle();
  assert.equal(toggle.innerHTML, `<span class="glyph"><i panel-left></span>`);
  assert.equal(toggle.title, "Toggle sidebar (⌘B)");
});

test("on a phone the first load opens the drawer when something waits, once", () => {
  const { context, calls } = sidebar();
  context.tabs.push(tab("asking", "needs_input"));
  context.drawSidebarToggle();
  context.drawSidebarToggle();
  assert.deepEqual(calls, [[true, true]]);
});

test("the drawer stays shut on a phone when nothing waits, when the user moved it, and on a wide screen", () => {
  let { context, calls } = sidebar();
  context.tabs.push(tab("working"));
  context.drawSidebarToggle();
  assert.deepEqual(calls, []);
  context.tabs.push(tab("asking", "needs_input"));
  context.drawSidebarToggle();
  assert.deepEqual(calls, [[true, true]], "a thread that starts waiting within the first seconds opens it");

  ({ context, calls } = sidebar());
  context.tabs.push(tab("asking", "needs_input"));
  vm.runInContext("sidebarTouched = true", context);
  context.drawSidebarToggle();
  assert.deepEqual(calls, []);

  ({ context, calls } = sidebar({ narrow: () => false }));
  context.tabs.push(tab("asking", "needs_input"));
  context.drawSidebarToggle();
  assert.deepEqual(calls, []);

  ({ context, calls } = sidebar());
  context.tabs.push(tab("asking", "needs_input"));
  vm.runInContext("phoneDrawerUntil = 1", context);
  context.drawSidebarToggle();
  assert.deepEqual(calls, [], "after the first seconds a new question doesn't move the sidebar");
});

test("a card names its machine as text when the project spans machines, and only then", () => {
  const { context } = sidebar();
  assert.equal(context.remoteMark("sweep-node-2", true), `<span class="remote named" title="sweep-node-2"><i monitor><span class="name">sweep-node-2</span></span>`);
  assert.equal(context.remoteMark("sweep-node-2", false), `<span class="remote" title="sweep-node-2"><i monitor></span>`);
  assert.equal(context.remoteMark(`a&b<`, true), `<span class="remote named" title="a&#38;b&#60;"><i monitor><span class="name">a&#38;b&#60;</span></span>`);
  context.tabs.push(tab("idle", null, "alpha", "m1"), tab("idle", null, "alpha", "m2"), tab("idle", null, "beta", "m1"), tab("idle", null, "beta", "m1"));
  assert.deepEqual([...context.projectsOnManyMachines()], ["alpha"]);
});

test("a project header breaks its machine list between names, with the commas kept", () => {
  const { context } = sidebar();
  assert.equal(context.projectMachinesHtml(["sweep-node-2", "sweep-node"]), `<span class="m">sweep-node-2,</span> <span class="m">sweep-node</span>`);
  assert.equal(context.projectMachinesHtml(["only"]), `<span class="m">only</span>`);
  assert.equal(context.projectMachinesHtml(["<b>"]), `<span class="m">&#60;b&#62;</span>`);
});

test("the sidebar's two clocks stop while the page is hidden", () => {
  assert.match(sidebarSrc, /^visibleInterval\(askBusyShells, 1000\);$/m);
  assert.match(sidebarSrc, /^visibleInterval\(\(\) => guiOn\(\) && tickDurations\(\), 1000\);$/m);
  assert.doesNotMatch(sidebarSrc, /^setInterval\(/m);
});

test("the sidebar list fades inside its own padding, so the first row is never dimmed", () => {
  const css = read("gui.css");
  const rule = css.match(/#thread-list \{[^}]*\}/)[0];
  const padding = Number(rule.match(/padding: (\d+)px/)[1]);
  const fade = Number(rule.match(/linear-gradient\(transparent, #000 (\d+)px/)[1]);
  assert.ok(padding >= fade, `padding ${padding}px holds the ${fade}px fade`);
  assert.match(css, /#sidebar \.side-head \{ padding: 0 8px;/);
});

test("a phone shows the version in the drawer and Account, and not over the page", () => {
  const css = read("gui-sidebar.css");
  assert.match(css, /@media \(max-width: 639px\) \{\s*body\[data-mode="gui"\] #version \{ display: none; \}/);
  assert.match(css, /#sidebar \.brand b\[data-version\]::after \{ content: attr\(data-version\)/);
});
