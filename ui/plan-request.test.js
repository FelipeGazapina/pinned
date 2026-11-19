import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const panelSrc = read("gui-panel.js");
const planSrc = read("gui-plan.js");
const modelSrc = read("requests-model.js");
const orchestratorSrc = read("gui-orchestrator.js");
const line = (src, from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const flush = () => new Promise((resolve) => setImmediate(resolve));
const plain = (value) => JSON.parse(JSON.stringify(value));
const decode = (html) => html.replace(/&#(\d+);/g, (_, c) => String.fromCharCode(c));

const STEP = (over) => ({ id: "s1", name: "Draft", what: "", state: "done", why: null, cost: null, task: null, fixes: [], ...over });
/** A PlanView as the hub replies to `{cmd: "plan", tab, plan}`. */
function view(over = {}) {
  return {
    id: "p1",
    rev: 3,
    title: "Ship the signup form",
    asked_at: 1763000000000,
    finished_at: null,
    closed: null,
    lists: [
      { id: "l1", name: "Build", task: null, steps: [STEP({ id: "s1", name: "Draft the form", state: "done" }), STEP({ id: "s2", name: "Wire the API", state: "running" })] },
      { id: "l2", name: "Review", task: null, steps: [STEP({ id: "s3", name: "Copy review", state: "waiting" })] },
    ],
    ...over,
  };
}
const plan = (id, over = {}) => ({ id, state: "working", title: `Plan ${id}`, done: 1, total: 3, now: "Test", strips: ["dr", "p"], requests: ["q1"], ...over });
const CAPTAINS = [
  { machine: "m1", path: "/work/storefront", project: "storefront", machine_name: "Mac mini" },
  { machine: "m2", path: "/work/billing", project: "billing", machine_name: "Laptop" },
];
/** A RequestGlance as the host sends it (wire contract). */
const rglance = (over = {}) => ({
  id: "q1", title: "Why are login codes late?", project: "storefront", opened_at: 1763000000000, kind: "asked", closed_at: null, needs: "", needs_at: null,
  last_user_at: null, last_who: null, last_text: "", last_at: null, replies: 0, captains: CAPTAINS, tab: "tab-20", ...over,
});
const tabOf = (machine, view) => ({ key: `${machine}:${view.id}`, machine, view, panel: { open: false, surfaces: [], active: null, max: false } });
const captainTab = (machine, id, cwd, plans) => tabOf(machine, { id, role: "orchestrator", session: `s-${id}`, cwd, plans });
/** The request's own Orchestrator tab, carrying the glances. */
const requestTab = (over = {}, plans = [], glance = rglance()) => tabOf("m1", { id: "tab-20", role: "orchestrator", orchestrator_global: true, session: "s-20", request: "q1", requests: [glance], plans, ...over });

/** gui-plan.js in a page, with the real request lookups from gui-orchestrator.js and requests-model.js. */
function harness({ reply, ...extra } = {}) {
  const log = { shown: [], saved: 0, added: [], on: [], renders: 0, activated: [], diff: [] };
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
      log.on.push({ machine, call: plain(call) });
      return reply(machine, call);
    },
    openDelegated: async () => {},
    keyOf: (machine, id) => `${machine}:${id}`,
    tabByKey: (key) => tabs.find((t) => t.key === key) ?? null,
    refreshTabs: async () => {},
    activate: (tab) => log.activated.push(tab.key),
    startTab: async () => {},
    setDiffState: (tab, change) => log.diff.push({ tab: tab.key, change: plain(change) }),
    ...extra,
  };
  vm.createContext(context);
  const run = (code) => vm.runInContext(code, context);
  run(line(settingsSrc, "const esc = "));
  run(fn(modelSrc, "requestPlans"));
  run(fn(orchestratorSrc, "threadRequestGlance"));
  run(fn(orchestratorSrc, "threadRequestPlans"));
  run(planSrc);
  const click = (sel, dataset = {}) => listeners.click({ target: { closest: (s) => (s === sel ? { dataset } : s === ".pl" ? { dataset: {} } : null) } });
  const push = () => models.forEach((f) => f());
  const html = () => log.shown.at(-1) ?? "";
  /** Opens the Plan surface on `tab` and draws it. */
  const open = async (tab) => {
    context.active = tab;
    tab.panel.open = true;
    tab.panel.surfaces.push({ id: "plan", kind: "plan", title: "Plan" });
    tab.panel.active = "plan";
    run("renderPlanPanel(shownSurface(active))");
    await flush();
  };
  const draw = async () => {
    run("renderPlanPanel(shownSurface(active))");
    await flush();
  };
  return { context, log, run, click, push, html, tabs, open, draw };
}

