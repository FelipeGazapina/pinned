/** The app view, after T3 Code: threads in a sidebar, the conversation in the middle, and a panel
 * with the diff, a browser and the pull request on the right. claude keeps running in its
 * terminal underneath, so switching views never touches the session. */

const threadList = document.getElementById("thread-list");
const threadView = document.getElementById("thread");
const composer = document.getElementById("composer");
const prompt = document.getElementById("prompt");
const approval = document.getElementById("approval");
const panelBody = document.getElementById("panel-body");

const guiOn = () => document.body.dataset.mode === "gui";
const POLL_MS = 1200;
const PANELS = ["diff", "browser", "pr"];
/** A thread nobody touched for this long moves under "Settled", as T3 Code's auto-settle does. */
const SETTLE_AFTER_S = 3 * 86400;
const narrow = (px) => matchMedia(`(max-width: ${px}px)`).matches;

/** What the GUI knows of each tab, by tab key. `items` is the transcript read so far, from `next` on. */
const threads = new Map();
const threadOf = (tab) => {
  if (!threads.has(tab.key)) threads.set(tab.key, { session: null, next: 0, items: [], loaded: false, workspace: null, history: null, open: new Set() });
  return threads.get(tab.key);
};

let seenAt = {};
try {
  seenAt = JSON.parse(load("gui-seen")) ?? {};
} catch {}

