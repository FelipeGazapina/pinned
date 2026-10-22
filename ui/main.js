const { invoke, Channel } = window.__TAURI__.core;
const win = window.__TAURI__.window.getCurrentWindow();

const applyOpacity = (percent) => document.documentElement.style.setProperty("--opacity", percent / 100);
invoke("get_opacity").then(applyOpacity);
win.listen("opacity", ({ payload }) => applyOpacity(payload));

const STATUS_TEXT = { idle: "", working: "working…", done: "done", "needs-input": "needs you", failed: "stopped" };
const EDGES = ["top", "bottom", "left", "right"];

const workspace = document.getElementById("workspace");
const tabBar = document.getElementById("tab-bar");
const tabList = document.getElementById("tab-list");
const terms = document.getElementById("terms");
const picker = document.getElementById("projects");

/** Tabs in display order. Each: { id, term, fit, el, button, cwd, status, message, title, running }. */
const tabs = [];
let active = null;

const tilde = (path) => path.replace(/^\/Users\/[^/]+/, "~");
const tabName = (tab) => tab.cwd.split("/").filter(Boolean).pop() || (tab.cwd ? "/" : "claude");
const isVertical = () => ["left", "right"].includes(document.body.dataset.tabs);

/** Opens a tab on the project at `path`, or on the launch folder when `path` is null. */
async function openTab(path, bypass = false) {
  return addTab(await invoke("open_tab", { path, bypass }));
}

/** The picker's entries in display order, and which one Enter opens. */
let choices = [];
let chosen = 0;
/** Whether the next tab opened from the picker skips every permission check. Off each time the picker opens. */
let bypassNext = false;
const pickerOpen = () => !picker.hidden;

function setBypassNext(on) {
  bypassNext = on;
  picker.dataset.bypass = on;
  picker.querySelector(".bypass").textContent = on ? "⚡ Skip all permissions: on" : "⚡ Skip all permissions: off";
}

