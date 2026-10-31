const win = TAURI?.window.getCurrentWindow();

const applyOpacity = (percent) => document.documentElement.style.setProperty("--opacity", percent / 100);
if (TAURI) {
  invoke("get_opacity").then(applyOpacity);
  listen("opacity", applyOpacity);
}

const STATUS_TEXT = { idle: "", working: "working…", done: "done", "needs-input": "needs you", failed: "stopped" };
const ALERTS = ["done", "needs-input", "failed"];
const EDGES = ["top", "bottom", "left", "right"];
const LOCAL = "local";
/** The account a tab runs under unless its project picks another: Claude Code's own ~/.claude login. */
const DEFAULT_ACCOUNT = "claude";

const workspace = document.getElementById("workspace");
const tabBar = document.getElementById("tab-bar");
const tabList = document.getElementById("tab-list");
const terms = document.getElementById("terms");
const picker = document.getElementById("projects");

const machines = new Map();
const viewListeners = [];
let account = null;
let hub = null;

const tabs = [];
let active = null;

const keyOf = (machine, id) => `${machine}:${id}`;
const tabByKey = (key) => tabs.find((t) => t.key === key);
const callOn = (machine, call) => machines.get(machine).transport.call(call);
const IS_MAC = /Mac/.test(navigator.platform);
const COMPUTER = IS_MAC ? "Mac" : "PC";
const statusOf = (tab) => tab.view.status ?? "idle";
const machineName = (id) => (id === LOCAL ? `This ${COMPUTER}` : machines.get(id)?.view.name ?? "machine");
const initials = (name) => name.split(/[\s._-]+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join("");
const tilde = (path) => path.replace(/^(\/Users\/[^/]+|\/home\/[^/]+|\/root)(?=\/|$)/, "~");
const accountOf = (machine, id) => machines.get(machine)?.accounts?.find((a) => a.id === id);
const accountName = (machine, id) => accountOf(machine, id)?.name ?? id;
/** A tab with no Claude Code transcript to show as a thread: a sign-in, or a provider that is not the harness. */
const HARNESS = new Set(["claude"]);
const terminalOnly = (tab) => {
  const provider = accountOf(tab.machine, tab.view.account)?.provider;
  return tab.view.program === "sign-in" || (provider != null && !HARNESS.has(provider));
};
const tabName = (tab) =>
  tab.view.program === "sign-in"
    ? `Sign in · ${accountName(tab.machine, tab.view.account)}`
    : tab.view.cwd.split(/[\\/]/).filter(Boolean).pop() || (tab.view.cwd ? "/" : "claude");
const isVertical = () => ["left", "right"].includes(document.body.dataset.tabs);

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

// Tabs closed here whose close the machine hasn't answered: a tabs event sent before it still lists them.
const closing = new Set();

function setTabs(machine, views, { forget = true } = {}) {
  const m = machines.get(machine);
  if (!m) return;
  views = views.filter((v) => !closing.has(keyOf(machine, v.id)));
  m.tabs = views;
  const keep = new Set(views.map((v) => keyOf(machine, v.id)));
  for (const tab of tabs.filter((t) => t.machine === machine && !keep.has(t.key))) removeTab(tab, forget);
  for (const view of views) {
    const tab = tabByKey(keyOf(machine, view.id));
    if (tab) updateView(tab, view);
    else addTab(machine, view);
  }
  placeTabs();
  const paths = views.map((v) => v.cwd).sort().join("\n");
  if (m.tabPaths !== paths) {
    m.tabPaths = paths;
    refreshProjects(machine);
  }
  render();
}

const refreshTabs = async (machine) => setTabs(machine, await callOn(machine, { cmd: "tabs" }));

/** Shows a tab the machine just opened, once its tab list has it. */
async function revealTab(machine, id) {
  await refreshTabs(machine);
  const tab = tabByKey(keyOf(machine, id));
  activate(tab);
  return tab;
}

/** The machine's accounts, asked once and kept for the tab badges and pickers; `fresh` asks again. Each ask runs every account's CLI. */
function accountsOf(machine, fresh = false) {
  const m = machines.get(machine);
  if (fresh || !m.accountsAsked) {
    // A list asked before a later ask or an add would undo it, so only the newest is kept.
    const asked = callOn(machine, { cmd: "list_accounts" }).then((list) => (m.accountsAsked === asked ? setAccounts(machine, list) : m.accounts ?? list));
    asked.catch(() => setTimeout(() => m.accountsAsked === asked && (m.accountsAsked = null), 30 * 1000));
    m.accountsAsked = asked;
  }
  return m.accountsAsked;
}

function setAccounts(machine, list) {
  const m = machines.get(machine);
  if (!m) return list;
  m.accounts = list;
  m.accountsAsked = Promise.resolve(list);
  render();
  return list;
}

/** Opens a tab running the CLI's login for the account and shows it. */
async function signIn(machine, account) {
  const id = await callOn(machine, { cmd: "sign_in", account });
  await revealTab(machine, id);
  if (guiOn()) setMode("terminal");
}

function updateView(tab, view) {
  const before = statusOf(tab);
  const key = threadKey(tab);
  tab.view = view;
  if (threadKey(tab) !== key) rekeyThread(key, threadKey(tab));
  const after = statusOf(tab);
  if (after === before) return;
  if (!TAURI) return attend(tab, ALERTS.includes(after));
  if (ALERTS.includes(after) && document.body.dataset.view === "pill") {
    pill.dataset.status = after;
    document.body.classList.remove("alert");
    void document.body.offsetWidth;
    document.body.classList.add("alert");
  }
}

/** In the browser, which has no pill to flash: the tabs that finished or need input since they were last in view. */
const unseen = new Set();
const PAGE_TITLE = document.title;
const inView = (tab) => !document.hidden && tab === active && !settingsOpen() && !pickerOpen();

function attend(tab, alert) {
  if (alert && !inView(tab)) {
    unseen.add(tab.key);
    notify(tab);
  } else unseen.delete(tab.key);
  renderUnseen();
}

function markSeen(tab) {
  if (tab && inView(tab) && unseen.delete(tab.key)) renderUnseen();
}

function renderUnseen() {
  document.title = unseen.size ? `(${unseen.size}) ${PAGE_TITLE}` : PAGE_TITLE;
  const badge = unseen.size ? navigator.setAppBadge?.(unseen.size) : navigator.clearAppBadge?.();
  badge?.catch(() => {});
}

// Android Chrome only shows a notification through the service worker, which also takes the click.
async function notify(tab) {
  if (window.Notification?.permission !== "granted") return;
  const registration = await navigator.serviceWorker?.getRegistration();
  registration?.showNotification(`${tabName(tab)} on ${machineName(tab.machine)}: ${STATUS_TEXT[statusOf(tab)]}`, {
    body: tab.view.message || tab.view.title || tilde(tab.view.cwd),
    tag: tab.key,
    renotify: true,
    data: { tab: tab.key },
  });
}

navigator.serviceWorker?.addEventListener("message", ({ data }) => {
  const tab = data?.type === "activate-tab" && tabByKey(data.tab);
  if (tab) activate(tab);
});
document.addEventListener("visibilitychange", () => markSeen(active));

async function openTab(machine, path, bypass = false) {
  const tab = await revealTab(machine, await callOn(machine, { cmd: "open_tab", path, bypass }));
  await startTab(tab);
  return tab;
}

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
  picker.querySelector(".bypass").textContent = on ? "⚠ Skip all permissions: on" : "⚠ Skip all permissions: off";
}

