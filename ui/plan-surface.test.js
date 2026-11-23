import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const panelSrc = read("gui-panel.js");
const planSrc = read("gui-plan.js");
const line = (src, from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const flush = () => new Promise((resolve) => setImmediate(resolve));
/** Plain copies of the values gui-plan.js builds in the page's own realm, for deepStrictEqual. */
const plain = (value) => JSON.parse(JSON.stringify(value));
const decode = (html) => html.replace(/&#(\d+);/g, (_, c) => String.fromCharCode(c));

/** A Captain's tab as the app keeps it: the view from the hub, and the thread's panel state. */
function captain(plans = [], extra = {}) {
  return { key: "m1:tab-1", machine: "m1", view: { id: "tab-1", role: "orchestrator", session: "s1", plans, ...extra }, panel: { open: false, surfaces: [], active: null, max: false } };
}
const worker = () => ({ key: "m1:tab-9", machine: "m1", view: { id: "tab-9", role: "worker", session: "s9", plans: [] }, panel: { open: false, surfaces: [], active: null, max: false } });
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

/** gui-plan.js in a page: the panel's globals stubbed, and the real esc. */
function harness(extra = {}) {
  const log = { shown: [], saved: 0, added: [], calls: [], opened: [], renders: 0 };
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
    renderPanel: () => {
      log.renders++;
      run("(() => { const s = shownSurface(active); if (s?.kind === 'plan') renderPlanPanel(s); })()");
    },
    callOn: async (machine, call) => {
      log.calls.push(call);
      return context.reply(call);
    },
    openDelegated: async (task) => log.opened.push(task),
    reply: () => [],
    ...extra,
  };
  vm.createContext(context);
  const run = (code) => vm.runInContext(code, context);
  run(line(settingsSrc, "const esc = "));
  run(planSrc);
  const listen = (type, target) => listeners[type]({ target });
  /** A click on an element matching `sel` (a selector gui-plan.js queries) inside the plan body. */
  const click = (sel, dataset = {}) => listen("click", { closest: (s) => (s === sel ? { dataset } : s === ".pl" ? { dataset: {} } : null) });
  const push = () => models.forEach((fn) => fn());
  const html = () => log.shown.at(-1) ?? "";
  return { context, log, run, click, push, html, listeners, models, tabs };
}

test("a Captain thread offers Plan in the add menu; a worker thread and the Orchestrator do not", () => {
  const { context, run } = harness();
  const menu = (tab) => {
    context.active = tab;
    return run(`${fn(panelSrc, "addMenu")}\naddMenu()`);
  };
  // The menu's own neighbours, as the panel stubs them.
  Object.assign(context, {
    threadOf: () => ({ workspace: {} }),
    shownPr: () => null,
    hasProject: () => true,
    isGlobalOrchestratorTab: (tab) => !!tab.view.orchestrator_global,
    globalOrchestratorName: () => "Orchestrator",
    newTerminalSurface: () => {},
    openFilesSurface: () => {},
    openSurface: () => {},
    newBrowser: () => ({}),
  });
  const labelsOf = (tab) => menu(tab).map((m) => m.label);
  assert.ok(labelsOf(captain()).includes("Plan"), "a Captain thread offers Plan");
  assert.ok(!labelsOf(worker()).includes("Plan"), "a worker thread does not offer Plan");
  assert.ok(!labelsOf({ ...captain(), view: { ...captain().view, orchestrator_global: true } }).includes("Plan"), "the Orchestrator does not offer Plan yet");
  const entry = menu(captain()).find((m) => m.label === "Plan");
  assert.equal(entry.icon, "git-branch");
  entry.run();
  assert.deepEqual(context.active.panel.surfaces.map((s) => s.kind), ["plan"], "running the entry adds the plan surface");
});

test("opening Plan fetches once; a push with the same glance sends nothing; a changed glance sends one", async () => {
  const h = harness({ reply: () => [plan()] });
  const tab = captain([glance()]);
  h.tabs.push(tab);
  h.context.active = tab;
  tab.panel.open = true;
  tab.panel.surfaces.push({ id: "plan", kind: "plan", title: "Plan" });
  tab.panel.active = "plan";

  h.run("renderPlanPanel(shownSurface(active))");
  await flush();
  assert.deepEqual(plain(h.log.calls), [{ cmd: "plan", tab: "tab-1" }], "opening sends one plan call");
  const html = h.html();
  assert.ok(html.includes("Ship the signup form"), "the plan title renders");
  assert.ok(html.includes("Build") && html.includes("Review"), "the list names render");
  assert.ok(html.includes("Draft the form") && html.includes("Copy review"), "the step names render");

  h.push();
  await flush();
  assert.equal(h.log.calls.length, 1, "a push with the same glance sends no call");

  tab.view.plans = [glance({ done: 2, strips: ["dd", "p"] })];
  h.push();
  await flush();
  assert.equal(h.log.calls.length, 2, "a changed glance sends exactly one call");
});

test("a plan text with markup renders as plain text", async () => {
  const h = harness({ reply: () => [plan({ lists: [{ id: "l1", name: "Build", task: null, steps: [STEP({ id: "s2", name: "<img src=x onerror=alert(1)>", state: "running", what: "**bold** text" })] }] })] });
  const tab = captain([glance()]);
  h.tabs.push(tab);
  h.context.active = tab;
  tab.panel.open = true;
  tab.panel.surfaces.push({ id: "plan", kind: "plan", title: "Plan" });
  tab.panel.active = "plan";
  h.run("renderPlanPanel(shownSurface(active))");
  await flush();
  h.click("[data-step]", { step: "s2" });
  await flush();
  const html = h.html();
  assert.ok(!html.includes("<img"), "no image element is created");
  assert.equal(decode(html).includes("<img src=x onerror=alert(1)>"), true, "the name shows as text");
  assert.ok(html.includes("**bold** text"), "markdown stays literal");
  assert.ok(!html.includes("<b>bold</b>") && !html.includes("<strong>"), "no bold element is created from the markup");
});

test("zoom, a folded list and the picked plan survive a re-render", async () => {
  const second = plan({ id: "p2", title: "Fix the export job", lists: [{ id: "l3", name: "Diagnose", task: null, steps: [STEP({ id: "s7", name: "Read the logs", state: "pending" })] }] });
  const h = harness({ reply: () => [plan(), second] });
  const tab = captain([glance(), glance({ id: "p2", title: "Fix the export job" })]);
  h.tabs.push(tab);
  h.context.active = tab;
  tab.panel.open = true;
  const s = { id: "plan", kind: "plan", title: "Plan" };
  tab.panel.surfaces.push(s);
  tab.panel.active = "plan";
  h.run("renderPlanPanel(shownSurface(active))");
  await flush();

  h.click("[data-plan]", { plan: "p2" });
  h.click("[data-zoom]", { zoom: "1" });
  h.click("[data-fold]", { fold: "l3" });
  await flush();
  assert.equal(s.zoom, 2, "zoom is kept in the surface's panel state");
  assert.deepEqual([...s.folded], ["l3"], "the folded list is kept");
  assert.equal(s.plan, "p2", "the picked plan is kept");
  assert.ok(h.log.saved > 0, "the panel state is saved");

  h.run("renderPlanPanel(shownSurface(active))");
  const html = h.html();
  assert.ok(html.includes('<span class="pl-zoom-name">Details</span>'), "the Details zoom shows again");
  assert.ok(/class="pl-list folded"[\s\S]*Diagnose/.test(html), "the folded list stays folded");
  assert.ok(/data-plan="p2"[^>]*aria-selected="true"/.test(html), "the picked plan stays picked");
});

test("auto-open: once per new plan id, on the viewed Captain with its panel closed", () => {
  const h = harness();
  const tab = captain([glance()]);
  h.tabs.push(tab);
  h.context.active = tab;
  h.push();
  assert.equal(h.log.added.length, 1, "a new plan on the viewed Captain opens the Plan surface");
  assert.equal(h.log.added[0].kind, "plan");
  assert.equal(tab.panel.open, true);
  h.push();
  assert.equal(h.log.added.length, 1, "the same plan id does not open it again");
});

test("auto-open: not on another thread, not over a shown surface, not after the user closed it", () => {
  // Another thread: the viewed Captain has no new plan, another Captain does.
  const viewed = captain([]);
  const other = { ...captain([glance({ id: "p7" })]), key: "m1:tab-5", view: { id: "tab-5", role: "orchestrator", session: "s5", plans: [glance({ id: "p7" })] } };
  const h1 = harness();
  h1.tabs.push(viewed, other);
  h1.context.active = viewed;
  h1.push();
  assert.deepEqual(h1.log.added, [], "a plan on another thread does not open here");

  // A shown surface: the panel is open on the diff.
  const h2 = harness();
  const busy = captain([glance()]);
  busy.panel.open = true;
  busy.panel.surfaces.push({ id: "diff", kind: "diff", title: "Diff" });
  busy.panel.active = "diff";
  h2.tabs.push(busy);
  h2.context.active = busy;
  h2.push();
  assert.deepEqual(h2.log.added, [], "a new plan does not replace the shown surface");
  assert.equal(busy.panel.active, "diff");

  // Closed by the user: the plan opened, the user closed the panel, the same plan arrives again.
  const h3 = harness();
  const closed = captain([glance()]);
  h3.tabs.push(closed);
  h3.context.active = closed;
  h3.push();
  closed.panel.open = false;
  h3.push();
  assert.equal(h3.log.added.length, 1, "the plan opened once");
  assert.equal(closed.panel.open, false, "it stays closed after the user closed it");
});

test("auto-open after a resume: a reused plan id opens the surface once in the new session", () => {
  const h = harness();
  const tab = captain([glance()]);
  h.tabs.push(tab);
  h.context.active = tab;
  h.push();
  assert.equal(h.log.added.length, 1, "the session's first plan p1 opens the Plan surface");

  // The user closes the panel; /resume starts a new session whose first plan is p1 again, with another title.
  tab.panel.open = false;
  tab.panel.active = null;
  tab.panel.surfaces = [];
  tab.view.session = "s2";
  tab.view.plans = [glance({ id: "p1", title: "Tidy the README" })];
  h.push();
  assert.equal(h.log.added.length, 2, "the new session's p1 opens the surface too");
  assert.equal(h.log.added[1].kind, "plan");
  assert.equal(tab.panel.open, true);

  h.push();
  assert.equal(h.log.added.length, 2, "the same p1 in the same session does not open it again");
  assert.deepEqual(plain(tab.panel.autoOpened), { session: "s2", ids: ["p1"] }, "the saved state holds only the new session's entries");

  // A session change with nothing new to open still drops the old session's entries.
  tab.view.session = "s3";
  tab.view.plans = [];
  h.push();
  assert.deepEqual(plain(tab.panel.autoOpened), { session: "s3", ids: [] }, "an empty new session keeps no old entries");
});

test("a resume that drops unfinished plans shows one line until it is dismissed", async () => {
  const two = [plan(), plan({ id: "p2", title: "Fix the export job", lists: [] })];
  const h = harness({ reply: () => two });
  const tab = captain([glance(), glance({ id: "p2", title: "Fix the export job" })]);
  h.tabs.push(tab);
  h.context.active = tab;
  tab.panel.open = true;
  tab.panel.surfaces.push({ id: "plan", kind: "plan", title: "Plan" });
  tab.panel.active = "plan";
  h.run("renderPlanPanel(shownSurface(active))");
  await flush();
  assert.ok(!h.html().includes("not carried over"), "no line while the session is the same");

  // The resumed session numbers its plans from p1 again: the old p1 is gone, its id is not.
  tab.view.session = "s2";
  tab.view.plans = [glance({ id: "p1", title: "Tidy the README" })];
  h.context.reply = () => [plan({ id: "p1", title: "Tidy the README", lists: [] })];
  h.push();
  await flush();
  assert.ok(
    h.html().includes("2 plans from before /resume were not carried over: Ship the signup form, Fix the export job."),
    "the line names both dropped plans, the reused id included",
  );

  h.click("[data-dismiss-dropped]", {});
  await flush();
  assert.ok(!h.html().includes("not carried over"), "dismiss removes the line");
});

test("a step shows who runs it: its by, else Worker for a task, else Captain", async () => {
  const runners = plan({
    lists: [
      {
        id: "l1",
        name: "Build",
        task: null,
        steps: [
          STEP({ id: "s1", name: "Named runner", state: "running", task: "task-1", by: "Worker A" }),
          STEP({ id: "s2", name: "Task without by", state: "running", task: "task-2" }),
          STEP({ id: "s3", name: "Captain's own", state: "done" }),
        ],
      },
    ],
  });
  const h = harness({ reply: () => [runners] });
  const tab = captain([glance()]);
  h.tabs.push(tab);
  h.context.active = tab;
  tab.panel.open = true;
  tab.panel.surfaces.push({ id: "plan", kind: "plan", title: "Plan" });
  tab.panel.active = "plan";
  h.run("renderPlanPanel(shownSurface(active))");
  await flush();
  const runBy = (id) => {
    h.click("[data-step]", { step: id });
    h.run("renderPlanPanel(shownSurface(active))");
    return /<dt>Run by<\/dt><dd>([^<]*)<\/dd>/.exec(h.html())?.[1];
  };
  assert.equal(runBy("s1"), "Worker A");
  assert.equal(runBy("s2"), "Worker");
  assert.equal(runBy("s3"), "Captain");
});

test("a push during a fetch fetches once more when the glance changed again", async () => {
  const pending = [];
  const h = harness({
    callOn: async (machine, call) => {
      h.log.calls.push(call);
      return new Promise((resolve) => pending.push(() => resolve([plan()])));
    },
  });
  const tab = captain([glance()]);
  h.tabs.push(tab);
  h.context.active = tab;
  tab.panel.open = true;
  tab.panel.surfaces.push({ id: "plan", kind: "plan", title: "Plan" });
  tab.panel.active = "plan";
  h.run("renderPlanPanel(shownSurface(active))");
  assert.equal(h.log.calls.length, 1);

  tab.view.plans = [glance({ done: 2, strips: ["dd", "p"] })];
  h.push();
  assert.equal(h.log.calls.length, 1, "a push while a fetch is in flight sends nothing yet");
  pending.shift()();
  await flush();
  assert.equal(h.log.calls.length, 2, "the glance changed during the fetch: one more fetch");
});