/** Answers `plan` calls with a view titled by where it lives, and `user_request` with `snapshots`. */
const hub = (snapshots = [], glance = rglance()) => (machine, call) => {
  if (call.cmd === "plan") return [view({ id: call.plan, title: `Plan ${machine}/${call.tab}/${call.plan}` })];
  if (call.cmd === "user_request") return { glance, stands: { asked: "", done: "", left: "", needs: "", links: [], at: 0 }, items: [], plans: snapshots };
  throw new Error(`unexpected ${call.cmd}`);
};
const planCalls = (h) => h.log.on.filter((c) => c.call.cmd === "plan").map((c) => `${c.machine}/${c.call.tab}/${c.call.plan}`).sort();
/** Where a plan's first list sits in the html, by its (machine, tab, plan) scope; -1 when the plan is not drawn. */
const place = (html, scope) => html.indexOf(`data-fold="${scope}|l1"`);
/** The html of one section: from its heading to the next heading. */
const under = (html, title) => {
  const parts = html.split(/(?=<section class="pl-sec-group">)/);
  return parts.find((part) => decode(part).includes(`>${title}</h3>`)) ?? "";
};
const sections = (html) => [...html.matchAll(/<h3 class="pl-sec">([^<]*)<\/h3>/g)].map((m) => decode(m[1]));

test("two Captains give two sections, in captains order, with every plan fetched by (machine, tab, plan)", async () => {
  const h = harness({ reply: hub() });
  // The Captain tabs come in the opposite order to the glance's captains; the same plan id p1 lives on both machines.
  const billing = captainTab("m2", "tab-7", "/work/billing", [plan("p1")]);
  const store = captainTab("m1", "tab-1", "/work/storefront", [plan("p1"), plan("p2")]);
  const request = requestTab();
  h.tabs.push(billing, store, request);
  await h.open(request);

  assert.deepEqual(planCalls(h), ["m1/tab-1/p1", "m1/tab-1/p2", "m2/tab-7/p1"], "one plan call per plan, on that plan's machine and tab");
  assert.deepEqual(sections(h.html()), ["Captain storefront · Mac mini", "Captain billing · Laptop"], "one section per Captain, in glance.captains order");
  const html = h.html();
  const at = (scope) => place(html, scope);
  assert.ok(at("m1|tab-1|p1") > 0 && at("m1|tab-1|p2") > at("m1|tab-1|p1"), "the storefront section holds its two plans");
  assert.ok(at("m2|tab-7|p1") > at("m1|tab-1|p2"), "the billing section follows");
  assert.ok(html.includes("Plan m1/tab-1/p2"), "a section with two plans names each plan");
  const storeHtml = under(html, "Captain storefront · Mac mini");
  const billHtml = under(html, "Captain billing · Laptop");
  assert.ok(place(storeHtml, "m1|tab-1|p1") >= 0 && place(storeHtml, "m1|tab-1|p2") >= 0 && place(storeHtml, "m2|tab-7|p1") < 0, "storefront's section holds only its own plans");
  assert.ok(place(billHtml, "m2|tab-7|p1") >= 0 && place(billHtml, "m1|tab-1|p1") < 0, "billing's section holds only billing's plan");
});

test("the Orchestrator's own lanes come first, under the header Orchestrator", async () => {
  const h = harness({ reply: hub() });
  const store = captainTab("m1", "tab-1", "/work/storefront", [plan("p1")]);
  const request = requestTab({}, [plan("p1", { requests: [] })]);
  h.tabs.push(store, request);
  await h.open(request);

  assert.deepEqual(sections(h.html()), ["Orchestrator", "Captain storefront · Mac mini", "Captain billing · Laptop"], "Orchestrator first, then each Captain");
  assert.ok(planCalls(h).includes("m1/tab-20/p1"), "the own plan is fetched on the request's tab");
  const html = h.html();
  assert.ok(place(html, "m1|tab-20|p1") > 0 && place(html, "m1|tab-20|p1") < place(html, "m1|tab-1|p1"), "the own plan draws above the Captain's");
});