const ui = {
  panel: PANELS.includes(load("gui-panel")) ? load("gui-panel") : "diff",
  panelOpen: load("gui-panel-open") === "true",
  sidebarOpen: load("gui-sidebar") !== "false" && !narrow(768),
  drawer: false,
  search: "",
  project: load("gui-project") ?? "",
  settledOpen: load("gui-settled-open") === "true",
  diffTurn: null,
  diffView: load("gui-diff-view") === "split" ? "split" : "unified",
  diffWrap: load("gui-diff-wrap") !== "false",
  collapsed: new Set(),
  browserUrl: load("gui-browser-url") ?? "",
  pr: null,
  prTab: "summary",
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

/** The app starts as a small panel; the three columns need room, and terminal view gets its own size back. */
async function fitWindowTo(mode) {
  const size = (await win.innerSize()).toLogical(await win.scaleFactor());
  let sizes = {};
  try {
    sizes = JSON.parse(load("mode-sizes")) ?? {};
  } catch {}
  sizes[mode === "gui" ? "terminal" : "gui"] = { width: size.width, height: size.height };
  save("mode-sizes", JSON.stringify(sizes));
  const want = sizes[mode] ?? (mode === "gui" ? { width: Math.max(size.width, 1180), height: Math.max(size.height, 760) } : null);
  if (!want || (Math.round(want.width) === Math.round(size.width) && Math.round(want.height) === Math.round(size.height))) return;
  await win.setSize(new TAURI.dpi.LogicalSize(Math.min(want.width, screen.availWidth - 40), Math.min(want.height, screen.availHeight - 40)));
}

// ── Sidebar ────────────────────────────────────────────────────────────────

const ago = (unix) => {
  if (!unix) return "";
  const s = Math.max(0, Date.now() / 1000 - unix);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
};
const projectName = (tab) => (threadOf(tab).workspace?.project ?? tab.view.cwd).split("/").filter(Boolean).pop() || "/";
const firstPrompt = (tab) => threadOf(tab).items.find((i) => i.kind === "user")?.text.split("\n")[0];
const threadTitle = (tab) => tab.view.title || firstPrompt(tab) || "New thread";
const prOf = (tab) => [...threadOf(tab).items].reverse().find((i) => i.kind === "pr");
const asksPermission = (tab) => statusOf(tab) === "needs-input" && /permission/i.test(tab.view.message ?? "");
const unseenDone = (tab) => statusOf(tab) === "done" && tab !== active && (tab.view.updated ?? 0) > (seenAt[tab.key] ?? 0);
const settled = (tab) => !!tab.view.updated && ["idle", "done"].includes(statusOf(tab)) && tab !== active && Date.now() / 1000 - tab.view.updated > SETTLE_AFTER_S;

function markThreadSeen(tab) {
  if (!tab || !guiOn() || document.hidden) return;
  seenAt[tab.key] = Date.now() / 1000;
  save("gui-seen", JSON.stringify(seenAt));
}

/** The row's status slot: a label while the session needs attention, otherwise how long ago it last moved. */
function statusSlot(tab) {
  const status = statusOf(tab);
  if (status === "working") return { icon: "circle-dashed", text: "Working", tone: "info" };
  if (asksPermission(tab)) return { icon: "shield-question", text: "Approval", tone: "warning" };
  if (status === "needs-input") return { icon: "circle-alert", text: "Input", tone: "input" };
  if (status === "failed") return tab.view.resume_at ? { icon: "circle-alert", text: "Limited", tone: "warning" } : { icon: "circle-alert", text: "Failed", tone: "error" };
  if (unseenDone(tab)) return { icon: "circle-check", text: "Done", tone: "success" };
  return null;
}

function renderSidebar() {
  const projects = [...new Set(tabs.map(projectName))].sort();
  if (ui.project && !projects.includes(ui.project)) ui.project = "";
  const filter = document.getElementById("project-filter");
  const options = ["", ...projects].map((p) => `<option value="${esc(p)}"${p === ui.project ? " selected" : ""}>${p ? esc(p) : "All projects"}</option>`).join("");
  if (filter.dataset.built !== options) {
    filter.innerHTML = options;
    filter.dataset.built = options;
  }
  const needle = ui.search.trim().toLowerCase();
  const shown = tabs.filter((t) => (!ui.project || projectName(t) === ui.project)
    && (!needle || `${threadTitle(t)} ${projectName(t)} ${threadOf(t).workspace?.branch ?? ""} ${prOf(t)?.number ?? ""}`.toLowerCase().includes(needle)));
  const live = shown.filter((t) => !settled(t));
  const old = shown.filter(settled);
  const rows = live.map(threadRow);
  if (old.length) {
    const head = document.createElement("button");
    head.className = "settled-head";
    head.dataset.open = ui.settledOpen;
    head.innerHTML = `<span>${ui.settledOpen ? "Settled" : `Settled (${old.length})`}</span><span class="line"></span>${icon("chevron-down")}`;
    head.onclick = () => {
      ui.settledOpen = !ui.settledOpen;
      save("gui-settled-open", String(ui.settledOpen));
      renderSidebar();
    };
    rows.push(head, ...(ui.settledOpen ? old.map(threadRow) : []));
  }
  threadList.replaceChildren(...rows);
  if (!shown.length) threadList.innerHTML = `<p class="empty">${tabs.length ? "No threads found" : "No threads yet"}</p>`;
}

function threadRow(tab) {
  const t = threadOf(tab);
  const row = document.createElement("button");
  row.className = "thread-row";
  row.classList.toggle("active", tab === active);
  const slot = statusSlot(tab);
  row.classList.toggle("receded", tab !== active && !slot && statusOf(tab) !== "needs-input");
  row.classList.toggle("working", statusOf(tab) === "working" && tab !== active);
  row.dataset.tab = tab.key;
  const pr = prOf(tab);
  const ws = t.workspace;
  const remote = tab.machine !== LOCAL ? `<span class="machine" title="${esc(machineName(tab.machine))}">${esc(initials(machineName(tab.machine)))}</span>` : "";
  const where = ws?.branch
    ? `${ws.worktree ? icon("folder-git", "dim") : ""}<span class="branch">${esc(ws.branch)}</span>`
    : `<span class="branch">${esc(tilde(tab.view.cwd))}</span>`;
  row.innerHTML = `<span class="l1"><span class="project">${esc(projectName(tab))}</span>${remote}${tab.view.bypass ? `<span class="warn" title="Permissions skipped">⚠</span>` : ""}
      ${slot ? `<span class="status ${slot.tone}">${icon(slot.icon)}${slot.text}</span>` : `<span class="when">${esc(ago(tab.view.updated))}</span>`}</span>
    <span class="l2">${esc(threadTitle(tab))}</span>
    <span class="l3">${where}${pr ? `<span class="pr" data-pr="${esc(pr.url)}" title="Open pull request #${pr.number}">${icon("git-pull-request")}#${pr.number}</span>` : ""}<span class="provider">${icon("claude")}</span></span>`;
  row.onclick = (e) => {
    activate(tab);
    if (e.target.closest(".pr")) openPanel("pr");
    if (narrow(768)) toggleSidebar(false);
  };
  row.onauxclick = (e) => e.button === 1 && closeTab(tab);
  return row;
}

document.getElementById("thread-search").oninput = (e) => {
  ui.search = e.target.value;
  renderSidebar();
};
document.getElementById("thread-search").onkeydown = (e) => {
  if (e.key !== "Escape") return;
  e.target.value = ui.search = "";
  renderSidebar();
};
document.getElementById("project-filter").onchange = (e) => {
  ui.project = e.target.value;
  save("gui-project", ui.project);
  renderSidebar();
};
document.getElementById("new-thread").onclick = () => toggleProjects();

function toggleSidebar(open = !ui.sidebarOpen) {
  ui.sidebarOpen = open;
  if (!narrow(768)) save("gui-sidebar", String(open));
  document.body.dataset.sidebar = open ? "open" : "closed";
}
for (const id of ["sidebar-toggle", "chat-sidebar-toggle"]) document.getElementById(id).onclick = () => toggleSidebar();
// On a phone the sidebar is a sheet over the thread; a tap beside it closes it.
document.getElementById("workspace").addEventListener("click", (e) => {
  if (e.target.id === "workspace" && narrow(768) && ui.sidebarOpen) toggleSidebar(false);
});

// ── Thread ─────────────────────────────────────────────────────────────────

/** Splits the items into turns: the prompt, the work done for it, and what the assistant said.
 * As in T3 Code, the first and the last message stay in view, and the tool calls and the
 * narration between them fold into one "Worked for" row. */
function turnsOf(items) {
  const turns = [];
  const results = new Map(items.filter((i) => i.kind === "result").map((r) => [r.tool, r]));
  let turn = null;
  for (const item of items) {
    if (item.kind === "result") continue;
    if (item.kind === "user" || !turn) {
      turn = { id: item.id ?? `t${turns.length}`, user: item.kind === "user" ? item : null, steps: [], prs: [], notes: [] };
      turns.push(turn);
      if (item.kind === "user") continue;
    }
    if (item.kind === "pr") turn.prs.push(item);
    else if (item.kind === "note") turn.notes.push(item);
    else turn.steps.push(item.kind === "tool" ? { ...item, result: results.get(item.id) } : item);
  }
  for (const t of turns) {
    const texts = t.steps.filter((s) => s.kind === "text");
    t.answer = t.steps.at(-1)?.kind === "text" ? t.steps.at(-1) : null;
    t.lead = texts[0] && texts[0] !== t.answer ? texts[0] : null;
    t.work = t.steps.filter((s) => s !== t.lead && s !== t.answer);
    const times = [t.user?.at, ...t.steps.flatMap((s) => [s.at, s.result?.at])].filter(Boolean).map(Date.parse).filter(Number.isFinite);
    t.start = times.length ? Math.min(...times) : 0;
    t.end = times.length ? Math.max(...times) : 0;
  }
  return turns;
}

/** T3 Code's formatDuration: `4.2s`, `38s`, `3m 43s`, `1h 2m`. */
function duration(ms) {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`;
  const total = Math.round(ms / 1000);
  if (total < 60) return `${total}s`;
  const [h, m, s] = [Math.floor(total / 3600), Math.floor((total % 3600) / 60), total % 60];
  return [h && `${h}h`, m && `${m}m`, s && `${s}s`].filter(Boolean).join(" ");
}

marked.use({ gfm: true, renderer: { html: ({ text }) => esc(text) } });
const markdown = (text) => marked.parse(text);

const TOOL_ICONS = { Bash: "terminal", Read: "eye", Edit: "square-pen", MultiEdit: "square-pen", Write: "square-pen", NotebookEdit: "square-pen", Grep: "search", Glob: "search", WebFetch: "globe", WebSearch: "globe", Agent: "bot", Task: "bot" };
const shortPath = (path, cwd) => (cwd && path.startsWith(`${cwd}/`) ? path.slice(cwd.length + 1) : tilde(path));

function toolLabel(step, cwd) {
  const what = /^(Read|Write|Edit|MultiEdit|NotebookEdit)$/.test(step.name) ? shortPath(step.summary, cwd) : step.summary;
  const verb = { Read: "Read", Edit: "Edited", MultiEdit: "Edited", Write: "Wrote", Grep: "Searched", Glob: "Listed", WebFetch: "Fetched", WebSearch: "Searched the web for", Agent: "Delegated", Task: "Delegated", Skill: "Used skill" }[step.name];
  if (step.name === "Bash") return what || step.detail.split("\n")[0];
  return verb ? `${verb} ${what}` : `${step.name.replace(/^mcp__([^_]+)__/, "$1 · ")}${what ? ` ${what}` : ""}`;
}

function toolHtml(step, cwd) {
  const name = /^mcp__/.test(step.name) ? "wrench" : TOOL_ICONS[step.name] ?? "zap";
  const state = !step.result ? "running" : step.result.error ? "error" : "ok";
  const body = [step.detail && `<pre class="input">${esc(step.detail)}</pre>`, step.result?.text && `<pre>${esc(step.result.text)}</pre>`].filter(Boolean).join("");
  return `<details class="tool" data-state="${state}"${body ? "" : " data-empty"}><summary>${icon(name, "kind")}<span class="label">${esc(toolLabel(step, cwd))}</span>
    ${state === "error" ? icon("x", "failed") : ""}${body ? icon("chevron-right", "chev") : ""}</summary>${body}</details>`;
}

/** The saved diff of each turn: a turn's diff is the one that ended after its prompt and before the next.
 * `ended` is in whole seconds, so it may read up to a second before the prompt that follows. */
function changesByTurn(history, turns) {
  const found = new Map();
  for (const h of history ?? []) {
    const turn = turns.findLast((t) => t.user && t.start <= h.ended * 1000 + 999);
    if (turn && !found.has(turn)) found.set(turn, h);
  }
  return found;
}

const sum = (files, key) => files.reduce((n, f) => n + Number(f[key]), 0);

function changesHtml(change) {
  const n = change.files.length;
  return `<details class="changes"><summary>${icon("chevron-right", "chev")}<span class="count">${n} changed file${n === 1 ? "" : "s"}</span>
      <span class="add">+${sum(change.files, "added")}</span><span class="del">−${sum(change.files, "removed")}</span><span class="show">Show files</span>
      <button class="open-diff" data-ended="${change.ended}">${icon("file-diff")}Open diff</button></summary>
    <ul>${change.files.map((f) => `<li data-ended="${change.ended}" data-file="${esc(f.path)}"><span class="path">${esc(f.path)}</span><span class="add">+${f.added}</span><span class="del">−${f.removed}</span></li>`).join("")}</ul></details>`;
}

function turnHtml(tab, turn, change, live) {
  const t = threadOf(tab);
  const user = turn.user
    ? `<div class="user"><div class="bubble">${esc(turn.user.text)}${turn.user.images ? `<span class="images">${turn.user.images} image${turn.user.images > 1 ? "s" : ""}</span>` : ""}</div></div>`
    : "";
  const steps = turn.work.map((s) => (s.kind === "tool" ? toolHtml(s, tab.view.cwd) : `<div class="narration md">${markdown(s.text)}</div>`)).join("");
  let fold = "";
  if (live) {
    const running = [...turn.steps].reverse().find((s) => s.kind === "tool");
    fold = `<details class="work live" data-turn="${esc(turn.id)}"${t.open.has(turn.id) ? " open" : ""}>
        <summary><span class="label">Working for <span class="ticker" data-since="${turn.start}">${duration(Date.now() - turn.start)}</span></span></summary>
        <div class="steps">${steps}</div></details>
      ${running && !t.open.has(turn.id) ? `<div class="live-tool">${toolHtml(running, tab.view.cwd)}</div>` : ""}`;
  } else if (turn.work.length) {
    fold = `<details class="work" data-turn="${esc(turn.id)}"${t.open.has(turn.id) ? " open" : ""}>
        <summary><span class="label">Worked for ${duration(turn.end - turn.start)}</span>${icon("chevron-right", "chev")}</summary>
        <div class="steps">${steps}</div></details>`;
  }
  const said = (s) => (s ? `<div class="assistant md">${markdown(s.text)}</div>` : "");
  return `<section class="turn">${user}${turn.notes.map((n) => `<p class="note">${esc(n.text)}</p>`).join("")}${fold}${said(turn.lead)}${said(turn.answer)}
    ${turn.prs.map((p) => `<p class="pr-line"><button class="pr-link" data-pr="${esc(p.url)}">${icon("github")}PR #${p.number}</button><span class="muted">${esc(p.repo)}</span></p>`).join("")}
    ${change ? changesHtml(change) : ""}</section>`;
}

function renderThread() {
  const tab = active;
  const crumb = document.querySelector("#chat-head .crumb");
  if (!tab) {
    crumb.textContent = "";
    threadView.dataset.key = "";
    threadView.innerHTML = `<div class="blank"><p>No threads open.</p><button class="primary" data-new-thread>New thread</button></div>`;
    renderComposer(null);
    return;
  }
  const t = threadOf(tab);
  crumb.innerHTML = `<span class="project">${esc(projectName(tab))}</span><span class="sep">/</span><span class="title" title="${esc(threadTitle(tab))}">${esc(threadTitle(tab))}</span>`;
  const pr = prOf(tab);
  const chip = document.getElementById("pr-chip");
  chip.hidden = !pr;
  if (pr) chip.querySelector("span").textContent = `#${pr.number}`;

  const turns = turnsOf(t.items);
  const working = ["working", "needs-input"].includes(statusOf(tab));
  const changes = changesByTurn(t.history, turns);
  const body = turns.length
    ? turns.map((turn, i) => turnHtml(tab, turn, changes.get(turn), working && i === turns.length - 1)).join("")
    : `<div class="blank"><p>${t.loaded ? "Send a message to start the conversation." : "Loading…"}</p><p class="hint">${esc(tilde(tab.view.cwd))}</p></div>`;
  const key = `${tab.key}|${working}|${t.items.length}|${!!t.history}|${[...t.open].join(",")}`;
  if (threadView.dataset.key === key) return renderComposer(tab);
  const stick = threadView.dataset.tab !== tab.key || threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight < 40;
  threadView.innerHTML = `<div class="lane">${body}</div>`;
  threadView.dataset.key = key;
  threadView.dataset.tab = tab.key;
  if (stick) threadView.scrollTop = threadView.scrollHeight;
  renderComposer(tab);
}

// The live turn's clock ticks on its own, without rebuilding the thread.
setInterval(() => {
  for (const ticker of threadView.querySelectorAll(".ticker")) ticker.textContent = duration(Date.now() - Number(ticker.dataset.since));
}, 1000);

threadView.addEventListener("click", (e) => {
  if (e.target.closest("[data-new-thread]")) return toggleProjects();
  if (e.target.closest("[data-mode-terminal]")) return setMode("terminal");
  const diff = e.target.closest(".open-diff, .changes li");
  if (diff) {
    e.preventDefault();
    ui.diffTurn = Number(diff.dataset.ended);
    ui.diffFile = diff.dataset.file ?? null;
    return openPanel("diff");
  }
  const pr = e.target.closest("[data-pr]");
  if (pr) return e.metaKey || e.ctrlKey ? openOutside(pr.dataset.pr) : openPanel("pr");
  const link = e.target.closest("a[href]");
  if (link) {
    e.preventDefault();
    openLink(link.href, e.metaKey || e.ctrlKey);
  }
});
threadView.addEventListener("toggle", (e) => {
  const turn = e.target.dataset?.turn;
  if (!turn || !active) return;
  const open = threadOf(active).open;
  if (e.target.open) open.add(turn);
  else open.delete(turn);
  threadView.dataset.key = "";
}, true);

const openOutside = (url) => (TAURI ? invoke("open_url", { url }).catch(console.warn) : window.open(url, "_blank", "noopener"));

/** A link to a local server opens in the side panel's browser; any other, or with ⌘, in the system browser. */
function openLink(url, outside = false) {
  if (!outside && /^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0)(:\d+)?/.test(url)) {
    setBrowserUrl(url);
    return openPanel("browser");
  }
  openOutside(url);
}

