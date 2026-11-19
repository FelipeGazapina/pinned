import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const cssSrc = read("gui-plan.css");
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
  const log = { shown: [], saved: 0, added: [], calls: [], opened: [], renders: 0, activated: [], diff: [] };
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
    keyOf: (machine, id) => `${machine}:${id}`,
    tabByKey: (key) => tabs.find((t) => t.key === key) ?? null,
    refreshTabs: async () => {},
    activate: (tab) => log.activated.push(tab.key),
    startTab: async () => {},
    setDiffState: (tab, change) => log.diff.push({ tab: tab.key, change: plain(change) }),
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

/** Opens a Captain's Plan on `lists` (each a list of STEP overrides) and returns the drawn html. */
async function draw(lists, { zoom = 1, edit = () => {} } = {}) {
  const view = plan({ lists });
  const h = harness({ reply: () => [view] });
  const tab = captain([glance()]);
  h.tabs.push(tab);
  h.context.active = tab;
  tab.panel.open = true;
  const s = { id: "plan", kind: "plan", title: "Plan", zoom };
  tab.panel.surfaces.push(s);
  tab.panel.active = "plan";
  edit(s);
  h.run("renderPlanPanel(shownSurface(active))");
  await flush();
  return { h, s, html: h.html(), draw: () => (h.run("renderPlanPanel(shownSurface(active))"), h.html()) };
}
const L = (id, name, steps) => ({ id, name, task: null, steps });
const FIXED = (state2) => [{ id: "f1", steps: [STEP({ id: "x1", name: "Find the cause", state: "done" }), STEP({ id: "x2", name: "Fix the message", state: state2 })] }];
const lists = (state2 = "done") => [
  L("l1", "Make a late code still work", [
    STEP({ id: "s1", name: "Write the test", state: "done" }),
    STEP({ id: "s2", name: "Code lasts 30 min", state: "done" }),
    STEP({ id: "s3", name: "Run the tests", state: "failed", fixes: FIXED(state2) }),
    STEP({ id: "s4", name: "Open the PR", state: "pending" }),
  ]),
  L("l2", "Tell the support team", [STEP({ id: "s5", name: "Draft the line", state: "done" }), STEP({ id: "s6", name: "Your answer", state: "waiting" })]),
];
/** The first tag whose class attribute is exactly `cls` (or starts with it and a space), as { tag, style, attrs }. */
const tags = (html, cls) => [...html.matchAll(new RegExp(`<(?:div|section|button|span|i)\\s[^>]*class="${cls}(?: [^"]*)?"[^>]*>`, "g"))].map((m) => m[0]);
const px = (tag, prop) => Number(new RegExp(`${prop}:(-?[\\d.]+)px`).exec(tag)?.[1] ?? NaN);
const laneHtml = (html) => html.split(/(?=<section class="lane)/).slice(1);

test("a list draws as a lane with one cell per main step, and the trunk runs to the last lane", async () => {
  const { html } = await draw(lists());
  const lanes = laneHtml(html);
  assert.equal(lanes.length, 2, "one lane per list");
  assert.equal((lanes[0].match(/class="cell(?: sel)?"/g) ?? []).length, 4, "one main cell per step of the first list");
  assert.equal((lanes[1].match(/class="cell(?: sel)?"/g) ?? []).length, 2, "and of the second");
  assert.ok(/class="cell(?: sel)?" data-step="s1"/.test(lanes[0]) && lanes[0].includes('data-state="done"'), "a cell names its step and state");
  assert.ok(/class="trunk"[^>]*style="height:100%"/.test(lanes[0]), "its trunk runs the whole lane");
  assert.ok(/class="trunk"[^>]*style="height:30px"/.test(lanes[1]), "the last lane's trunk stops at its head");
  assert.ok(lanes.every((l) => /class="lane-head"><div class="lane-head-in"><button type="button" class="lane-toggle"/.test(l)), "each lane has its head and fold toggle");
  assert.ok(lanes[0].includes('class="segs"') && lanes[0].includes('class="elbow"') && lanes[0].includes('class="base"'), "segs, elbow and base line");
});

test("a failed step with a fix draws a ubranch from that step's x, with one cell per fix step", async () => {
  const { html } = await draw(lists("running"));
  const lane = laneHtml(html)[0];
  const [u] = tags(lane, "ubranch");
  assert.ok(u, "the lane draws a ubranch");
  const failed = tags(lane, "cell").find((t) => t.includes('data-step="s3"'));
  assert.equal(px(u, "left"), px(failed, "left") + px(failed, "width") / 2, "it leaves from the failed step's x, not the first step's");
  assert.equal(px(u, "left"), 268, "the Steps zoom puts the third step at 268");
  assert.equal(px(u, "width"), 288, "it runs under both fix cells and back to the next step: (2 fix steps + 1) cells");
  const fixes = tags(lane, "cell fix");
  assert.equal(fixes.length, 2, "one cell per fix step");
  assert.deepEqual(fixes.map((t) => px(t, "left") + px(t, "width") / 2), [364, 460], "after the failed step, one cell apart");
  assert.equal(px(tags(lane, "cell").find((t) => t.includes('data-step="s4"')), "left") + 48, 556, "the next main step moves right by the fix cells");
  assert.ok(!/class="ubranch merged"/.test(lane), "a fix with a running step is not merged");
});

test("the ubranch turns merged when every fix step is done", async () => {
  const { html } = await draw(lists("done"));
  assert.ok(/class="ubranch merged"/.test(laneHtml(html)[0]), "all fix steps done: merged");
  const { html: other } = await draw(lists("failed"));
  assert.ok(!/class="ubranch merged"/.test(laneHtml(other)[0]), "a failed fix step: not merged");
});

test("the fix toggle hides the ubranch and its cells, and the next step comes back", async () => {
  const { h, s, html } = await draw(lists("running"));
  assert.ok(html.includes('class="chip"') && html.includes("Hide the fix branch"), "the chip offers to hide");
  h.click("[data-fixchip]", { fixchip: "l1" });
  const hidden = laneHtml(h.html())[0];
  assert.ok(!hidden.includes('class="ubranch') && !hidden.includes("cell fix"), "no ubranch, no fix cells");
  assert.ok(hidden.includes("Show the fix branch"), "the chip offers to show");
  assert.equal(s.fixOpen["s3"], false);
});

test("the prog line ends at the last done main step", async () => {
  const { html } = await draw(lists());
  const lane = laneHtml(html)[0];
  assert.equal(px(tags(lane, "prog")[0], "width"), 172 - 28, "s2 is the last done step, at x 172: the line runs from 28 to it");
  assert.equal(px(tags(lane, "base")[0], "width"), 556 - 28, "the base line runs to the last main step, past the fix cells");
  const none = laneHtml((await draw([L("l1", "Build", [STEP({ id: "s1", state: "pending" })])])).html)[0];
  assert.equal(px(tags(none, "prog")[0], "width"), 0, "no done step, no green line");
});

test("the root row has its trunk dot", async () => {
  const { html } = await draw(lists());
  assert.ok(/<div class="root-row">/.test(html), "the plan opens with a root row");
  assert.ok(html.indexOf('class="root-row"') < html.indexOf('class="lane'), "before the first lane");
  assert.match(cssSrc, /\.pl \.root-row::before\s*\{[^}]*border-radius:\s*50%/, "the row draws its dot");
  assert.match(cssSrc, /\.pl \.root-row::after\s*\{[^}]*width:\s*2px/, "and the trunk that leaves it");
});

test("the zoom is Dots, Steps and Details, and the cell width follows it", async () => {
  const widths = [];
  for (const zoom of [0, 1, 2]) {
    const { html } = await draw(lists(), { zoom });
    assert.ok(html.includes(["Dots", "Steps", "Details"][zoom]), `zoom ${zoom} is named`);
    widths.push(px(tags(html, "cell")[0], "width"));
  }
  assert.deepEqual(widths, [36, 96, 150], "the board's cell widths");
});

test("a folded lane keeps its head and draws no cells", async () => {
  const { html } = await draw(lists(), { edit: (s) => (s.folded = ["l1"]) });
  const lane = laneHtml(html)[0];
  assert.ok(/class="lane folded"/.test(lane) && lane.includes('aria-expanded="false"'), "folded");
  assert.ok(!lane.includes('class="cell'), "no cells");
});

test("the tree shows the real state: no Play or Next step", async () => {
  const { html } = await draw(lists());
  assert.ok(!/Play|Next step/.test(html), "the board's simulation is not drawn");
});

test("the footer names the board's facts and buttons", async () => {
  const view = lists("running");
  view[0].steps[1].task = "task-2";
  view[0].steps[1].branch = "fix/login-code";
  const { h } = await draw(view);
  h.click("[data-step]", { step: "s2" });
  h.run("renderPlanPanel(shownSurface(active))");
  assert.ok(h.html().includes("Open worker thread"), "the board's label");
  assert.ok(/class="fact">Branch<b class="mono">fix\/login-code<\/b>/.test(h.html()), "the git branch, in mono");
  assert.ok(/class="fact">List<b>Make a late code still work<\/b>/.test(h.html()));
});

test("phone, 390 px: Steps gives lanes that scroll sideways, Dots gives none", async () => {
  const room = 390 - 2 * 14;
  const steps = await draw(lists("running"), { zoom: 1 });
  assert.ok(px(tags(steps.html, "track")[0], "width") > room, "a Steps lane is wider than the phone's scroll area");
  assert.match(cssSrc, /\.pl-scroll\s*\{[^}]*overflow:\s*auto/, "the area scrolls");
  const dots = await draw(lists("running"), { zoom: 0 });
  for (const t of tags(dots.html, "track")) assert.ok(px(t, "width") <= room, `a Dots lane fits ${room}px, got ${px(t, "width")}`);
  const phone = cssSrc.slice(cssSrc.indexOf("body[data-phone-screen]"));
  assert.match(phone, /\.pl[^{]*button[^{]*\{[^}]*min-height:\s*44px/, "every button is touch-sized on the phone");
  assert.match(phone, /\.pl-zoom button[^{]*\{[^}]*min-width:\s*44px/, "so is the zoom");
});

test("gui-plan.css uses theme tokens only: no hex, rgb() or hsl() literal", () => {
  assert.deepEqual(cssSrc.replace(/\/\*[\s\S]*?\*\//g, "").match(/#[0-9a-fA-F]{3,8}\b|rgba?\(\s*\d|hsla?\(/g) ?? [], []);
});
