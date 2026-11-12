import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const composerSrc = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");

// A stand-in element: enough of the DOM for renderControls to set and read its state.
function element(id) {
  return {
    id,
    hidden: false,
    title: "",
    innerHTML: "",
    dataset: {},
    children: [],
    firstChild: null,
    setAttribute() {},
    closest: () => null,
    querySelector: () => ({ textContent: "", outerHTML: "", classList: { contains: () => true } }),
    classList: { contains: () => false, add() {}, remove() {} },
  };
}

// claude's Shift+Tab cycle, as the screen shows it: manual, accept edits, plan, and round again.
const CYCLE = ["manual", "accept", "plan"];

function load(screen = "manual") {
  const els = new Map();
  const get = (id) => els.get(id) ?? els.set(id, element(id)).get(id);
  const term = { mode: screen, writes: [] };
  const tab = { key: "t1", view: { id: "t1", running: true }, machine: "m1", mode: undefined, traits: null, menu: null, dialog: null, cycling: false };
  const t = { access: undefined, items: [] };
  const context = {
    document: { getElementById: get },
    traitsButton: get("traits-button"),
    composer: { dataset: { compact: "false" } },
    tab,
    active: tab,
    threadOf: () => t,
    terminalMode: () => term.mode,
    writeTo: async (_tab, data) => {
      term.writes.push(data);
      if (data === "\x1b[Z") term.mode = CYCLE[(CYCLE.indexOf(term.mode) + 1) % CYCLE.length];
    },
    until: async (done) => done(),
    waitsOnMenu: () => false,
    renderComposer() {},
    setHtml: (el, html) => (el.innerHTML = html),
    icon: (name) => `<i data-icon="${name}"></i>`,
    esc: (text) => String(text ?? ""),
    bindingLabel: () => "",
    modelName: () => "Default",
    parentOf: () => null,
    delegatedModels: () => new Map(),
    modelLabel: () => null,
    authOf: () => "claude",
    traitsLabel: () => null,
    traitEntries: () => [],
    REASONING: {},
    isGlobalOrchestratorTab: () => false,
    console,
  };
  vm.createContext(context);
  const slice = (from, to) => composerSrc.slice(composerSrc.indexOf(from), composerSrc.indexOf(to));
  const line = (start) => composerSrc.slice(composerSrc.indexOf(start), composerSrc.indexOf("\n", composerSrc.indexOf(start)));
  vm.runInContext(
    [
      slice("const MODES = {", "let drafts"),
      line("const SHIFT_TAB = "),
      slice("const modeButton = ", "modeButton.onclick"),
      slice("function togglePlan(tab)", "/** T3 Code folds"),
    ].join("\n"),
    context,
  );
  return { context, term, tab, t, chip: get("plan-chip"), modeButton: get("mode-button"), modeBlock: get("mode-block") };
}

const settle = () => new Promise((done) => setTimeout(done, 0));

test("the composer no longer draws the Build | Plan selector", () => {
  assert.doesNotMatch(html, /plan-toggle|data-plan-seg/);
  assert.doesNotMatch(composerSrc, /planToggle|plan-toggle|planSeg|plan-seg/);
  assert.match(html, /<span id="plan-chip" hidden[^>]*>Plan<\/span>/);
});

test("sessions start in build: the plan chip stays hidden and Access shows the mode claude started in", () => {
  const { context, tab, chip, modeButton, modeBlock } = load("manual");
  tab.mode = "manual";
  context.renderControls(tab);
  assert.equal(chip.hidden, true);
  assert.equal(modeBlock.hidden, false);
  assert.match(modeButton.innerHTML, /Supervised/);
});

test("the plan chip shows only while claude is in plan mode", () => {
  const { context, tab, term, chip } = load("manual");
  context.renderControls(tab);
  assert.equal(chip.hidden, true);
  term.mode = "plan";
  tab.mode = "plan";
  context.renderControls(tab);
  assert.equal(chip.hidden, false);
  term.mode = "accept";
  tab.mode = "accept";
  context.renderControls(tab);
  assert.equal(chip.hidden, true);
});

test("the plan chip stays hidden when the session is not running", () => {
  const { context, tab, chip } = load("plan");
  tab.mode = "plan";
  tab.view.running = false;
  context.renderControls(tab);
  assert.equal(chip.hidden, true);
});

test("plan mode is entered and left through Shift+Tab's cycle, the one path the composer keeps", async () => {
  const { context, tab, term, chip } = load("manual");
  tab.mode = "manual";

  context.togglePlan(tab);
  await settle();
  assert.equal(tab.mode, "plan");
  assert.equal(term.mode, "plan");
  assert.ok(term.writes.every((data) => data === "\x1b[Z"), "claude is only sent Shift+Tab");
  context.renderControls(tab);
  assert.equal(chip.hidden, false);

  context.togglePlan(tab);
  await settle();
  assert.equal(tab.mode, "manual");
  assert.equal(term.mode, "manual");
  context.renderControls(tab);
  assert.equal(chip.hidden, true);
});

test("leaving plan returns to the access mode picked before it", async () => {
  const { context, tab, term, t } = load("accept");
  tab.mode = "accept";
  t.access = "accept";

  context.togglePlan(tab);
  await settle();
  assert.equal(tab.mode, "plan");
  assert.equal(t.access, "accept", "entering plan keeps the access mode");

  context.togglePlan(tab);
  await settle();
  assert.equal(tab.mode, "accept", "leaving plan goes round the cycle to the access mode");
  assert.equal(term.mode, "accept");
});