async function showProjects({ machine, tab = null } = {}) {
  machine ??= machines.get(picking.machine)?.view.online ? picking.machine : onlineMachines()[0];
  delete document.body.dataset.page;
  picker.hidden = false;
  choices = [];
  if (!machine) {
    picker.dataset.mode = "open";
    picker.innerHTML = `<p class="loading">No machine is online. Open pinned on a Mac signed into this account, or add a server under Settings → Machines.</p>`;
    return;
  }
  picking = { machine, tab };
  picker.dataset.mode = tab ? "move" : "open";
  picker.innerHTML = `<button class="bypass" title="Opens the next tab with --dangerously-skip-permissions (⇧Enter)"></button>
    ${pickerMachinesHtml(machine, !!tab)}<div class="choices"><p class="loading">Loading projects…</p></div>`;
  picker.querySelector(".bypass").onclick = () => setBypassNext(!bypassNext);
  for (const b of picker.querySelectorAll(".machine")) b.onclick = () => showProjects({ machine: b.dataset.machine, tab });
  setBypassNext(false);
  let projects = [];
  let shared = [];
  try {
    [projects, shared] = await Promise.all([callOn(machine, { cmd: "list_projects" }), hub ? hub.hub({ cmd: "shared_projects" }).catch(() => []) : []]);
  } catch (e) {
    console.warn("list_projects", e);
  }
  const here = new Set(projects.map((p) => p.repo).filter(Boolean));
  if (picking.machine !== machine || !pickerOpen()) return;
  const local = machine === LOCAL;
  choices = [
    ...projects.map((p) => ({ name: p.name, path: tilde(p.path), open: () => pickFolder(p.path) })),
    // Projects of the account another machine has, cloned here when chosen.
    ...shared.filter((p) => !here.has(p.key)).map((p) => ({ name: p.name, path: `clone · ${p.key}`, open: () => cloneAndOpen(machine, p.repo), keep: true })),
    { name: "Clone from GitHub…", open: () => cloneFromGithub(), keep: true },
    local
      ? { name: "Open folder…", open: () => pickLocalFolder() }
      : { name: "Browse folders…", open: () => browseFolders(), keep: true },
  ];
  picker.querySelector(".choices").replaceChildren(...choiceButtons());
  picker.querySelector(".choices").insertAdjacentHTML("beforeend", `<p class="error" id="picker-error"></p>`);
  choose(0);
}

function choiceButtons() {
  return choices.map((choice, i) => {
    const button = document.createElement("button");
    for (const [cls, text] of [["name", choice.name], ["path", choice.path]]) {
      if (!text) continue;
      const span = document.createElement("span");
      span.className = cls;
      span.textContent = text;
      button.append(span);
    }
    button.onmousemove = () => chosen !== i && choose(i);
    button.onclick = (e) => openChoice(i, e.shiftKey);
    return button;
  });
}

