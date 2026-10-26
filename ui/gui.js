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
  sidebarOpen: !narrow(768),
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
const projectName = (tab) => (threadOf(tab).workspace?.project ?? tab.view.cwd).split(/[\\/]/).filter(Boolean).pop() || "/";
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
    if (page.reset) t.items = [];
    if (page.session !== t.session) {
      const restart = t.next > 0;
      Object.assign(t, { session: page.session, items: [], next: 0, history: null });
      if (restart) page = await callOn(tab.machine, { cmd: "thread", tab: tab.view.id, from: 0 });
    }
    t.unsupported = false;
    const grew = page.items.length > 0 || !t.loaded;
    if (page.items.some((i) => i.kind === "model")) t.picked = null;
    if (page.context) t.context = page.context;
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
    tab.dialog = tab.menu || tab.answering ? null : terminalDialog(tab);
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

/** T3 Code's keybindings (packages/shared/src/keybindings.ts) for the commands the app view has.
 * `mod` is ⌘ on a Mac and Ctrl elsewhere, and `when` reads `guiContext()`. ⌘1-9 jump on the web
 * too, where T3 Code leaves them to the browser. */
const DEFAULT_KEYBINDINGS = [
  { key: "mod+b", command: "sidebar.toggle" },
  { key: "mod+[", command: "navigation.back", when: "!terminalFocus" },
  { key: "mod+]", command: "navigation.forward", when: "!terminalFocus" },
  { key: "mod+j", command: "terminal.toggle" },
  { key: "mod+alt+b", command: "rightPanel.toggle" },
  { key: "mod+shift+b", command: "threadPanel.toggle", when: "!terminalFocus" },
  { key: "mod+t", command: "rightPanel.new", when: "rightPanelOpen && !terminalFocus" },
  { key: "mod+d", command: "terminal.split", when: "terminalFocus" },
  { key: "mod+shift+d", command: "terminal.splitVertical", when: "terminalFocus" },
  { key: "mod+n", command: "terminal.new", when: "terminalFocus" },
  { key: "mod+w", command: "terminal.close", when: "terminalFocus" },
  { key: "mod+w", command: "rightPanel.close", when: "!terminalFocus" },
  { key: "mod+shift+t", command: "view.reopenClosed" },
  { key: "mod+d", command: "diff.toggle", when: "!terminalFocus" },
  { key: "mod+shift+j", command: "preview.toggle" },
  { key: "mod+k", command: "commandPalette.toggle", when: "!terminalFocus" },
  { key: "mod+p", command: "filePicker.toggle", when: "!terminalFocus" },
  { key: "mod+shift+f", command: "projectSearch.toggle", when: "!terminalFocus" },
  { key: "mod+u", command: "usage.open", when: "!terminalFocus" },
  { key: "mod+alt+a", command: "theme.select", when: "!terminalFocus" },
  { key: "mod+alt+shift+a", command: "appearance.cycle", when: "!terminalFocus" },
  { key: "mod+alt+shift+t", command: "themeEditor.toggle" },
  { key: "mod+s", command: "composer.stash", when: "!terminalFocus" },
  { key: "mod+n", command: "chat.new", when: "!terminalFocus" },
  { key: "mod+shift+o", command: "chat.new", when: "!terminalFocus" },
  { key: "mod+shift+n", command: "chat.newLocal", when: "!terminalFocus" },
  { key: "mod+alt+n", command: "chat.newWithoutProject", when: "!terminalFocus" },
  { key: "mod+f", command: "chat.find", when: "!terminalFocus && !previewFocus" },
  { key: "mod+shift+m", command: "modelPicker.toggle", when: "!terminalFocus" },
  { key: "mod+shift+h", command: "composer.host", when: "!terminalFocus" },
  { key: "mod+shift+e", command: "composer.effort", when: "!terminalFocus" },
  { key: "mod+shift+a", command: "composer.mode", when: "!terminalFocus" },
  { key: "mod+shift+x", command: "composer.workspace", when: "!terminalFocus" },
  { key: "mod+shift+g", command: "composer.branch", when: "!terminalFocus" },
  { key: "mod+shift+l", command: "composer.previousWorktree", when: "!terminalFocus" },
  { key: "mod+shift+k", command: "pullRequest.copyNumber", when: "!terminalFocus" },
  { key: "mod+o", command: "editor.openFavorite" },
  { key: "mod+shift+[", command: "thread.previous" },
  { key: "mod+shift+]", command: "thread.next" },
  { key: "mod+shift+c", command: "thread.copyReference", when: "!terminalFocus" },
  { key: "mod+shift+s", command: "thread.settle", when: "!terminalFocus" },
  { key: "mod+shift+p", command: "thread.pin", when: "!terminalFocus" },
  ...[1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => ({ key: `mod+${n}`, command: `thread.jump.${n}`, when: "!modelPickerOpen" })),
  ...[1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => ({ key: `mod+${n}`, command: `modelPicker.jump.${n}`, when: "modelPickerOpen" })),
];

