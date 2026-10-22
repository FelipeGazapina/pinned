const win = TAURI?.window.getCurrentWindow();

const applyOpacity = (percent) => document.documentElement.style.setProperty("--opacity", percent / 100);
if (TAURI) {
  invoke("get_opacity").then(applyOpacity);
  listen("opacity", applyOpacity);
}

const STATUS_TEXT = { idle: "", working: "working…", done: "done", "needs-input": "needs you", failed: "stopped" };
const ALERTS = ["done", "needs-input", "failed"];
const EDGES = ["top", "bottom", "left", "right"];
/** The app's own machine. Every other key in `machines` is the id the hub gave a machine. */
const LOCAL = "local";

const workspace = document.getElementById("workspace");
const tabBar = document.getElementById("tab-bar");
const tabList = document.getElementById("tab-list");
const terms = document.getElementById("terms");
const picker = document.getElementById("projects");

/** Every machine the UI shows, by id. Each: { view: MachineView, tabs: TabView[], transport, usage }. */
const machines = new Map();
/** While signed in: { hub, email, machine_id, user_token }, and the hub's client socket. */
let account = null;
let hub = null;

/** Tabs of every machine in display order. Each: { key, machine, view: TabView, term, fit, el, button, sink, attached }. */
const tabs = [];
let active = null;