async function fetchThread(tab) {
  const t = threadOf(tab);
  if (t.fetching) return;
  t.fetching = true;
  try {
    let page = await callOn(tab.machine, { cmd: "thread", tab: tab.view.id, from: t.next });
    if (page.session !== t.session) {
      const restart = t.next > 0;
      Object.assign(t, { session: page.session, items: [], next: 0, history: null });
      if (restart) page = await callOn(tab.machine, { cmd: "thread", tab: tab.view.id, from: 0 });
    }
    t.unsupported = false;
    const grew = page.items.length > 0 || !t.loaded;
    t.items.push(...page.items);
    t.next = page.next;
    t.loaded = true;
    if (grew && guiOn()) {
      if (tab === active) renderThread();
      renderSidebar();
    }
    if (grew && t.session && !t.history && tab === active) fetchHistory(tab);
  } catch (e) {
    t.loaded = true;
    t.unsupported = /unknown variant|can't answer/.test(String(e));
    if (t.unsupported && tab === active && guiOn()) renderUnsupported();
  } finally {
    t.fetching = false;
  }
}

function renderUnsupported() {
  threadView.dataset.key = "";
  threadView.innerHTML = `<div class="blank"><p>${esc(machineName(active.machine))} runs an older pinned that can't show conversations.</p>
    <p class="hint">Update pinned there, or use the terminal view.</p><button class="primary" data-mode-terminal>Terminal view</button></div>`;
}