async function cloneFromGithub() {
  picker.querySelector(".choices").innerHTML = `<form class="type-path browse"><input placeholder="Filter your repositories" autocomplete="off" spellcheck="false"></form>
    <div class="folders"><p class="loading">Loading your repositories…</p></div><p class="error" id="picker-error"></p>`;
  const field = picker.querySelector(".browse input");
  const list = picker.querySelector(".folders");
  field.focus();
  picker.querySelector(".browse").onsubmit = (e) => {
    e.preventDefault();
    if (choices[chosen]) openChoice(chosen);
  };
  const { machine } = picking;
  let repos;
  try {
    repos = await connectorCall("github_repos");
  } catch (e) {
    list.innerHTML = "";
    document.getElementById("picker-error").textContent = /not connected/.test(e) ? "Connect GitHub under Settings → Connectors first." : String(e);
    return;
  }
  const show = () => {
    const typed = field.value.trim().toLowerCase();
    const found = repos.filter((r) => r.full_name.toLowerCase().includes(typed)).slice(0, 100);
    choices = found.map((r) => ({ name: r.full_name, path: r.private ? "private" : "", open: () => cloneAndOpen(machine, r.full_name), keep: true }));
    if (!found.length) return (list.innerHTML = `<p class="loading">No repository matches</p>`);
    list.replaceChildren(...choiceButtons());
    choose(0);
  };
  field.oninput = show;
  show();
}

async function cloneAndOpen(machine, repo) {
  choices = [];
  picker.querySelector(".choices").innerHTML = `<p class="loading">Cloning ${esc(repo)} on ${esc(machineName(machine))}…</p><p class="error" id="picker-error"></p>`;
  try {
    const path = hub ? await hub.hub({ cmd: "clone", machine: hubIdOf(machine), repo }) : await invoke("clone_repo", { repo });
    if (await pickFolder(path)) closeProjects();
  } catch (e) {
    document.getElementById("picker-error").textContent = String(e);
  }
}

/** Counts browse requests, so an answer to an older keystroke can't replace a newer one. */
let browseSeq = 0;

// Like T3 Code's: the machine lists the folder typed so far, and choosing a folder steps into it.
function browseFolders(start = "~/") {
  picker.querySelector(".choices").innerHTML = `<form class="type-path browse"><input placeholder="~/" autocomplete="off" spellcheck="false"></form>
    <div class="folders"></div><p class="error" id="picker-error"></p>`;
  const field = picker.querySelector(".browse input");
  field.value = start;
  field.oninput = () => listFolders(field.value);
  picker.querySelector(".browse").onsubmit = (e) => {
    e.preventDefault();
    openChoice(chosen, false);
  };
  field.focus();
  listFolders(start);
}

async function listFolders(typed) {
  const { machine } = picking;
  const seq = ++browseSeq;
  const error = document.getElementById("picker-error");
  let found;
  try {
    found = await callOn(machine, { cmd: "browse", path: typed });
  } catch (e) {
    if (seq === browseSeq) error.textContent = String(e);
    return;
  }
  if (seq !== browseSeq || picking.machine !== machine || !pickerOpen()) return;
  error.textContent = "";
  const field = picker.querySelector(".browse input");
  const into = (path) => () => {
    field.value = path.endsWith("/") ? path : `${path}/`;
    field.focus();
    listFolders(field.value);
  };
  const up = found.parent.replace(/\/[^/]*$/, "") || "/";
  choices = [
    { name: `Open ${found.parent}`, open: () => pickFolder(found.parent).then((ok) => ok && closeProjects()), keep: true },
    ...(found.parent === "/" ? [] : [{ name: "..", path: up, open: into(up), keep: true }]),
    ...found.entries.map((f) => ({ name: f.name, path: f.repo ? "git" : "", open: into(f.path), keep: true })),
  ];
  picker.querySelector(".folders").replaceChildren(...choiceButtons());
  const prefix = typed.slice(typed.lastIndexOf("/") + 1);
  const firstFolder = choices.length - found.entries.length;
  choose(prefix && found.entries.length ? firstFolder : 0);
}

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
  if (!choices[i]) return;
  if (bypass) bypassNext = true;
  if (!choices[i].keep) closeProjects();
  choices[i].open();
}

function closeProjects() {
  picker.hidden = true;
  focusInput();
  markSeen(active);
}

const toggleProjects = () => (pickerOpen() ? closeProjects() : showProjects());

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
  const tab = await revealTab(machine, id);
  await startTab(tab);
  return tab;
}

async function moveTab(tab, path) {
  await callOn(tab.machine, { cmd: "set_folder", tab: tab.view.id, path });
  tab.term.reset();
  await startTab(tab);
  focusInput();
}

