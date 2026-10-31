import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function read(name) {
  return readFileSync(new URL(name, import.meta.url), "utf8");
}

function between(src, startAt, endAt) {
  const start = src.indexOf(startAt);
  const end = src.indexOf(endAt, start + startAt.length);
  if (start < 0 || end < 0) throw new Error(`missing slice ${startAt}`);
  return src.slice(start, end);
}

function element() {
  return {
    hidden: true,
    dataset: {},
    innerHTML: "",
    attrs: {},
    style: {},
    setAttribute(name, value) {
      this.attrs[name] = String(value);
    },
    getAttribute(name) {
      return this.attrs[name] ?? null;
    },
    toggleAttribute(name, force) {
      this.attrs[name] = !!force;
    },
    append() {},
    querySelector() {
      return null;
    },
    querySelectorAll() {
      return [];
    },
    addEventListener() {},
  };
}

const tasksTab = element();
const contextMeter = element();
const composerBanners = element();
const threadView = element();
const contextCard = { hidden: true };

const sandbox = {
  tasksTab,
  contextMeter,
  composerBanners,
  threadView,
  contextCard,
  approval: { hidden: true },
  composer: { dataset: {} },
  tabs: [],
  active: null,
  document: {
    hidden: false,
    getElementById(id) {
      if (id === "context-meter") return contextMeter;
      return element();
    },
  },
  callOn: async () => null,
};
vm.createContext(sandbox);

const gui = read("./gui.js");
const agents = read("./gui-agents.js");
const composer = read("./gui-composer.js");
const script = `
function threadOf(tab) { return tab.thread; }
function statusOf(tab) { return tab.view.status ?? "idle"; }
function guiOn() { return true; }
function keyOf(machine, id) { return machine + ":" + id; }
function tabByKey(key) { return tabs.find((tab) => tab.key === key) ?? null; }
function modelOf() { return null; }
function threadKey(tab) { return tab.key; }
function limitBanner() { return null; }
function icon(name, cls = "") { return '<svg data-icon="' + name + '" class="' + cls + '"></svg>'; }
function esc(text) { return String(text ?? "").replace(/[&<>"']/g, (c) => "&#" + c.charCodeAt(0) + ";"); }
function duration(ms) { return String(ms); }
function terminalOnly() { return false; }
function renderSidebar() {}
function markThreadSeen() {}
function fetchHistory() {}
function renderTerminalOnly() {}
function renderUnsupported() {}
const bannerDismissed = new Set();
let restoringBranch = null;
let bannersOpen = false;
function renderThread() {
  renderTasks(active);
  renderContext(active);
  renderBanners(active);
  threadView.innerHTML = agentCardsHtml(active, null, true) + agentsFooterHtml(active);
}

${agents.slice(agents.indexOf("const agentsOf"))}
${between(gui, "function hookPhase", "const unseenDone")}
${between(composer, "let tasksOpen = false;", "tasksTab.addEventListener")}
${between(composer, "function branchBanner", "composerBanners.addEventListener")}
${between(composer, "const formatTokens", "const contextCard")}
${between(gui, "function takeThreadPage", "async function fetchWorkspace")}
${between(gui, "let watchedSignature", "setInterval(() => {\n  publishThreadWatch")}
`;
vm.runInContext(script, sandbox);

const user = { kind: "user", at: "t0", text: "start" };
const quietAgents = [];
const runningAgent = { id: "a1", name: "Look around", state: "running", tool: "toolu_1", updated: "t5", steps: [] };
const todos = { at: "t1", steps: [{ step: "Ship the fix", status: "in_progress" }] };
const context = { tokens: 500000, at: "t2" };

const seed = { session: "s", from: 0, next: 10, reset: false, items: [user], agents: quietAgents };
const todosPage = { session: "s", from: 10, next: 20, reset: false, items: [], todos, agents: quietAgents };
const contextPage = { session: "s", from: 20, next: 30, reset: false, items: [], context, agents: quietAgents };
const branchPage = { session: "s", from: 30, next: 40, reset: false, items: [], branch: "feature", agents: quietAgents };
const agentPage = { session: "s", from: 40, next: 55, reset: false, items: [{ kind: "text", at: "t4", text: "done" }], agents: [runningAgent] };
const deltas = [seed, todosPage, contextPage, branchPage, agentPage];

function fresh(id = "tab-1") {
  return {
    key: `box:${id}`,
    machine: "box",
    view: { id, status: "working", cwd: "/repo", message: "" },
    thread: { session: null, next: 0, items: [], loaded: false, history: null, open: new Set(), workspace: { branch: "main" } },
  };
}