test("a plan named in two requests appears only in the request being viewed", async () => {
  const h = harness({ reply: hub() });
  const store = captainTab("m1", "tab-1", "/work/storefront", [plan("p1", { requests: ["q1", "q2"] }), plan("p2", { requests: ["q2"] })]);
  const request = requestTab();
  h.tabs.push(store, request);
  await h.open(request);

  assert.deepEqual(planCalls(h), ["m1/tab-1/p1"], "only the plan that names q1 is fetched");
  assert.ok(place(h.html(), "m1|tab-1|p1") > 0 && place(h.html(), "m1|tab-1|p2") < 0, "the other request's plan is not drawn");
});

test("a snapshot plan draws without a fetch, marked ended, in its Captain's section", async () => {
  const old = view({ id: "p9", title: "Old plan", lists: [{ id: "l9", name: "Cleanup", task: null, steps: [STEP({ id: "s9", name: "Delete the flag", state: "done" })] }] });
  const one = rglance({ captains: [CAPTAINS[0]] });
  const h = harness({ reply: hub([{ machine: "m1", path: "/work/storefront", plan: old }], one) });
  const request = requestTab({}, [], one);
  h.tabs.push(request);
  await h.open(request);

  assert.deepEqual(planCalls(h), [], "a snapshot is never fetched as a plan");
  assert.equal(h.log.on.filter((c) => c.call.cmd === "user_request").length, 1, "the snapshots come from one user_request call");
  assert.deepEqual(h.log.on.find((c) => c.call.cmd === "user_request"), { machine: "m1", call: { cmd: "user_request", id: "q1" } });
  const html = h.html();
  assert.ok(html.includes("Old plan") && html.includes("Cleanup") && html.includes("Delete the flag"), "the snapshot's lists draw");
  assert.ok(html.includes('class="pl-ended">ended<'), "it is marked ended");
  assert.deepEqual(sections(html), ["Captain storefront · Mac mini"], "it sits in its Captain's section");
});

test("a changed glance re-fetches once; the same glance sends nothing", async () => {
  const h = harness({ reply: hub() });
  const store = captainTab("m1", "tab-1", "/work/storefront", [plan("p1")]);
  const request = requestTab();
  h.tabs.push(store, request);
  await h.open(request);
  const before = h.log.on.length;
  assert.equal(before, 2, "opening sends one plan call and one user_request call");

  h.push();
  await flush();
  assert.equal(h.log.on.length, before, "a push with the same glances sends no call");

  store.view.plans = [plan("p1", { done: 2, strips: ["dd", "p"] })];
  h.push();
  await flush();
  assert.equal(h.log.on.length, before + 1, "a changed glance sends exactly one call");
  assert.deepEqual(h.log.on.at(-1), { machine: "m1", call: { cmd: "plan", tab: "tab-1", plan: "p1" } }, "and it is that plan's");
});