const SHIFTED_KEYS = { "!": "1", "@": "2", "#": "3", $: "4", "%": "5", "^": "6", "&": "7", "*": "8", "(": "9", ")": "0", "~": "`", _: "-", "+": "=", "{": "[", "}": "]", "|": "\\", ":": ";", '"': "'", "<": ",", ">": ".", "?": "/" };
const CODE_KEYS = { Backquote: "`", Backslash: "\\", BracketLeft: "[", BracketRight: "]", Comma: ",", Equal: "=", Minus: "-", Period: ".", Quote: "'", Semicolon: ";", Slash: "/" };
for (let n = 0; n < 10; n++) CODE_KEYS[`Digit${n}`] = String(n);

/** `"mod+shift+["` as `{ key, mod, meta, ctrl, shift, alt }`, or null. */
function parseShortcut(value) {
  const tokens = String(value).toLowerCase().split("+").map((t) => t.trim());
  if (tokens.at(-1) === "") {
    while (tokens.at(-1) === "") tokens.pop();
    tokens.push("+");
  }
  const sc = { key: null, mod: false, meta: false, ctrl: false, shift: false, alt: false };
  for (const t of tokens) {
    if (!t) return null;
    if (t === "cmd" || t === "meta") sc.meta = true;
    else if (t === "ctrl" || t === "control") sc.ctrl = true;
    else if (t === "shift") sc.shift = true;
    else if (t === "alt" || t === "option") sc.alt = true;
    else if (t === "mod") sc.mod = true;
    else if (sc.key !== null) return null;
    else sc.key = t === "space" ? " " : t === "esc" ? "escape" : t;
  }
  return sc.key === null ? null : sc;
}
/** The keys an event can stand for: the layout's, the letter at its position, and the US key at it. */
function eventKeys(e) {
  const layout = e.key.toLowerCase() === "esc" ? "escape" : e.key.toLowerCase();
  const keys = new Set([layout]);
  const letter = e.code?.match(/^Key([A-Z])$/)?.[1];
  if (letter && !/^[a-z]$/.test(layout)) keys.add(letter.toLowerCase());
  keys.add(/^[a-z]$/.test(layout) ? layout : CODE_KEYS[e.code] ?? layout);
  return keys;
}
function matchesShortcut(e, sc) {
  if (!IS_MAC && e.getModifierState?.("AltGraph") && !/^[a-z0-9]$/i.test(e.key)) return false;
  const meta = sc.meta || (sc.mod && IS_MAC);
  const ctrl = sc.ctrl || (sc.mod && !IS_MAC);
  return e.metaKey === meta && e.ctrlKey === ctrl && e.shiftKey === sc.shift && e.altKey === sc.alt && eventKeys(e).has(sc.key);
}