function clearScreen() {
  tasksTab.innerHTML = "";
  tasksTab.hidden = true;
  tasksTab.dataset.html = "";
  contextMeter.innerHTML = "";
  contextMeter.hidden = true;
  contextMeter.attrs = {};
  contextMeter.dataset.over = false;
  composerBanners.innerHTML = "";
  composerBanners.hidden = true;
  composerBanners.dataset.html = "";
  threadView.innerHTML = "";
}

function show(tab) {
  const t = tab.thread;
  return {
    items: t.items.map((item) => item.text ?? item.kind),
    todos: t.todos ?? null,
    context: t.context ?? null,
    branch: t.branch ?? null,
    agents: (t.agents ?? []).map((agent) => agent.name),
    next: t.next,
    tasks: tasksTab.innerHTML,
    contextLabel: contextMeter.getAttribute("aria-label") ?? "",
    branchHtml: composerBanners.innerHTML,
    agentsHtml: threadView.innerHTML,
  };
}

function folded(pages) {
  const page = { session: "s", reset: false, items: [], agents: quietAgents, next: 0 };
  for (const step of pages) {
    page.items = page.items.concat(step.items);
    page.next = step.next;
    if (step.todos) page.todos = step.todos;
    if (step.context) page.context = step.context;
    if (step.branch) page.branch = step.branch;
    if (step.agents) page.agents = step.agents;
  }
  return page;
}

async function played(pages, how) {
  clearScreen();
  const tab = fresh();
  sandbox.tabs = [tab];
  sandbox.active = tab;
  if (how === "push") {
    for (const page of pages) sandbox.onThreadEvent("box", { ...page, tab: tab.view.id });
  } else {
    sandbox.callOn = async () => folded(pages);
    await sandbox.fetchThread(tab);
  }
  return { tab, screen: show(tab) };
}

test("a todos-only update repaints the task list", async () => {
  const { screen } = await played([seed, todosPage], "push");
  assert.match(screen.tasks, /Ship the fix/);
  assert.equal(screen.todos.steps[0].step, "Ship the fix");
});

test("a context-only update repaints the context meter", async () => {
  const { screen } = await played([seed, todosPage, contextPage], "push");
  assert.equal(screen.contextLabel, "Context window 50% used");
  assert.equal(screen.context.tokens, 500000);
});

test("a branch-only update repaints the branch notice", async () => {
  const { screen } = await played([seed, todosPage, contextPage, branchPage], "push");
  assert.match(screen.branchHtml, /feature/);
  assert.equal(screen.branch, "feature");
});

test("a push during a poll leaves the agent card on screen", async () => {
  clearScreen();
  const tab = fresh();
  sandbox.tabs = [tab];
  sandbox.active = tab;
  sandbox.onThreadEvent("box", { ...seed, tab: tab.view.id });
  let release;
  sandbox.callOn = () => new Promise((resolve) => {
    release = resolve;
  });
  const flight = sandbox.fetchThread(tab);
  await Promise.resolve();
  sandbox.onThreadEvent("box", { ...agentPage, from: tab.thread.next, tab: tab.view.id });
  assert.match(threadView.innerHTML, /Look around/);
  release({ ...agentPage, items: agentPage.items });
  await flight;
  assert.match(threadView.innerHTML, /Look around/);
  assert.equal(tab.thread.agents[0].name, "Look around");
});

test("a background tab's agents appear and the tab stays watched", async () => {
  clearScreen();
  const tab = fresh("tab-1");
  const other = fresh("tab-2");
  tab.view.status = "done";
  sandbox.tabs = [tab, other];
  sandbox.active = other;
  sandbox.onThreadEvent("box", { ...seed, items: [user], agents: [runningAgent], tab: "tab-1" });
  assert.equal(sandbox.phaseOf(tab), "delegating");
  const watched = [];
  sandbox.callOn = async (_machine, call) => {
    watched.push(call);
    return null;
  };
  sandbox.publishThreadWatch();
  await Promise.resolve();
  const ids = watched.filter((call) => call.cmd === "watch_threads").flatMap((call) => call.tabs);
  assert.deepEqual([...ids].sort(), ["tab-1", "tab-2"]);
  sandbox.active = tab;
  sandbox.renderThread();
  assert.match(threadView.innerHTML, /Look around/);
});

test("push-driven updates and poll-only updates leave the same screen", async () => {
  for (let n = 1; n <= deltas.length; n++) {
    const pages = deltas.slice(0, n);
    const pushed = await played(pages, "push");
    const polled = await played(pages, "poll");
    assert.deepEqual(pushed.screen, polled.screen, `after ${n} appends`);
  }
});