test("plan titles, step names, Captain names and a snapshot's text with markup draw as text", async () => {
  const evil = "<img src=x onerror=alert(1)>";
  const link = "[x](javascript:alert(1))";
  const bad = (id) => view({ id, title: `${evil} ${link}`, lists: [{ id: "l1", name: `${evil} ${link}`, task: null, steps: [STEP({ id: "s1", name: `${evil} ${link}`, state: "running", what: `${evil} ${link}`, why: link })] }] });
  const captains = [{ machine: "m1", path: "/work/storefront", project: `${evil} ${link}`, machine_name: `${evil} ${link}` }];
  const glance = rglance({ captains, title: `${evil} ${link}`, project: `${evil}`, last_text: `${evil} ${link}`, needs: `${evil} ${link}` });
  const h = harness({ reply: (machine, call) => (call.cmd === "plan" ? [bad(call.plan)] : { glance, stands: {}, items: [], plans: [{ machine: "m1", path: "/work/storefront", plan: bad("p9") }] }) });
  const store = captainTab("m1", "tab-1", "/work/storefront", [plan("p1", { title: `${evil} ${link}` })]);
  const request = requestTab({}, [], glance);
  h.tabs.push(store, request);
  await h.open(request);
  h.click("[data-step]", { step: "m1|tab-1|p1|s1" });
  await h.draw();

  const html = h.html();
  assert.ok(html.length > 500 && html.includes("pl-list"), "the plan drew");
  assert.ok(!/<img/i.test(html), "no image element is created");
  assert.ok(!/href\s*=\s*["']?javascript:/i.test(html), "no javascript: href is created");
  assert.ok(!/<a[\s>]/i.test(html), "no link element is created");
  assert.ok(decode(html).includes(evil), "the markup shows as text");
});

test("a list folded in one Captain's section does not fold the same list id in another", async () => {
  const h = harness({ reply: hub() });
  const store = captainTab("m1", "tab-1", "/work/storefront", [plan("p1")]);
  const billing = captainTab("m2", "tab-7", "/work/billing", [plan("p1")]);
  const request = requestTab();
  h.tabs.push(store, billing, request);
  await h.open(request);
  const scoped = (m, t) => `${m}|${t}|p1|l1`;
  assert.equal((h.html().match(/class="pl-list"/g) ?? []).length, 4, "two plans of two lists draw open");

  h.click("[data-fold]", { fold: scoped("m1", "tab-1") });
  await h.draw();
  assert.equal((h.html().match(/class="pl-list folded"/g) ?? []).length, 1, "only the picked plan's list folds");
  assert.ok(h.html().includes('data-fold="m1|tab-1|p1|l1" aria-expanded="false"'), "it is the storefront's");
  assert.ok(h.html().includes('data-fold="m2|tab-7|p1|l1" aria-expanded="true"'), "billing's stays open");
});

test("a host that answers user_request with an error still draws the plans, with no banner", async () => {
  const h = harness({
    reply: (machine, call) => {
      if (call.cmd === "user_request") throw new Error("unknown cmd user_request");
      return hub()(machine, call);
    },
  });
  const store = captainTab("m1", "tab-1", "/work/storefront", [plan("p1")]);
  const request = requestTab();
  h.tabs.push(store, request);
  await h.open(request);
  assert.ok(place(h.html(), "m1|tab-1|p1") > 0, "the live plan still draws");
  assert.ok(!/could not|error|failed/i.test(decode(h.html())), "nothing reports the missing call");
});

test("the add menu offers Plan on a request thread", () => {
  const h = harness({ reply: hub() });
  Object.assign(h.context, {
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
  h.context.active = requestTab();
  const labels = h.run(`${fn(panelSrc, "addMenu")}\naddMenu()`).map((m) => m.label);
  assert.ok(labels.includes("Plan"), "a request thread offers Plan");
  h.context.active = requestTab({ request: undefined });
  assert.ok(!h.run("addMenu()").map((m) => m.label).includes("Plan"), "the anchor does not");
});

test("a plan goes to its Captain by machine and path, never by folder or project name", async () => {
  // The checkout folder is personal-harness, the project is pinned.
  const captains = [{ machine: "m1", path: "/Users/x/personal-harness", project: "pinned", machine_name: "Mac mini" }];
  const glance = rglance({ captains });
  const h = harness({ reply: hub([], glance) });
  const tab = captainTab("m1", "tab-1", "/Users/x/personal-harness", [plan("p1")]);
  const stranger = captainTab("m1", "tab-2", "/Users/x/pinned", [plan("p2")]);
  const request = requestTab({}, [], glance);
  h.tabs.push(tab, stranger, request);
  await h.open(request);
  const html = h.html();
  assert.deepEqual(sections(html).slice(0, 1), ["Captain pinned · Mac mini"], "the section is headed with the project key");
  assert.ok(place(html, "m1|tab-1|p1") > html.indexOf("Captain pinned"), "the plan on that path draws under it");
  assert.equal(sections(html).length, 2, "a tab on a folder that merely carries the project name is its own section");
  assert.equal(sections(html)[1], "Other Captain · pinned", "and is not taken for the Captain");
});

/** A plan of 4 main steps (2 done) and one fix of 2 steps (1 done): the board counts 3 of 6. */
const withFix = (id = "p1") =>
  view({
    id,
    lists: [
      {
        id: "l1",
        name: "Build",
        task: null,
        steps: [
          STEP({ id: "s1", name: "One", state: "done" }),
          STEP({ id: "s2", name: "Two", state: "done" }),
          STEP({ id: "s3", name: "Three", state: "pending" }),
          STEP({ id: "s4", name: "Four", state: "failed", fixes: [{ id: "f1", steps: [STEP({ id: "x1", name: "Fix a", state: "done" }), STEP({ id: "x2", name: "Fix b", state: "running" })] }] }),
        ],
      },
    ],
  });

test("the request panel counts fix steps with the main steps: 3 of 6", async () => {
  const h = harness({ reply: (machine, call) => (call.cmd === "plan" ? [withFix(call.plan)] : hub()(machine, call)) });
  const store = captainTab("m1", "tab-1", "/work/storefront", [plan("p1")]);
  const request = requestTab();
  h.tabs.push(store, request);
  await h.open(request);
  const html = h.html();
  assert.match(html, /<span class="pl-count">1 list, 3 of 6 steps, 1 fix branch<\/span>/, "lists, done of all steps, fix branches");
  assert.ok(html.includes('class="pl-progress"><span style="width:50%"'), "the bar uses the same numbers");
});

const SNAP = (id, title) => view({ id, title, lists: [{ id: "l9", name: `List of ${title}`, task: null, steps: [STEP({ id: "s9", name: `Step of ${title}`, state: "done" })] }] });

test("two Captains, one snapshot each: each draws under its own Captain, marked ended, with no Ended plans section", async () => {
  const snaps = [
    { machine: "m2", path: "/work/billing", plan: SNAP("p9", "Billing leftovers") },
    { machine: "m1", path: "/work/storefront", plan: SNAP("p8", "Storefront leftovers") },
  ];
  const h = harness({ reply: hub(snaps) });
  const request = requestTab();
  h.tabs.push(request);
  await h.open(request);
  const html = h.html();
  assert.deepEqual(sections(html), ["Captain storefront · Mac mini", "Captain billing · Laptop"], "no Ended plans section");
  const store = under(html, "Captain storefront · Mac mini");
  const bill = under(html, "Captain billing · Laptop");
  assert.ok(store.includes("Storefront leftovers") && !store.includes("Billing leftovers"), "the storefront snapshot is under storefront");
  assert.ok(bill.includes("Billing leftovers") && !bill.includes("Storefront leftovers"), "the billing snapshot is under billing");
  assert.equal((html.match(/class="pl-ended">ended</g) ?? []).length, 2, "each is marked ended");
});

test("a snapshot whose machine and exact path match no Captain goes to Ended plans", async () => {
  const snaps = [
    { machine: "m1", path: "/work/storefront-copy", plan: SNAP("p7", "Same machine, other path") },
    { machine: "m3", path: "/work/storefront", plan: SNAP("p6", "Other machine, same path") },
  ];
  const h = harness({ reply: hub(snaps) });
  const request = requestTab();
  h.tabs.push(request);
  await h.open(request);
  const html = h.html();
  assert.deepEqual(sections(html), ["Captain storefront · Mac mini", "Captain billing · Laptop", "Ended plans"]);
  const ended = under(html, "Ended plans");
  assert.ok(ended.includes("Same machine, other path") && ended.includes("Other machine, same path"), "both sit under Ended plans");
  assert.ok(!under(html, "Captain storefront · Mac mini").includes("Same machine, other path"), "a path that only starts the same is not the Captain");
});

test("See the diff on a request thread: a live step with a task and a branch shows it, with its Captain as owner, and drawing calls only plan and user_request", async () => {
  const live = view({ lists: [{ id: "l1", name: "Build", task: null, steps: [STEP({ id: "s2", name: "Wire the API", state: "running", task: "task-2", branch: "feat/wire-api" })] }] });
  const h = harness({ reply: (machine, call) => (call.cmd === "plan" ? [live] : hub()(machine, call)) });
  const store = captainTab("m1", "tab-1", "/work/storefront", [plan("p1")]);
  const request = requestTab();
  h.tabs.push(store, request);
  await h.open(request);
  assert.match(h.html(), /data-see-diff="task-2"[^>]*data-diff-machine="m1"[^>]*data-diff-tab="tab-1"/, "the Captain's machine and tab own the click");
  assert.deepEqual([...new Set(h.log.on.map((c) => c.call.cmd))].sort(), ["plan", "user_request"], "no open_task while drawing");
});

test("See the diff: an ended snapshot never shows it, even with a task and a branch", async () => {
  const old = view({ id: "p9", title: "Old plan", lists: [{ id: "l9", name: "Cleanup", task: null, steps: [STEP({ id: "s9", name: "Delete the flag", state: "running", task: "task-9", branch: "feat/old" })] }] });
  const one = rglance({ captains: [CAPTAINS[0]] });
  const h = harness({ reply: hub([{ machine: "m1", path: "/work/storefront", plan: old }], one) });
  const request = requestTab({}, [], one);
  h.tabs.push(request);
  await h.open(request);
  assert.ok(h.html().includes("Delete the flag"), "the snapshot draws");
  assert.ok(!h.html().includes("data-see-diff") && !h.html().includes("See the diff"), "with no See the diff");
});