const keyOf = (machine, id) => `${machine}:${id}`;
const tabByKey = (key) => tabs.find((t) => t.key === key);
const callOn = (machine, call) => machines.get(machine).transport.call(call);
const statusOf = (tab) => tab.view.status ?? "idle";
const machineName = (id) => (id === LOCAL ? "This Mac" : machines.get(id)?.view.name ?? "machine");
const initials = (name) => name.split(/[\s._-]+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join("");
const tilde = (path) => path.replace(/^(\/Users\/[^/]+|\/home\/[^/]+|\/root)(?=\/|$)/, "~");
const tabName = (tab) => tab.view.cwd.split("/").filter(Boolean).pop() || (tab.view.cwd ? "/" : "claude");
const isVertical = () => ["left", "right"].includes(document.body.dataset.tabs);

/** This UI's tab order as `machine:tab` keys, kept across reloads; a tab of a machine that is offline keeps its place. */
let order = [];
try {
  order = JSON.parse(load("tab-order")) ?? [];
} catch {}

function placeTabs() {
  for (const tab of tabs) if (!order.includes(tab.key)) order.push(tab.key);
  tabs.sort((a, b) => order.indexOf(a.key) - order.indexOf(b.key));
  for (const tab of tabs) tabList.append(tab.button);
  save("tab-order", JSON.stringify(order.slice(-200)));
}

/** Shows `views` as the machine's tabs: adds the new ones, updates the rest and removes those gone. */
function setTabs(machine, views, { forget = true } = {}) {
  const m = machines.get(machine);
  if (!m) return;
  m.tabs = views;
  const keep = new Set(views.map((v) => keyOf(machine, v.id)));
  for (const tab of tabs.filter((t) => t.machine === machine && !keep.has(t.key))) removeTab(tab, forget);
  for (const view of views) {
    const tab = tabByKey(keyOf(machine, view.id));
    if (tab) updateView(tab, view);
    else addTab(machine, view);
  }
  placeTabs();
  render();
}

const refreshTabs = async (machine) => setTabs(machine, await callOn(machine, { cmd: "tabs" }));

function updateView(tab, view) {
  const before = statusOf(tab);
  tab.view = view;
  const after = statusOf(tab);
  if (after !== before && ALERTS.includes(after) && document.body.dataset.view === "pill") {
    pill.dataset.status = after;
    document.body.classList.remove("alert");
    void document.body.offsetWidth;
    document.body.classList.add("alert");
  }
}

/** Opens a tab on `machine` at `path`, or its default folder when `path` is null, and starts claude there. */
async function openTab(machine, path, bypass = false) {
  const id = await callOn(machine, { cmd: "open_tab", path, bypass });
  await refreshTabs(machine);
  const tab = tabByKey(keyOf(machine, id));
  activate(tab);
  await startTab(tab);
  return tab;
}

/** The picker's target: a machine to open a new tab on, or `tab` to move to another folder. */
let picking = { machine: LOCAL, tab: null };
/** The picker's entries in display order, and which one Enter opens. */
let choices = [];
let chosen = 0;
/** Whether the next tab opened from the picker skips every permission check. Off each time the picker opens. */
let bypassNext = false;
const pickerOpen = () => !picker.hidden;
const onlineMachines = () => [...machines.entries()].filter(([, m]) => m.view.online).map(([id]) => id);

function setBypassNext(on) {
  bypassNext = on;
  picker.dataset.bypass = on;
  picker.querySelector(".bypass").textContent = on ? "⚡ Skip all permissions: on" : "⚡ Skip all permissions: off";
}

/** Opens the picker on `machine`, or the first machine online, to start a tab or to move `tab`. */
async function showProjects({ machine, tab = null } = {}) {
  machine ??= machines.get(picking.machine)?.view.online ? picking.machine : onlineMachines()[0];
  if (!machine) return;
  picking = { machine, tab };
  delete document.body.dataset.page;
  picker.hidden = false;
  picker.dataset.mode = tab ? "move" : "open";
  picker.innerHTML = `<button class="bypass" title="Opens the next tab with --dangerously-skip-permissions (⇧Enter)"></button>
    ${pickerMachinesHtml(machine, !!tab)}<div class="choices"><p class="loading">Loading projects…</p></div>`;
  picker.querySelector(".bypass").onclick = () => setBypassNext(!bypassNext);
  for (const b of picker.querySelectorAll(".machine")) b.onclick = () => showProjects({ machine: b.dataset.machine, tab });
  setBypassNext(false);
  let projects = [];
  try {
    projects = await callOn(machine, { cmd: "list_projects" });
  } catch (e) {
    console.warn("list_projects", e);
  }
  if (picking.machine !== machine || !pickerOpen()) return;
  const local = machine === LOCAL;
  choices = [
    ...projects.map((p) => ({ name: p.name, path: tilde(p.path), open: () => pickFolder(p.path) })),
    local
      ? { name: "Open folder…", open: () => pickLocalFolder() }
      : { name: "Type a path…", open: () => typePath(), keep: true },
  ];
  picker.querySelector(".choices").replaceChildren(
    ...choices.map((choice, i) => {
      const button = document.createElement("button");
      for (const [cls, text] of [["name", choice.name], ["path", choice.path]]) {
        if (!text) continue;
        const span = document.createElement("span");
        span.className = cls;
        span.textContent = text;
        button.append(span);
      }
      button.onmouseenter = () => choose(i);
      button.onclick = (e) => openChoice(i, e.shiftKey);
      return button;
    }),
  );
  picker.querySelector(".choices").insertAdjacentHTML("beforeend", `<p class="error" id="picker-error"></p>`);
  choose(0);
}

/** The row of machines on top of the picker, shown once there is more than this one. Offline ones can't be picked. */
function pickerMachinesHtml(current, lockToCurrent) {
  if (machines.size < 2 && current === LOCAL) return "";
  const row = [...machines.entries()].map(([id, m]) => {
    const off = !m.view.online || (lockToCurrent && id !== current);
    return `<button class="machine${id === current ? " chosen" : ""}" data-machine="${esc(id)}"${off ? " disabled" : ""}
      title="${esc(m.view.online ? m.view.name : `${m.view.name} is offline`)}"><span class="dot" data-online="${m.view.online}"></span>${esc(machineName(id))}</button>`;
  });
  return `<div class="machines">${row.join("")}</div>`;
}

function stepMachine(step) {
  if (picking.tab) return;
  const online = onlineMachines();
  const i = online.indexOf(picking.machine);
  if (online.length > 1) showProjects({ machine: online[(i + step + online.length) % online.length] });
}

function choose(i) {
  chosen = (i + choices.length) % choices.length;
  const entries = picker.querySelectorAll(".choices button");
  entries.forEach((b, j) => b.classList.toggle("chosen", j === chosen));
  entries[chosen]?.scrollIntoView({ block: "nearest" });
}

function openChoice(i, bypass = false) {
  if (bypass) bypassNext = true;
  if (!choices[i].keep) closeProjects();
  choices[i].open();
}

function closeProjects() {
  picker.hidden = true;
  active?.term.focus();
}

const toggleProjects = () => (pickerOpen() ? closeProjects() : showProjects());

/** Opens a new tab at `path` on the picker's machine, or moves the picker's tab there. Shows a refusal in the picker. */
async function pickFolder(path) {
  const { machine, tab } = picking;
  try {
    if (tab) await moveTab(tab, path);
    else if (machine === LOCAL) await openTab(machine, path, bypassNext);
    else await openRemote(machine, path, bypassNext);
    return true;
  } catch (e) {
    if (!pickerOpen()) await showProjects({ machine, tab });
    document.getElementById("picker-error").textContent = String(e);
    return false;
  }
}

async function pickLocalFolder() {
  const path = await invoke("pick_folder", { start: picking.tab?.view.cwd ?? null });
  if (path) pickFolder(path);
}

// A remote machine opens any path it is given, so the folder is checked by moving a fresh tab there.
async function openRemote(machine, path, bypass) {
  const id = await callOn(machine, { cmd: "open_tab", path: null, bypass });
  try {
    await callOn(machine, { cmd: "set_folder", tab: id, path });
  } catch (e) {
    callOn(machine, { cmd: "close_tab", tab: id });
    throw e;
  }
  await refreshTabs(machine);
  const tab = tabByKey(keyOf(machine, id));
  activate(tab);
  await startTab(tab);
}

/** Moves the tab to `path` on its machine and starts claude there afresh. */
async function moveTab(tab, path) {
  await callOn(tab.machine, { cmd: "set_folder", tab: tab.view.id, path });
  tab.term.reset();
  await startTab(tab);
  tab.term.focus();
}

/** Replaces the picker's last entry with an input for a path on the remote machine. */
function typePath() {
  const last = picker.querySelectorAll(".choices button")[choices.length - 1];
  const form = document.createElement("form");
  form.className = "type-path";
  form.innerHTML = `<input placeholder="/path/on/${esc(machineName(picking.machine))}" autocomplete="off" spellcheck="false">`;
  last.replaceWith(form);
  const field = form.querySelector("input");
  field.focus();
  form.onsubmit = async (e) => {
    e.preventDefault();
    const path = field.value.trim();
    if (!path) return;
    document.getElementById("picker-error").textContent = "";
    if (await pickFolder(path)) closeProjects();
  };
}

/** Shows the tab `view` of `machine`, attached to its output. A tab that someone else opened stays in the background. */
function addTab(machine, view) {
  const el = document.createElement("div");
  el.className = "term";
  el.hidden = true;
  terms.append(el);
  const term = new Terminal({
    fontFamily: "ui-monospace, Menlo, monospace",
    fontSize: 12,
    cursorBlink: true,
    allowTransparency: true,
    theme: { background: "#00000000" },
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open(el);

  const button = document.createElement("div");
  button.className = "tab";
  button.innerHTML = `<span class="dot"></span><span class="badge" hidden></span><span class="name"></span><button class="close" title="Close tab (⌘W)">✕</button><span class="resize" title="Drag to resize the tabs"></span>`;
  tabList.append(button);

  const sink = { write: (bytes) => term.write(bytes), reset: () => term.reset() };
  const tab = { key: keyOf(machine, view.id), machine, view, term, fit, el, button, sink };
  tab.attached = Promise.resolve(machines.get(machine).transport.attach(view.id, sink)).catch((e) => console.warn("attach", e));
  tabs.push(tab);
  term.onData((data) => typeInto(tab, data));
  button.querySelector(".close").onclick = () => closeTab(tab);
  button.onmousedown = (e) => {
    if (e.target.closest(".close")) return;
    if (e.target.closest(".resize")) {
      e.preventDefault();
      const left = button.getBoundingClientRect().left;
      return drag(e, { move: (e) => setSize("tab-width", e.clientX - left) });
    }
    activate(tab);
    drag(e, { move: (e) => reorder(tab, e) });
  };
  if (!active) activate(tab);
  return tab;
}

/** Keystrokes for the tab's session; Enter restarts claude once it has exited. */
function typeInto(tab, data) {
  if (tab.view.running) return callOn(tab.machine, { cmd: "write_input", tab: tab.view.id, data });
  if (data === "\r") startTab(tab);
}

async function startTab(tab) {
  await tab.attached;
  refit(tab);
  try {
    await callOn(tab.machine, { cmd: "start_claude", tab: tab.view.id, cols: tab.term.cols, rows: tab.term.rows });
    tab.view = { ...tab.view, running: true };
    render();
  } catch (e) {
    tab.term.write(`\r\n\x1b[31m[could not start claude: ${e}]\x1b[0m\r\n`);
  }
}

/** Ends the tab on its machine, which removes it from every device showing it. */
function closeTab(tab) {
  removeTab(tab, true);
  render();
  // The app quits with its last tab while signed out, so the local close goes through its own command.
  if (tab.machine === LOCAL) invoke("close_tab", { tab: tab.view.id });
  else callOn(tab.machine, { cmd: "close_tab", tab: tab.view.id }).catch(console.warn);
}

/** Takes the tab off this UI. `forget` drops its place in the order too, as it is gone for good. */
function removeTab(tab, forget) {
  const i = tabs.indexOf(tab);
  if (i < 0) return;
  tabs.splice(i, 1);
  machines.get(tab.machine)?.transport.detach(tab.view.id, tab.sink);
  tab.term.dispose();
  tab.el.remove();
  tab.button.remove();
  if (forget) order = order.filter((k) => k !== tab.key);
  if (active === tab) {
    active = null;
    if (tabs.length) activate(tabs[Math.min(i, tabs.length - 1)]);
  }
}

function activate(tab) {
  delete document.body.dataset.page;
  picker.hidden = true;
  active = tab;
  for (const t of tabs) {
    t.el.hidden = t !== tab;
    t.button.classList.toggle("active", t === tab);
  }
  refit(tab);
  render();
  tab.term.focus();
}

// A hidden terminal can't measure itself, so each tab fits when it is shown.
function refit(tab) {
  if (!tab || tab.el.hidden || document.body.dataset.view !== "expanded") return;
  tab.fit.fit();
  if (tab.view.running) callOn(tab.machine, { cmd: "resize_pty", tab: tab.view.id, cols: tab.term.cols, rows: tab.term.rows }).catch(() => {});
}

function render() {
  for (const tab of tabs) {
    const remote = tab.machine !== LOCAL;
    const where = remote ? `${machineName(tab.machine)}: ` : "";
    tab.button.dataset.status = statusOf(tab);
    tab.button.querySelector(".name").textContent = (tab.view.bypass ? "⚡ " : "") + tabName(tab);
    const badge = tab.button.querySelector(".badge");
    badge.hidden = !remote;
    badge.textContent = remote ? initials(machineName(tab.machine)) : "";
    tab.button.title = `${where}${tab.view.cwd}${tab.view.bypass ? "\nPermissions skipped" : ""}`;
  }
  document.body.dataset.empty = !tabs.length;
  renderCarousel();
  renderUsage();
  const header = document.querySelector("header");
  header.dataset.remote = !!active && active.machine !== LOCAL;
  if (!active) return;
  header.dataset.status = statusOf(active);
  document.getElementById("title").textContent =
    (active.machine === LOCAL ? "" : `${machineName(active.machine)}  `) + (tilde(active.view.cwd) || "claude");
  document.getElementById("status-text").textContent =
    active.view.status === "failed" && active.view.resume_at
      ? `limit, continues ${new Date(active.view.resume_at * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
      : STATUS_TEXT[statusOf(active)];
}

const pill = document.getElementById("pill");
const carousel = document.getElementById("carousel");
const track = document.getElementById("track");
// A card's three lines need this much height; below it the pill shows one compact row.
const CARDS_FROM_HEIGHT = 64;
const CAROUSEL_SPEED = 40;
const stillMotion = matchMedia("(prefers-reduced-motion: reduce)");
/** What the track was last built for; items are rebuilt only when this changes. */
let builtFor = "";

/** Loops every tab through the pill. Status and title changes update the items in place so the scroll doesn't jump. */
function renderCarousel() {
  const { width } = carousel.getBoundingClientRect();
  if (!width) return;
  const mode = pill.getBoundingClientRect().height >= CARDS_FROM_HEIGHT ? "cards" : "compact";
  const key = JSON.stringify([mode, Math.round(width), stillMotion.matches, tabs.map((t) => [t.key, tabName(t)])]);
  if (key !== builtFor) {
    builtFor = key;
    pill.dataset.mode = mode;
    buildTrack(width);
  }
  for (const item of track.querySelectorAll(".item")) {
    const tab = tabByKey(item.dataset.tab);
    item.dataset.status = statusOf(tab);
    item.querySelector(".status").textContent = tab.view.message || STATUS_TEXT[statusOf(tab)] || "idle";
    item.querySelector(".title").textContent = tab.view.title || tilde(tab.view.cwd);
    item.title = tab.view.title ? `${tab.view.cwd}\n${tab.view.title}` : tab.view.cwd;
  }
}

function buildTrack(width) {
  if (!tabs.length) return track.replaceChildren();
  const set = document.createElement("div");
  set.className = "set";
  for (const tab of tabs) {
    const item = document.createElement("button");
    item.className = "item";
    item.dataset.tab = tab.key;
    item.innerHTML = `<span class="dot"></span><span class="name"></span><span class="status"></span><span class="title"></span>`;
    item.querySelector(".name").textContent = tabName(tab);
    set.append(item);
  }
  track.replaceChildren(set);
  const setWidth = set.getBoundingClientRect().width;
  // Enough copies that the track still covers the pill when shifted by one whole set.
  const copies = stillMotion.matches ? 1 : Math.ceil(width / setWidth) + 1;
  for (let i = 1; i < copies; i++) track.append(set.cloneNode(true));
  track.style.setProperty("--set-width", `${setWidth}px`);
  track.style.setProperty("--period", `${setWidth / CAROUSEL_SPEED}s`);
}

/** Calls `move` while the mouse is dragged past a few pixels from `down`, then `drop` on release. */
function drag(down, { move, drop }) {
  let moved = false;
  const onMove = (e) => {
    if (!moved && Math.hypot(e.clientX - down.clientX, e.clientY - down.clientY) < 4) return;
    moved = true;
    move(e);
  };
  const onUp = (e) => {
    removeEventListener("mousemove", onMove);
    removeEventListener("mouseup", onUp);
    if (moved) drop?.(e);
  };
  addEventListener("mousemove", onMove);
  addEventListener("mouseup", onUp);
}

function reorder(tab, e) {
  const vertical = isVertical();
  const pos = vertical ? e.clientY : e.clientX;
  const others = tabs.filter((t) => t !== tab);
  let i = others.findIndex((t) => {
    const r = t.button.getBoundingClientRect();
    return pos < (vertical ? r.top + r.height / 2 : r.left + r.width / 2);
  });
  if (i < 0) i = others.length;
  others.splice(i, 0, tab);
  tabs.splice(0, tabs.length, ...others);
  tabList.insertBefore(tab.button, others[i + 1]?.button ?? null);
  const shown = new Set(tabs.map((t) => t.key));
  order = [...tabs.map((t) => t.key), ...order.filter((k) => !shown.has(k))];
  save("tab-order", JSON.stringify(order.slice(-200)));
}

function nearestEdge(e) {
  const r = workspace.getBoundingClientRect();
  const distance = { top: e.clientY - r.top, bottom: r.bottom - e.clientY, left: e.clientX - r.left, right: r.right - e.clientX };
  return EDGES.reduce((a, b) => (distance[b] < distance[a] ? b : a));
}

function placeTabBar(edge) {
  document.body.dataset.tabs = edge;
  save("tab-bar", edge);
}

const savedEdge = load("tab-bar");
placeTabBar(EDGES.includes(savedEdge) ? savedEdge : "top");

// Horizontal tabs share one width; the vertical bar has its own.
const SIZE_LIMITS = { "tab-width": [60, 400], "tab-bar-width": [90, 400] };
function setSize(key, px) {
  const [min, max] = SIZE_LIMITS[key];
  const clamped = Math.round(Math.min(max, Math.max(min, px)));
  document.documentElement.style.setProperty(`--${key}`, `${clamped}px`);
  save(key, clamped);
}
for (const key in SIZE_LIMITS) {
  const saved = Number(load(key));
  if (saved) setSize(key, saved);
}

document.getElementById("bar-resize").addEventListener("mousedown", (e) => {
  e.preventDefault();
  e.stopPropagation();
  const r = tabBar.getBoundingClientRect();
  drag(e, {
    move: (e) => setSize("tab-bar-width", document.body.dataset.tabs === "left" ? e.clientX - r.left : r.right - e.clientX),
  });
});

// Dragging the bar itself (not a tab) moves it to whichever side of the panel the mouse ends nearest.
tabBar.addEventListener("mousedown", (e) => {
  if (e.target.closest(".tab, button")) return;
  e.preventDefault();
  drag(e, {
    move: (e) => (workspace.dataset.drop = nearestEdge(e)),
    drop: (e) => {
      delete workspace.dataset.drop;
      placeTabBar(nearestEdge(e));
    },
  });
});

// ＋ and the folder button toggle the picker themselves, so a click on them doesn't count as outside.
window.addEventListener("mousedown", (e) => {
  if (pickerOpen() && !e.target.closest("#projects, #new-tab, #folder")) closeProjects();
}, true);

// The panel is nonactivating, so a click alone doesn't give it keyboard focus.
if (TAURI) {
  window.addEventListener("mousedown", () => invoke("focus_panel").then(() => settingsOpen() || active?.term.focus()), true);
  invoke("focus_panel");
}

const IS_MAC = /Mac/.test(navigator.platform);
const TAB_STEP = { ArrowLeft: -1, ArrowRight: 1 };
const PICKER_STEP = { ArrowUp: -1, ArrowDown: 1 };

/** Moves to the neighbouring tab, wrapping around at either end. */
function stepTab(step) {
  const i = tabs.indexOf(active);
  if (tabs.length > 1) activate(tabs[(i + step + tabs.length) % tabs.length]);
}

// Caught before xterm, which would otherwise swallow them.
window.addEventListener("keydown", (e) => {
  // The path field keeps its keys, except Escape, which goes back to the list.
  if (pickerOpen() && e.target.closest?.(".type-path")) {
    if (e.key === "Escape") showProjects(picking);
    return;
  }
  // Keys other than ⌘ shortcuts belong to the open picker, not the terminal behind it.
  if (pickerOpen() && !e.metaKey) {
    if (e.key in PICKER_STEP) choose(chosen + PICKER_STEP[e.key]);
    else if (e.key in TAB_STEP) stepMachine(TAB_STEP[e.key]);
    else if (e.key === "Enter") openChoice(chosen, e.shiftKey);
    else if (e.key === "b" && !e.ctrlKey && !e.altKey && !picking.tab) setBypassNext(!bypassNext);
    else if (e.key === "Escape") closeProjects();
    e.preventDefault();
    e.stopPropagation();
    return;
  }
  if (e.shiftKey || e.altKey) return;
  const tabKey = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  // In settings the arrows keep their text-editing meaning.
  if (tabKey && e.key in TAB_STEP && active && !settingsOpen()) {
    stepTab(TAB_STEP[e.key]);
    e.preventDefault();
    e.stopPropagation();
    return;
  }
  if (!e.metaKey || e.ctrlKey) return;
  if (e.key === ",") toggleSettings();
  else if (e.key === "t") toggleProjects();
  else if (e.key === "w" && active) closeTab(active);
  else if (/^[1-9]$/.test(e.key) && tabs[e.key - 1]) activate(tabs[e.key - 1]);
  else return;
  e.preventDefault();
  e.stopPropagation();
}, true);

// Claude Code attaches an image when a file path is pasted, so a drop becomes a paste of the escaped paths.
// The paths only mean something on this Mac, so a remote tab ignores the drop.
TAURI?.webview.getCurrentWebview().onDragDropEvent(({ payload }) => {
  if (payload.type !== "drop" || active?.machine !== LOCAL || !active.view.running) return;
  const escaped = payload.paths.map((p) => p.replace(/[^\w@%+=:,./-]/g, "\\$&"));
  active.term.paste(escaped.join(" ") + " ");
  active.term.focus();
});

// A clipboard image has no text for xterm to paste; Ctrl+V makes Claude Code read the image itself.
document.addEventListener("paste", (e) => {
  const types = [...(e.clipboardData?.types ?? [])];
  if (active?.machine !== LOCAL || !active.view.running || types.includes("text/plain") || !types.includes("Files")) return;
  e.preventDefault();
  e.stopPropagation();
  callOn(LOCAL, { cmd: "write_input", tab: active.view.id, data: "\x16" });
}, true);

/** A `wire::MachineEvent` from `machine`. Output never comes here: it goes to the attached terminal. */
function onMachineEvent(machine, event) {
  if (!machines.has(machine)) return;
  if (event.kind === "tabs") return setTabs(machine, event.tabs);
  if (event.kind === "usage") {
    machines.get(machine).usage = event.usage;
    return renderUsage();
  }
  if (event.kind !== "exit") return;
  const tab = tabByKey(keyOf(machine, event.tab));
  if (!tab) return;
  tab.view = { ...tab.view, running: false };
  tab.term.write("\r\n\x1b[2m[claude exited, press Enter to restart]\x1b[0m\r\n");
}
listen("machine", (event) => onMachineEvent(LOCAL, event));

const usageMeter = document.getElementById("usage");
const LIMITS = [
  ["five_hour", "5h", "5-hour limit"],
  ["seven_day", "7d", "7-day limit"],
];

function resetsText(resetsAt) {
  const date = new Date(resetsAt * 1000);
  const time = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (date.toDateString() === new Date().toDateString()) return `resets ${time}`;
  return `resets ${date.toLocaleDateString([], { weekday: "short" })} ${time}`;
}

// The meter shows the subscription of the active tab's machine, which may be signed into another one.
// A window's figure stops holding once it resets, so it leaves the meter then.
function renderUsage() {
  const usage = machines.get(active?.machine ?? LOCAL)?.usage ?? {};
  const now = Date.now() / 1000;
  const shown = LIMITS.filter(([key]) => usage[key]?.resets_at > now);
  usageMeter.hidden = !shown.length;
  usageMeter.replaceChildren(
    ...shown.map(([key, label]) => {
      const used = usage[key].used_percentage;
      const limit = document.createElement("span");
      limit.className = "limit";
      limit.dataset.level = used >= 90 ? "critical" : used >= 70 ? "high" : "ok";
      limit.innerHTML = `<span class="text"></span><span class="bar"><span class="fill"></span></span>`;
      limit.querySelector(".text").textContent = `${label} ${Math.round(used)}%`;
      limit.querySelector(".fill").style.width = `${Math.min(100, used)}%`;
      return limit;
    }),
  );
  usageMeter.title = shown
    .map(([key, , name]) => `${name}: ${+usage[key].used_percentage.toFixed(1)}% used, ${resetsText(usage[key].resets_at)}`)
    .join("\n");
}
setInterval(renderUsage, 60 * 1000);

async function setView(view) {
  document.body.classList.remove("alert");
  await invoke("set_view", { view });
  document.body.dataset.view = view;
}
document.getElementById("new-tab").onclick = () => toggleProjects();
document.getElementById("minimize").onclick = () => setView("pill");
track.onclick = async (e) => {
  const tab = tabByKey(e.target.closest(".item")?.dataset.tab);
  if (!tab) return;
  await setView("expanded");
  activate(tab);
};
new ResizeObserver(renderCarousel).observe(pill);
stillMotion.addEventListener("change", renderCarousel);
const resizeFrom = (direction) => (e) => {
  e.preventDefault();
  win.startResizeDragging(direction);
};
document.getElementById("pill-edge").onmousedown = resizeFrom("East");
document.getElementById("pill-corner").onmousedown = resizeFrom("SouthEast");
document.getElementById("grip").onmousedown = resizeFrom("SouthEast");

// Only this Mac has a folder dialog; a remote tab picks its folder from the machine's projects or a typed path.
document.getElementById("folder").onclick = async () => {
  const tab = active;
  if (!tab) return;
  if (tab.machine !== LOCAL) return pickerOpen() ? closeProjects() : showProjects({ machine: tab.machine, tab });
  const path = await invoke("pick_folder", { start: tab.view.cwd });
  if (path) await moveTab(tab, path);
};

// Phone keyboards lack these keys, so a row of them sits above the terminal on touch screens.
for (const key of document.querySelectorAll("#keys button")) {
  key.onpointerdown = (e) => e.preventDefault();
  key.onclick = () => active && typeInto(active, JSON.parse(`"${key.dataset.send}"`));
}

async function checkUpdate() {
  try {
    const version = await invoke("check_update");
    if (version) document.body.dataset.update = version;
    else delete document.body.dataset.update;
    for (const b of document.querySelectorAll(".update")) {
      b.disabled = false;
      b.textContent = b.closest("#pill") ? "↑" : `Update to v${version}`;
    }
    return version;
  } catch (e) {
    console.warn("update check failed", e);
  }
}
async function installUpdate() {
  for (const b of document.querySelectorAll(".update")) {
    b.disabled = true;
    b.textContent = "updating…";
  }
  try {
    await invoke("install_update");
  } catch (e) {
    active?.term.write(`\r\n\x1b[31m[update failed: ${e}]\x1b[0m\r\n`);
    checkUpdate();
  }
}
for (const button of document.querySelectorAll(".update")) button.onclick = installUpdate;
if (TAURI) {
  // Launch installs straight away, before a session has anything to lose; later checks only offer the button.
  checkUpdate().then((version) => version && installUpdate());
  setInterval(checkUpdate, 6 * 60 * 60 * 1000);
}

new ResizeObserver(() => refit(active)).observe(terms);

/** Connects to the hub as `signedIn` and shows its machines next to this one. */
function connectHub(signedIn) {
  account = signedIn;
  hub = new Hub(signedIn.hub, signedIn.user_token, {
    hello(frame) {
      account.email = frame.account.email;
      showMachines(frame.machines, frame.tabs);
      hub.hub({ cmd: "get_settings" }).then(onHubSettings, console.warn);
    },
    machines: (list) => showMachines(list),
    // This Mac's own events come over Tauri, not back through the hub.
    event: (machine, event) => machine !== account.machine_id && onMachineEvent(machine, event),
    settings: (latest) => onHubSettings(latest),
    online(on) {
      document.body.dataset.hub = on ? "online" : "offline";
      if (settingsOpen() && page === "account") showAccount();
    },
    refused: (reason) => (TAURI ? (document.body.dataset.hub = "refused") : signOutWeb(reason)),
  });
}

/** Mirrors the account's machines; `tabsByMachine` comes with the hub's hello. */
function showMachines(list, tabsByMachine) {
  for (const view of list) {
    if (view.id === account.machine_id) {
      machines.get(LOCAL).hubView = view;
      continue;
    }
    const known = machines.get(view.id);
    const wasOnline = known?.view.online;
    if (known) known.view = view;
    else machines.set(view.id, { view, tabs: [], transport: hub.transport(view.id), usage: {} });
    if (!view.online) setTabs(view.id, [], { forget: false });
    else if (tabsByMachine) setTabs(view.id, tabsByMachine[view.id] ?? []);
    else if (!wasOnline) refreshTabs(view.id).catch(console.warn);
  }
  for (const id of [...machines.keys()]) {
    if (id === LOCAL || list.some((v) => v.id === id)) continue;
    setTabs(id, [], { forget: false });
    machines.delete(id);
  }
  render();
  if (settingsOpen() && page === "account") showAccount();
  // Redrawing would take the name being typed away.
  if (settingsOpen() && page === "machines" && !machinesUi.renaming) showMachinesPage();
}

/** Forgets the hub: its machines and their tabs leave this UI. */
function disconnectHub() {
  hub?.close();
  hub = null;
  account = null;
  delete document.body.dataset.hub;
  for (const id of [...machines.keys()]) {
    if (id === LOCAL) continue;
    setTabs(id, [], { forget: false });
    machines.delete(id);
  }
  delete machines.get(LOCAL)?.hubView;
  render();
}

async function bootApp() {
  machines.set(LOCAL, {
    view: { id: LOCAL, name: "This Mac", os: "macos", online: true, last_seen: 0 },
    tabs: [],
    transport: localTransport,
    usage: {},
  });
  invoke("get_usage").then((usage) => onMachineEvent(LOCAL, { kind: "usage", usage }));
  // Sessions still going when pinned quit come back resumed; a reloaded webview finds its tabs already open.
  const restored = await invoke("restore_tabs");
  await refreshTabs(LOCAL);
  for (const id of restored) await startTab(tabByKey(keyOf(LOCAL, id)));
  if (!machines.get(LOCAL).tabs.length) await openTab(LOCAL, null);
  // Before the app can sign in, it has no account command.
  const signedIn = await invoke("account").catch(() => null);
  if (signedIn?.signed_in) connectHub(signedIn);
}