async function fetchWorkspace(tab) {
  const t = threadOf(tab);
  if (t.workspaceFor === tab.view.cwd) return;
  t.workspaceFor = tab.view.cwd;
  t.workspace = await callOn(tab.machine, { cmd: "workspace", tab: tab.view.id }).catch(() => null);
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
  const turns = await callOn(tab.machine, { cmd: "project_history", path: t.workspace.project, session }).catch(() => []);
  t.fetchingHistory = false;
  if (t.session !== session) return;
  t.history = turns.filter((h) => h.session === session).reverse();
  if (tab === active && guiOn()) {
    renderThread();
    if (ui.panelOpen && ui.panel === "diff") renderPanel();
  }
}

/** Each tab's status at the last render, so a finished turn refetches its diff. */
const lastStatus = new Map();

function renderGui() {
  if (!guiOn()) return;
  document.body.dataset.sidebar = ui.sidebarOpen ? "open" : "closed";
  for (const tab of tabs) {
    const t = threadOf(tab);
    if (!t.loaded) fetchThread(tab);
    fetchWorkspace(tab);
    const before = lastStatus.get(tab.key);
    lastStatus.set(tab.key, statusOf(tab));
    if (before === "working" && statusOf(tab) !== "working") {
      t.history = null;
      fetchThread(tab).then(() => fetchHistory(tab));
    }
  }
  for (const key of threads.keys()) if (!tabByKey(key)) threads.delete(key);
  markThreadSeen(active);
  renderSidebar();
  if (active && threadOf(active).unsupported) renderUnsupported();
  else renderThread();
  renderPanel();
  renderGuiUsage();
}

setInterval(() => {
  if (!guiOn() || document.hidden) return;
  for (const tab of tabs) if (tab === active || statusOf(tab) === "working") fetchThread(tab);
}, POLL_MS);
setInterval(() => guiOn() && renderSidebar(), 60 * 1000);
document.addEventListener("visibilitychange", () => markThreadSeen(active));

// ── Composer ───────────────────────────────────────────────────────────────

const MODELS_MENU = [
  ["default", "Default", "The model your Claude Code settings pick"],
  ["opus", "Opus", "Most capable for complex work"],
  ["sonnet", "Sonnet", "Fast and capable for everyday tasks"],
  ["haiku", "Haiku", "Fastest for quick answers"],
];

