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

const sandbox = {
  tasksTab,
  contextMeter,
  composerBanners,
  threadView,
  contextCard: { hidden: true },
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
vm.runInContext(
  `
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
  threadView.dataset.items = JSON.stringify(threadOf(active).items.map((item) => item.text));
  threadView.innerHTML = agentCardsHtml(active, null, true) + agentsFooterHtml(active);
}

${agents.slice(agents.indexOf("const agentsOf"))}
${between(gui, "function hookPhase", "async function fetchWorkspace")}
${between(composer, "let tasksOpen = false;", "tasksTab.addEventListener")}
${between(composer, "function branchBanner", "composerBanners.addEventListener")}
${between(composer, "const formatTokens", "const contextCard")}
${between(gui, "let watchedSignature", "setInterval(() => {\n  publishThreadWatch")}
`,
  sandbox,
);

/** A session's transcript as `thread` reads it: whole lines from a byte cursor, the last task list,
 * context and branch among them, and every agent. A line with no item still moves the cursor. */
function transcript(session = "s") {
  let lines = [{ size: 10, item: { kind: "user", at: "t0", text: "start" }, branch: "main" }];
  return {
    session,
    agents: [],
    partial: 0,
    append(line) {
      lines.push({ size: 10, ...line });
    },
    rewind(keep) {
      lines = lines.slice(0, keep);
    },
    restart(next, first) {
      this.session = next;
      lines = [{ size: 10, ...first }];
    },
    page(from) {
      const whole = lines.reduce((n, line) => n + line.size, 0);
      const reset = from > whole + this.partial;
      const start = reset ? 0 : from;
      let at = 0;
      const delta = [];
      for (const line of lines) {
        if (at >= start) delta.push(line);
        at += line.size;
      }
      const page = {
        session: this.session,
        items: delta.filter((line) => line.item).map((line) => line.item),
        next: start + delta.reduce((n, line) => n + line.size, 0),
        reset,
        agents: this.agents,
      };
      for (const key of ["context", "branch", "todos"]) {
        const last = delta.findLast((line) => line[key]);
        if (last) page[key] = last[key];
      }
      return structuredClone(page);
    },
  };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));

/** The hub between the page and machine `box`. A reply carries the page as it was when the call
 * reached the machine. Replies and nudges wait in `queue` until a test delivers them, in any order. */
function network(transcripts) {
  const queue = [];
  const watches = [];
  sandbox.callOn = (_machine, call) => {
    if (call.cmd === "watch_threads") {
      watches.push([...call.tabs].sort());
      return Promise.resolve(null);
    }
    assert.equal(call.cmd, "thread");
    const page = transcripts[call.tab].page(call.from);
    return new Promise((resolve) => queue.push(() => resolve(page)));
  };
  return {
    queue,
    watches,
    nudge(tab) {
      queue.push(() => sandbox.onThreadEvent("box", { kind: "thread", tab }));
    },
    async deliver(at = 0) {
      const [next] = queue.splice(at, 1);
      next();
      await settle();
    },
    async drain(pick = () => 0) {
      while (queue.length) await this.deliver(pick(queue.length));
    },
  };
}

function tabFor(id, status = "working") {
  return {
    key: `box:${id}`,
    machine: "box",
    view: { id, status, cwd: "/repo", message: "" },
    thread: { session: null, next: 0, items: [], loaded: false, history: null, open: new Set(), workspace: { branch: "main" } },
  };
}

function clearScreen() {
  Object.assign(tasksTab, { innerHTML: "", hidden: true, dataset: {} });
  Object.assign(contextMeter, { innerHTML: "", hidden: true, attrs: {}, dataset: {} });
  Object.assign(composerBanners, { innerHTML: "", hidden: true, dataset: {} });
  Object.assign(threadView, { innerHTML: "", dataset: {} });
}

function painted() {
  return {
    items: JSON.parse(threadView.dataset.items ?? "[]"),
    tasks: tasksTab.hidden ? "" : tasksTab.innerHTML,
    context: contextMeter.hidden ? null : contextMeter.getAttribute("aria-label"),
    branch: composerBanners.innerHTML,
    agents: threadView.innerHTML,
  };
}

