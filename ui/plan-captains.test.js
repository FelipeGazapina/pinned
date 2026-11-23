import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// #349 unit 6: the Captains surface on the Orchestrator thread, the rail's word, and the unit-5 leftovers.

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const panelSrc = read("gui-panel.js");
const planSrc = read("gui-plan.js");
const orchSrc = read("gui-orchestrator.js");
const globalSrc = read("global-orchestrator.js");
const line = (src, from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
/** The rail's functions, taken from their own files, to run beside gui-plan.js. */
const railSrc = [line(globalSrc, "const workersLabel"), line(globalSrc, "const CAPTAIN_STATE_LABELS"), fn(globalSrc, "captainRailSub"), fn(orchSrc, "railHtml")].join("\n");
const flush = () => new Promise((resolve) => setImmediate(resolve));
const plain = (value) => JSON.parse(JSON.stringify(value));
const decode = (html) => html.replace(/&#(\d+);/g, (_, c) => String.fromCharCode(c));

/** A Captain's tab as the app keeps it: the view from the hub, and the thread's panel state. */
function captain(id, { name = "Atlas", plans = [], session = "s1" } = {}) {
  return { key: `m1:${id}`, machine: "m1", view: { id, role: "orchestrator", session, name, plans }, panel: { open: false, surfaces: [], active: null, max: false } };
}
/** The Orchestrator's own thread. */
const globalTab = () => ({ key: "m1:tab-o", machine: "m1", view: { id: "tab-o", role: "orchestrator", orchestrator_global: true, session: "so", plans: [] }, panel: { open: false, surfaces: [], active: null, max: false } });
const glance = (over = {}) => ({ id: "p1", title: "Ship the signup form", state: "working", done: 1, total: 3, now: "Test", strips: ["dr", "p"], ...over });
const STEP = (over) => ({ id: "s1", name: "Draft", what: "", state: "done", why: null, cost: null, task: null, fixes: [], ...over });
/** A PlanView as the hub replies to `{cmd: "plan"}`. */
function plan(over = {}) {
  return {
    id: "p1",
    rev: 3,
    title: "Ship the signup form",
    asked_at: 1763000000000,
    finished_at: null,
    closed: null,
    lists: [
      {
        id: "l1",
        name: "Build",
        task: null,
        steps: [
          STEP({ id: "s1", name: "Draft the form", state: "done", cost: 0.12 }),
          STEP({ id: "s2", name: "Wire the API", state: "running", task: "task-2", what: "Hook the form to **the** API" }),
        ],
      },
      { id: "l2", name: "Review", task: null, steps: [STEP({ id: "s3", name: "Copy review", state: "waiting" })] },
    ],
    ...over,
  };
}

/** gui-plan.js in a page: the panel's globals stubbed, the real esc, and the Captain helpers stubbed. */
function harness(extra = {}) {
  const log = { shown: [], saved: 0, added: [], calls: [], opened: [], activated: [] };
  const listeners = {};
  const models = [];
  const tabs = [];
  const context = {
    console,
    Date,
    Math,
    JSON,
    Promise,
    icon: (name, cls = "") => `<svg data-icon="${name}" class="${cls}"></svg>`,
    panelBody: { dataset: {}, addEventListener: (type, fn) => (listeners[type] = fn) },
    onModelChange: (fn) => models.push(fn),
    guiOn: () => true,
    active: null,
    tabs,
    waiting: [],
    working: [],
    facts: [],
    panelOf: (tab) => tab.panel,
    savePanel: () => log.saved++,
    showPanel: (html) => log.shown.push(html),
    shownSurface: (tab) => tab.panel.surfaces.find((s) => s.id === tab.panel.active) ?? null,
    addSurface: (tab, s) => {
      log.added.push(s);
      if (!tab.panel.surfaces.some((x) => x.id === s.id)) tab.panel.surfaces.push(s);
      tab.panel.active = s.id;
      tab.panel.open = true;
      context.renderPanel();
    },
    renderPanel: () => run("(() => { const s = shownSurface(active); if (s?.kind === 'plan') renderPlanPanel(s); })()"),
    callOn: async (machine, call) => {
      log.calls.push(call);
      return context.reply(call);
    },
    openDelegated: async (task) => log.opened.push(task),
    activate: (tab) => log.activated.push(tab.key),
    tabByKey: (key) => tabs.find((t) => t.key === key) ?? null,
    threadState: (tab) => (context.waiting.includes(tab.view.id) ? "needs" : context.working.includes(tab.view.id) ? "working" : "idle"),
    captainFacts: () => context.facts,
    projectName: (tab) => tab.view.name,
    machineName: (id) => `Machine ${id}`,
    isGlobalOrchestratorTab: (tab) => tab?.view?.orchestrator_global === true,
    reply: () => [],
    ...extra,
  };
  vm.createContext(context);
  const run = (code) => vm.runInContext(code, context);
  run(line(settingsSrc, "const esc = "));
  run("function captainMonoHtml(name) { return `<span class=\"captain-mono\">${esc(name)}</span>`; }");
  run(planSrc);
  /** A click on an element matching `sel` (a selector gui-plan.js queries) inside the plan body. */
  const click = (sel, dataset = {}) => listeners.click({ target: { closest: (s) => (s === sel ? { dataset } : s === ".pl" ? { dataset: {} } : null) } });
  const push = () => models.forEach((fn) => fn());
  const html = () => log.shown.at(-1) ?? "";
  return { context, log, run, click, push, html, listeners, models, tabs };
}

/** Puts the Orchestrator's thread on screen with the Captains surface open, as the + menu leaves it. */
function openCaptains(h, orchestrator) {
  h.tabs.push(orchestrator);
  h.context.active = orchestrator;
  orchestrator.panel.open = true;
  const s = { id: "plan", kind: "plan", title: "Captains" };
  orchestrator.panel.surfaces.push(s);
  orchestrator.panel.active = "plan";
  h.run("renderPlanPanel(shownSurface(active))");
  return s;
}

test("the Captains surface: cards by urgency, plain Captains under No plan running, one header count", () => {
  const h = harness();
  const O = globalTab();
  h.tabs.push(
    captain("tab-d", { name: "Delta" }),
    captain("tab-c", { name: "Comet", plans: [glance({ id: "p1", state: "working" })] }),
    captain("tab-a", { name: "Atlas", plans: [glance({ id: "p1", state: "needs_you", title: "Pick the palette" })] }),
    captain("tab-b", { name: "Beacon", plans: [glance({ id: "p1", state: "fixing" })] }),
  );
  openCaptains(h, O);
  const html = h.html();
  assert.equal(/<span class="pl-count">([^<]*)<\/span>/.exec(html)?.[1], "1 needs you · 1 fixing · 1 working · 1 idle", "the header count");
  const order = [...html.matchAll(/data-card="([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual(order, ["m1:tab-a", "m1:tab-b", "m1:tab-c"], "cards: needs you, then fixing, then working");
  const rest = html.indexOf("No plan running");
  assert.ok(rest > html.lastIndexOf('data-card="'), "the plain Captains come after the cards");
  assert.ok(html.indexOf("Delta", rest) > rest, "Delta, which has no plan, is listed under No plan running");
  assert.ok(/Delta[\s\S]*Idle/.test(html.slice(rest)), "Delta reads Idle");
});

test("a Captain's own wait makes its card Needs you even when its plans only run", () => {
  const h = harness({ waiting: ["tab-w"] });
  const O = globalTab();
  h.tabs.push(captain("tab-w", { name: "Willow", plans: [glance({ id: "p1", state: "working" })] }));
  openCaptains(h, O);
  assert.ok(h.html().includes('data-cstate="needs_you"'), "the card is in the needs-you state");
  assert.ok(h.html().includes('data-tone="needs_you">Needs you</span>'), "and reads Needs you");
});

/** The facts the rail reads for one Captain tab; the card's state decides the row when the Captain is here. */
const railFact = (slug, name, tab, state = "idle") => ({ slug, name, machine: { id: "m1", name: "Machine m1" }, path: "/x", captain: tab.key, state, workers: 0, working: 0, needs: [], reported: null, seen: null });

test("a Captain with no plan whose thread is working reads Working on its card and rail row, under No plan running", () => {
  const h = harness({ working: ["tab-v"] });
  h.run(railSrc);
  const O = globalTab();
  const V = captain("tab-v", { name: "Vesper" });
  h.tabs.push(V);
  h.context.facts = [railFact("vesper", "Vesper", V)];
  const rail = h.run("railHtml(facts)");
  openCaptains(h, O);
  const html = h.html();
  assert.equal(/<span class="pl-count">([^<]*)<\/span>/.exec(html)?.[1], "1 working", "the header counts it as 1 working");
  const rest = html.indexOf("No plan running");
  assert.ok(rest >= 0 && html.indexOf("Vesper", rest) > rest, "Vesper sits under No plan running");
  assert.match(html.slice(rest), /<span class="pl-word" data-tone="working">Working<\/span>/, "its card word is Working");
  assert.match(rail, /data-captain-tab="m1:tab-v"[\s\S]*?<span class="state">Working<\/span>/, "its rail row reads Working");
});

test("a Captain whose only plan is finished reads Working while its thread works, not Finished at", () => {
  const h = harness({ working: ["tab-r"] });
  h.run(railSrc);
  const O = globalTab();
  const R = captain("tab-r", { name: "Rowan", plans: [glance({ id: "p1", state: "finished", finished_at: 1763000100000, done: 3, total: 3, strips: ["ddd"] })] });
  h.tabs.push(R);
  h.context.facts = [railFact("rowan", "Rowan", R, "working")];
  const rail = h.run("railHtml(facts)");
  openCaptains(h, O);
  const html = h.html();
  assert.equal(/data-card="m1:tab-r"[\s\S]*?<span class="pl-word"[^>]*>([^<]*)<\/span>/.exec(html)?.[1], "Working", "the card reads Working");
  assert.equal(/data-captain-tab="m1:tab-r"[\s\S]*?<span class="state">([^<]*)<\/span>/.exec(rail)?.[1], "Working", "the rail row reads Working");
});

test("a Captain with no plan whose thread is idle still reads Idle on its card and rail row", () => {
  const h = harness();
  h.run(railSrc);
  const O = globalTab();
  const I = captain("tab-i", { name: "Ivy" });
  h.tabs.push(I);
  h.context.facts = [railFact("ivy", "Ivy", I)];
  const rail = h.run("railHtml(facts)");
  openCaptains(h, O);
  const html = h.html();
  assert.equal(/<span class="pl-count">([^<]*)<\/span>/.exec(html)?.[1], "1 idle", "the header counts it as 1 idle");
  assert.match(html.slice(html.indexOf("No plan running")), /<span class="pl-word" data-tone="idle">Idle<\/span>/, "its card word is Idle");
  assert.match(rail, /data-captain-tab="m1:tab-i"[\s\S]*?<span class="state">Idle<\/span>/, "its rail row reads Idle");
});

test("a Captain with a fixing plan reads Fixing while its thread works: the plan is more urgent", () => {
  const h = harness({ working: ["tab-x"] });
  h.run(railSrc);
  const O = globalTab();
  const X = captain("tab-x", { name: "Xeno", plans: [glance({ id: "p1", state: "fixing" })] });
  h.tabs.push(X);
  h.context.facts = [railFact("xeno", "Xeno", X, "working")];
  const rail = h.run("railHtml(facts)");
  openCaptains(h, O);
  const html = h.html();
  assert.equal(/data-card="m1:tab-x"[\s\S]*?<span class="pl-word"[^>]*>([^<]*)<\/span>/.exec(html)?.[1], "Fixing", "the card reads Fixing");
  assert.equal(/data-captain-tab="m1:tab-x"[\s\S]*?<span class="state">([^<]*)<\/span>/.exec(rail)?.[1], "Fixing", "the rail row reads Fixing");
});

test("folded cards send no plan call; unfolding one sends one for its tab; an unchanged glance none, a changed glance one", async () => {
  const h = harness({ reply: () => [plan()] });
  const O = globalTab();
  const A = captain("tab-a", { name: "Atlas", plans: [glance({ id: "p1", state: "working" })] });
  h.tabs.push(A, captain("tab-b", { name: "Beacon", plans: [glance({ id: "p1", state: "idle", title: "Tidy the README" })] }));
  openCaptains(h, O);
  await flush();
  assert.deepEqual(plain(h.log.calls), [], "every card folded: no plan call");
  h.push();
  await flush();
  assert.equal(h.log.calls.length, 0, "a push with every card folded sends none");

  h.click("[data-card]", { card: "m1:tab-a" });
  await flush();
  assert.deepEqual(plain(h.log.calls), [{ cmd: "plan", tab: "tab-a" }], "unfolding Atlas sends one call for its tab");
  assert.ok(h.html().includes('data-z="0"'), "the tree in place is drawn at the Dots zoom");
  assert.ok(!h.html().includes('class="pl-name"'), "the tree in place has no step names");

  h.push();
  await flush();
  assert.equal(h.log.calls.length, 1, "an unchanged glance sends no call");

  A.view.plans = [glance({ id: "p1", state: "working", done: 2, strips: ["dd", "rp"] })];
  h.push();
  await flush();
  assert.equal(h.log.calls.length, 2, "a changed glance sends one call");
  assert.deepEqual(plain(h.log.calls[1]), { cmd: "plan", tab: "tab-a" });
});

test("Open the full plan keeps the Orchestrator thread; All Captains returns to the strip; Open thread is the only switch", async () => {
  const h = harness({ reply: () => [plan()] });
  const O = globalTab();
  h.tabs.push(captain("tab-a", { name: "Atlas", plans: [glance({ id: "p1" })] }));
  openCaptains(h, O);
  h.click("[data-card]", { card: "m1:tab-a" });
  await flush();
  h.click("[data-full]", { full: "m1:tab-a" });
  await flush();
  assert.match(h.html(), /data-view="full"/, "the whole plan is shown");
  assert.ok(h.html().includes("All Captains"), "with an All Captains button");
  assert.equal(h.context.active, O, "the thread stays the Orchestrator's");
  assert.deepEqual(h.log.activated, [], "opening the full plan switches nothing");

  h.click("[data-back]", {});
  await flush();
  assert.match(h.html(), /data-view="captains"/, "All Captains returns to the strip");
  assert.deepEqual(h.log.activated, [], "going back switches nothing");

  h.click("[data-full]", { full: "m1:tab-a" });
  await flush();
  h.click("[data-open-thread]", { openThread: "m1:tab-a" });
  await flush();
  assert.deepEqual(h.log.activated, ["m1:tab-a"], "Open thread is the one control that switches thread");
});

test("the rail row and the Captain's card show the same word, Needs you included", () => {
  const h = harness({ waiting: ["tab-w"] });
  h.run(railSrc);
  const O = globalTab();
  const W = captain("tab-w", { name: "Willow", plans: [glance({ id: "p1", state: "working" })] });
  const F = captain("tab-f", { name: "Fern", plans: [glance({ id: "p1", state: "fixing" })] });
  h.tabs.push(W, F);
  const fact = (slug, name, tab) => ({ slug, name, machine: { id: "m1", name: "Machine m1" }, path: "/x", captain: tab.key, state: "working", workers: 0, working: 0, needs: [], reported: null, seen: null });
  h.context.facts = [fact("willow", "Willow", W), fact("fern", "Fern", F)];
  const rail = h.run("railHtml(facts)");
  openCaptains(h, O);
  const card = h.html();
  const railWord = (key) => new RegExp(`data-captain-tab="${key}"[\\s\\S]*?<span class="state">([^<]*)</span>`).exec(rail)?.[1];
  const cardWord = (key) => new RegExp(`data-card="${key}"[\\s\\S]*?<span class="pl-word"[^>]*>([^<]*)</span>`).exec(card)?.[1];
  assert.equal(railWord("m1:tab-w"), "Needs you", "the rail reads Needs you for Willow");
  assert.equal(cardWord("m1:tab-w"), "Needs you", "the card reads Needs you for Willow");
  assert.equal(railWord("m1:tab-f"), "Fixing", "the rail reads Fixing for Fern");
  assert.equal(cardWord("m1:tab-f"), "Fixing", "the card reads Fixing for Fern");
});

test("a /resume into a new session without carried plans: the new plan opens unfolded with no picked step", async () => {
  const h = harness({ reply: () => [plan()] });
  const A = captain("tab-a", { name: "Atlas", plans: [glance({ id: "p1", title: "Ship the signup form" })] });
  h.tabs.push(A);
  h.context.active = A;
  A.panel.open = true;
  const s = { id: "plan", kind: "plan", title: "Plan" };
  A.panel.surfaces.push(s);
  A.panel.active = "plan";
  h.run("renderPlanPanel(shownSurface(active))");
  await flush();
  h.click("[data-fold]", { fold: "l2" });
  h.click("[data-step]", { step: "s2" });
  await flush();
  assert.ok(h.html().includes('class="pl-list folded"'), "precondition: Review is folded");
  assert.ok(h.html().includes(" picked"), "precondition: a step is picked");

  A.view.session = "s2";
  A.view.plans = [glance({ id: "p1", title: "Tidy the README" })];
  h.context.reply = () => [plan({ id: "p1", title: "Tidy the README" })];
  h.push();
  await flush();
  assert.ok(h.html().includes("Tidy the README"), "the new session's plan shows");
  assert.ok(!h.html().includes('class="pl-list folded"'), "its lists open unfolded");
  assert.ok(!h.html().includes(" picked"), "no step of the old session is picked");
  assert.equal(s.step ?? null, null, "the picked step is gone");
});

test("a carried plan with the panel closed does not open the surface; a new plan after it still does", () => {
  const h = harness();
  const A = captain("tab-a", { name: "Atlas", plans: [glance({ id: "p1", title: "Ship the signup form" })] });
  h.tabs.push(A);
  h.context.active = A;
  h.push();
  assert.equal(h.log.added.length, 1, "the plan opens the surface once");
  A.panel.open = false;
  A.panel.surfaces = [];
  A.panel.active = null;
  h.push();

  // /resume carries the plan to the new session with its id and title.
  A.view.session = "s2";
  h.push();
  assert.equal(h.log.added.length, 1, "a carried plan does not reopen the closed panel");
  assert.equal(A.panel.open, false, "the panel stays closed");

  A.view.plans = [glance({ id: "p1", title: "Ship the signup form" }), glance({ id: "p2", title: "Fix the export job" })];
  h.push();
  assert.equal(h.log.added.length, 2, "a plan that is new in the new session opens it");
  assert.equal(h.log.added[1].kind, "plan");
});

test("plan text with markup renders as literal text in the card and in the tree in place", async () => {
  const markup = "<b>x</b> [link](http://example.test)";
  const h = harness({ reply: () => [plan({ title: markup, lists: [{ id: "l1", name: markup, task: null, steps: [STEP({ id: "s1", name: "Step", state: "running" })] }] })] });
  const O = globalTab();
  h.tabs.push(captain("tab-a", { name: "Atlas", plans: [glance({ id: "p1", title: markup })] }));
  openCaptains(h, O);
  await flush();
  assert.ok(!h.html().includes("<b>x</b>"), "the card shows no bold element");
  assert.ok(!/<a\s/.test(h.html()), "the card shows no link");
  assert.ok(decode(h.html()).includes(markup), "the card shows the text as typed");

  h.click("[data-card]", { card: "m1:tab-a" });
  await flush();
  assert.ok(!h.html().includes("<b>x</b>"), "the tree in place shows no bold element");
  assert.ok(!/<a\s/.test(h.html()), "the tree in place shows no link");
  assert.ok(decode(h.html()).includes(markup), "the tree in place shows the text as typed");
});

test("the + menu offers Captains on the Orchestrator thread and Plan on a Captain's", () => {
  const h = harness();
  Object.assign(h.context, {
    threadOf: () => ({ workspace: {} }),
    shownPr: () => null,
    hasProject: () => true,
    globalOrchestratorName: () => "Orchestrator",
    newTerminalSurface: () => {},
    openFilesSurface: () => {},
    openSurface: () => {},
    newBrowser: () => ({}),
  });
  const labelsOf = (tab) => {
    h.context.active = tab;
    return h.run(`${fn(panelSrc, "addMenu")}\naddMenu()`).map((m) => m.label);
  };
  assert.ok(labelsOf(globalTab()).includes("Captains"), "the Orchestrator offers Captains");
  assert.ok(!labelsOf(globalTab()).includes("Plan"), "the Orchestrator does not offer Plan");
  assert.ok(labelsOf(captain("tab-a")).includes("Plan"), "a Captain offers Plan");
  assert.ok(!labelsOf(captain("tab-a")).includes("Captains"), "a Captain does not offer Captains");
});