function renderComposer(tab) {
  const working = !!tab && statusOf(tab) === "working";
  composer.dataset.working = working;
  composer.dataset.empty = !prompt.value.trim();
  document.querySelector("#send").innerHTML = icon(working && prompt.value.trim() ? "list-plus" : "arrow-up");
  document.querySelector("#send").title = working ? "Queue message (Enter)" : "Send (Enter)";
  renderApproval(tab);
  const footer = document.getElementById("composer-footer");
  if (!tab) return (footer.innerHTML = "");
  const t = threadOf(tab);
  const pr = prOf(tab);
  const ws = t.workspace;
  footer.innerHTML = `<span class="where" title="${esc(tab.view.cwd)}">${icon(ws?.worktree ? "folder-git" : "folder")}${ws?.worktree ? "Worktree" : "Local checkout"}</span>
    ${tab.machine !== LOCAL ? `<span class="machine">${icon("monitor")}${esc(machineName(tab.machine))}</span>` : ""}
    <span class="spacer"></span>
    ${pr ? `<button type="button" class="pr" data-pr="${esc(pr.url)}" title="Open pull request #${pr.number}">${icon("git-pull-request")}#${pr.number}</button>` : ""}
    ${ws?.branch ? `<span class="branch" title="${esc(ws.branch)}">${icon("git-branch")}<span>${esc(ws.branch)}</span></span>` : ""}`;
  prompt.placeholder = !tab.view.running
    ? "claude isn't running. Press Enter to start it."
    : t.items.length ? "Ask for changes, send follow-ups, or attach images" : "Ask anything, or / for commands";
}

/** Claude asks for permission in its terminal; the banner answers with the keys that prompt takes. */
function renderApproval(tab) {
  const asking = !!tab && statusOf(tab) === "needs-input";
  approval.hidden = !asking;
  if (!asking) return (approval.dataset.key = "");
  const permission = asksPermission(tab);
  const pending = turnsOf(threadOf(tab).items).at(-1)?.steps.findLast?.((s) => s.kind === "tool" && !s.result);
  const key = `${tab.key}|${tab.view.message}|${pending?.id}`;
  if (approval.dataset.key === key) return;
  approval.dataset.key = key;
  approval.dataset.kind = permission ? "approval" : "input";
  const kind = !permission ? "Claude needs you" : pending?.name === "Bash" ? "Command approval" : /^(Edit|Write|MultiEdit)$/.test(pending?.name) ? "File change approval" : "Permission approval";
  approval.innerHTML = `${icon(permission ? "shield" : "circle-alert", "lead")}<div class="body"><p class="kind">${esc(kind)}</p>
      <p class="what">${esc(tab.view.message || "")}</p>${pending && permission ? `<pre>${esc(pending.detail || pending.summary)}</pre>` : ""}</div>
    <div class="actions">${permission
      ? `<button type="button" data-key="\u001b" class="outline">Decline</button><button type="button" data-key="2" class="outline" title="Don't ask again for this in this session">Always allow</button><button type="button" data-key="1" class="primary">Approve</button>`
      : ""}<button type="button" data-show-terminal class="outline">Terminal</button></div>`;
}

approval.addEventListener("click", (e) => {
  const key = e.target.closest("[data-key]")?.dataset.key;
  if (key && active) return callOn(active.machine, { cmd: "write_input", tab: active.view.id, data: key });
  if (e.target.closest("[data-show-terminal]")) toggleDrawer(true);
});

function autosize() {
  prompt.style.height = "auto";
  prompt.style.height = `${Math.min(prompt.scrollHeight, 208)}px`;
  if (active) renderComposer(active);
}
prompt.addEventListener("input", autosize);
prompt.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    send();
  } else if (e.key === "Escape" && active && statusOf(active) === "working" && !prompt.value) {
    e.preventDefault();
    stop();
  }
});
document.getElementById("send").onclick = () => send();
document.getElementById("stop").onclick = () => stop();

async function send() {
  const tab = active;
  if (!tab) return toggleProjects();
  if (!tab.view.running) return startTab(tab);
  const text = prompt.value.trim();
  if (!text) return;
  prompt.value = "";
  autosize();
  try {
    await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text });
  } catch (e) {
    prompt.value = text;
    autosize();
    showAttachNote(String(e), true);
  }
  setTimeout(() => fetchThread(tab), 300);
}

const stop = () => active && callOn(active.machine, { cmd: "write_input", tab: active.view.id, data: "\x1b" });

/** Attached files go into the prompt as paths, which Claude Code reads, images included. */
function insertPaths(paths) {
  const text = paths.map((p) => p.replace(/[^\w@%+=:,./-]/g, "\\$&")).join(" ");
  prompt.value = `${prompt.value}${prompt.value && !prompt.value.endsWith(" ") ? " " : ""}${text} `;
  autosize();
  prompt.focus();
}
document.getElementById("composer-attach").onclick = () => document.getElementById("attach").click();

const menu = document.getElementById("composer-menu");
function openMenu(anchor, entries) {
  menu.replaceChildren(...entries.map(([label, hint, run, cls]) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = cls ?? "";
    b.innerHTML = `<span>${esc(label)}</span>${hint ? `<span class="hint">${esc(hint)}</span>` : ""}`;
    b.onclick = () => {
      menu.hidden = true;
      run();
    };
    return b;
  }));
  menu.hidden = false;
  menu.style.left = `${anchor.offsetLeft}px`;
}
window.addEventListener("mousedown", (e) => {
  if (!menu.hidden && !e.target.closest("#composer-menu, #model-button, #more-button")) menu.hidden = true;
}, true);

const sendSlash = (text) => active && callOn(active.machine, { cmd: "prompt", tab: active.view.id, text }).then(() => setTimeout(() => fetchThread(active), 400));
document.getElementById("model-button").onclick = (e) =>
  openMenu(e.currentTarget, MODELS_MENU.map(([id, label, hint]) => [label, hint, () => {
    sendSlash(`/model ${id}`);
    save("gui-model", id);
    renderModel();
  }]));
function renderModel() {
  const id = load("gui-model") ?? "default";
  document.querySelector("#model-button .name").textContent = MODELS_MENU.find(([m]) => m === id)?.[1] ?? "Default";
}
renderModel();

document.getElementById("more-button").onclick = (e) => {
  const tab = active;
  if (!tab) return;
  openMenu(e.currentTarget, [
    tab.view.bypass
      ? ["Ask for permissions again", "Restarts claude and resumes this conversation", () => setBypass(tab, false)]
      : ["Full access", "Restarts claude with no permission checks and resumes", () => setBypass(tab, true), "danger"],
    ["Compact the conversation", "/compact", () => sendSlash("/compact")],
    ["Change folder…", "", () => document.getElementById("folder").click()],
    ["Show the terminal", "⌘J", () => toggleDrawer(true)],
    ["Close thread", "⌘W", () => closeTab(tab)],
  ]);
};

// ── Terminal drawer ────────────────────────────────────────────────────────

function toggleDrawer(open = !ui.drawer) {
  ui.drawer = open;
  document.body.dataset.drawer = open ? "open" : "closed";
  if (open && active) refit(active);
  focusInput();
}
document.getElementById("toggle-terminal").onclick = () => toggleDrawer();

// ── Side panel ─────────────────────────────────────────────────────────────

function openPanel(kind) {
  ui.panel = kind;
  ui.panelOpen = true;
  save("gui-panel", kind);
  save("gui-panel-open", "true");
  renderPanel();
}
function closePanel() {
  ui.panelOpen = false;
  save("gui-panel-open", "false");
  renderPanel();
}
const togglePanel = () => (ui.panelOpen ? closePanel() : openPanel(ui.panel));
document.getElementById("toggle-panel").onclick = togglePanel;
document.getElementById("panel-close").onclick = closePanel;
document.getElementById("pr-chip").onclick = (e) => (e.metaKey || e.ctrlKey ? openOutside(prOf(active).url) : openPanel("pr"));
for (const b of document.querySelectorAll("#panel-tabs [data-panel]")) b.onclick = () => openPanel(b.dataset.panel);
document.getElementById("panel-expand").onclick = () => {
  document.body.dataset.panelMax = document.body.dataset.panelMax === "true" ? "false" : "true";
};

const PANEL_MIN = 360;
const setPanelWidth = (px) => {
  const max = Math.max(PANEL_MIN, Math.min(Math.floor(innerWidth * 0.7), innerWidth - 360 - (ui.sidebarOpen ? 256 : 0)));
  const width = Math.round(Math.min(max, Math.max(PANEL_MIN, px)));
  document.documentElement.style.setProperty("--panel-width", `${width}px`);
  return width;
};
setPanelWidth(Number(load("gui-panel-width")) || 540);
document.getElementById("panel-resize").onmousedown = (down) => {
  down.preventDefault();
  const right = document.getElementById("panel").getBoundingClientRect().right;
  document.body.classList.add("resizing");
  drag(down, {
    move: (e) => setPanelWidth(right - e.clientX),
    drop: (e) => save("gui-panel-width", String(setPanelWidth(right - e.clientX))),
  });
  addEventListener("mouseup", () => document.body.classList.remove("resizing"), { once: true });
};

function renderPanel() {
  document.body.dataset.panel = ui.panelOpen ? "open" : "closed";
  if (!guiOn() || !ui.panelOpen) return;
  const pr = active && prOf(active);
  for (const b of document.querySelectorAll("#panel-tabs [data-panel]")) {
    b.classList.toggle("active", b.dataset.panel === ui.panel);
    if (b.dataset.panel === "pr") b.querySelector("span").textContent = pr ? `#${pr.number}` : "Pull request";
  }
  panelBody.dataset.kind = ui.panel;
  if (ui.panel === "diff") renderDiffPanel();
  else if (ui.panel === "browser") renderBrowser();
  else renderPrPanel();
}