async function showProjects() {
  const projects = await invoke("list_projects");
  choices = [
    ...projects.map((p) => ({ name: p.name, path: tilde(p.path), open: () => openTab(p.path, bypassNext) })),
    {
      name: "Open folder…",
      open: async () => {
        const path = await invoke("pick_project");
        if (path) openTab(path, bypassNext);
      },
    },
  ];
  const toggle = document.createElement("button");
  toggle.className = "bypass";
  toggle.title = "Opens the next tab with --dangerously-skip-permissions (⇧Enter)";
  toggle.onclick = () => setBypassNext(!bypassNext);
  picker.replaceChildren(
    toggle,
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
  delete document.body.dataset.page;
  picker.hidden = false;
  setBypassNext(false);
  choose(0);
}

function choose(i) {
  chosen = (i + choices.length) % choices.length;
  const entries = picker.querySelectorAll("button:not(.bypass)");
  entries.forEach((b, j) => b.classList.toggle("chosen", j === chosen));
  entries[chosen].scrollIntoView({ block: "nearest" });
}

function openChoice(i, bypass = false) {
  if (bypass) bypassNext = true;
  closeProjects();
  choices[i].open();
}

function closeProjects() {
  picker.hidden = true;
  active?.term.focus();
}

const toggleProjects = () => (pickerOpen() ? closeProjects() : showProjects());

/** Shows the tab `id` that the backend created. `focus: false` leaves the active tab in front. */
async function addTab(id, { focus = true } = {}) {
  const el = document.createElement("div");
  el.className = "term";
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
  button.innerHTML = `<span class="dot"></span><span class="name"></span><button class="close" title="Close tab (⌘W)">✕</button><span class="resize" title="Drag to resize the tabs"></span>`;
  tabList.append(button);

  const bypass = await invoke("tab_bypass", { tab: id });
  const tab = { id, term, fit, el, button, bypass, cwd: "", status: "idle", message: null, title: null, running: false };
  tabs.push(tab);
  term.onData((data) => {
    if (tab.running) return invoke("write_input", { tab: id, data });
    if (data === "\r") startClaude(tab);
  });
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

  if (focus || !active) activate(tab);
  else el.hidden = true;
  await startClaude(tab);
  return tab;
}

async function startClaude(tab) {
  const output = new Channel();
  output.onmessage = (bytes) => tab.term.write(new Uint8Array(bytes));
  tab.cwd = await invoke("start_claude", { tab: tab.id, cols: tab.term.cols, rows: tab.term.rows, output });
  tab.title = null;
  tab.running = true;
  setStatus(tab, "idle");
}

function closeTab(tab) {
  const i = tabs.indexOf(tab);
  if (i < 0) return;
  tabs.splice(i, 1);
  tab.term.dispose();
  tab.el.remove();
  tab.button.remove();
  if (active === tab && tabs.length) activate(tabs[Math.min(i, tabs.length - 1)]);
  render();
  invoke("close_tab", { tab: tab.id });
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
  if (!tab || document.body.dataset.view !== "expanded") return;
  tab.fit.fit();
  if (tab.running) invoke("resize_pty", { tab: tab.id, cols: tab.term.cols, rows: tab.term.rows });
}

function setStatus(tab, status, message) {
  tab.status = status;
  tab.message = message;
  render();
}

function render() {
  for (const tab of tabs) {
    tab.button.dataset.status = tab.status;
    tab.button.querySelector(".name").textContent = (tab.bypass ? "⚡ " : "") + tabName(tab);
    tab.button.title = tab.bypass ? `${tab.cwd}\nPermissions skipped` : tab.cwd;
  }
  renderCarousel();
  if (!active) return;
  const header = document.querySelector("header");
  header.dataset.status = active.status;
  document.getElementById("title").textContent = tilde(active.cwd) || "claude";
  document.getElementById("status-text").textContent =
    active.status === "failed" && active.resumeAt
      ? `limit, continues ${new Date(active.resumeAt * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
      : STATUS_TEXT[active.status];
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
  const { width, height } = carousel.getBoundingClientRect();
  if (!width) return;
  const mode = pill.getBoundingClientRect().height >= CARDS_FROM_HEIGHT ? "cards" : "compact";
  const key = JSON.stringify([mode, Math.round(width), stillMotion.matches, tabs.map((t) => [t.id, tabName(t)])]);
  if (key !== builtFor) {
    builtFor = key;
    pill.dataset.mode = mode;
    buildTrack(width);
  }
  for (const item of track.querySelectorAll(".item")) {
    const tab = tabs.find((t) => t.id === item.dataset.tab);
    item.dataset.status = tab.status;
    item.querySelector(".status").textContent = tab.message || STATUS_TEXT[tab.status] || "idle";
    item.querySelector(".title").textContent = tab.title || tilde(tab.cwd);
    item.title = tab.title ? `${tab.cwd}\n${tab.title}` : tab.cwd;
  }
}

function buildTrack(width) {
  if (!tabs.length) return track.replaceChildren();
  const set = document.createElement("div");
  set.className = "set";
  for (const tab of tabs) {
    const item = document.createElement("button");
    item.className = "item";
    item.dataset.tab = tab.id;
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
}

function nearestEdge(e) {
  const r = workspace.getBoundingClientRect();
  const distance = { top: e.clientY - r.top, bottom: r.bottom - e.clientY, left: e.clientX - r.left, right: r.right - e.clientX };
  return EDGES.reduce((a, b) => (distance[b] < distance[a] ? b : a));
}

function load(key) {
  try {
    return localStorage.getItem(key);
  } catch {}
}
function save(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch {}
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

// ＋ toggles the picker itself, so a click on it doesn't count as outside.
window.addEventListener("mousedown", (e) => {
  if (pickerOpen() && !e.target.closest("#projects, #new-tab")) closeProjects();
}, true);

// The panel is nonactivating, so a click alone doesn't give it keyboard focus.
window.addEventListener("mousedown", () => invoke("focus_panel").then(() => settingsOpen() || active?.term.focus()), true);
invoke("focus_panel");

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
  // Keys other than ⌘ shortcuts belong to the open picker, not the terminal behind it.
  if (pickerOpen() && !e.metaKey) {
    if (e.key in PICKER_STEP) choose(chosen + PICKER_STEP[e.key]);
    else if (e.key === "Enter") openChoice(chosen, e.shiftKey);
    else if (e.key === "b" && !e.ctrlKey && !e.altKey) setBypassNext(!bypassNext);
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
window.__TAURI__.webview.getCurrentWebview().onDragDropEvent(({ payload }) => {
  if (payload.type !== "drop" || !active?.running) return;
  const escaped = payload.paths.map((p) => p.replace(/[^\w@%+=:,./-]/g, "\\$&"));
  active.term.paste(escaped.join(" ") + " ");
  active.term.focus();
});

// A clipboard image has no text for xterm to paste; Ctrl+V makes Claude Code read the image itself.
document.addEventListener("paste", (e) => {
  const types = [...(e.clipboardData?.types ?? [])];
  if (!active?.running || types.includes("text/plain") || !types.includes("Files")) return;
  e.preventDefault();
  e.stopPropagation();
  invoke("write_input", { tab: active.id, data: "\x16" });
}, true);

win.listen("claude-exit", ({ payload }) => {
  const tab = tabs.find((t) => t.id === payload);
  if (!tab) return;
  tab.running = false;
  tab.term.write("\r\n\x1b[2m[claude exited, press Enter to restart]\x1b[0m\r\n");
});

// A connector started a session. It opens in the background so typing in the current tab isn't hijacked.
win.listen("tab-opened", async ({ payload }) => {
  const tab = await addTab(payload, { focus: false });
  setStatus(tab, "working");
});

win.listen("agent-event", ({ payload }) => {
  const tab = tabs.find((t) => t.id === payload.tab);
  if (!tab) return;
  tab.title = payload.title ?? tab.title;
  setStatus(tab, payload.status, payload.status === "needs-input" ? payload.message : null);
  tab.resumeAt = payload.status === "failed" ? payload.resume_at : null;
  if (payload.status !== "working" && document.body.dataset.view === "pill") {
    pill.dataset.status = payload.status;
    document.body.classList.remove("alert");
    void document.body.offsetWidth;
    document.body.classList.add("alert");
  }
});

const usageMeter = document.getElementById("usage");
const LIMITS = [
  ["five_hour", "5h", "5-hour limit"],
  ["seven_day", "7d", "7-day limit"],
];
/** The subscription usage Claude Code last reported, keyed like LIMITS. Each window: { used_percentage, resets_at }. */
let usage = {};

function resetsText(resetsAt) {
  const date = new Date(resetsAt * 1000);
  const time = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (date.toDateString() === new Date().toDateString()) return `resets ${time}`;
  return `resets ${date.toLocaleDateString([], { weekday: "short" })} ${time}`;
}

// A window's figure stops holding once it resets, so it leaves the meter then.
function renderUsage() {
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

function showUsage(latest) {
  usage = latest;
  renderUsage();
}
invoke("get_usage").then(showUsage);
win.listen("usage", ({ payload }) => showUsage(payload));
setInterval(renderUsage, 60 * 1000);

async function setView(view) {
  document.body.classList.remove("alert");
  await invoke("set_view", { view });
  document.body.dataset.view = view;
}
document.getElementById("new-tab").onclick = toggleProjects;
document.getElementById("minimize").onclick = () => setView("pill");
track.onclick = async (e) => {
  const tab = tabs.find((t) => t.id === e.target.closest(".item")?.dataset.tab);
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

document.getElementById("folder").onclick = async () => {
  const tab = active;
  if (await invoke("choose_folder", { tab: tab.id })) {
    tab.term.reset();
    await startClaude(tab);
    tab.term.focus();
  }
};

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
// Launch installs straight away, before a session has anything to lose; later checks only offer the button.
checkUpdate().then((version) => version && installUpdate());
setInterval(checkUpdate, 6 * 60 * 60 * 1000);

new ResizeObserver(() => refit(active)).observe(terms);

// Sessions still going when pinned quit come back resumed; with none, start fresh.
invoke("restore_tabs").then(async (ids) => {
  if (!ids.length) return openTab(null);
  for (const id of ids) await addTab(id, { focus: id === ids[0] });
});
