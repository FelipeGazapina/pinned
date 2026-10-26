const threadList = document.getElementById("thread-list");
const threadView = document.getElementById("thread");
const composer = document.getElementById("composer");
const prompt = document.getElementById("prompt");
const approval = document.getElementById("approval");
const panelBody = document.getElementById("panel-body");

const guiOn = () => document.body.dataset.mode === "gui";
const POLL_MS = 1200;
const narrow = (px) => matchMedia(`(max-width: ${px}px)`).matches;

const threads = new Map();
const threadOf = (tab) => {
  if (!threads.has(tab.key)) threads.set(tab.key, { session: null, next: 0, items: [], loaded: false, workspace: null, history: null, open: new Set() });
  return threads.get(tab.key);
};

/** What the page saves about a thread goes under its claude session once the machine reports one,
 * since tab ids start over when the app does. */
const threadKey = (tab) => (tab.view.session ? `session:${tab.view.session}` : tab.key);
const rekeys = [];
const onRekey = (move) => rekeys.push(move);
/** A tab's session changed, so what was saved under its old key moves to the new one. */
const rekeyThread = (from, to) => rekeys.forEach((move) => move(from, to));
/** Saved entries of a past run under a tab id would land on whichever thread gets that id next. */
const sessionsOnly = (saved) => Object.fromEntries(Object.entries(saved ?? {}).filter(([k]) => k.startsWith("session:")));

let seenAt = {};
try {
  seenAt = sessionsOnly(JSON.parse(load("gui-seen")));
} catch {}
onRekey((from, to) => {
  if (!(from in seenAt)) return;
  seenAt[to] = seenAt[from];
  delete seenAt[from];
  save("gui-seen", JSON.stringify(seenAt));
});

const ui = {
  get panel() {
    return shownSurface(active)?.kind;
  },
  get panelOpen() {
    return panelOf(active).open;
  },
  sidebarOpen: load("gui-sidebar") !== "false" && !narrow(768),
  drawer: false,
  search: "",
  project: load("gui-project") ?? "",
  settledOpen: load("gui-settled-open") === "true",
  diffScope: "turn",
  diffTurn: null,
  diffView: load("gui-diff-view") === "split" ? "split" : "unified",
  diffWrap: load("gui-diff-wrap") !== "false",
  diffHideSpace: load("gui-diff-whitespace") !== "false",
  diffTree: load("gui-diff-tree") === "true",
  collapsed: new Set(),
  seenFiles: new Set(),
  treeClosed: new Set(),
  commentAt: null,
};

function setMode(mode) {
  document.body.dataset.mode = mode;
  save("mode", mode);
  if (mode === "terminal") {
    if (active) refit(active);
  } else renderGui();
  focusInput();
  if (TAURI) fitWindowTo(mode).catch(console.warn);
}

async function fitWindowTo(mode) {
  const size = (await win.innerSize()).toLogical(await win.scaleFactor());
  let sizes = {};
  try {
    sizes = JSON.parse(load("view-sizes")) ?? {};
  } catch {}
  sizes[mode === "gui" ? "terminal" : "gui"] = { width: size.width, height: size.height };
  const want = sizes[mode] ?? (mode === "gui" ? { width: Math.max(size.width, 1180), height: Math.max(size.height, 760) } : size);
  const fit = { width: Math.min(want.width, screen.availWidth - 40), height: Math.min(want.height, screen.availHeight - 40) };
  sizes[mode] = fit;
  save("view-sizes", JSON.stringify(sizes));
  if (Math.round(fit.width) !== Math.round(size.width) || Math.round(fit.height) !== Math.round(size.height)) await win.setSize(new TAURI.dpi.LogicalSize(fit.width, fit.height));
}