/** `git diff` output as files of hunks, each line with its old and new number. */
function parseDiff(patch) {
  const files = [];
  let file = null;
  let hunk = null;
  let oldNo = 0;
  let newNo = 0;
  for (const line of patch.split("\n")) {
    if (line.startsWith("diff --git ")) {
      file = { path: line.match(/ b\/(.*)$/)?.[1] ?? line.slice(11), status: "changed", added: 0, removed: 0, hunks: [], binary: false };
      files.push(file);
      hunk = null;
    } else if (!file) continue;
    else if (!hunk && line.startsWith("new file")) file.status = "added";
    else if (!hunk && line.startsWith("deleted file")) file.status = "deleted";
    else if (!hunk && line.startsWith("Binary files")) file.binary = true;
    else if (line.startsWith("@@")) {
      const m = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      oldNo = Number(m?.[1] ?? 0);
      newNo = Number(m?.[2] ?? 0);
      hunk = { oldStart: oldNo, lines: [] };
      file.hunks.push(hunk);
    } else if (hunk && !line.startsWith("\\")) {
      const type = line[0] === "+" ? "add" : line[0] === "-" ? "del" : "ctx";
      if (type === "ctx" && line === "") continue;
      if (type === "add") file.added++;
      if (type === "del") file.removed++;
      hunk.lines.push({ type, old: type === "add" ? null : oldNo++, new: type === "del" ? null : newNo++, text: line.slice(1) });
    }
  }
  return files;
}

function diffRows(hunk) {
  if (ui.diffView === "unified") {
    return hunk.lines.map((l) => `<div class="dl ${l.type}"><span class="no">${l.type === "del" ? l.old : l.new}</span><span class="code">${esc(l.text) || " "}</span></div>`).join("");
  }
  // Removed and added runs pair up side by side; context lines sit on both sides.
  const rows = [];
  let dels = [];
  let adds = [];
  const flush = () => {
    for (let i = 0; i < Math.max(dels.length, adds.length); i++) rows.push([dels[i], adds[i]]);
    dels = [];
    adds = [];
  };
  for (const l of hunk.lines) {
    if (l.type === "del") dels.push(l);
    else if (l.type === "add") adds.push(l);
    else {
      flush();
      rows.push([l, l]);
    }
  }
  flush();
  const side = (l, n) => (l ? `<span class="no ${l.type}">${n}</span><span class="code ${l.type}">${esc(l.text) || " "}</span>` : `<span class="no none"></span><span class="code none"></span>`);
  return rows.map(([a, b]) => `<div class="dl split">${side(a, a?.old)}${side(b, b?.new)}</div>`).join("");
}

function filesHtml(files, key) {
  if (!files.length) return `<p class="panel-empty">No net changes in this selection.</p>`;
  return files.map((f) => {
    const id = `${key}:${f.path}`;
    const collapsed = ui.collapsed.has(id);
    let next = 1;
    const hunks = f.hunks.map((h) => {
      const gap = h.oldStart - next;
      next = h.oldStart + h.lines.filter((l) => l.type !== "add").length;
      return `${gap > 0 ? `<div class="gap">${gap} unmodified line${gap === 1 ? "" : "s"}</div>` : ""}${diffRows(h)}`;
    }).join("");
    return `<section class="file" data-id="${esc(id)}" data-status="${f.status}"${collapsed ? " data-collapsed" : ""}>
      <header>${icon(collapsed ? "chevron-right" : "chevron-down", "fold")}<span class="change"></span><span class="path" title="${esc(f.path)}"><bdi>${esc(f.path)}</bdi></span>
        <button class="copy" data-copy="${esc(f.path)}" title="Copy path">${icon("copy")}</button><span class="spacer"></span><span class="del">-${f.removed}</span><span class="add">+${f.added}</span></header>
      ${collapsed ? "" : `<div class="hunks">${f.binary ? `<p class="panel-empty">Binary file</p>` : hunks}</div>`}</section>`;
  }).join("");
}

function diffToolbar(added, removed, scope) {
  const on = (b) => (b ? " on" : "");
  return `<div class="diff-bar">${scope}<span class="spacer"></span><span class="add">+${added}</span><span class="del">-${removed}</span>
    <button data-diff="collapse" class="icon" title="Collapse or expand all files">${icon("chevrons-up-down")}</button>
    <span class="seg"><button data-diff="unified" class="icon${on(ui.diffView === "unified")}" title="Stacked">${icon("rows-3")}</button><button data-diff="split" class="icon${on(ui.diffView === "split")}" title="Split">${icon("columns-2")}</button></span>
    <button data-diff="wrap" class="icon${on(ui.diffWrap)}" title="Wrap lines">${icon("text-wrap")}</button></div>`;
}

function renderDiffPanel() {
  const tab = active;
  const history = tab && threadOf(tab).history;
  if (!tab) return (panelBody.innerHTML = `<p class="panel-empty">Select a thread to inspect turn diffs.</p>`);
  if (threadOf(tab).workspace?.repo === false) {
    return (panelBody.innerHTML = `<p class="panel-empty">Turn diffs are unavailable because this folder is not a git repository.</p>`);
  }
  if (!history) {
    panelBody.innerHTML = `<p class="panel-empty">Loading changes…</p>`;
    return fetchHistory(tab);
  }
  if (!history.length) return (panelBody.innerHTML = `<p class="panel-empty">No completed turns with changes yet.</p>`);
  const chosen = history.find((h) => h.ended === ui.diffTurn) ?? history.at(-1);
  const scope = `<label class="scope"><select id="diff-turn">${history.map((h, i) => `<option value="${h.ended}"${h === chosen ? " selected" : ""}>Turn ${i + 1}</option>`).join("")}</select>${icon("chevron-down")}</label>`;
  const files = parseDiff(chosen.patch);
  panelBody.innerHTML = `${diffToolbar(sum(files, "added"), sum(files, "removed"), scope)}
    <div class="diff ${ui.diffView}${ui.diffWrap ? " wrap" : ""}">${filesHtml(files, chosen.ended)}
    ${chosen.truncated ? `<p class="panel-empty">This diff exceeds the size limit, so its end is cut.</p>` : ""}</div>`;
  document.getElementById("diff-turn").onchange = (e) => {
    ui.diffTurn = Number(e.target.value);
    renderDiffPanel();
  };
  if (ui.diffFile) {
    panelBody.querySelector(`.file[data-id="${CSS.escape(`${chosen.ended}:${ui.diffFile}`)}"]`)?.scrollIntoView({ block: "start" });
    ui.diffFile = null;
  }
}