function addTab(machine, view) {
  const el = document.createElement("div");
  el.className = "term";
  el.hidden = true;
  terms.append(el);
  const term = new Terminal({
    fontFamily: "ui-monospace, Menlo, monospace",
    fontSize: 12,
    ...(typeof terminalLook === "function" ? terminalLook() : {}),
    cursorBlink: true,
    allowTransparency: true,
    theme: { background: "#00000000" },
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open(el);

  const button = document.createElement("div");
  button.className = "tab";
  button.innerHTML = `<span class="dot"></span><span class="badge" hidden></span><span class="badge account" hidden></span><span class="name"></span><button class="close" title="Close tab (⌘W)">✕</button><span class="resize" title="Drag to resize the tabs"></span>`;
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
  button.addEventListener("touchstart", (e) => holdToReorder(tab, e), { passive: true });
  button.oncontextmenu = (e) => e.preventDefault();
  if (!active) show(tab);
  return tab;
}

function typeInto(tab, data) {
  if (tab.view.running) return callOn(tab.machine, { cmd: "write_input", tab: tab.view.id, data });
  if (data === "\r") startTab(tab);
}

const starting = new Map();
/** `start_claude` replaces a running session, so starts of one tab that overlap share the first. */
function startTab(tab) {
  if (!starting.has(tab.key)) starting.set(tab.key, startClaude(tab).finally(() => starting.delete(tab.key)));
  return starting.get(tab.key);
}
async function startClaude(tab) {
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

async function setBypass(tab, on) {
  refit(tab);
  try {
    await callOn(tab.machine, { cmd: "set_bypass", tab: tab.view.id, on, cols: tab.term.cols, rows: tab.term.rows });
    await refreshTabs(tab.machine);
  } catch (e) {
    tab.term.write(`\r\n\x1b[31m[could not restart claude: ${e}]\x1b[0m\r\n`);
  }
  focusInput();
}

/** The other Claude Code accounts signed in on the tab's machine, which can take its conversation over. */
const switchTargets = (tab) =>
  tab.view.program === "sign-in" || terminalOnly(tab)
    ? []
    : (machines.get(tab.machine)?.accounts ?? []).filter((a) => a.provider === "claude" && a.signed_in && a.id !== tab.view.account);

/** Session menu items that move the tab's conversation to another account, to continue past a usage limit. */
const switchItems = (tab) =>
  switchTargets(tab).map((a) => ({
    label: tab.view.resume_at ? `Continue under ${a.name}` : `Move to ${a.name}`,
    hint: "Copies this conversation to the account and resumes it there",
    run: () => switchAccount(tab, a.id),
  }));

async function switchAccount(tab, account) {
  refit(tab);
  try {
    await callOn(tab.machine, { cmd: "switch_account", tab: tab.view.id, account, cols: tab.term.cols, rows: tab.term.rows });
    await refreshTabs(tab.machine);
  } catch (e) {
    tab.term.write(`\r\n\x1b[31m[could not switch accounts: ${e}]\x1b[0m\r\n`);
    if (guiOn()) showComposerNote(String(e), true);
  }
  focusInput();
}

function closeTab(tab) {
  rememberClosed(tab);
  removeTab(tab, true);
  render();
  closing.add(tab.key);
  // The app quits with its last tab while signed out, so the local close goes through its own command.
  const closed = tab.machine === LOCAL ? invoke("close_tab", { tab: tab.view.id }) : callOn(tab.machine, { cmd: "close_tab", tab: tab.view.id });
  closed.catch(console.warn).finally(() => closing.delete(tab.key));
}

function removeTab(tab, forget) {
  const i = tabs.indexOf(tab);
  if (i < 0) return;
  tabs.splice(i, 1);
  machines.get(tab.machine)?.transport.detach(tab.view.id, tab.sink);
  tab.term.dispose();
  tab.el.remove();
  tab.button.remove();
  if (forget) order = order.filter((k) => k !== tab.key);
  if (unseen.delete(tab.key)) renderUnseen();
  if (active === tab) {
    active = null;
    if (tabs.length) activate(tabs[Math.min(i, tabs.length - 1)]);
  }
}

function activate(tab) {
  delete document.body.dataset.page;
  picker.hidden = true;
  show(tab);
}

function show(tab) {
  if (tab !== active) showTermText(false);
  active = tab;
  for (const t of tabs) {
    t.el.hidden = t !== tab;
    t.button.classList.toggle("active", t === tab);
  }
  refit(tab);
  render();
  if (!pickerOpen() && !settingsOpen()) focusInput();
  markSeen(tab);
}

function focusInput() {
  if (!active) return;
  if (guiOn() && document.body.dataset.drawer !== "open") prompt.focus();
  else active.term.focus();
}

// The window's minimum height leaves no room for a terminal; a fit there would reflow the
// buffer into a few cells and claude would redraw at that size, so such a size is ignored.
const MIN_TERM_COLS = 20;
const MIN_TERM_ROWS = 4;
// A hidden terminal can't measure itself, so each tab fits when it is shown.
function refit(tab) {
  if (!tab || tab.el.hidden || document.body.dataset.view !== "expanded") return;
  const dims = tab.fit.proposeDimensions();
  if (!dims || dims.cols < MIN_TERM_COLS || dims.rows < MIN_TERM_ROWS) return;
  tab.fit.fit();
  if (tab.view.running) callOn(tab.machine, { cmd: "resize_pty", tab: tab.view.id, cols: tab.term.cols, rows: tab.term.rows }).catch(() => {});
}

function render() {
  for (const tab of tabs) {
    const remote = tab.machine !== LOCAL;
    const where = remote ? `${machineName(tab.machine)}: ` : "";
    tab.button.dataset.status = statusOf(tab);
    const name = tab.button.querySelector(".name");
    name.textContent = tabName(tab);
    if (tab.view.bypass) name.prepend(Object.assign(document.createElement("span"), { className: "warn", textContent: "⚠ " }));
    const badge = tab.button.querySelector(".badge");
    badge.hidden = !remote;
    badge.textContent = remote ? initials(machineName(tab.machine)) : "";
    const other = tab.view.account !== DEFAULT_ACCOUNT;
    if ((other || tab.view.resume_at || tab === active) && !machines.get(tab.machine).accountsAsked) accountsOf(tab.machine).catch(console.warn);
    const named = accountOf(tab.machine, tab.view.account);
    const accountBadge = tab.button.querySelector(".badge.account");
    accountBadge.hidden = !other;
    accountBadge.textContent = other ? accountName(tab.machine, tab.view.account) : "";
    accountBadge.title = named ? `${named.name} (${named.provider_name})` : tab.view.account;
    tab.button.title = `${where}${tab.view.cwd}${tab.view.bypass ? "\nPermissions skipped" : ""}`;
  }
  document.body.dataset.empty = !tabs.length;
  renderGui();
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
/** What the track was last built for; items are rebuilt only when this changes. */
let builtFor = "";

/** The pill lists the sidebar's active threads, leaving settled and snoozed ones out. */
const pillTabs = () => tabs.filter((t) => !["settled", "snoozed"].includes(sectionOf(t)));

/** Lays the active threads out in a row to scroll through. Status and title changes update the items in place so the scroll doesn't jump. */
function renderCarousel() {
  const { width } = carousel.getBoundingClientRect();
  if (!width) return;
  const mode = pill.getBoundingClientRect().height >= CARDS_FROM_HEIGHT ? "cards" : "compact";
  const shown = pillTabs();
  const key = JSON.stringify([mode, shown.map((t) => [t.key, tabName(t)])]);
  if (key !== builtFor) {
    builtFor = key;
    pill.dataset.mode = mode;
    buildTrack(shown);
  }
  for (const item of track.querySelectorAll(".item")) {
    const tab = tabByKey(item.dataset.tab);
    item.dataset.status = statusOf(tab);
    item.querySelector(".status").textContent = tab.view.message || STATUS_TEXT[statusOf(tab)] || "idle";
    item.querySelector(".title").textContent = tab.view.title || tilde(tab.view.cwd);
    item.title = tab.view.title ? `${tab.view.cwd}\n${tab.view.title}` : tab.view.cwd;
  }
}

function buildTrack(shown) {
  if (!shown.length) return track.replaceChildren(Object.assign(document.createElement("span"), { className: "none", textContent: "No active threads" }));
  track.replaceChildren(
    ...shown.map((tab) => {
      const item = document.createElement("button");
      item.className = "item";
      item.dataset.tab = tab.key;
      item.innerHTML = `<span class="dot"></span><span class="name"></span><span class="status"></span><span class="title"></span>`;
      item.querySelector(".name").textContent = tabName(tab);
      return item;
    }),
  );
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

// A swipe scrolls the tab row, so a finger first holds a tab still to pick it up for dragging.
const HOLD_MS = 400;
function holdToReorder(tab, start) {
  if (start.target.closest(".close") || start.touches.length > 1) return;
  const from = start.touches[0];
  let held = false;
  const timer = setTimeout(() => {
    held = true;
    tab.button.classList.add("held");
  }, HOLD_MS);
  const move = (e) => {
    const touch = e.touches[0];
    if (held) {
      e.preventDefault();
      reorder(tab, touch);
    } else if (Math.hypot(touch.clientX - from.clientX, touch.clientY - from.clientY) > 8) end();
  };
  const end = () => {
    clearTimeout(timer);
    tab.button.classList.remove("held");
    removeEventListener("touchmove", move);
    removeEventListener("touchend", end);
    removeEventListener("touchcancel", end);
  };
  addEventListener("touchmove", move, { passive: false });
  addEventListener("touchend", end);
  addEventListener("touchcancel", end);
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

window.addEventListener("mousedown", (e) => {
  if (pickerOpen() && !e.target.closest("#projects, #new-tab, #folder")) closeProjects();
}, true);

// The panel is nonactivating, so a click alone doesn't give it keyboard focus.
if (TAURI) {
  const typesIntoTerminal = (e) => !guiOn() || e.target.closest("#terms");
  window.addEventListener("mousedown", (e) => invoke("focus_panel").then(() => {
    if (!settingsOpen() && typesIntoTerminal(e)) focusInput();
  }), true);
  invoke("focus_panel");
}

const TAB_STEP = { ArrowLeft: -1, ArrowRight: 1 };
const PICKER_STEP = { ArrowUp: -1, ArrowDown: 1 };

/** Moves to the neighbouring tab, wrapping around at either end. */
function stepTab(step) {
  const i = tabs.indexOf(active);
  if (tabs.length > 1) activate(tabs[(i + step + tabs.length) % tabs.length]);
}

// Caught before xterm, which would otherwise swallow them.
window.addEventListener("keydown", (e) => {
  if (e.target.closest?.("[data-keybinding-capture]")) return;
  if (pickerOpen() && e.target.closest?.(".type-path")) {
    if (e.key === "Escape") showProjects(picking);
    else if (e.key in PICKER_STEP) choose(chosen + PICKER_STEP[e.key]);
    else if (e.key === "Tab" && chosen > 0) openChoice(chosen);
    else return;
    e.preventDefault();
    return;
  }
  // Keys other than ⌘ shortcuts belong to the open picker, not the terminal behind it.
  if (pickerOpen() && !e.metaKey) {
    if (e.key in PICKER_STEP) choose(chosen + PICKER_STEP[e.key]);
    else if (e.key in TAB_STEP) stepMachine(TAB_STEP[e.key]);
    else if (e.key === "Enter") openChoice(chosen, e.shiftKey);
    else if (e.key === "b" && !e.ctrlKey && !e.altKey && !picking.tab && picker.querySelector(".bypass")) setBypassNext(!bypassNext);
    else if (e.key === "Escape") closeProjects();
    e.preventDefault();
    e.stopPropagation();
    return;
  }
  if (e.shiftKey || e.altKey) return;
  const tabKey = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  // The app view's chords are gui.js's; there, as in settings, the arrows keep their text-editing meaning.
  if (tabKey && e.key in TAB_STEP && active && !settingsOpen() && !guiOn()) {
    stepTab(TAB_STEP[e.key]);
    e.preventDefault();
    e.stopPropagation();
    return;
  }
  if (!e.metaKey || e.ctrlKey) return;
  if (e.key === ",") toggleSettings();
  else if (guiOn()) return;
  else if (e.key === "t") toggleProjects();
  else if (e.key === "w" && active) closeTab(active);
  else if (/^[1-9]$/.test(e.key) && tabs[e.key - 1]) activate(tabs[e.key - 1]);
  else return;
  e.preventDefault();
  e.stopPropagation();
}, true);

const UPLOAD_LIMIT = 25 << 20;

// Claude Code attaches an image when a file path is pasted, so an attached file becomes a paste of its escaped path.
function pastePaths(tab, paths, files) {
  if (guiOn() && tab === active) return insertPaths(paths, files);
  tab.term.paste(paths.map((p) => p.replace(/[^\w@%+=:,./-]/g, "\\$&")).join(" ") + " ");
  tab.term.focus();
}

const attachNote = document.createElement("p");
attachNote.id = "attach-note";
attachNote.hidden = true;
terms.append(attachNote);
let attachNoteTimer;
function showAttachNote(text, error = false) {
  if (guiOn()) return showComposerNote(text, error);
  clearTimeout(attachNoteTimer);
  attachNote.textContent = text;
  attachNote.toggleAttribute("data-error", error);
  attachNote.hidden = !text;
  if (error) attachNoteTimer = setTimeout(() => (attachNote.hidden = true), 6000);
}

const base64Of = (file) =>
  new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result.slice(reader.result.indexOf(",") + 1));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });

// The session reads a path on its own machine, so each file is saved there first.
async function attachFiles(tab, files) {
  if (!tab || !files.length) return;
  if (!tab.view.running) return showAttachNote("Start claude in this tab first.", true);
  const paths = [];
  try {
    for (const file of files) {
      if (file.size > UPLOAD_LIMIT) throw `${file.name} is over 25 MB, the largest file pinned attaches`;
      showAttachNote(`Sending ${file.name} to ${machineName(tab.machine)}…`);
      paths.push(await callOn(tab.machine, { cmd: "upload", name: file.name, data: await base64Of(file) }));
    }
    showAttachNote("");
  } catch (e) {
    showAttachNote(String(e), true);
  }
  if (paths.length && tabs.includes(tab)) pastePaths(tab, paths, files);
}

const attachInput = document.getElementById("attach-input");
let attachTo = null;
document.getElementById("attach").onclick = () => {
  if (!active) return;
  attachTo = active;
  attachInput.click();
};
attachInput.onchange = () => {
  attachFiles(attachTo, [...attachInput.files]);
  attachInput.value = "";
};

TAURI?.webview.getCurrentWebview().onDragDropEvent(async ({ payload }) => {
  if (payload.type !== "drop" || !active || dialogOpen()) return;
  const tab = threadAt(payload.position) ?? active;
  if (tab.machine === LOCAL) return tab.view.running && pastePaths(tab, payload.paths);
  try {
    const files = await Promise.all(payload.paths.map(async (path) => new File([await invoke("read_dropped", { path })], path.split(/[\\/]/).pop())));
    await attachFiles(tab, files);
  } catch (e) {
    showAttachNote(String(e), true);
  }
});

// The app gets drops from Tauri above; a browser gets them here.
if (!TAURI) {
  const dragsFiles = (e) => e.dataTransfer?.types.includes("Files");
  addEventListener("dragover", (e) => {
    if (!dragsFiles(e)) return;
    e.preventDefault();
    terms.dataset.dropping = "";
  });
  addEventListener("dragleave", (e) => {
    if (!e.relatedTarget) delete terms.dataset.dropping;
  });
  addEventListener("drop", (e) => {
    if (!dragsFiles(e)) return;
    e.preventDefault();
    delete terms.dataset.dropping;
    attachFiles(active, [...e.dataTransfer.files]);
  });
}

document.addEventListener("paste", (e) => {
  const types = [...(e.clipboardData?.types ?? [])];
  if (!active || settingsOpen() || pickerOpen() || types.includes("text/plain") || !types.includes("Files")) return;
  e.preventDefault();
  e.stopPropagation();
  // A clipboard image has no text for xterm to paste. On this computer Claude Code reads the image itself
  // on Ctrl+V, or Alt+V on Windows. The app view attaches it instead, so it shows in the composer before it's sent.
  if (active.machine === LOCAL && !guiOn()) callOn(LOCAL, { cmd: "write_input", tab: active.view.id, data: IS_MAC ? "\x16" : "\x1bv" });
  else attachFiles(active, [...e.clipboardData.files]);
}, true);

function onMachineEvent(machine, event) {
  if (!machines.has(machine)) return;
  if (event.kind === "tabs") return setTabs(machine, event.tabs);
  if (event.kind === "usage") {
    machines.get(machine).usage[event.account ?? DEFAULT_ACCOUNT] = event.usage;
    return renderUsage();
  }
  if (event.kind !== "exit") return;
  const tab = tabByKey(keyOf(machine, event.tab));
  if (!tab) return;
  tab.view = { ...tab.view, running: false };
  if (tab.view.program !== "sign-in") return tab.term.write("\r\n\x1b[2m[claude exited, press Enter to restart]\x1b[0m\r\n");
  tab.term.write("\r\n\x1b[2m[sign-in ended, press Enter to run it again]\x1b[0m\r\n");
  accountsOf(machine, true).then(() => settingsOpen() && page === "accounts" && showAccountsPage(), console.warn);
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

// A window's figure stops holding once it resets, so it leaves the meter then.
function renderUsage() {
  const usage = machines.get(active?.machine ?? LOCAL)?.usage[active?.view.account ?? DEFAULT_ACCOUNT] ?? {};
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

// set_view resizes the native window. The panel is hidden before the window shrinks and shown
// after it grows, so it is never laid out in a window too small for it: the terminals would fit
// to a few cells and claude would redraw at that size.
async function setView(view) {
  document.body.classList.remove("alert");
  const previous = document.body.dataset.view;
  if (view === "pill") document.body.dataset.view = view;
  try {
    await invoke("set_view", { view });
  } catch (e) {
    document.body.dataset.view = previous;
    throw e;
  }
  document.body.dataset.view = view;
}
document.getElementById("new-tab").onclick = () => toggleProjects();
document.getElementById("minimize").onclick = () => setView("pill");
// The row scrolls sideways: a trackpad swipes it, a mouse wheel turns into a sideways scroll, and a
// drag pulls it along without opening the thread it started on.
let dragged = false;
carousel.addEventListener("wheel", (e) => {
  if (Math.abs(e.deltaX) >= Math.abs(e.deltaY)) return;
  e.preventDefault();
  carousel.scrollLeft += e.deltaY;
}, { passive: false });
carousel.onmousedown = (e) => {
  if (e.button !== 0) return;
  const from = carousel.scrollLeft;
  dragged = false;
  drag(e, {
    move: (m) => {
      dragged = true;
      carousel.scrollLeft = from - (m.clientX - e.clientX);
    },
  });
};
track.onclick = async (e) => {
  if (dragged) return (dragged = false);
  const tab = tabByKey(e.target.closest(".item")?.dataset.tab);
  if (!tab) return;
  await setView("expanded");
  activate(tab);
};
new ResizeObserver(renderCarousel).observe(pill);
const resizeFrom = (direction) => (e) => {
  e.preventDefault();
  win.startResizeDragging(direction);
};
document.getElementById("pill-edge").onmousedown = resizeFrom("East");
document.getElementById("pill-corner").onmousedown = resizeFrom("SouthEast");
document.getElementById("grip").onmousedown = resizeFrom("SouthEast");

document.getElementById("folder").onclick = async () => {
  const tab = active;
  if (!tab) return;
  if (tab.machine !== LOCAL) return pickerOpen() ? closeProjects() : showProjects({ machine: tab.machine, tab });
  const path = await invoke("pick_folder", { start: tab.view.cwd });
  if (path) await moveTab(tab, path);
};

const sessionActions = document.getElementById("actions");
const sessionMenu = document.getElementById("actions-menu");

function closeSessionMenu() {
  sessionMenu.hidden = true;
  delete sessionActions.dataset.open;
}

// pinned can't see the session's permission mode, so the toggle offers the opposite of the tab's flag.
function openSessionMenu(tab) {
  const item = (label, hint, run, className = "") => {
    const button = document.createElement("button");
    button.className = className;
    button.append(Object.assign(document.createElement("span"), { textContent: label }));
    if (hint) button.append(Object.assign(document.createElement("span"), { className: "hint", textContent: hint }));
    button.onclick = () => {
      closeSessionMenu();
      run();
    };
    return button;
  };
  sessionMenu.replaceChildren(
    tab.view.bypass
      ? item("Ask for permissions again", "Restarts claude and resumes this conversation.", () => setBypass(tab, false))
      : item("⚠ Skip all permissions", "Restarts claude with no permission checks and resumes this conversation.", () => setBypass(tab, true), "danger"),
    ...switchItems(tab).map((s) => item(s.label, `${s.hint}.`, s.run)),
    item("Attach files…", "", () => document.getElementById("attach").click()),
    item("Change folder…", "", () => document.getElementById("folder").click()),
    item("Close tab", "", () => closeTab(tab)),
  );
  sessionMenu.hidden = false;
  sessionActions.dataset.open = "";
}

document.getElementById("actions-toggle").onclick = () => (sessionMenu.hidden && active ? openSessionMenu(active) : closeSessionMenu());
window.addEventListener("mousedown", (e) => {
  if (!sessionMenu.hidden && !e.target.closest("#actions")) closeSessionMenu();
}, true);
window.addEventListener("keydown", (e) => {
  if (sessionMenu.hidden || e.key !== "Escape") return;
  closeSessionMenu();
  e.preventDefault();
  e.stopPropagation();
}, true);

// Phone keyboards lack these keys, so a row of them sits above the terminal on touch screens.
for (const key of document.querySelectorAll("#keys button")) {
  key.onpointerdown = (e) => e.preventDefault();
  if (key.dataset.send) key.onclick = () => active && typeInto(active, JSON.parse(`"${key.dataset.send}"`));
}

// A touch screen has no paste menu on the terminal, which xterm draws on a canvas.
document.getElementById("paste-key").onclick = async () => {
  if (!active) return;
  try {
    const text = await navigator.clipboard.readText();
    if (text) active.term.paste(text);
  } catch {
    showAttachNote("pinned may not read the clipboard. Allow it in the site's settings.", true);
  }
};

// xterm selects only with a mouse, so on a touch screen the text opens as a page the phone can select.
const termText = document.createElement("pre");
termText.id = "term-text";
termText.hidden = true;
terms.append(termText);
const selectKey = document.getElementById("select-key");

function showTermText(on) {
  termText.hidden = !on || !active;
  selectKey.classList.toggle("on", !termText.hidden);
  if (termText.hidden) return;
  const buffer = active.term.buffer.active;
  const lines = [];
  for (let i = 0; i < buffer.length; i++) {
    const line = buffer.getLine(i);
    const text = line.translateToString(true);
    if (line.isWrapped && lines.length) lines[lines.length - 1] += text;
    else lines.push(text);
  }
  termText.textContent = lines.join("\n").trimEnd();
  termText.scrollTop = termText.scrollHeight;
  document.activeElement?.blur();
}
selectKey.onclick = () => showTermText(termText.hidden);

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
function headerUpdateResult(status) {
  return "wait";
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

// A browser runs the UI its hub serves, so it shows the hub's version.
(TAURI ? TAURI.app.getVersion() : fetch("/api/version").then((r) => (r.ok ? r.text() : "")))
  .then((version) => (document.getElementById("version").textContent = version && `v${version}`))
  .catch(() => {});

new ResizeObserver(() => refit(active)).observe(terms);

// Safari leaves the page its full height under the on-screen keyboard, shrinks only the visual
// viewport, and pans that down to the focused field; the page follows both. A pinch-zoomed page
// keeps its size and lets the pan show its other parts. The browser revealed the field against the
// page's old height, so the shorter settings page scrolls it back into view.
if (!TAURI && window.visualViewport) {
  const fitVisible = () => {
    const root = document.documentElement.style;
    root.height = `${visualViewport.height * visualViewport.scale}px`;
    root.transform = visualViewport.scale > 1.01 ? "" : `translateY(${visualViewport.offsetTop}px)`;
    scrollTo(0, 0);
    if (document.activeElement?.closest("#settings-page")) document.activeElement.scrollIntoView({ block: "nearest" });
  };
  visualViewport.addEventListener("resize", fitVisible);
  visualViewport.addEventListener("scroll", fitVisible);
  fitVisible();
}

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
      if (on) retryUploadImages();
      if (settingsOpen() && page === "account") showAccount();
    },
    refused: () => (TAURI ? (document.body.dataset.hub = "refused") : signOutWeb("You were signed out. Sign in again.")),
  });
}

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
    else if (!wasOnline) {
      refreshTabs(view.id).catch(console.warn);
      retryUploadImages();
    }
  }
  for (const id of [...machines.keys()]) {
    if (id === LOCAL || list.some((v) => v.id === id)) continue;
    setTabs(id, [], { forget: false });
    machines.delete(id);
  }
  for (const listen of [...viewListeners]) listen();
  render();
  if (settingsOpen() && page === "account") showAccount();
  // Redrawing would take the name being typed away.
  if (settingsOpen() && page === "machines" && !machinesUi.renaming) showMachinesPage();
}

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
    view: { id: LOCAL, name: `This ${COMPUTER}`, os: IS_MAC ? "macos" : "windows", online: true, last_seen: 0 },
    tabs: [],
    transport: localTransport,
    usage: {},
  });
  invoke("get_usage").then((byAccount) => {
    for (const [account, usage] of Object.entries(byAccount)) onMachineEvent(LOCAL, { kind: "usage", usage, account });
  });
  const restored = await invoke("restore_tabs");
  await refreshTabs(LOCAL);
  for (const id of restored) await startTab(tabByKey(keyOf(LOCAL, id)));
  if (!machines.get(LOCAL).tabs.length) await openTab(LOCAL, null);
  const signedIn = await invoke("account");
  if (signedIn.signed_in) connectHub(signedIn);
}