const ago = (unix) => {
  if (!unix) return "";
  const s = Math.max(0, Date.now() / 1000 - unix);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
};
const projectName = (tab) => (threadOf(tab).workspace?.scratch ? "No project" : (threadOf(tab).workspace?.project ?? tab.view.cwd).split("/").filter(Boolean).pop() || "/");
const firstPrompt = (tab) => threadOf(tab).items.find((i) => i.kind === "user")?.text.split("\n")[0];
const threadTitle = (tab) => sideMarks.titles[threadKey(tab)] || tab.view.title || firstPrompt(tab) || "New thread";
/** `text` as a head that CSS truncates and its last 8 characters, once it is longer than `over`. */
const middle = (text, over = 14) => (text.length > over ? `<span class="head">${esc(text.slice(0, -8))}</span><span class="tail">${esc(text.slice(-8))}</span>` : `<span class="head">${esc(text)}</span>`);
const prOf = (tab) => [...threadOf(tab).items].reverse().find((i) => i.kind === "pr");
/** claude's hooks report no answer to a permission menu, so a permission notice with no menu on
 * screen means the turn went on. It sends "waiting for your input" a while after a turn ends. */
function hookPhase(tab) {
  if (tab.menu) return "asking";
  const status = statusOf(tab);
  const message = tab.view.message ?? "";
  if (status !== "needs-input") return status;
  if (/permission/i.test(message)) return "working";
  return /waiting for your input/i.test(message) ? "done" : "input";
}
/** Agents outlive the turn that spawned them, so the thread is in progress while one runs. */
function phaseOf(tab) {
  const phase = hookPhase(tab);
  return ["done", "idle"].includes(phase) && runningAgents(tab).length ? "delegating" : phase;
}
const turnLive = (tab) => ["working", "asking", "input"].includes(phaseOf(tab));
const unseenDone = (tab) => phaseOf(tab) === "done" && tab !== active && lastActivity(tab) > (seenAt[threadKey(tab)] ?? 0);

async function fetchThread(tab) {
  const t = threadOf(tab);
  if (t.fetching) return;
  t.fetching = true;
  try {
    let page = await callOn(tab.machine, { cmd: "thread", tab: tab.view.id, from: t.next });
    t.error = null;
    if (!page.session && t.session) return;
    if (page.reset) Object.assign(t, { items: [], todos: null, branch: null });
    if (page.session !== t.session) {
      const restart = t.next > 0;
      Object.assign(t, { session: page.session, items: [], next: 0, history: null, todos: null, branch: null });
      if (restart) page = await callOn(tab.machine, { cmd: "thread", tab: tab.view.id, from: 0 });
    }
    t.unsupported = false;
    const grew = page.items.length > 0 || !t.loaded;
    if (page.items.some((i) => i.kind === "model")) t.picked = null;
    if (page.context) t.context = page.context;
    if (page.todos) t.todos = page.todos;
    if (page.branch) t.branch = page.branch;
    t.items.push(...page.items);
    t.next = page.next;
    t.loaded = true;
    const moved = takeAgents(t, page.agents);
    if ((grew || moved) && guiOn()) {
      if (tab === active) {
        renderThread();
        markThreadSeen(tab);
      }
      renderSidebar();
    }
    if (grew && t.session && !t.history && tab === active) fetchHistory(tab);
  } catch (e) {
    t.unsupported = /unknown variant|can't answer/.test(String(e));
    t.loaded = t.unsupported;
    t.error = String(e);
    if (tab === active && guiOn()) t.unsupported ? renderUnsupported() : renderThread();
  } finally {
    t.fetching = false;
  }
}

async function fetchWorkspace(tab) {
  const t = threadOf(tab);
  const cwd = tab.view.cwd;
  if (t.workspaceFor === cwd) return;
  t.workspaceFor = cwd;
  const workspace = await callOn(tab.machine, { cmd: "workspace", tab: tab.view.id }).catch(() => null);
  // A tab opened on a remote machine starts in its home folder and moves; the older answer loses.
  if (t.workspaceFor !== cwd) return;
  t.workspace = workspace;
  if (!t.workspace) t.workspaceFor = null;
  if (!guiOn()) return;
  renderSidebar();
  if (tab === active) {
    renderThread();
    if (!t.history) fetchHistory(tab);
  }
}

async function fetchHistory(tab) {
  const t = threadOf(tab);
  if (!t.session || !t.workspace || t.fetchingHistory) return;
  t.fetchingHistory = true;
  const session = t.session;
  const turns = await callOn(tab.machine, { cmd: "project_history", path: t.workspace.project, session }).catch(() => null);
  t.fetchingHistory = false;
  if (t.session !== session || !turns) return;
  t.history = turns.reverse();
  if (tab === active && guiOn()) {
    renderThread();
    if (ui.panelOpen && ui.panel === "diff") renderPanel();
  }
}