panelBody.addEventListener("click", (e) => {
  const action = e.target.closest("[data-diff]")?.dataset.diff;
  if (action === "unified" || action === "split") {
    ui.diffView = action;
    save("gui-diff-view", action);
  } else if (action === "wrap") {
    ui.diffWrap = !ui.diffWrap;
    save("gui-diff-wrap", String(ui.diffWrap));
  } else if (action === "collapse") {
    const ids = [...panelBody.querySelectorAll(".file")].map((f) => f.dataset.id);
    const all = ids.every((id) => ui.collapsed.has(id));
    for (const id of ids) all ? ui.collapsed.delete(id) : ui.collapsed.add(id);
  } else if (e.target.closest("[data-copy]")) {
    navigator.clipboard?.writeText(e.target.closest("[data-copy]").dataset.copy);
    return;
  } else if (e.target.closest(".file > header")) {
    const id = e.target.closest(".file").dataset.id;
    ui.collapsed.has(id) ? ui.collapsed.delete(id) : ui.collapsed.add(id);
  } else if (e.target.closest("[data-pr-tab]")) {
    ui.prTab = e.target.closest("[data-pr-tab]").dataset.prTab;
  } else if (e.target.closest("[data-refresh-pr]")) {
    ui.pr = null;
  } else if (e.target.closest("[data-open-url]")) {
    return openOutside(e.target.closest("[data-open-url]").dataset.openUrl);
  } else if (e.target.closest("a[href]")) {
    e.preventDefault();
    return openLink(e.target.closest("a[href]").href, e.metaKey || e.ctrlKey);
  } else return;
  renderPanel();
});

// ── Browser ────────────────────────────────────────────────────────────────