async function opened() {
  clearScreen();
  const tab = tabFor("tab-1");
  const s = transcript();
  const net = network({ "tab-1": s });
  sandbox.tabs = [tab];
  sandbox.active = tab;
  sandbox.fetchThread(tab);
  await net.drain();
  assert.deepEqual(painted().items, ["start"]);
  return { tab, s, net };
}

async function poll(tab, net) {
  sandbox.fetchThread(tab);
  await net.drain();
}

const text = (body) => ({ kind: "text", at: "t3", text: body });
const todos = { at: "t1", steps: [{ step: "Ship the fix", status: "in_progress" }] };
const runningAgent = { id: "a1", name: "Look around", state: "running", tool: "toolu_1", updated: "t5", steps: [] };

test("a task list line with no other item repaints the tasks", async () => {
  const { tab, s, net } = await opened();
  s.append({ todos });
  await poll(tab, net);
  assert.match(painted().tasks, /Ship the fix/);
});

test("a usage line with no other item repaints the context ring", async () => {
  const { tab, s, net } = await opened();
  s.append({ context: { tokens: 500000, at: "t2" } });
  await poll(tab, net);
  assert.equal(painted().context, "Context window 50% used");
});

test("a branch line with no other item repaints the branch notice", async () => {
  const { tab, s, net } = await opened();
  s.append({ branch: "feature" });
  await poll(tab, net);
  assert.match(painted().branch, /<code[^>]*>feature<\/code>/);
});

test("a nudge reads the new line without waiting for the poll", async () => {
  const { s, net } = await opened();
  s.append({ item: text("visible") });
  net.nudge("tab-1");
  await net.drain();
  assert.deepEqual(painted().items, ["start", "visible"]);
});

test("a nudge during a read reads again, so the new line and its agent card show", async () => {
  const { tab, s, net } = await opened();
  sandbox.fetchThread(tab);
  s.append({ item: { kind: "tool", at: "t4", id: "toolu_1", text: "spawning" } });
  s.agents = [runningAgent];
  net.nudge("tab-1");
  await net.deliver(1);
  await net.drain();
  assert.deepEqual(painted().items, ["start", "spawning"]);
  assert.match(painted().agents, /Look around/);
});

test("a background tab's agents arrive during its turn and keep it watched after", async () => {
  clearScreen();
  const background = tabFor("tab-1", "working");
  const shown = tabFor("tab-2", "idle");
  const busy = transcript();
  const net = network({ "tab-1": busy, "tab-2": transcript() });
  sandbox.tabs = [background, shown];
  sandbox.active = shown;
  sandbox.fetchThread(background);
  sandbox.fetchThread(shown);
  await net.drain();
  sandbox.fetchThread(background);
  busy.append({ item: { kind: "tool", at: "t4", id: "toolu_1", text: "spawning" } });
  busy.agents = [runningAgent];
  net.nudge("tab-1");
  await net.deliver(1);
  await net.drain();
  background.view.status = "done";
  assert.equal(sandbox.phaseOf(background), "delegating");
  sandbox.forgetThreadWatch();
  sandbox.publishThreadWatch();
  await settle();
  assert.deepEqual(net.watches.at(-1), ["tab-1", "tab-2"]);
  sandbox.active = background;
  sandbox.renderThread();
  assert.match(painted().agents, /Look around/);
});

test("a rewound transcript replaces the lines, the tasks, the branch, and the context ring", async () => {
  const { tab, s, net } = await opened();
  s.append({ item: text("gone"), todos, context: { tokens: 500000, at: "t2" }, branch: "feature" });
  await poll(tab, net);
  assert.equal(painted().context, "Context window 50% used");
  s.rewind(1);
  await poll(tab, net);
  assert.deepEqual(painted(), { items: ["start"], tasks: "", context: null, branch: "", agents: "" });
});