/** T3 Code's `when` grammar: identifiers, `!`, `&&`, `||` and parentheses, as `{ type, ... }` nodes. */
function parseWhen(expression) {
  const tokens = [];
  for (let i = 0; i < expression.length; ) {
    const rest = expression.slice(i);
    const word = /^[A-Za-z_][A-Za-z0-9_.-]*/.exec(rest)?.[0];
    if (/^\s/.test(rest)) i++;
    else if (rest.startsWith("&&") || rest.startsWith("||")) (tokens.push(rest.slice(0, 2)), (i += 2));
    else if ("!()".includes(rest[0])) tokens.push(expression[i++]);
    else if (word) (tokens.push({ name: word }), (i += word.length));
    else return null;
  }
  let at = 0;
  const primary = (depth) => {
    const t = tokens[at];
    if (depth > 32 || !t) return null;
    if (t.name) return (at++, { type: "identifier", name: t.name });
    if (t !== "(") return null;
    at++;
    const node = or(depth + 1);
    if (!node || tokens[at] !== ")") return null;
    at++;
    return node;
  };
  const unary = (depth) => {
    let nots = 0;
    while (tokens[at] === "!") (at++, nots++);
    let node = primary(depth);
    while (node && nots--) node = { type: "not", node };
    return node;
  };
  const binary = (type, op, next) => (depth) => {
    let left = next(depth);
    while (left && tokens[at] === op) {
      at++;
      const right = next(depth);
      left = right && { type, left, right };
    }
    return left;
  };
  const and = binary("and", "&&", unary);
  const or = binary("or", "||", and);
  const ast = tokens.length ? or(0) : null;
  return ast && at === tokens.length ? ast : null;
}
function evalWhen(node, context) {
  if (!node) return true;
  if (node.type === "identifier") return node.name === "true" || (node.name !== "false" && !!context[node.name]);
  if (node.type === "not") return !evalWhen(node.node, context);
  if (node.type === "and") return evalWhen(node.left, context) && evalWhen(node.right, context);
  return evalWhen(node.left, context) || evalWhen(node.right, context);
}

/** What `when` clauses read, as T3 Code's ShortcutMatchContext. */
function guiContext() {
  const el = document.activeElement;
  return {
    terminalFocus: !!el?.closest?.(".xterm"),
    terminalOpen: ui.drawer,
    previewFocus: !!el?.closest?.("#panel-body .browser"),
    previewOpen: !!active && ui.panelOpen && ui.panel === "browser",
    rightPanelOpen: !!active && ui.panelOpen,
    modelPickerOpen: !composerMenu.hidden && composerMenu.dataset.kind === "models",
    usagePageOpen: document.body.dataset.page === "usage",
    composerFocus: el === prompt,
    editableFocus: !!el?.closest?.("input, textarea, select, [contenteditable]"),
    isDesktop: !!TAURI,
    isWeb: !TAURI,
  };
}

/** The user's keybindings, every rule the way T3 Code's keybindings.json keeps them: the defaults
 * once, then the user's edits. Defaults added since are put in unless their chord is taken. */
let keybindingRules = [];
function loadKeybindings() {
  let saved = null;
  try {
    saved = JSON.parse(load("gui-keybindings"));
  } catch {}
  const rules = Array.isArray(saved) ? saved.filter((r) => typeof r?.command === "string" && parseShortcut(r.key) && (r.when === undefined || parseWhen(r.when))) : [];
  const commands = new Set(rules.map((r) => r.command));
  for (const d of DEFAULT_KEYBINDINGS) if (!commands.has(d.command) && !rules.some((r) => r.key === d.key && r.when === d.when)) rules.push({ ...d });
  keybindingRules = rules.map((r) => ({ ...r, shortcut: parseShortcut(r.key), whenAst: r.when ? parseWhen(r.when) : null }));
}
function saveKeybindings(rules) {
  save("gui-keybindings", JSON.stringify(rules.map(({ key, command, when }) => (when ? { key, command, when } : { key, command }))));
  loadKeybindings();
}
loadKeybindings();

/** The command a key press runs now: the last rule whose chord and `when` match, as T3 Code resolves it. */
function resolveGuiCommand(e, context = guiContext()) {
  for (let i = keybindingRules.length - 1; i >= 0; i--) {
    const rule = keybindingRules[i];
    if (evalWhen(rule.whenAst, context) && matchesShortcut(e, rule.shortcut)) return rule.command;
  }
  return null;
}

