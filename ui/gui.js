const threadList = document.getElementById("thread-list");
const threadView = document.getElementById("thread");
const composer = document.getElementById("composer");
const prompt = document.getElementById("prompt");
const approval = document.getElementById("approval");
const panelBody = document.getElementById("panel-body");

const guiOn = () => document.body.dataset.mode === "gui";
const POLL_MS = 1200;
const PANELS = ["diff", "browser", "pr"];
const SETTLE_AFTER_S = 3 * 86400;
const narrow = (px) => matchMedia(`(max-width: ${px}px)`).matches;

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

async function fitWindowTo(mode) {
  const size = (await win.innerSize()).toLogical(await win.scaleFactor());
  let sizes = {};
  try {
    sizes = JSON.parse(load("mode-sizes")) ?? {};
  } catch {}
  sizes[mode === "gui" ? "terminal" : "gui"] = { width: size.width, height: size.height };
  const want = sizes[mode] ?? (mode === "gui" ? { width: Math.max(size.width, 1180), height: Math.max(size.height, 760) } : size);
  const fit = { width: Math.min(want.width, screen.availWidth - 40), height: Math.min(want.height, screen.availHeight - 40) };
  sizes[mode] = fit;
  save("mode-sizes", JSON.stringify(sizes));
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
const projectName = (tab) => (threadOf(tab).workspace?.project ?? tab.view.cwd).split("/").filter(Boolean).pop() || "/";
const firstPrompt = (tab) => threadOf(tab).items.find((i) => i.kind === "user")?.text.split("\n")[0];
const threadTitle = (tab) => tab.view.title || firstPrompt(tab) || "New thread";
const prOf = (tab) => [...threadOf(tab).items].reverse().find((i) => i.kind === "pr");
/** claude's hooks report no answer to a permission menu, so a permission notice with no menu on
 * screen means the turn went on. It sends "waiting for your input" a while after a turn ends. */
function phaseOf(tab) {
  if (tab.menu) return "asking";
  const status = statusOf(tab);
  const message = tab.view.message ?? "";
  if (status !== "needs-input") return status;
  if (/permission/i.test(message)) return "working";
  return /waiting for your input/i.test(message) ? "done" : "input";
}
const turnLive = (tab) => ["working", "asking", "input"].includes(phaseOf(tab));
const unseenDone = (tab) => phaseOf(tab) === "done" && tab !== active && (tab.view.updated ?? 0) > (seenAt[tab.key] ?? 0);
const settled = (tab) => !!tab.view.updated && ["idle", "done"].includes(statusOf(tab)) && tab !== active && Date.now() / 1000 - tab.view.updated > SETTLE_AFTER_S;

function markThreadSeen(tab) {
  if (!tab || !guiOn() || document.hidden) return;
  seenAt[tab.key] = tab.view.updated ?? 0;
  save("gui-seen", JSON.stringify(seenAt));
}

function statusSlot(tab) {
  const phase = phaseOf(tab);
  if (phase === "asking") return { icon: "shield-question", text: "Approval", tone: "warning" };
  if (phase === "working") return { icon: "circle-dashed", text: "Working", tone: "info" };
  if (phase === "input") return { icon: "circle-alert", text: "Input", tone: "input" };
  if (phase === "failed") return tab.view.resume_at ? { icon: "circle-alert", text: "Limited", tone: "warning" } : { icon: "circle-alert", text: "Failed", tone: "error" };
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
  row.classList.toggle("receded", tab !== active && !slot);
  row.classList.toggle("working", phaseOf(tab) === "working" && tab !== active);
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
document.getElementById("new-thread").onclick = () => {
  if (narrow(768)) toggleSidebar(false);
  toggleProjects();
};

function toggleSidebar(open = !ui.sidebarOpen) {
  ui.sidebarOpen = open;
  if (!narrow(768)) save("gui-sidebar", String(open));
  document.body.dataset.sidebar = open ? "open" : "closed";
}
for (const id of ["sidebar-toggle", "chat-sidebar-toggle"]) document.getElementById(id).onclick = () => toggleSidebar();
document.getElementById("workspace").addEventListener("click", (e) => {
  if (e.target.id === "workspace" && narrow(768) && ui.sidebarOpen) toggleSidebar(false);
});

function turnsOf(items) {
  const turns = [];
  const results = new Map(items.filter((i) => i.kind === "result").map((r) => [r.tool, r]));
  let turn = null;
  for (const item of items) {
    if (item.kind === "result" || item.kind === "model") continue;
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

function duration(ms) {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`;
  const total = Math.round(ms / 1000);
  if (total < 60) return `${total}s`;
  const [h, m, s] = [Math.floor(total / 3600), Math.floor((total % 3600) / 60), total % 60];
  return [h && `${h}h`, m && `${m}m`, s && `${s}s`].filter(Boolean).join(" ");
}

const WEB_URL = /^https?:\/\//i;
/** Assistant text and pull request comments come from others, so only web links render, and an image
 * is a link to it rather than a request made on sight. */
marked.use({
  gfm: true,
  renderer: {
    html: ({ text }) => esc(text),
    link({ href, tokens }) {
      const text = this.parser.parseInline(tokens);
      return WEB_URL.test(href) ? `<a href="${esc(href)}">${text}</a>` : text;
    },
    image: ({ href, text }) => (WEB_URL.test(href) ? `<a href="${esc(href)}">${esc(text || href)}</a>` : esc(text)),
  },
});
const rendered = new Map();
function markdown(text) {
  let html = rendered.get(text);
  if (html === undefined) {
    if (rendered.size > 4000) rendered.clear();
    html = marked.parse(text);
    rendered.set(text, html);
  }
  return html;
}

const TOOL_ICONS = { Bash: "terminal", Read: "eye", Edit: "square-pen", MultiEdit: "square-pen", Write: "square-pen", NotebookEdit: "square-pen", Grep: "search", Glob: "search", WebFetch: "globe", WebSearch: "globe", Agent: "bot", Task: "bot" };
const shortPath = (path, cwd) => (cwd && path.startsWith(`${cwd}/`) ? path.slice(cwd.length + 1) : tilde(path));

function toolLabel(step, cwd) {
  const what = /^(Read|Write|Edit|MultiEdit|NotebookEdit)$/.test(step.name) ? shortPath(step.summary, cwd) : step.summary;
  const verb = { Read: "Read", Edit: "Edited", MultiEdit: "Edited", Write: "Wrote", Grep: "Searched", Glob: "Listed", WebFetch: "Fetched", WebSearch: "Searched the web for", Agent: "Delegated", Task: "Delegated", Skill: "Used skill" }[step.name];
  if (step.name === "Bash") return what || step.detail.split("\n")[0];
  return verb ? `${verb} ${what}` : `${step.name.replace(/^mcp__([^_]+)__/, "$1 · ")}${what ? ` ${what}` : ""}`;
}

function toolHtml(step, cwd, open) {
  const name = /^mcp__/.test(step.name) ? "wrench" : TOOL_ICONS[step.name] ?? "zap";
  const state = !step.result ? "running" : step.result.error ? "error" : "ok";
  const body = [step.detail && `<pre class="input">${esc(step.detail)}</pre>`, step.result?.text && `<pre>${esc(step.result.text)}</pre>`].filter(Boolean).join("");
  return `<details class="tool" data-fold="${esc(step.id)}" data-state="${state}"${body ? "" : " data-empty"}${open.has(step.id) ? " open" : ""}><summary>${icon(name, "kind")}<span class="label">${esc(toolLabel(step, cwd))}</span>
    ${state === "error" ? icon("x", "failed") : ""}${body ? icon("chevron-right", "chev") : ""}</summary>${body}</details>`;
}

const endOfSecond = (seconds) => seconds * 1000 + 999;

function changesByTurn(history, turns) {
  const found = new Map();
  for (const h of history ?? []) {
    const turn = turns.findLast((t) => t.user && t.start <= endOfSecond(h.ended));
    if (turn && !found.has(turn)) found.set(turn, h);
  }
  return found;
}

const sum = (files, key) => files.reduce((n, f) => n + Number(f[key]), 0);

function changesHtml(change, open) {
  const n = change.files.length;
  const fold = `changes:${change.ended}`;
  return `<details class="changes" data-fold="${fold}"${open.has(fold) ? " open" : ""}><summary>${icon("chevron-right", "chev")}<span class="count">${n} changed file${n === 1 ? "" : "s"}</span>
      <span class="add">+${sum(change.files, "added")}</span><span class="del">−${sum(change.files, "removed")}</span><span class="show">Show files</span>
      <button class="open-diff" data-ended="${change.ended}">${icon("file-diff")}Open diff</button></summary>
    <ul>${change.files.map((f) => `<li data-ended="${change.ended}" data-file="${esc(f.path)}"><span class="path">${esc(f.path)}</span><span class="add">+${f.added}</span><span class="del">−${f.removed}</span></li>`).join("")}</ul></details>`;
}

function turnHtml(tab, turn, change, live) {
  const t = threadOf(tab);
  const user = turn.user
    ? `<div class="user"><div class="bubble">${esc(turn.user.text)}${turn.user.images ? `<span class="images">${turn.user.images} image${turn.user.images > 1 ? "s" : ""}</span>` : ""}</div></div>`
    : "";
  const steps = turn.work.map((s) => (s.kind === "tool" ? toolHtml(s, tab.view.cwd, t.open) : `<div class="narration md">${markdown(s.text)}</div>`)).join("");
  let fold = "";
  if (live) {
    const running = [...turn.steps].reverse().find((s) => s.kind === "tool");
    fold = `<details class="work live" data-turn="${esc(turn.id)}"${t.open.has(turn.id) ? " open" : ""}>
        <summary><span class="label">Working for <span class="ticker" data-since="${turn.start || Date.now()}">${duration(turn.start ? Date.now() - turn.start : 0)}</span></span></summary>
        <div class="steps">${steps}</div></details>
      ${running && !t.open.has(turn.id) ? `<div class="live-tool">${toolHtml(running, tab.view.cwd, t.open)}</div>` : ""}`;
  } else if (turn.work.length) {
    fold = `<details class="work" data-turn="${esc(turn.id)}"${t.open.has(turn.id) ? " open" : ""}>
        <summary><span class="label">Worked for ${duration(turn.end - turn.start)}</span>${icon("chevron-right", "chev")}</summary>
        <div class="steps">${steps}</div></details>`;
  }
  const said = (s) => (s ? `<div class="assistant md">${markdown(s.text)}</div>` : "");
  return `<section class="turn">${user}${turn.notes.map((n) => `<p class="note">${esc(n.text)}</p>`).join("")}${fold}${said(turn.lead)}${said(turn.answer)}
    ${turn.prs.map((p) => `<p class="pr-line"><button class="pr-link" data-pr="${esc(p.url)}">${icon("github")}PR #${p.number}</button><span class="muted">${esc(p.repo)}</span></p>`).join("")}
    ${change ? changesHtml(change, t.open) : ""}</section>`;
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
  const working = turnLive(tab);
  const changes = changesByTurn(t.history, turns);
  const body = turns.length
    ? turns.map((turn, i) => turnHtml(tab, turn, changes.get(turn), working && i === turns.length - 1)).join("")
    : `<div class="blank"><p>${t.loaded ? "Send a message to start the conversation." : t.error ? `This thread couldn't be read: ${esc(t.error)}` : "Loading…"}</p><p class="hint">${esc(tilde(tab.view.cwd))}</p></div>`;
  const key = JSON.stringify([tab.key, tab.view.cwd, t.loaded, t.error, t.session, working, t.items.length, !!t.history, [...t.open]]);
  if (threadView.dataset.key === key) return renderComposer(tab);
  const stick = threadView.dataset.tab !== tab.key || threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight < 40;
  threadView.innerHTML = `<div class="lane">${body}</div>`;
  threadView.dataset.key = key;
  threadView.dataset.tab = tab.key;
  if (stick) threadView.scrollTop = threadView.scrollHeight;
  renderComposer(tab);
}

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
  const fold = e.target.dataset?.turn ?? e.target.dataset?.fold;
  if (!fold || !active) return;
  const open = threadOf(active).open;
  if (e.target.open) open.add(fold);
  else open.delete(fold);
  if (e.target.dataset.turn) threadView.dataset.key = "";
}, true);

const openOutside = (url) => WEB_URL.test(url) && (TAURI ? invoke("open_url", { url }).catch(console.warn) : window.open(url, "_blank", "noopener"));

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
    t.items.push(...page.items);
    t.next = page.next;
    t.loaded = true;
    if (grew && guiOn()) {
      if (tab === active) renderThread();
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

function renderUnsupported() {
  threadView.dataset.key = "";
  threadView.innerHTML = `<div class="blank"><p>This thread can't be shown here yet: ${esc(machineName(active.machine))} or the hub runs an older pinned.</p>
    <p class="hint">Update pinned there, or use the terminal view.</p><button class="primary" data-mode-terminal>Terminal view</button></div>`;
}

async function fetchWorkspace(tab) {
  const t = threadOf(tab);
  if (t.workspaceFor === tab.view.cwd) return;
  t.workspaceFor = tab.view.cwd;
  t.workspace = await callOn(tab.machine, { cmd: "workspace", tab: tab.view.id }).catch(() => null);
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
    if (finished) Object.assign(threadOf(tab), { history: null, workspaceFor: null });
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
    tab.menu = terminalMenu(tab);
    if (!t.loaded) fetchThread(tab);
    fetchWorkspace(tab);
    if (tab === active && !t.history) fetchHistory(tab);
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
  for (const tab of tabs) if (tab === active || turnLive(tab)) fetchThread(tab);
}, POLL_MS);
setInterval(() => guiOn() && renderSidebar(), 60 * 1000);
document.addEventListener("visibilitychange", () => markThreadSeen(active));

const MODELS_MENU = [
  ["default", "Default", "The model your Claude Code settings pick"],
  ["opus", "Opus", "Most capable for complex work"],
  ["sonnet", "Sonnet", "Fast and capable for everyday tasks"],
  ["haiku", "Haiku", "Fastest for quick answers"],
];

function renderComposer(tab) {
  const working = !!tab && phaseOf(tab) === "working";
  composer.dataset.working = working;
  composer.dataset.empty = !prompt.value.trim();
  document.querySelector("#send").innerHTML = icon(working && prompt.value.trim() ? "list-plus" : "arrow-up");
  document.querySelector("#send").title = working ? "Queue message (Enter)" : "Send (Enter)";
  renderApproval(tab);
  document.querySelector("#model-button .name").textContent = tab ? modelName(tab) : "Default";
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

/** The menu claude shows in its terminal, if one is up: the folder trust check, a permission
 * request, or any other choice. It is a column of options at the bottom of the screen, one marked
 * ❯, numbered or not, under the text it asks, which starts after the rule claude draws above. */
function terminalMenu(tab) {
  const buffer = tab.term.buffer.active;
  const bottom = buffer.baseY + tab.term.rows;
  const lines = [];
  for (let i = buffer.baseY; i < bottom; i++) {
    const row = buffer.getLine(i);
    const text = row?.translateToString(true) ?? "";
    if (row?.isWrapped && lines.length) lines[lines.length - 1] += text;
    else lines.push(text);
  }
  let end = lines.length - 1;
  while (end >= 0 && (!lines[end].trim() || /\b(Esc|Enter|Tab)\b/.test(lines[end])) && !CURSOR.test(lines[end])) end--;
  let cursor = end;
  while (cursor >= Math.max(0, end - 12) && !CURSOR.test(lines[cursor])) cursor--;
  if (cursor < Math.max(0, end - 12)) return null;
  const at = lines[cursor].search(CURSOR) + 2;
  const option = (l) => l.length > at && !l.slice(0, at).replace(CURSOR, "").trim() && l[at] !== " ";
  const continued = (l) => l.length > at && !l.slice(0, at + 1).trim();
  let first = cursor;
  while (first > 0 && (option(lines[first - 1]) || continued(lines[first - 1]))) first--;
  while (!option(lines[first])) first++;
  const options = [];
  for (let k = first; k <= end; k++) {
    if (option(lines[k])) options.push({ n: options.length + 1, label: lines[k].slice(at).trim().replace(/^\d+\.\s+/, ""), chosen: CURSOR.test(lines[k]) });
    else if (continued(lines[k]) && options.length) options.at(-1).label += ` ${lines[k].trim()}`;
    else return null;
  }
  if (options.length < 2) return null;
  const rule = lines.slice(0, first).findLastIndex((l) => /^\s*[─━]{8,}/.test(l));
  const above = lines.slice(rule + 1, first).map((l) => l.trim()).filter(Boolean);
  const dashed = (l) => /^[─━╌┄┈═-]+$/.test(l);
  const boxStart = above.findIndex(dashed);
  const boxEnd = above.findLastIndex(dashed);
  const boxed = (i) => boxStart >= 0 && i > boxStart && i < boxEnd;
  const box = above.filter((l, i) => boxed(i)).join("\n");
  const question = above.filter((l, i) => !dashed(l) && !boxed(i)).join("\n");
  return { question, box, options };
}
const CURSOR = /[❯›](?= )/;

function watchMenu(tab) {
  let queued = false;
  tab.term.onWriteParsed(() => {
    if (queued || !guiOn()) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      const menu = terminalMenu(tab);
      if (JSON.stringify(menu) === JSON.stringify(tab.menu ?? null)) return;
      tab.menu = menu;
      if (tab === active) renderComposer(tab);
      renderSidebar();
    });
  });
}

const ARROW_DOWN = "\x1b[B";
const ARROW_UP = "\x1b[A";

function chooseOption(tab, n, label) {
  const menu = terminalMenu(tab);
  if (menu?.options[n - 1]?.label !== label) {
    tab.menu = menu;
    return renderComposer(tab);
  }
  const from = menu.options.find((o) => o.chosen)?.n ?? 1;
  const keys = (n > from ? ARROW_DOWN : ARROW_UP).repeat(Math.abs(n - from)) + "\r";
  tab.menu = null;
  renderComposer(tab);
  renderSidebar();
  return callOn(tab.machine, { cmd: "write_input", tab: tab.view.id, data: keys });
}

/** claude heads a permission menu with what it wants to run, such as "Bash command", then a
 * description and its own tips, the command in a dashed box, and the question last, such as "Do you
 * want to proceed?". */
function renderApproval(tab) {
  const menu = tab?.menu;
  approval.hidden = !menu;
  if (!menu) return (approval.dataset.key = "");
  const key = JSON.stringify([tab.key, menu]);
  if (approval.dataset.key === key) return;
  approval.dataset.key = key;
  const lines = menu.question.split("\n");
  const permission = lines.length > 1 && /proceed|allow|make this edit|permission/i.test(lines.at(-1));
  approval.dataset.kind = permission ? "approval" : "input";
  const kind = !permission ? "Claude is asking" : /command/i.test(lines[0]) ? "Command approval" : /file|edit/i.test(lines[0]) ? "File change approval" : "Permission approval";
  const what = permission ? lines.at(-1) : menu.question;
  const middle = permission ? lines.slice(1, -1).filter((l) => !/^Tip:/.test(l)) : [];
  const detail = menu.box || middle.join("\n");
  const about = menu.box ? middle.join("\n") : "";
  const options = menu.options.map((o) => `<button type="button" data-option="${o.n}" class="${o.chosen ? "primary" : "outline"}">${esc(o.label)}</button>`);
  approval.innerHTML = `${icon(permission ? "shield" : "circle-alert", "lead")}<div class="body"><p class="kind">${esc(kind)}</p>
      <p class="what">${esc(what)}</p>${about ? `<p class="about">${esc(about)}</p>` : ""}${detail ? `<pre>${esc(detail)}</pre>` : ""}
      <div class="options">${options.join("")}</div></div>
    <div class="actions"><button type="button" data-show-terminal class="outline">Terminal</button></div>`;
}

approval.addEventListener("click", (e) => {
  const option = e.target.closest("[data-option]");
  if (option && active) return chooseOption(active, Number(option.dataset.option), option.textContent);
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
  } else if (e.key === "Escape" && active && phaseOf(active) === "working" && !prompt.value) {
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
  // Typing into a menu would filter its options or pick one, so the prompt waits for an answer.
  if (tab.menu) return approval.animate([{ transform: "translateX(-4px)" }, { transform: "translateX(4px)" }, { transform: "none" }], 180);
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

function insertPaths(paths) {
  const text = paths.map((p) => p.replace(/[^\w@%+=:,./-]/g, "\\$&")).join(" ");
  prompt.value = `${prompt.value}${prompt.value && !prompt.value.endsWith(" ") ? " " : ""}${text} `;
  autosize();
  prompt.focus();
}
document.getElementById("composer-attach").onclick = () => document.getElementById("attach").click();

const composerMenu = document.getElementById("composer-menu");
function openMenu(anchor, entries) {
  composerMenu.replaceChildren(...entries.map(([label, hint, run, cls]) => {
    const b = document.createElement("button");
    b.type = "button";
    b.className = cls ?? "";
    b.innerHTML = `<span>${esc(label)}</span>${hint ? `<span class="hint">${esc(hint)}</span>` : ""}`;
    b.onclick = () => {
      composerMenu.hidden = true;
      run();
    };
    return b;
  }));
  composerMenu.hidden = false;
  composerMenu.style.left = `${anchor.offsetLeft}px`;
}
window.addEventListener("mousedown", (e) => {
  if (!composerMenu.hidden && !e.target.closest("#composer-menu, #model-button, #more-button")) composerMenu.hidden = true;
}, true);

const sendSlash = (text) => active && callOn(active.machine, { cmd: "prompt", tab: active.view.id, text }).then(() => setTimeout(() => fetchThread(active), 400));
document.getElementById("model-button").onclick = (e) =>
  openMenu(e.currentTarget, MODELS_MENU.map(([id, label, hint]) => [label, hint, () => {
    sendSlash(`/model ${id}`);
    threadOf(active).picked = label;
    renderComposer(active);
  }]));

function modelName(tab) {
  const t = threadOf(tab);
  return t.picked ?? [...t.items].reverse().find((i) => i.kind === "model")?.name ?? "Default";
}

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

function toggleDrawer(open = !ui.drawer) {
  ui.drawer = open;
  document.body.dataset.drawer = open ? "open" : "closed";
  if (open && active) refit(active);
  focusInput();
}
document.getElementById("toggle-terminal").onclick = () => toggleDrawer();

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

let shownPanel = "";
/** Sets the panel's HTML only when it changed, so a render that changes nothing keeps its scroll. */
function showPanel(html) {
  if (html === shownPanel && panelBody.isConnected) return;
  shownPanel = html;
  panelBody.innerHTML = html;
}

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
  if (!tab) return showPanel(`<p class="panel-empty">Select a thread to inspect turn diffs.</p>`);
  if (threadOf(tab).workspace?.repo === false) {
    return showPanel(`<p class="panel-empty">Turn diffs are unavailable because this folder is not a git repository.</p>`);
  }
  if (!history) {
    showPanel(`<p class="panel-empty">Loading changes…</p>`);
    return fetchHistory(tab);
  }
  if (!history.length) return showPanel(`<p class="panel-empty">No completed turns with changes yet.</p>`);
  const chosen = history.find((h) => h.ended === ui.diffTurn) ?? history.at(-1);
  const scope = `<label class="scope"><select id="diff-turn">${history.map((h, i) => `<option value="${h.ended}"${h === chosen ? " selected" : ""}>Turn ${i + 1}</option>`).join("")}</select>${icon("chevron-down")}</label>`;
  const files = parseDiff(chosen.patch);
  showPanel(`${diffToolbar(sum(files, "added"), sum(files, "removed"), scope)}
    <div class="diff ${ui.diffView}${ui.diffWrap ? " wrap" : ""}">${filesHtml(files, chosen.ended)}
    ${chosen.truncated ? `<p class="panel-empty">This diff exceeds the size limit, so its end is cut.</p>` : ""}</div>`);
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
  showPanel(`<div class="browser"><form class="url-bar">
      <button type="button" class="icon" data-nav="back" title="Back">${icon("arrow-left")}</button><button type="button" class="icon" data-nav="forward" title="Forward">${icon("arrow-right")}</button>
      <button type="button" class="icon" data-nav="reload" title="Reload">${icon("rotate-cw")}</button>
      <input name="url" value="${esc(ui.browserUrl)}" placeholder="Search or enter URL" spellcheck="false" autocomplete="off">
      <button type="button" class="icon" data-nav="external" title="Open in system browser">${icon("external-link")}</button></form>
    ${ui.browserUrl
      ? `<iframe src="${esc(ui.browserUrl)}" title="Preview" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads"></iframe>`
      : `<div class="browser-empty">${icon("globe", "big")}<p class="title">No preview yet</p><p>Type a URL above, or start a dev server in this thread.</p>
        ${urls.length ? `<p class="group">Local servers</p><div class="servers">${urls.map((u) => `<button type="button" data-url="${esc(u)}">${icon("radio-tower")}<span><b>Listening</b><span>${esc(u.replace(/^https?:\/\//, ""))}</span></span></button>`).join("")}</div>` : ""}
        ${remote ? `<p class="hint">This thread runs on ${esc(machineName(active.machine))}, so its local servers are that machine's, not this device's.</p>` : ""}</div>`}</div>`);
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
  try {
    const data = await callOn(tab.machine, { cmd: "pull_request", tab: tab.view.id, pr: pr.url });
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
    showPanel(`<p class="panel-empty">No pull request in this thread yet. When the session opens one with <code>gh pr create</code>, it shows here.</p>`);
    return;
  }
  const key = `${tab.key}:${pr.url}`;
  if (ui.pr?.key !== key) return loadPr(tab, pr);
  if (ui.pr.loading) return showPanel(`<p class="panel-empty">Loading #${pr.number}…</p>`);
  if (ui.pr.error) {
    showPanel(`<div class="panel-empty"><p>${esc(ui.pr.error)}</p><button class="outline" data-open-url="${esc(pr.url)}">${icon("external-link")}Open #${pr.number} on GitHub</button></div>`);
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
  showPanel(`<div class="pr">${head}${ui.prTab === "code" ? code : summary}</div>`);
}

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

/** Off the Mac the chords are Ctrl ones, which a shell in the terminal needs: Ctrl+E ends the line.
 * With Option a key goes by position, since Option turns B into ∫ on a Mac. */
window.addEventListener("keydown", (e) => {
  if (settingsOpen() || pickerOpen()) return;
  const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!mod) return;
  const key = (e.altKey ? e.code.replace(/^Key/, "") : e.key).toLowerCase();
  const switchView = key === "e" && !e.altKey && e.shiftKey === !IS_MAC;
  if (switchView) setMode(guiOn() ? "terminal" : "gui");
  else if (!guiOn() || (!IS_MAC && e.target.closest?.("#terms"))) return;
  else if (key === "b" && e.altKey) togglePanel();
  else if (e.altKey) return;
  else if (key === "j" && !e.shiftKey) toggleDrawer();
  else if (key === "b" && !e.shiftKey) toggleSidebar();
  else if (key === "d" && !e.shiftKey) ui.panelOpen && ui.panel === "diff" ? closePanel() : openPanel("diff");
  else if (key === "n" && !e.shiftKey) toggleProjects();
  else if (e.shiftKey && (e.key === "[" || e.key === "{")) stepThread(-1);
  else if (e.shiftKey && (e.key === "]" || e.key === "}")) stepThread(1);
  else return;
  e.preventDefault();
  e.stopPropagation();
}, true);

document.getElementById("mode-gui").onclick = () => setMode("gui");
document.getElementById("mode-terminal").onclick = () => setMode("terminal");
document.getElementById("gui-settings").onclick = () => toggleSettings();
document.getElementById("gui-minimize").onclick = () => setView("pill");

document.body.dataset.mode = load("mode") === "terminal" ? "terminal" : "gui";
const firstLaunch = !load("mode-sizes");
if (TAURI && guiOn() && firstLaunch) fitWindowTo("gui").catch(console.warn);
document.body.dataset.panel = "closed";
document.body.dataset.drawer = "closed";
document.body.dataset.sidebar = ui.sidebarOpen ? "open" : "closed";