test("a new session hides the old session's context ring", async () => {
  const { tab, s, net } = await opened();
  s.append({ item: text("answer"), context: { tokens: 500000, at: "t2" } });
  await poll(tab, net);
  assert.equal(painted().context, "Context window 50% used");
  s.restart("s2", { item: { kind: "user", at: "t9", text: "fresh" } });
  await poll(tab, net);
  assert.deepEqual(painted().items, ["fresh"]);
  assert.equal(painted().context, null);
});

function random(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Appends a run of lines picked by `seed`. Each kind is a field of the thread page alone. */
function appends(seed) {
  const rand = random(seed);
  const pick = (list) => list[Math.floor(rand() * list.length)];
  const steps = [];
  const count = 1 + Math.floor(rand() * 10);
  for (let i = 1; i <= count; i++) {
    const kind = pick(["item", "todos", "context", "branch", "agents", "partial"]);
    if (kind === "item") steps.push({ item: text(`line ${i}`) });
    if (kind === "todos") steps.push({ todos: { at: `t${i}`, steps: [{ step: `task ${i}`, status: "in_progress" }] } });
    if (kind === "context") steps.push({ context: { tokens: 100000 * (1 + (i % 9)), at: `t${i}` } });
    if (kind === "branch") steps.push({ branch: pick(["main", "feature", "fix"]) });
    if (kind === "agents") steps.push({ agents: [{ ...runningAgent, id: `a${i}`, name: `agent ${i}`, tool: `toolu_${i}` }] });
    if (kind === "partial") steps.push({ partial: true, item: text(`slow ${i}`) });
  }
  return { steps, rand };
}

/** What a fresh page shows after reading these lines in one poll. */
function expected(steps) {
  const last = (key) => steps.findLast((step) => step[key])?.[key];
  const agentsNow = last("agents") ?? [];
  const tasks = last("todos");
  const context = last("context");
  const branch = last("branch") ?? "main";
  return {
    items: ["start", ...steps.filter((step) => step.item).map((step) => step.item.text)],
    tasks: tasks ? new RegExp(`<span class="current">${tasks.steps[0].step}</span>`) : /^$/,
    context: context ? `Context window ${(context.tokens / 1e4).toFixed(0)}% used` : null,
    branch: branch === "main" ? /^$/ : new RegExp(`<code[^>]*>${branch}</code>`),
    agents: agentsNow.map((agent) => agent.name),
  };
}

async function play(seed, how) {
  const { tab, s, net } = await opened();
  const { steps, rand } = appends(seed);
  for (const step of steps) {
    const changes = step.partial
      ? [() => (s.partial = 4), () => ((s.partial = 0), s.append({ item: step.item }))]
      : [() => (step.agents ? (s.agents = step.agents) : s.append(step))];
    for (const change of changes) {
      change();
      if (how === "push") net.nudge("tab-1");
      else if (rand() < 0.4) sandbox.fetchThread(tab);
      const deliveries = Math.floor(rand() * (net.queue.length + 1));
      for (let i = 0; i < deliveries && net.queue.length; i++) await net.deliver(Math.floor(rand() * net.queue.length));
    }
  }
  await net.drain(() => Math.floor(rand() * net.queue.length));
  if (how === "poll") await poll(tab, net);
  return { screen: painted(), steps };
}

test("nudged reads and poll-only reads leave the screen a fresh read shows, for any run of appends", async () => {
  for (let seed = 1; seed <= 300; seed++) {
    const pushed = await play(seed, "push");
    const polled = await play(seed, "poll");
    const want = expected(pushed.steps);
    assert.deepEqual(pushed.screen, polled.screen, `seed ${seed}: nudged and polled screens differ`);
    assert.deepEqual(pushed.screen.items, want.items, `seed ${seed} items`);
    assert.match(pushed.screen.tasks, want.tasks, `seed ${seed} tasks`);
    assert.equal(pushed.screen.context, want.context, `seed ${seed} context`);
    assert.match(pushed.screen.branch, want.branch, `seed ${seed} branch`);
    for (const name of want.agents) assert.match(pushed.screen.agents, new RegExp(name), `seed ${seed} agents`);
  }
});