const KEY_LABELS = { " ": "Space", escape: "Esc", arrowup: "Up", arrowdown: "Down", arrowleft: "Left", arrowright: "Right" };
/** T3 Code's formatShortcutLabel: ⌃⌥⇧⌘K on a Mac, Ctrl+Alt+Shift+K elsewhere. */
function shortcutText(sc) {
  const key = KEY_LABELS[sc.key] ?? (sc.key.length === 1 ? sc.key.toUpperCase() : sc.key[0].toUpperCase() + sc.key.slice(1));
  const meta = sc.meta || (sc.mod && IS_MAC);
  const ctrl = sc.ctrl || (sc.mod && !IS_MAC);
  if (IS_MAC) return `${ctrl ? "⌃" : ""}${sc.alt ? "⌥" : ""}${sc.shift ? "⇧" : ""}${meta ? "⌘" : ""}${key}`;
  return [ctrl && "Ctrl", sc.alt && "Alt", sc.shift && "Shift", meta && "Meta", key].filter(Boolean).join("+");
}
/** The label of the chord that runs `command` with nothing focused, or with `context`, as T3 Code's
 * shortcutLabelForCommand: a later rule that takes the same chord hides it. */
function bindingLabel(command, context = {}) {
  const ctx = { isDesktop: !!TAURI, isWeb: !TAURI, ...context };
  const claimed = new Set();
  for (let i = keybindingRules.length - 1; i >= 0; i--) {
    const { shortcut: sc, whenAst } = keybindingRules[i];
    if (!evalWhen(whenAst, ctx)) continue;
    const chord = shortcutText(sc);
    if (claimed.has(chord)) continue;
    claimed.add(chord);
    if (keybindingRules[i].command === command) return chord;
  }
  return null;
}

/** The app view's commands, which each part of it adds with `onGuiKey` by its default chord or with
 * `onGuiCommand` by name. A handler given `when` overrides the others while `when()` holds. */
const guiCommands = new Map();
const onGuiCommand = (command, run, { when = null } = {}) => (guiCommands.get(command) ?? guiCommands.set(command, []).get(command)).push({ run, when });
const onGuiKey = (key, run, { shift = false, alt = false, when = null } = {}) => {
  const commands = new Set([key].flat().map((k) => {
    const chord = `mod+${alt ? "alt+" : ""}${shift ? "shift+" : ""}${SHIFTED_KEYS[k] ?? k.toLowerCase()}`;
    return DEFAULT_KEYBINDINGS.find((r) => r.key === chord && r.when !== "terminalFocus")?.command ?? chord;
  }));
  for (const command of commands) onGuiCommand(command, run, { when });
};
/** Runs `command` as its chord would, true when a handler took it. */
function runGuiCommand(command, e) {
  const live = (guiCommands.get(command) ?? []).filter((h) => h.when?.(e) !== false);
  const hit = live.find((h) => h.when) ?? live[0];
  hit?.run(e);
  return !!hit;
}

window.addEventListener("keydown", (e) => {
  const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  // ⌘E, which T3 Code doesn't have: Ctrl+Shift+E off the Mac, where a shell needs Ctrl+E to end the line.
  if (mod && !e.altKey && e.key && eventKeys(e).has("e") && e.shiftKey === !IS_MAC) {
    setMode(guiOn() ? "terminal" : "gui");
    e.preventDefault();
    e.stopPropagation();
    return;
  }
  if (!guiOn() || !e.key || e.target.closest?.("[data-keybinding-capture]")) return;
  const command = resolveGuiCommand(e);
  if (!command) return;
  if (paletteOpen() && paletteChord(command)) {
    e.preventDefault();
    e.stopPropagation();
    return;
  }
  if (!runGuiCommand(command, e)) return;
  e.preventDefault();
  e.stopPropagation();
}, true);
onGuiCommand("rightPanel.new", () => document.getElementById("panel-add").click());
onGuiCommand("rightPanel.close", () => ui.panelOpen && closeSurface());
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
