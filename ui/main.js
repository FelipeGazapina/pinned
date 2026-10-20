const { invoke, Channel } = window.__TAURI__.core;
const win = window.__TAURI__.window.getCurrentWindow();

const applyOpacity = (percent) => document.documentElement.style.setProperty("--opacity", percent / 100);
invoke("get_opacity").then(applyOpacity);
win.listen("opacity", ({ payload }) => applyOpacity(payload));

const STATUS_TEXT = { idle: "", working: "working…", done: "done", "needs-input": "needs you" };
// The pill reports the first tab found in this order.
const URGENCY = ["needs-input", "done", "working", "idle"];
const EDGES = ["top", "bottom", "left", "right"];

const workspace = document.getElementById("workspace");
const tabBar = document.getElementById("tab-bar");
const tabList = document.getElementById("tab-list");
const terms = document.getElementById("terms");

/** Tabs in display order. Each: { id, term, fit, el, button, cwd, status, message, running }. */
const tabs = [];
let active = null;

const tabName = (tab) => tab.cwd.split("/").filter(Boolean).pop() || (tab.cwd ? "/" : "claude");
const isVertical = () => ["left", "right"].includes(document.body.dataset.tabs);

async function openTab(from) {
  return addTab(await invoke("open_tab", { from: from?.id ?? null }));
}

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

  const tab = { id, term, fit, el, button, cwd: "", status: "idle", message: null, running: false };
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
    tab.button.querySelector(".name").textContent = tabName(tab);
    tab.button.title = tab.cwd;
  }
  if (!active) return;
  const header = document.querySelector("header");
  header.dataset.status = active.status;
  document.getElementById("title").textContent = active.cwd.replace(/^\/Users\/[^/]+/, "~") || "claude";
  document.getElementById("status-text").textContent = STATUS_TEXT[active.status];

  const urgent = pillTab();
  const text = urgent.message || STATUS_TEXT[urgent.status];
  document.getElementById("pill").dataset.status = urgent.status;
  document.getElementById("pill-text").textContent =
    tabs.length > 1 && text ? `${tabName(urgent)} · ${text}` : text || tabName(urgent);
}

const pillTab = () => URGENCY.map((s) => tabs.find((t) => t.status === s)).find(Boolean);

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

// The panel is nonactivating, so a click alone doesn't give it keyboard focus.
window.addEventListener("mousedown", () => invoke("focus_panel").then(() => settingsOpen() || active?.term.focus()), true);
invoke("focus_panel");

const IS_MAC = /Mac/.test(navigator.platform);
const TAB_STEP = { ArrowLeft: -1, ArrowRight: 1 };

/** Moves to the neighbouring tab, wrapping around at either end. */
function stepTab(step) {
  const i = tabs.indexOf(active);
  if (tabs.length > 1) activate(tabs[(i + step + tabs.length) % tabs.length]);
}

// Caught before xterm, which would otherwise swallow them.
window.addEventListener("keydown", (e) => {
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
  else if (e.key === "t") openTab(active);
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
  setStatus(tab, payload.status, payload.status === "needs-input" ? payload.message : null);
  if (payload.status !== "working" && document.body.dataset.view === "pill") {
    document.body.classList.remove("alert");
    void document.body.offsetWidth;
    document.body.classList.add("alert");
  }
});

async function setView(view) {
  document.body.classList.remove("alert");
  await invoke("set_view", { view });
  document.body.dataset.view = view;
}
document.getElementById("new-tab").onclick = () => openTab(active);
document.getElementById("minimize").onclick = () => setView("pill");
// Opening from the pill lands on the tab it was reporting.
document.getElementById("expand").onclick = async () => {
  const tab = pillTab();
  await setView("expanded");
  activate(tab);
};
document.getElementById("grip").onmousedown = (e) => {
  e.preventDefault();
  win.startResizeDragging("SouthEast");
};

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

openTab(null);