const statusBeforeRender = new Map();

function renderGui() {
  for (const tab of tabs) {
    const finished = statusBeforeRender.get(tab.key) === "working" && statusOf(tab) !== "working";
    statusBeforeRender.set(tab.key, statusOf(tab));
    // The turn's last lines are in the transcript by now; read them before its changes, which attach to the newest turn.
    if (finished) {
      Object.assign(threadOf(tab), { history: null, workspaceFor: null, gitDiff: null });
      fetchThread(tab);
    }
  }
  for (const key of statusBeforeRender.keys()) if (!tabByKey(key)) statusBeforeRender.delete(key);
  if (!guiOn()) return;
  document.body.dataset.sidebar = ui.sidebarOpen ? "open" : "closed";
  for (const tab of tabs) {
    const t = threadOf(tab);
    if (!t.watching) {
      t.watching = true;
      watchMenu(tab);
    }
    tab.menu = tab.answering ? null : terminalMenu(tab);
    if (!t.loaded) fetchThread(tab);
    fetchWorkspace(tab);
    if (tab === active && !t.history) fetchHistory(tab);
  }
  for (const key of threads.keys()) if (!tabByKey(key)) threads.delete(key);
  markThreadSeen(active);
  noteVisit();
  renderSidebar();
  if (active && threadOf(active).unsupported) renderUnsupported();
  else renderThread();
  renderPanel();
}

setInterval(() => {
  if (!guiOn() || document.hidden) return;
  for (const tab of tabs) if (tab === active || turnLive(tab) || phaseOf(tab) === "delegating") fetchThread(tab);
}, POLL_MS);
setInterval(() => guiOn() && renderSidebar(), 60 * 1000);
document.addEventListener("visibilitychange", () => markThreadSeen(active));

function stepThread(step) {
  const order = [...threadList.querySelectorAll(".thread-row")].map((r) => tabByKey(r.dataset.tab));
  const i = order.indexOf(active);
  if (order.length > 1) activate(order[(i + step + order.length) % order.length]);
}

/** The app view's shortcuts, which each part of it adds with `onGuiKey`. They are ⌘ chords, Ctrl ones
 * off the Mac, where a shell in the terminal needs Ctrl+E to end the line. With Option a key goes by
 * position, since Option turns B into ∫ on a Mac. A chord given `when` overrides the others while `when()` holds. */
const guiKeys = [];
const onGuiKey = (key, run, { shift = false, alt = false, when = null } = {}) => guiKeys.push({ keys: [key].flat(), run, shift, alt, when });

window.addEventListener("keydown", (e) => {
  if (settingsOpen() || pickerOpen()) return;
  const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!mod) return;
  const key = (e.altKey ? e.code.replace(/^Key/, "") : e.key).toLowerCase();
  const switchView = key === "e" && !e.altKey && e.shiftKey === !IS_MAC;
  if (switchView) setMode(guiOn() ? "terminal" : "gui");
  else if (!guiOn() || (!IS_MAC && e.target.closest?.("#terms"))) return;
  else {
    const hits = guiKeys.filter((k) => k.keys.includes(key) && k.shift === e.shiftKey && k.alt === e.altKey && k.when?.() !== false);
    const hit = hits.find((k) => k.when) ?? hits[0];
    if (!hit) return;
    hit.run(e);
  }
  e.preventDefault();
  e.stopPropagation();
}, true);
onGuiKey(["[", "{"], () => stepThread(-1), { shift: true });
onGuiKey(["]", "}"], () => stepThread(1), { shift: true });

document.getElementById("mode-gui").onclick = () => setMode("gui");
document.getElementById("mode-terminal").onclick = () => setMode("terminal");
document.getElementById("gui-settings").onclick = () => toggleSettings();
document.getElementById("gui-minimize").onclick = () => setView("pill");

document.body.dataset.mode = load("mode") === "terminal" ? "terminal" : "gui";
// The window always opens at the terminal's size.
if (TAURI && guiOn()) fitWindowTo("gui").catch(console.warn);
document.body.dataset.panel = "closed";
document.body.dataset.drawer = "closed";
document.body.dataset.sidebar = ui.sidebarOpen ? "open" : "closed";