/** Local servers the session mentioned, newest first, offered in the browser's empty state. */
function seenUrls(tab) {
  const found = new Set();
  const pattern = /https?:\/\/(?:localhost|127\.0\.0\.1|0\.0\.0\.0)(?::\d+)?(?:\/[^\s"'`)<>\]]*)?/g;
  for (const item of [...threadOf(tab).items].reverse()) {
    for (const m of `${item.text ?? ""} ${item.detail ?? ""}`.matchAll(pattern)) found.add(m[0].replace(/[.,;:]+$/, ""));
    if (found.size >= 8) break;
  }
  return [...found];
}

function setBrowserUrl(url) {
  ui.browserUrl = url;
  save("gui-browser-url", url);
}

function renderBrowser() {
  if (panelBody.querySelector(".browser") && panelBody.dataset.url === ui.browserUrl) return;
  panelBody.dataset.url = ui.browserUrl;
  const urls = active ? seenUrls(active) : [];
  const remote = active && active.machine !== LOCAL;
  panelBody.innerHTML = `<div class="browser"><form class="url-bar">
      <button type="button" class="icon" data-nav="back" title="Back">${icon("arrow-left")}</button><button type="button" class="icon" data-nav="forward" title="Forward">${icon("arrow-right")}</button>
      <button type="button" class="icon" data-nav="reload" title="Reload">${icon("rotate-cw")}</button>
      <input name="url" value="${esc(ui.browserUrl)}" placeholder="Search or enter URL" spellcheck="false" autocomplete="off">
      <button type="button" class="icon" data-nav="external" title="Open in system browser">${icon("external-link")}</button></form>
    ${ui.browserUrl
      ? `<iframe src="${esc(ui.browserUrl)}" title="Preview"></iframe>`
      : `<div class="browser-empty">${icon("globe", "big")}<p class="title">No preview yet</p><p>Type a URL above, or start a dev server in this thread.</p>
        ${urls.length ? `<p class="group">Local servers</p><div class="servers">${urls.map((u) => `<button type="button" data-url="${esc(u)}">${icon("radio-tower")}<span><b>Listening</b><span>${esc(u.replace(/^https?:\/\//, ""))}</span></span></button>`).join("")}</div>` : ""}
        ${remote ? `<p class="hint">This thread runs on ${esc(machineName(active.machine))}, so its local servers are that machine's, not this device's.</p>` : ""}</div>`}</div>`;
  const form = panelBody.querySelector(".url-bar");
  form.onsubmit = (e) => {
    e.preventDefault();
    let url = form.url.value.trim();
    if (url && !/^https?:\/\//.test(url)) {
      url = /^(localhost|127\.|0\.0\.0\.0|\[::1\])/.test(url) ? `http://${url}` : /\.\w{2,}($|\/)/.test(url) ? `https://${url}` : `https://duckduckgo.com/?q=${encodeURIComponent(url)}`;
    }
    setBrowserUrl(url);
    renderBrowser();
  };
  form.onclick = (e) => {
    const nav = e.target.closest("[data-nav]")?.dataset.nav;
    const frame = panelBody.querySelector("iframe");
    if (nav === "external" && ui.browserUrl) openOutside(ui.browserUrl);
    else if (nav === "reload" && frame) frame.src = frame.src;
    else if ((nav === "back" || nav === "forward") && frame) {
      try {
        frame.contentWindow.history[nav]();
      } catch {}
    }
  };
  for (const b of panelBody.querySelectorAll("[data-url]")) {
    b.onclick = () => {
      setBrowserUrl(b.dataset.url);
      renderBrowser();
    };
  }
}

// ── Pull request ───────────────────────────────────────────────────────────

const checkState = (c) => {
  const v = (c.conclusion || c.state || c.status || "").toUpperCase();
  if (["SUCCESS", "NEUTRAL", "SKIPPED"].includes(v)) return "ok";
  if (["FAILURE", "ERROR", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE"].includes(v)) return "fail";
  return "pending";
};
const CHECK_ICONS = { ok: "circle-check", fail: "circle-alert", pending: "circle-dashed" };

async function loadPr(tab, pr) {
  const key = `${tab.key}:${pr.url}`;
  ui.pr = { key, loading: true };
  renderPrPanel();
  const call = { cmd: "pull_request", tab: tab.view.id, pr: pr.url };
  try {
    const data = tab.machine === LOCAL ? await invoke("machine_call_slow", { call }) : await callOn(tab.machine, call);
    ui.pr = { key, data, at: Date.now() };
  } catch (e) {
    ui.pr = { key, error: String(e) };
  }
  if (ui.panel === "pr" && ui.panelOpen) renderPrPanel();
}

function renderPrPanel() {
  const tab = active;
  const pr = tab && prOf(tab);
  if (!pr) {
    panelBody.innerHTML = `<p class="panel-empty">No pull request in this thread yet. When the session opens one with <code>gh pr create</code>, it shows here.</p>`;
    return;
  }
  const key = `${tab.key}:${pr.url}`;
  if (ui.pr?.key !== key) return loadPr(tab, pr);
  if (ui.pr.loading) return (panelBody.innerHTML = `<p class="panel-empty">Loading #${pr.number}…</p>`);
  if (ui.pr.error) {
    panelBody.innerHTML = `<div class="panel-empty"><p>${esc(ui.pr.error)}</p><button class="outline" data-open-url="${esc(pr.url)}">${icon("external-link")}Open #${pr.number} on GitHub</button></div>`;
    return;
  }
  const d = ui.pr.data;
  const state = d.isDraft ? "draft" : (d.state ?? "open").toLowerCase();
  const checks = d.statusCheckRollup ?? [];
  const failed = checks.filter((c) => checkState(c) === "fail").length;
  const passed = checks.filter((c) => checkState(c) === "ok").length;
  const files = ui.prTab === "code" ? parseDiff(d.diff ?? "") : [];
  const head = `<div class="pr-head">
      <div class="crumbs"><span class="muted">${esc(pr.repo)}</span><button class="num ${state}" data-open-url="${esc(d.url)}" title="Open on GitHub">#${d.number}${icon("external-link")}</button>
        <span class="spacer"></span><button class="icon" data-refresh-pr title="Refresh">${icon("rotate-cw")}</button><span class="state ${state}">${icon("git-pull-request")}${esc(state[0].toUpperCase() + state.slice(1))}</span></div>
      <h3>${esc(d.title)}</h3>
      <p class="muted">${esc(d.author?.login ?? "")}${d.createdAt ? ` · opened ${esc(ago(Date.parse(d.createdAt) / 1000))} ago` : ""}</p>
      <p class="refs"><code>${esc(d.baseRefName)}</code>${icon("arrow-left")}<code>${esc(d.headRefName)}</code><span class="spacer"></span>${icon("file-diff")}${d.changedFiles}<span class="add">+${d.additions}</span><span class="del">-${d.deletions}</span></p>
      <div class="tabs"><span class="seg"><button data-pr-tab="summary" class="${ui.prTab === "summary" ? "on" : ""}">Summary</button><button data-pr-tab="code" class="${ui.prTab === "code" ? "on" : ""}">Code</button></span>
        <span class="checks-line ${failed ? "fail" : passed === checks.length ? "ok" : "pending"}">${checks.length ? `${icon(CHECK_ICONS[failed ? "fail" : passed === checks.length ? "ok" : "pending"])}${passed} passed${failed ? ` / ${failed} failed` : ""}` : "No checks"}</span>
        ${d.reviewDecision ? `<span class="muted">${esc(d.reviewDecision.toLowerCase().replace(/_/g, " "))}</span>` : ""}</div></div>`;
  const summary = `<div class="pr-body">
      <h4>Description</h4><div class="md">${d.body ? markdown(d.body) : `<p class="muted"><em>No description provided.</em></p>`}</div>
      ${checks.length ? `<h4>Checks</h4><ul class="checks">${checks.map((c) => `<li class="${checkState(c)}">${icon(CHECK_ICONS[checkState(c)])}<span>${esc(c.name || c.context || "check")}</span><span class="muted">${esc((c.conclusion || c.status || c.state || "").toLowerCase())}</span>${c.detailsUrl || c.targetUrl ? `<a href="${esc(c.detailsUrl || c.targetUrl)}">Details</a>` : ""}</li>`).join("")}</ul>` : ""}
      ${(d.reviews ?? []).length ? `<h4>Reviews</h4><ul class="reviews">${d.reviews.map((r) => `<li><b>${esc(r.author?.login ?? "")}</b> <span class="muted">${esc((r.state ?? "").toLowerCase().replace(/_/g, " "))}</span>${r.body ? `<div class="md">${markdown(r.body)}</div>` : ""}</li>`).join("")}</ul>` : ""}
      ${(d.comments ?? []).length ? `<h4>Comments (${d.comments.length})</h4><ul class="reviews">${d.comments.map((c) => `<li><b>${esc(c.author?.login ?? "")}</b><div class="md">${markdown(c.body ?? "")}</div></li>`).join("")}</ul>` : ""}</div>`;
  const code = `${diffToolbar(d.additions, d.deletions, `<span class="muted">${d.changedFiles} file${d.changedFiles === 1 ? "" : "s"}</span>`)}
    <div class="diff ${ui.diffView}${ui.diffWrap ? " wrap" : ""}">${filesHtml(files, `pr${d.number}`)}${d.truncated ? `<p class="panel-empty">This diff exceeds the size limit, so its end is cut.</p>` : ""}</div>`;
  panelBody.innerHTML = `<div class="pr">${head}${ui.prTab === "code" ? code : summary}</div>`;
}

// ── Usage, keys, start ─────────────────────────────────────────────────────

function renderGuiUsage() {
  const meter = document.getElementById("gui-usage");
  meter.replaceChildren(...[...usageMeter.children].map((c) => c.cloneNode(true)));
  meter.title = usageMeter.title;
}

function stepThread(step) {
  const order = [...threadList.querySelectorAll(".thread-row")].map((r) => tabByKey(r.dataset.tab));
  const i = order.indexOf(active);
  if (order.length > 1) activate(order[(i + step + order.length) % order.length]);
}

window.addEventListener("keydown", (e) => {
  if (!guiOn() || settingsOpen() || pickerOpen()) return;
  const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!mod) return;
  const key = e.key.toLowerCase();
  if (key === "b" && e.altKey) togglePanel();
  else if (e.altKey) return;
  else if (key === "j" && !e.shiftKey) toggleDrawer();
  else if (key === "b" && !e.shiftKey) toggleSidebar();
  else if (key === "d" && !e.shiftKey) ui.panelOpen && ui.panel === "diff" ? closePanel() : openPanel("diff");
  else if (key === "n" && !e.shiftKey) toggleProjects();
  else if (key === "e" && !e.shiftKey) setMode("terminal");
  else if (e.shiftKey && (e.key === "[" || e.key === "{")) stepThread(-1);
  else if (e.shiftKey && (e.key === "]" || e.key === "}")) stepThread(1);
  else return;
  e.preventDefault();
  e.stopPropagation();
}, true);
window.addEventListener("keydown", (e) => {
  if (guiOn() || !(IS_MAC ? e.metaKey : e.ctrlKey) || e.key.toLowerCase() !== "e" || e.shiftKey || e.altKey) return;
  setMode("gui");
  e.preventDefault();
  e.stopPropagation();
}, true);

document.getElementById("mode-gui").onclick = () => setMode("gui");
document.getElementById("mode-terminal").onclick = () => setMode("terminal");
document.getElementById("gui-settings").onclick = () => toggleSettings();
document.getElementById("gui-minimize").onclick = () => setView("pill");

document.body.dataset.mode = load("mode") === "terminal" ? "terminal" : "gui";
document.body.dataset.panel = "closed";
document.body.dataset.drawer = "closed";
document.body.dataset.sidebar = ui.sidebarOpen ? "open" : "closed";
