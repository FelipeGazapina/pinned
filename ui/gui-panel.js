Object.assign(ICON_PATHS, {
  "minimize-2": `<path d="m14 10 7-7"/><path d="M20 10h-6V4"/><path d="m3 21 7-7"/><path d="M4 14h6v6"/>`,
  pilcrow: `<path d="M13 4v16"/><path d="M17 4v16"/><path d="M19 4H9.5a4.5 4.5 0 0 0 0 9H13"/>`,
  "folder-tree": `<path d="M20 10a1 1 0 0 0 1-1V6a1 1 0 0 0-1-1h-2.5a1 1 0 0 1-.8-.4l-.9-1.2A1 1 0 0 0 15 3h-2a1 1 0 0 0-1 1v5a1 1 0 0 0 1 1Z"/><path d="M20 21a1 1 0 0 0 1-1v-3a1 1 0 0 0-1-1h-2.9a1 1 0 0 1-.88-.55l-.42-.85a1 1 0 0 0-.92-.6H13a1 1 0 0 0-1 1v5a1 1 0 0 0 1 1Z"/><path d="M3 5a2 2 0 0 0 2 2h3"/><path d="M3 3v13a2 2 0 0 0 2 2h3"/>`,
  "message-circle": `<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>`,
  "trash-2": `<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/><line x1="10" x2="10" y1="11" y2="17"/><line x1="14" x2="14" y1="11" y2="17"/>`,
  history: `<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l4 2"/>`,
  smartphone: `<rect width="14" height="20" x="5" y="2" rx="2" ry="2"/><path d="M12 18h.01"/>`,
  "rotate-ccw-square": `<path d="M20 9V7a2 2 0 0 0-2-2h-6"/><path d="m15 2-3 3 3 3"/><path d="M20 13v5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h2"/>`,
  "git-commit": `<circle cx="12" cy="12" r="3"/><line x1="3" x2="9" y1="12" y2="12"/><line x1="15" x2="21" y1="12" y2="12"/>`,
  "message-square": `<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>`,
  "git-merge": `<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M6 21V9a9 9 0 0 0 9 9"/>`,
  "square-arrow-out-up-right": `<path d="M21 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h6"/><path d="m21 3-9 9"/><path d="M15 3h6v6"/>`,
  "circle-dot": `<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="1"/>`,
  tag: `<path d="M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z"/><circle cx="7.5" cy="7.5" r=".5" fill="currentColor"/>`,
  users: `<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>`,
});

const SURFACES = {
  diff: { icon: "file-diff", label: "Diff", key: "D" },
  browser: { icon: "globe", label: "Browser", key: "B" },
  pr: { icon: "git-pull-request", label: "Pull request", key: "P" },
};

/** Each thread's right panel: its open surfaces, the one shown, open or closed, maximized, width. */
const panelStates = new Map();
const noPanel = { open: false, surfaces: [], active: null, max: false, width: 0 };
function panelOf(tab) {
  if (!tab) return noPanel;
  if (!panelStates.has(tab.key)) {
    let saved = null;
    try {
      saved = JSON.parse(load(`gui-panel:${tab.key}`));
    } catch {}
    panelStates.set(tab.key, { ...noPanel, surfaces: [], ...saved });
  }
  return panelStates.get(tab.key);
}
const savePanel = (tab) => tab && save(`gui-panel:${tab.key}`, JSON.stringify(panelOf(tab)));
const shownSurface = (tab) => {
  const p = panelOf(tab);
  return p.surfaces.find((s) => s.id === p.active) ?? null;
};
const browserOf = (tab) => (shownSurface(tab)?.kind === "browser" ? shownSurface(tab) : panelOf(tab).surfaces.find((s) => s.kind === "browser"));
const newBrowser = (url = "") => ({ id: `browser:${Date.now().toString(36)}${Math.random().toString(36).slice(2, 5)}`, kind: "browser", url });

function addSurface(tab, surface) {
  const p = panelOf(tab);
  const had = p.surfaces.find((s) => s.id === surface.id);
  if (!had) p.surfaces.push(surface);
  p.active = surface.id;
  p.open = true;
  savePanel(tab);
  renderPanel();
  return had ?? surface;
}

function openSurface(kind) {
  const tab = active;
  if (!tab) return;
  if (kind === "diff") return addSurface(tab, { id: "diff", kind });
  if (kind === "browser") return addSurface(tab, browserOf(tab) ?? newBrowser());
  const pr = shownPr(tab);
  if (pr) return addSurface(tab, { id: `pr:${pr.url}`, kind, url: pr.url });
}

/** Opens the panel on `kind`, as the timeline does: a diff shows the turn it picked. */
function openPanel(kind) {
  if (kind === "diff") ui.diffScope = "turn";
  openSurface(kind);
}

function closePanel() {
  const tab = active;
  if (!tab) return;
  panelOf(tab).open = false;
  savePanel(tab);
  renderPanel();
}
const togglePanel = () => {
  const p = panelOf(active);
  if (!active) return;
  if (p.open) return closePanel();
  p.open = true;
  savePanel(active);
  renderPanel();
};

/** Closes a panel tab; the last one closes the panel too. */
function closeSurface(id = panelOf(active).active) {
  const tab = active;
  const p = panelOf(tab);
  const i = p.surfaces.findIndex((s) => s.id === id);
  if (i < 0) return closePanel();
  p.surfaces.splice(i, 1);
  if (p.active === id) p.active = (p.surfaces[i] ?? p.surfaces[i - 1])?.id ?? null;
  if (!p.surfaces.length) p.open = false;
  savePanel(tab);
  renderPanel();
}
function keepSurfaces(keep) {
  const p = panelOf(active);
  p.surfaces = p.surfaces.filter(keep);
  if (!p.surfaces.some((s) => s.id === p.active)) p.active = p.surfaces.at(-1)?.id ?? null;
  if (!p.surfaces.length) p.open = false;
  savePanel(active);
  renderPanel();
}

const surfaceTitle = (tab, s) => {
  if (s.kind === "diff") return "Diff";
  if (s.kind === "pr") return `#${s.url.match(/\/pull\/(\d+)/)?.[1] ?? "?"}`;
  return s.title || hostOf(s.url) || "Browser";
};
const hostOf = (url) => {
  try {
    return new URL(url).host;
  } catch {
    return "";
  }
};

document.getElementById("toggle-panel").onclick = togglePanel;
onGuiKey("b", togglePanel, { alt: true });
onGuiKey("d", () => (panelOf(active).open && shownSurface(active)?.kind === "diff" ? closePanel() : openSurface("diff")));
onGuiKey("j", () => (panelOf(active).open && shownSurface(active)?.kind === "browser" ? closePanel() : openSurface("browser")), { shift: true });
onGuiKey("k", () => {
  const s = shownSurface(active);
  const pr = s?.kind === "pr" ? { number: surfaceTitle(active, s).slice(1) } : active && shownPr(active);
  if (pr) copyText(`#${pr.number}`, `PR #${pr.number} number copied`);
}, { shift: true });
onGuiKey("o", () => openInEditor());
document.getElementById("pr-chip").onclick = (e) => (e.metaKey || e.ctrlKey ? openOutside(prOf(active).url) : openPr(active, prOf(active).url));

const panelTabs = document.getElementById("panel-tabs");
const tabStrip = panelTabs.querySelector(".strip");
const chatHead = document.getElementById("chat-head");
const layoutControls = chatHead.querySelector(".controls");
const expandButton = document.getElementById("panel-expand");
expandButton.onclick = () => {
  const p = panelOf(active);
  p.max = !p.max;
  savePanel(active);
  renderPanel();
};
document.getElementById("panel-add").onclick = (e) => toggleMenu(e.currentTarget, addMenu());

function addMenu() {
  const tab = active;
  const repo = tab && threadOf(tab).workspace?.repo !== false;
  const pr = tab && shownPr(tab);
  return [
    { label: "Browser", icon: "globe", kbd: "B", run: () => tab && addSurface(tab, newBrowser()) },
    { label: "Diff", icon: "file-diff", kbd: "D", disabled: !repo, hint: repo ? "" : "Not a git repository", run: () => openSurface("diff") },
    { label: "Pull request", icon: "git-pull-request", kbd: "P", disabled: !pr, hint: pr ? `#${pr.number}` : "No pull request in this thread", run: () => openSurface("pr") },
  ];
}

function renderPanelTabs(tab) {
  const p = panelOf(tab);
  const html = p.surfaces.map((s) => {
    const title = surfaceTitle(tab, s);
    return `<div class="ptab${s.id === p.active ? " active" : ""}" data-surface="${esc(s.id)}" data-panel="${s.kind}" title="${esc(title)}" draggable="true">
      <button type="button" class="lead" data-close-surface="${esc(s.id)}" title="Close tab">${icon(SURFACES[s.kind].icon, "kind")}${icon("x", "x")}</button><span>${esc(title)}</span></div>`;
  }).join("");
  if (tabStrip.dataset.html !== html) {
    tabStrip.dataset.html = html;
    tabStrip.innerHTML = html;
  }
}

tabStrip.addEventListener("click", (e) => {
  const close = e.target.closest("[data-close-surface]");
  if (close) return closeSurface(close.dataset.closeSurface);
  const t = e.target.closest(".ptab");
  if (!t) return;
  panelOf(active).active = t.dataset.surface;
  savePanel(active);
  renderPanel();
});
tabStrip.addEventListener("auxclick", (e) => {
  const t = e.target.closest(".ptab");
  if (t && e.button === 1) closeSurface(t.dataset.surface);
});
tabStrip.addEventListener("contextmenu", (e) => {
  const t = e.target.closest(".ptab");
  if (!t) return;
  e.preventDefault();
  const id = t.dataset.surface;
  const ids = panelOf(active).surfaces.map((s) => s.id);
  popMenu(t, [
    { label: "Close", run: () => closeSurface(id) },
    { label: "Close others", disabled: ids.length < 2, run: () => keepSurfaces((s) => s.id === id) },
    { label: "Close to the right", disabled: ids.indexOf(id) === ids.length - 1, run: () => keepSurfaces((s) => ids.indexOf(s.id) <= ids.indexOf(id)) },
    { label: "Close all", run: () => keepSurfaces(() => false) },
  ]);
});
tabStrip.addEventListener("dragstart", (e) => {
  const t = e.target.closest(".ptab");
  if (t) e.dataTransfer.setData("text/x-pinned-surface", t.dataset.surface);
});
tabStrip.addEventListener("dragover", (e) => {
  if (e.dataTransfer.types.includes("text/x-pinned-surface")) e.preventDefault();
});
tabStrip.addEventListener("drop", (e) => {
  const id = e.dataTransfer.getData("text/x-pinned-surface");
  const onto = e.target.closest(".ptab")?.dataset.surface;
  if (!id || !onto || id === onto) return;
  e.preventDefault();
  const p = panelOf(active);
  const moved = p.surfaces.find((s) => s.id === id);
  p.surfaces = p.surfaces.filter((s) => s !== moved);
  p.surfaces.splice(p.surfaces.findIndex((s) => s.id === onto) + (e.offsetX > e.target.closest(".ptab").offsetWidth / 2 ? 1 : 0), 0, moved);
  savePanel(active);
  renderPanel();
});
tabStrip.addEventListener("wheel", (e) => {
  if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) tabStrip.scrollLeft += e.deltaY;
}, { passive: true });

const PANEL_MIN = 360;
const setPanelWidth = (px) => {
  const max = Math.max(PANEL_MIN, Math.min(Math.floor(innerWidth * 0.7), innerWidth - 360 - (ui.sidebarOpen ? 256 : 0)));
  const width = Math.round(Math.min(max, Math.max(PANEL_MIN, px)));
  document.documentElement.style.setProperty("--panel-width", `${width}px`);
  return width;
};
document.getElementById("panel-resize").onmousedown = (down) => {
  down.preventDefault();
  const right = document.getElementById("panel").getBoundingClientRect().right;
  document.body.classList.add("resizing");
  drag(down, {
    move: (e) => setPanelWidth(right - e.clientX),
    drop: (e) => {
      const width = setPanelWidth(right - e.clientX);
      save("gui-panel-width", String(width));
      if (!active) return;
      panelOf(active).width = width;
      savePanel(active);
    },
  });
  addEventListener("mouseup", () => document.body.classList.remove("resizing"), { once: true });
};

let shownPanel = "";
/** Sets the panel's HTML only when it changed, so a render that changes nothing keeps its scroll. */
function showPanel(html) {
  if (html === shownPanel && panelBody.isConnected) return false;
  shownPanel = html;
  panelBody.innerHTML = html;
  return true;
}

function renderPanel() {
  const tab = active;
  const p = panelOf(tab);
  document.body.dataset.panel = p.open ? "open" : "closed";
  document.body.dataset.panelMax = String(p.open && p.max);
  expandButton.innerHTML = icon(p.max ? "minimize-2" : "maximize-2");
  expandButton.title = p.max ? "Restore panel size" : "Maximize panel";
  expandButton.classList.toggle("on", p.max);
  // Open, the panel's tab row holds the layout controls, at the window's top right.
  const home = p.open ? panelTabs : chatHead;
  if (layoutControls.parentElement !== home) home.append(layoutControls);
  setPanelWidth(p.width || Number(load("gui-panel-width")) || 540);
  renderOpenIn(tab);
  if (!guiOn() || !p.open) return;
  renderPanelTabs(tab);
  const s = shownSurface(tab);
  panelBody.dataset.kind = s?.kind ?? "launcher";
  if (!s) renderLauncher();
  else if (s.kind === "diff") renderDiffPanel();
  else if (s.kind === "browser") renderBrowser(s);
  else renderPrPanel(s);
}

function renderLauncher() {
  showPanel(`<div class="launcher"><p class="title">Open a surface</p>${addMenu().map((m, i) => `<button type="button" data-launch="${i}"${m.disabled ? ` disabled title="${esc(m.hint)}"` : ""}>${icon(m.icon)}<span>${m.label}</span><kbd>${m.kbd}</kbd></button>`).join("")}</div>`);
}

/** The letter keys of the "+" menu and of the empty panel's launcher. */
addEventListener("keydown", (e) => {
  if (e.metaKey || e.ctrlKey || e.altKey || !guiOn()) return;
  if (!panelMenu.hidden) {
    if (e.key === "Escape") closeMenu();
    else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const items = [...panelMenu.querySelectorAll("button:not([disabled])")];
      const i = items.indexOf(document.activeElement);
      items[(i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
    } else {
      const i = menuEntries.findIndex((m) => m.kbd?.toLowerCase() === e.key.toLowerCase() && !m.disabled);
      if (i < 0) return;
      panelMenu.querySelector(`[data-i="${i}"]`).click();
    }
    e.preventDefault();
    return;
  }
  if (panelBody.dataset.kind !== "launcher" || !panelOf(active).open || e.target.closest?.("input, textarea, select, [contenteditable], #terms")) return;
  const i = addMenu().findIndex((m) => m.kbd.toLowerCase() === e.key.toLowerCase() && !m.disabled);
  if (i < 0) return;
  e.preventDefault();
  addMenu()[i].run();
});

const panelMenu = document.createElement("div");
panelMenu.id = "panel-menu";
panelMenu.hidden = true;
document.body.append(panelMenu);
let menuEntries = [];
let menuAnchor = null;
/** A menu under `anchor`. An entry with `confirm` asks again, in its place, before it runs. */
function popMenu(anchor, entries) {
  menuEntries = entries;
  menuAnchor = anchor;
  panelMenu.innerHTML = entries.map((m, i) => (m === "-" ? `<hr>` : m.group ? `<p class="group">${esc(m.group)}</p>`
    : `<button type="button" data-i="${i}"${m.disabled ? " disabled" : ""}${m.danger ? ` class="danger"` : ""}>${m.checked ? icon("check") : m.icon ? icon(m.icon) : `<span class="i"></span>`}<span class="label">${esc(m.label)}</span>${m.hint ? `<span class="hint">${esc(m.hint)}</span>` : ""}${m.kbd ? `<kbd>${esc(m.kbd)}</kbd>` : ""}</button>`)).join("");
  panelMenu.hidden = false;
  const r = anchor.getBoundingClientRect();
  panelMenu.style.top = `${Math.max(8, Math.min(r.bottom + 4, innerHeight - panelMenu.offsetHeight - 8))}px`;
  panelMenu.style.left = `${Math.max(8, Math.min(r.left, innerWidth - panelMenu.offsetWidth - 8))}px`;
  panelMenu.querySelector("button:not([disabled])")?.focus({ preventScroll: true });
}
const toggleMenu = (anchor, entries) => (!panelMenu.hidden && menuAnchor === anchor ? closeMenu() : popMenu(anchor, entries));
function closeMenu() {
  panelMenu.hidden = true;
  menuEntries = [];
  menuAnchor = null;
}
panelMenu.addEventListener("click", (e) => {
  const b = e.target.closest("button[data-i]");
  if (!b) return;
  const m = menuEntries[b.dataset.i];
  if (m.confirm && !b.dataset.armed) {
    b.dataset.armed = "true";
    b.querySelector(".label").textContent = m.confirm;
    return;
  }
  closeMenu();
  m.run();
});
addEventListener("pointerdown", (e) => {
  if (!panelMenu.hidden && !panelMenu.contains(e.target) && !menuAnchor?.contains(e.target)) closeMenu();
}, true);

let noteTimer;
/** A short note at the bottom of the window, such as what a copy put on the clipboard. */
function flash(text) {
  let note = document.getElementById("panel-note");
  if (!note) {
    note = Object.assign(document.createElement("p"), { id: "panel-note" });
    document.body.append(note);
  }
  note.textContent = text;
  note.hidden = false;
  clearTimeout(noteTimer);
  noteTimer = setTimeout(() => (note.hidden = true), 2000);
}
const copyText = (text, note = "Copied") => {
  navigator.clipboard?.writeText(text).then(() => flash(note), () => flash("Couldn't reach the clipboard"));
};

/** Appends a review comment on `path:line` to the composer's prompt, which sends it with the next message. */
function addReviewComment(path, line, text) {
  const entry = `${path}:${line}: ${text.trim().replace(/\s*\n\s*/g, " ")}`;
  const head = prompt.value.replace(/\s+$/, "");
  prompt.value = `${head}${head ? "\n" : ""}${entry}\n`;
  prompt.dispatchEvent(new Event("input"));
  return entry;
}
/** The thread's review comments still in the prompt, by path and then `side:line`. */
function reviewComments(tab) {
  const t = threadOf(tab);
  t.comments = (t.comments ?? []).filter((c) => prompt.value.includes(c.entry));
  const byPath = new Map();
  for (const c of t.comments) {
    if (!byPath.has(c.path)) byPath.set(c.path, new Map());
    const lines = byPath.get(c.path);
    lines.set(`${c.side}:${c.line}`, [...(lines.get(`${c.side}:${c.line}`) ?? []), c]);
  }
  return byPath;
}
prompt.addEventListener("input", () => {
  if (active && threadOf(active).comments?.length && shownSurface(active)?.kind === "diff") renderPanel();
});

const shortTime = (unix) => {
  const d = new Date(unix * 1000);
  return d.toDateString() === new Date().toDateString() ? d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }) : d.toLocaleDateString([], { month: "short", day: "numeric" });
};

async function fetchGitDiff(tab, scope) {
  const t = threadOf(tab);
  const key = `${scope}:${ui.diffHideSpace}`;
  if (t.gitDiff?.key === key) return;
  t.gitDiff = { key, loading: true, data: t.gitDiff?.key === key ? t.gitDiff.data : null };
  try {
    t.gitDiff = { key, data: await callOn(tab.machine, { cmd: "diff", tab: tab.view.id, scope, ignore_whitespace: ui.diffHideSpace }) };
  } catch (e) {
    t.gitDiff = { key, error: /unknown variant/.test(String(e)) ? `${machineName(tab.machine)} runs an older pinned, which can't compare the working tree. Update it to see these changes.` : String(e).replace(/^git: /, "") };
  }
  if (tab === active && shownSurface(tab)?.kind === "diff") renderPanel();
}

const skeleton = `<div class="diff-skeleton" aria-label="Loading changes">${[0, 1, 2].map(() => `<span class="head"></span><span class="sep"></span><span class="line" style="width:62%"></span><span class="line" style="width:44%"></span><span class="line" style="width:78%"></span>`).join("")}</div>`;

/** Hides every file of a selection the first time it shows, as T3 does, unless it opened on a file. */
function collapseNew(files, key) {
  for (const f of files) {
    const id = `${key}:${f.path}`;
    if (ui.seenFiles.has(id)) continue;
    ui.seenFiles.add(id);
    if (f.path !== ui.diffFile) ui.collapsed.add(id);
  }
  if (ui.diffFile) ui.collapsed.delete(`${key}:${ui.diffFile}`);
}

function renderDiffPanel() {
  const tab = active;
  if (!tab) return showPanel(`<p class="panel-empty">Select a thread to inspect turn diffs.</p>`);
  const t = threadOf(tab);
  if (t.workspace?.repo === false) return showPanel(`<p class="panel-empty">Turn diffs are unavailable because this folder is not a git repository.</p>`);
  const history = t.history;
  const turnValue = ui.diffTurn == null ? "latest" : `turn:${ui.diffTurn}`;
  const value = ui.diffScope === "turn" ? turnValue : ui.diffScope;
  const turns = history ?? [];
  const scope = `<label class="scope"><select id="diff-scope" aria-label="Diff scope">
      <option value="base"${value === "base" ? " selected" : ""}>Changes</option><option value="uncommitted"${value === "uncommitted" ? " selected" : ""}>Uncommitted</option>
      <option value="latest"${value === "latest" ? " selected" : ""}>Latest turn</option>
      ${turns.length ? `<optgroup label="Turn">${turns.map((h, i) => `<option value="turn:${h.ended}"${value === `turn:${h.ended}` ? " selected" : ""}>Turn ${i + 1} · ${esc(shortTime(h.ended))}</option>`).join("")}</optgroup>` : ""}
    </select>${icon("chevron-down")}</label>`;
  let files = null;
  let key = "";
  let note = "";
  let empty = "";
  const git = ui.diffScope !== "turn";
  if (git) {
    fetchGitDiff(tab, ui.diffScope);
    const d = t.gitDiff;
    if (d?.error) empty = esc(d.error);
    else if (d?.data) {
      key = `${tab.key}:${ui.diffScope}`;
      files = parseDiff(d.data.patch);
      if (d.data.truncated) note = "This diff exceeds the size limit, so its end is cut.";
      if (!files.length) empty = ui.diffScope === "base" ? `No changes against ${esc(d.data.base ?? "the base branch")}.` : "No uncommitted changes.";
    }
  } else if (!history) fetchHistory(tab);
  else if (!history.length) empty = "No completed turns with changes yet.";
  else {
    const chosen = history.find((h) => h.ended === ui.diffTurn) ?? history.at(-1);
    key = String(chosen.ended);
    files = parseDiff(chosen.patch);
    if (ui.diffHideSpace) files = withoutWhitespace(files);
    if (chosen.truncated) note = "This diff exceeds the size limit, so its end is cut.";
  }
  if (files && !empty) collapseNew(files, key);
  const body = empty ? `<p class="panel-empty">${empty}</p>` : files ? diffBody(tab, files, key) : skeleton;
  showPanel(`${diffToolbar(files ? sum(files, "added") : 0, files ? sum(files, "removed") : 0, scope, { refresh: git })}${body}${note ? `<p class="panel-empty">${note}</p>` : ""}`);
  if (ui.commentAt) {
    const box = panelBody.querySelector("[data-comment-text]");
    if (box && document.activeElement !== box && ui.commentAt.focus) {
      box.focus();
      box.setSelectionRange(box.value.length, box.value.length);
      ui.commentAt.focus = false;
    }
  }
  if (ui.diffFile && files) {
    panelBody.querySelector(`.file[data-id="${CSS.escape(`${key}:${ui.diffFile}`)}"]`)?.scrollIntoView({ block: "start" });
    ui.diffFile = null;
  }
}

function diffBody(tab, files, key) {
  const opts = { comments: reviewComments(tab), draft: ui.commentAt?.key === key ? ui.commentAt : null, openTitle: editorFor(tab.machine) ? `Open in ${editorFor(tab.machine).name}` : undefined };
  const diff = `<div class="diff ${ui.diffView}${ui.diffWrap ? " wrap" : ""}" data-comments data-key="${esc(key)}">${filesHtml(files, key, opts)}</div>`;
  return ui.diffTree && files.length ? `<div class="diff-split">${diff}${fileTreeHtml(files)}</div>` : diff;
}

function openFile(path) {
  const tab = active;
  const editor = tab && editorFor(tab.machine);
  if (!editor) return false;
  callOn(tab.machine, { cmd: "open_in", tab: tab.view.id, editor: editor.id, path }).then(() => flash(`Opened ${path} in ${editor.name}`), (e) => flash(String(e)));
  return true;
}

panelBody.addEventListener("change", (e) => {
  if (e.target.id === "diff-scope") {
    const v = e.target.value;
    ui.diffScope = v.startsWith("turn:") || v === "latest" ? "turn" : v;
    ui.diffTurn = v.startsWith("turn:") ? Number(v.slice(5)) : null;
    renderPanel();
  } else if (e.target.dataset.viewed) {
    const s = shownSurface(active);
    const file = parseDiff(prCache.get(`${active.key}:${s.url}`)?.data?.diff ?? "").find((f) => f.path === e.target.dataset.viewed);
    setViewed(s.url, file, e.target.checked);
    const id = `pr:${s.url}:${file.path}`;
    if (e.target.checked) ui.collapsed.add(id);
    renderPanel();
  }
});
panelBody.addEventListener("input", (e) => {
  if (e.target.matches("[data-comment-text]") && ui.commentAt) ui.commentAt.text = e.target.value;
});
panelBody.addEventListener("keydown", (e) => {
  if (e.target.matches("[data-comment-text]")) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      e.stopPropagation();
      saveComment();
    } else if (e.key === "Escape") {
      ui.commentAt = null;
      renderPanel();
    }
  }
});

function saveComment() {
  const at = ui.commentAt;
  const text = panelBody.querySelector("[data-comment-text]")?.value.trim();
  if (!at || !text) return;
  const t = threadOf(active);
  const entry = addReviewComment(at.path, at.line, text);
  t.comments = [...(t.comments ?? []), { id: `c${Date.now()}`, path: at.path, line: at.line, side: at.side, text, entry }];
  ui.commentAt = null;
  renderPanel();
}

panelBody.addEventListener("click", (e) => {
  const action = e.target.closest("[data-diff]")?.dataset.diff;
  const launch = e.target.closest("[data-launch]");
  if (launch) return addMenu()[launch.dataset.launch].run();
  if (action === "unified" || action === "split") {
    ui.diffView = action;
    save("gui-diff-view", action);
  } else if (action === "wrap") {
    ui.diffWrap = !ui.diffWrap;
    save("gui-diff-wrap", String(ui.diffWrap));
  } else if (action === "whitespace") {
    ui.diffHideSpace = !ui.diffHideSpace;
    save("gui-diff-whitespace", String(ui.diffHideSpace));
  } else if (action === "tree") {
    ui.diffTree = !ui.diffTree;
    save("gui-diff-tree", String(ui.diffTree));
  } else if (action === "refresh") {
    if (active) threadOf(active).gitDiff = null;
  } else if (action === "collapse") {
    const ids = [...panelBody.querySelectorAll(".file")].map((f) => f.dataset.id);
    const all = ids.every((id) => ui.collapsed.has(id));
    for (const id of ids) all ? ui.collapsed.delete(id) : ui.collapsed.add(id);
  } else if (e.target.closest("[data-copy]")) {
    const b = e.target.closest("[data-copy]");
    if (!b.classList.contains("copy")) return copyText(b.dataset.copy, `Copied ${b.dataset.copy}`);
    navigator.clipboard?.writeText(b.dataset.copy);
    b.classList.add("copied");
    b.innerHTML = icon("check");
    setTimeout(() => {
      b.classList.remove("copied");
      b.innerHTML = icon("copy");
    }, 1500);
    return;
  } else if (e.target.closest("[data-tree-dir]")) {
    const dir = e.target.closest("[data-tree-dir]").dataset.treeDir;
    ui.treeClosed.has(dir) ? ui.treeClosed.delete(dir) : ui.treeClosed.add(dir);
  } else if (e.target.closest("[data-tree-file]")) {
    const path = e.target.closest("[data-tree-file]").dataset.treeFile;
    const key = panelBody.querySelector(".diff").dataset.key;
    ui.collapsed.delete(`${key}:${path}`);
    renderPanel();
    panelBody.querySelector(`.file[data-id="${CSS.escape(`${key}:${path}`)}"]`)?.scrollIntoView({ block: "start" });
    return;
  } else if (e.target.closest("[data-viewed], label.viewed")) {
    return;
  } else if (e.target.closest(".diff[data-comments] .no[data-line]")) {
    const no = e.target.closest(".no[data-line]");
    ui.commentAt = { key: no.closest(".diff").dataset.key, path: no.closest(".file").dataset.path, line: Number(no.dataset.line), side: no.dataset.side, text: "", focus: true };
  } else if (e.target.closest("[data-comment-cancel]")) {
    ui.commentAt = null;
  } else if (e.target.closest("[data-comment-save]")) {
    return saveComment();
  } else if (e.target.closest("[data-comment-delete]")) {
    const t = threadOf(active);
    const c = t.comments.find((c) => c.id === e.target.closest("[data-comment-delete]").dataset.commentDelete);
    t.comments = t.comments.filter((x) => x !== c);
    if (c) {
      prompt.value = prompt.value.replace(`${c.entry}\n`, "").replace(c.entry, "");
      prompt.dispatchEvent(new Event("input"));
    }
  } else if (e.target.closest(".file > header .path") && e.target.closest(".diff[data-comments]") && openFile(e.target.closest(".file").dataset.path)) {
    return;
  } else if (e.target.closest(".file > header")) {
    const id = e.target.closest(".file").dataset.id;
    ui.collapsed.has(id) ? ui.collapsed.delete(id) : ui.collapsed.add(id);
  } else if (e.target.closest("[data-pr-tab]")) {
    shownSurface(active).prTab = e.target.closest("[data-pr-tab]").dataset.prTab;
    savePanel(active);
  } else if (e.target.closest("[data-pr-do]")) {
    return prPrimary(e.target.closest("[data-pr-do]"));
  } else if (e.target.closest("[data-pr-more]")) {
    return toggleMenu(e.target.closest("[data-pr-more]"), prMenu(shownSurface(active)));
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

let recentUrls = [];
try {
  recentUrls = JSON.parse(load("gui-browser-recent")) ?? [];
} catch {}
function remember(url) {
  if (!url) return;
  recentUrls = [url, ...recentUrls.filter((u) => u !== url)].slice(0, 8);
  save("gui-browser-recent", JSON.stringify(recentUrls));
}

/** Points the thread's browser tab at `url`, opening one when it has none; `openPanel("browser")` shows it. */
function setBrowserUrl(url) {
  const tab = active;
  if (!tab) return;
  const p = panelOf(tab);
  let s = browserOf(tab);
  if (!s) p.surfaces.push((s = newBrowser()));
  Object.assign(s, { url, title: "" });
  p.active = s.id;
  remember(url);
  savePanel(tab);
}

async function fetchPorts(tab) {
  const t = threadOf(tab);
  if (t.portsAt > Date.now() - 3000 || t.portsBusy) return;
  t.portsBusy = true;
  try {
    t.ports = await callOn(tab.machine, { cmd: "ports", tab: tab.view.id });
  } catch (e) {
    t.ports = /unknown variant/.test(String(e)) ? null : [];
  }
  t.portsBusy = false;
  t.portsAt = Date.now();
  const s = shownSurface(tab);
  if (tab === active && s?.kind === "browser" && !s.url) renderBrowser(s, true);
}
setInterval(() => {
  const s = active && panelOf(active).open && shownSurface(active);
  if (guiOn() && !document.hidden && s?.kind === "browser" && !s.url) fetchPorts(active);
}, 3000);

const DEVICES = [
  ["Responsive", 0, 0],
  ["iPhone SE", 375, 667],
  ["iPhone 15 Pro", 393, 852],
  ["Pixel 7", 412, 915],
  ["iPad mini", 768, 1024],
  ["iPad Pro 11″", 834, 1194],
  ["Laptop", 1280, 800],
];

function frameStyle(s) {
  const zoom = s.zoom ?? 1;
  const [w, h] = s.device ? [s.device.w, s.device.h] : [null, null];
  return `width:${w ? `${w}px` : `${100 / zoom}%`};height:${h ? `${h}px` : `${100 / zoom}%`};transform:scale(${zoom})`;
}

function renderBrowser(s, servers = false) {
  const tab = active;
  const shown = `${s.id}\n${s.url}\n${JSON.stringify(s.device)}\n${s.devices}`;
  if (panelBody.querySelector(".browser") && panelBody.dataset.url === shown && !servers) return;
  if (servers && panelBody.dataset.url !== shown) return;
  panelBody.dataset.url = shown;
  const t = tab && threadOf(tab);
  if (tab && !s.url && !t.ports && t.ports !== null) fetchPorts(tab);
  const ports = t?.ports;
  // The session's own servers, as T3 lists; the machine's others only while it has none.
  const own = ports?.filter((p) => p.own);
  const local = ports ? (own.length ? own : ports).map((p) => ({ url: `http://localhost:${p.port}`, name: p.own ? p.command : "Listening" })) : tab ? seenUrls(tab).map((url) => ({ url, name: "Listening" })) : [];
  const remote = tab && tab.machine !== LOCAL;
  const device = s.device;
  const cards = (rows, ic) => `<div class="servers">${rows.map((r) => `<button type="button" data-url="${esc(r.url)}">${icon(ic)}<span><b>${esc(r.name)}</b><span>${esc(r.url.replace(/^https?:\/\//, ""))}</span></span></button>`).join("")}</div>`;
  const recent = recentUrls.map((url) => ({ url, name: hostOf(url) || url }));
  const changed = showPanel(`<div class="browser"><form class="url-bar">
      <button type="button" class="icon" data-nav="back" title="Back">${icon("arrow-left")}</button><button type="button" class="icon" data-nav="forward" title="Forward">${icon("arrow-right")}</button>
      <button type="button" class="icon" data-nav="reload" title="Reload (${IS_MAC ? "⌘" : "Ctrl+"}R)">${icon("rotate-cw")}</button>
      <input name="url" value="${esc(s.url)}" placeholder="Search or enter URL" spellcheck="false" autocomplete="off" autocapitalize="off">
      <button type="button" class="icon" data-nav="external" title="Open in system browser">${icon("external-link")}</button>
      <button type="button" class="icon" data-nav="more" title="More">${icon("ellipsis")}</button>
      <span class="loading-bar" hidden></span></form>
    ${s.devices ? `<div class="device-bar"><label class="scope"><select data-device aria-label="Device">${DEVICES.map(([name, w, h]) => `<option value="${name}"${(device?.name ?? "Responsive") === name ? " selected" : ""}>${esc(name)}${w ? ` · ${w}×${h}` : ""}</option>`).join("")}</select>${icon("chevron-down")}</label>
      <input data-size="w" inputmode="numeric" value="${device?.w ?? ""}" placeholder="Width" aria-label="Width"><span class="muted">×</span><input data-size="h" inputmode="numeric" value="${device?.h ?? ""}" placeholder="Height" aria-label="Height">
      <button type="button" class="icon" data-nav="rotate" title="Rotate"${device ? "" : " disabled"}>${icon("rotate-ccw-square")}</button></div>` : ""}
    ${s.url
      ? `<div class="viewport${device ? " device" : ""}"><iframe src="${esc(s.url)}" title="Preview" style="${frameStyle(s)}" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads"></iframe></div>`
      : `<div class="browser-empty">${icon("globe", "big")}<p class="title">No preview yet</p><p>Type a URL above, or start a dev server in this thread.</p>
        ${recent.length ? `<p class="group">Recently used</p>${cards(recent, "history")}` : ""}
        ${local.length ? `<p class="group">Local servers</p>${cards(local, "radio-tower")}` : ""}
        ${remote ? `<p class="hint">This thread runs on ${esc(machineName(tab.machine))}, so its local servers are that machine's, not this device's.</p>` : ""}</div>`}</div>`);
  if (!changed) return;
  const form = panelBody.querySelector(".url-bar");
  const frame = panelBody.querySelector("iframe");
  const bar = form.querySelector(".loading-bar");
  const reload = form.querySelector('[data-nav="reload"]');
  const loading = (on) => {
    bar.hidden = !on;
    panelBody.querySelector(".browser").toggleAttribute("data-loading", on);
    reload.innerHTML = icon(on ? "x" : "rotate-cw");
    reload.title = on ? "Stop" : `Reload (${IS_MAC ? "⌘" : "Ctrl+"}R)`;
  };
  if (frame) {
    loading(true);
    frame.onload = () => {
      loading(false);
      let title = "";
      // Safari reports even a caught read of another origin's document as an error.
      try {
        if (sameOrigin(frame)) title = frame.contentDocument?.title ?? "";
      } catch {}
      if (title !== (s.title ?? "")) {
        s.title = title;
        savePanel(tab);
        renderPanelTabs(tab);
      }
    };
  }
  const go = (url) => {
    Object.assign(s, { url, title: "" });
    remember(url);
    savePanel(tab);
    renderPanel();
  };
  form.onsubmit = (e) => {
    e.preventDefault();
    let url = form.url.value.trim();
    if (url && !/^https?:\/\//.test(url)) {
      url = /^(localhost|127\.|0\.0\.0\.0|\[::1\])/.test(url) ? `http://${url}` : /\.\w{2,}($|\/)/.test(url) ? `https://${url}` : `https://duckduckgo.com/?q=${encodeURIComponent(url)}`;
    }
    go(url);
  };
  form.onclick = (e) => {
    const nav = e.target.closest("[data-nav]")?.dataset.nav;
    if (nav === "external" && s.url) openOutside(s.url);
    else if (nav === "reload" && frame) {
      if (!bar.hidden) {
        try {
          if (sameOrigin(frame)) frame.contentWindow.stop();
        } catch {}
        loading(false);
      } else {
        loading(true);
        frame.src = frame.src;
      }
    } else if ((nav === "back" || nav === "forward") && frame && sameOrigin(frame)) {
      try {
        frame.contentWindow.history[nav]();
      } catch {}
    } else if (nav === "more") {
      const key = IS_MAC ? "⌘" : "Ctrl+";
      toggleMenu(e.target.closest("[data-nav]"), [
        { label: `${s.devices ? "Hide" : "Show"} device toolbar`, icon: "smartphone", run: () => {
          s.devices = !s.devices;
          if (!s.devices) s.device = null;
          savePanel(tab);
          renderBrowser(s);
        } },
        "-",
        { label: "Zoom in", kbd: `${key}=`, run: () => zoomBrowser(s, 0.1) },
        { label: "Zoom out", kbd: `${key}-`, run: () => zoomBrowser(s, -0.1) },
        { label: "Actual size", kbd: `${key}0`, disabled: (s.zoom ?? 1) === 1, run: () => zoomBrowser(s, 0) },
      ]);
    }
  };
  const bars = panelBody.querySelector(".device-bar");
  if (bars) {
    const setDevice = (device) => {
      s.device = device;
      savePanel(tab);
      renderBrowser(s);
    };
    bars.querySelector("[data-device]").onchange = (e) => {
      const [name, w, h] = DEVICES.find(([n]) => n === e.target.value);
      setDevice(w ? { name, w, h } : null);
    };
    for (const input of bars.querySelectorAll("[data-size]")) {
      input.onchange = () => {
        const [w, h] = [...bars.querySelectorAll("[data-size]")].map((i) => Math.round(Number(i.value)));
        setDevice(w > 0 && h > 0 ? { name: "Custom", w: Math.min(w, 4000), h: Math.min(h, 4000) } : null);
      };
    }
    bars.querySelector('[data-nav="rotate"]').onclick = () => device && setDevice({ ...device, w: device.h, h: device.w });
  }
  for (const b of panelBody.querySelectorAll("[data-url]")) b.onclick = () => go(b.dataset.url);
}

const sameOrigin = (frame) => hostOf(frame.src) !== "" && new URL(frame.src).origin === location.origin;

/** Zooms the page by `step`, or back to 100% when it is 0. */
function zoomBrowser(s, step) {
  s.zoom = step ? Math.min(3, Math.max(0.25, Math.round(((s.zoom ?? 1) + step) * 100) / 100)) : 1;
  const frame = panelBody.querySelector("iframe");
  if (frame) frame.style.cssText = frameStyle(s);
  flash(`Zoom ${Math.round(s.zoom * 100)}%`);
  savePanel(active);
}

/** ⌘R, ⌘L and zoom, while the browser surface (not the page inside it) has focus. */
document.getElementById("panel").addEventListener("keydown", (e) => {
  const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  const s = active && shownSurface(active);
  if (!mod || e.altKey || s?.kind !== "browser" || !e.target.closest(".browser")) return;
  const zoom = { "=": 0.1, "+": 0.1, "-": -0.1, 0: 0 }[e.key];
  if (e.key === "r") panelBody.querySelector('[data-nav="reload"]').click();
  else if (e.key === "l") panelBody.querySelector(".url-bar input").select();
  else if (zoom !== undefined) zoomBrowser(s, zoom);
  else return;
  e.preventDefault();
  e.stopPropagation();
});

const checkState = (c) => {
  const v = (c.conclusion || c.state || c.status || "").toUpperCase();
  if (["SUCCESS", "NEUTRAL", "SKIPPED"].includes(v)) return "ok";
  if (["FAILURE", "ERROR", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE"].includes(v)) return "fail";
  return "pending";
};
const CHECK_ICONS = { ok: "circle-check", fail: "circle-alert", pending: "circle-dashed" };

/** Pull requests as gh answered for them, by `tab.key:url`. */
const prCache = new Map();
async function loadPr(tab, url) {
  const key = `${tab.key}:${url}`;
  prCache.set(key, { loading: true, data: prCache.get(key)?.data });
  if (!prCache.get(key).data) renderPanel();
  try {
    prCache.set(key, { data: await callOn(tab.machine, { cmd: "pull_request", tab: tab.view.id, pr: url }), at: Date.now() });
  } catch (e) {
    prCache.set(key, { error: String(e) });
  }
  if (tab === active && shownSurface(tab)?.url === url) renderPanel();
}

/** The pull request the thread last clicked, or else its latest. */
const shownPr = (tab) => {
  const t = threadOf(tab);
  return t.items.find((i) => i.kind === "pr" && i.url === t.prUrl) ?? prOf(tab);
};
function openPr(tab, url) {
  threadOf(tab).prUrl = url;
  addSurface(tab, { id: `pr:${url}`, kind: "pr", url });
}

let viewedFiles = {};
try {
  viewedFiles = JSON.parse(load("gui-pr-viewed")) ?? {};
} catch {}
const fileHash = (f) => {
  let h = 0;
  for (const hunk of f.hunks) for (const l of hunk.lines) for (let i = 0; i < l.text.length; i++) h = (h * 31 + l.text.charCodeAt(i) + l.type.length) | 0;
  return String(h);
};
function setViewed(url, file, on) {
  if (on) viewedFiles[`${url}\n${file.path}`] = fileHash(file);
  else delete viewedFiles[`${url}\n${file.path}`];
  save("gui-pr-viewed", JSON.stringify(viewedFiles));
}
/** A file's Viewed state, against what gh sent for it before whitespace was hidden. */
const viewedOf = (url, raw) => (f) => {
  const hash = viewedFiles[`${url}\n${f.path}`];
  const now = fileHash(raw.find((r) => r.path === f.path) ?? f);
  return { on: hash === now, changed: hash !== undefined && hash !== now };
};

const MERGE_METHODS = { merge: "Merge", squash: "Squash and merge", rebase: "Rebase and merge" };
const mergeMethod = () => (MERGE_METHODS[load("gui-merge-method")] ? load("gui-merge-method") : "merge");

async function prAct(s, action) {
  const tab = active;
  const key = `${tab.key}:${s.url}`;
  const entry = prCache.get(key);
  entry.busy = action;
  renderPanel();
  try {
    await callOn(tab.machine, { cmd: "pr_action", tab: tab.view.id, pr: s.url, action });
    flash(action === "ready" ? "Marked ready for review" : "Merged");
    prCache.delete(key);
  } catch (e) {
    Object.assign(entry, { busy: null, failed: String(e).replace(/^gh: /, "") });
  }
  renderPanel();
}

let prArmed = null;
function prPrimary(b) {
  const s = shownSurface(active);
  const action = b.dataset.prDo;
  if (prArmed?.button !== action) {
    clearTimeout(prArmed?.timer);
    prArmed = { button: action, timer: setTimeout(() => ((prArmed = null), renderPanel()), 4000) };
    return renderPanel();
  }
  clearTimeout(prArmed.timer);
  prArmed = null;
  prAct(s, action);
}

function prMenu(s) {
  const d = prCache.get(`${active.key}:${s.url}`)?.data;
  if (!d) return [{ label: "Refresh", icon: "rotate-cw", run: () => loadPr(active, s.url) }];
  const open = (d.state ?? "").toUpperCase() === "OPEN";
  const method = mergeMethod();
  return [
    { label: "Refresh", icon: "rotate-cw", run: () => loadPr(active, s.url) },
    ...(open ? ["-", { group: "Merge method" }, ...Object.entries(MERGE_METHODS).map(([m, label]) => ({ label, checked: m === method, run: () => (save("gui-merge-method", m), renderPanel()) })),
      ...(d.isDraft ? [{ label: "Mark ready for review", icon: "eye", confirm: "Confirm ready?", run: () => prAct(s, "ready") }] : [{ label: "Merge now", icon: "git-merge", confirm: "Confirm merge?", run: () => prAct(s, method) }])] : []),
    "-",
    { label: "Open on GitHub", icon: "external-link", run: () => openOutside(d.url) },
    { label: "Copy link", icon: "copy", run: () => copyText(d.url, "Link copied") },
    { label: "Copy PR number", icon: "copy", kbd: `${IS_MAC ? "⌘⇧" : "Ctrl+Shift+"}K`, run: () => copyText(`#${d.number}`, `PR #${d.number} number copied`) },
  ];
}

const login = (a) => a?.login || a?.name || a?.slug || "";
const agoText = (iso) => (iso ? `${ago(Date.parse(iso) / 1000)} ago` : "");
const took = (c) => {
  const ms = Date.parse(c.completedAt) - Date.parse(c.startedAt);
  return Number.isFinite(ms) && ms > 0 ? duration(ms) : "";
};

function prTimeline(d) {
  const events = [
    d.createdAt && { at: d.createdAt, icon: "git-pull-request", html: `<b>${esc(login(d.author))}</b> opened this pull request` },
    ...(d.commits ?? []).map((c) => ({ at: c.committedDate || c.authoredDate, icon: "git-commit", html: `<b>${esc(login(c.authors?.[0]) || "someone")}</b> committed <span class="mono">${esc((c.oid ?? "").slice(0, 7))}</span> ${esc(c.messageHeadline ?? "")}${c.oid ? ` <a href="${esc(`${d.url}/commits/${c.oid}`)}">View commit</a>` : ""}` })),
    ...(d.reviews ?? []).map((r) => ({ at: r.submittedAt, icon: "eye", html: `<b>${esc(login(r.author))}</b> ${esc((r.state ?? "reviewed").toLowerCase().replace(/_/g, " "))}`, body: r.body })),
    ...(d.comments ?? []).map((c) => ({ at: c.createdAt, icon: "message-square", html: `<b>${esc(login(c.author))}</b> commented`, body: c.body })),
    d.mergedAt && { at: d.mergedAt, icon: "git-merge", html: "Merged" },
    d.closedAt && !d.mergedAt && { at: d.closedAt, icon: "circle-dot", html: "Closed" },
  ].filter((e) => e && e.at).sort((a, b) => Date.parse(a.at) - Date.parse(b.at));
  return `<div class="pr-body"><ol class="pr-timeline">${events.map((e) => `<li>${icon(e.icon)}<div><p>${e.html} <span class="muted">${esc(agoText(e.at))}</span></p>${e.body ? `<div class="md">${markdown(e.body)}</div>` : ""}</div></li>`).join("")}</ol>
    <button type="button" class="outline" data-open-url="${esc(d.url)}">${icon("external-link")}Open activity on GitHub</button></div>`;
}

function renderPrPanel(s) {
  const tab = active;
  const key = `${tab.key}:${s.url}`;
  const entry = prCache.get(key);
  const number = surfaceTitle(tab, s);
  if (!entry) return loadPr(tab, s.url);
  if (!entry.data && entry.loading) return showPanel(`<p class="panel-empty">Loading ${number}…</p>`);
  if (!entry.data) {
    showPanel(`<div class="panel-empty"><p>${esc(entry.error)}</p><button class="outline" data-open-url="${esc(s.url)}">${icon("external-link")}Open ${number} on GitHub</button></div>`);
    return;
  }
  const d = entry.data;
  const prTab = s.prTab ?? "summary";
  const state = d.isDraft ? "draft" : (d.state ?? "open").toLowerCase();
  const checks = d.statusCheckRollup ?? [];
  const failed = checks.filter((c) => checkState(c) === "fail").length;
  const passed = checks.filter((c) => checkState(c) === "ok").length;
  const files = prTab === "code" ? (ui.diffHideSpace ? withoutWhitespace(parseDiff(d.diff ?? "")) : parseDiff(d.diff ?? "")) : [];
  const action = state === "draft" ? ["ready", "Ready for review", "Confirm ready?"] : state === "open" ? [mergeMethod(), MERGE_METHODS[mergeMethod()], "Confirm merge?"] : null;
  const primary = action && `<button type="button" class="${prArmed?.button === action[0] ? "primary armed" : "outline"}" data-pr-do="${action[0]}"${entry.busy ? " disabled" : ""}>${entry.busy ? "Working…" : prArmed?.button === action[0] ? action[2] : action[1]}</button>`;
  const reviewers = [...new Set([...(d.reviewRequests ?? []).map(login), ...(d.reviews ?? []).map((r) => login(r.author))].filter(Boolean))];
  const latest = (who) => [...(d.reviews ?? [])].reverse().find((r) => login(r.author) === who)?.state;
  const labels = d.labels ?? [];
  const head = `<div class="pr-head">
      <div class="crumbs"><span class="muted">${esc(d.url.match(/github\.com\/([^/]+\/[^/]+)/)?.[1] ?? "")}</span><button class="num ${state}" data-open-url="${esc(d.url)}" title="Open on GitHub">#${d.number}${icon("external-link")}</button>
        <span class="spacer"></span>${primary || `<span class="state ${state}">${icon(state === "merged" ? "git-merge" : "git-pull-request")}${esc(state[0].toUpperCase() + state.slice(1))}</span>`}
        <button type="button" class="icon" data-pr-more title="More">${icon("ellipsis")}</button></div>
      ${entry.failed ? `<p class="pr-error">${icon("circle-alert")}${esc(entry.failed)}</p>` : ""}
      <h3>${esc(d.title)}</h3>
      <p class="muted">${esc(login(d.author))}${d.updatedAt || d.createdAt ? ` · updated ${esc(agoText(d.updatedAt || d.createdAt))}` : ""}
        <button type="button" class="checkout" data-copy="gh pr checkout ${d.number}" title="Copy the checkout command"><code>gh pr checkout ${d.number}</code></button></p>
      <p class="refs"><code>${esc(d.baseRefName)}</code>${icon("arrow-left")}<code>${esc(d.headRefName)}</code><span class="spacer"></span>${icon("file-diff")}${d.changedFiles}<span class="add">+${d.additions}</span><span class="del">-${d.deletions}</span></p>
      <div class="tabs"><span class="seg">${["summary", "timeline", "code"].map((t) => `<button data-pr-tab="${t}" class="${prTab === t ? "on" : ""}">${t[0].toUpperCase() + t.slice(1)}</button>`).join("")}</span>
        <span class="checks-line ${failed ? "fail" : passed === checks.length ? "ok" : "pending"}">${checks.length ? `${icon(CHECK_ICONS[failed ? "fail" : passed === checks.length ? "ok" : "pending"])}${passed} passed${failed ? ` / ${failed} failed` : ""}` : "No checks"}</span>
        <span class="spacer"></span>
        <span class="muted count" title="Comments">${icon("message-square")}${(d.comments ?? []).length}</span><span class="muted count" title="Commits">${icon("git-commit")}${(d.commits ?? []).length}</span>
        ${d.reviewDecision ? `<span class="muted">${esc(d.reviewDecision.toLowerCase().replace(/_/g, " "))}</span>` : ""}</div></div>`;
  const summary = `<div class="pr-body">
      <dl class="meta"><dt>${icon("users")}Reviewers</dt><dd>${reviewers.length ? reviewers.map((r) => `<span class="chip">${esc(r)}${latest(r) ? ` <span class="muted">${esc(latest(r).toLowerCase().replace(/_/g, " "))}</span>` : ""}</span>`).join("") : `<span class="muted">None</span>`}</dd>
        <dt>${icon("tag")}Labels</dt><dd>${labels.length ? labels.map((l) => `<span class="chip label"${/^[0-9a-f]{6}$/i.test(l.color ?? "") ? ` style="--label:#${l.color}"` : ""}>${esc(l.name)}</span>`).join("") : `<span class="muted">None</span>`}</dd></dl>
      <h4>Description</h4><div class="md">${d.body ? markdown(d.body) : `<p class="muted"><em>No description provided.</em></p>`}</div>
      ${checks.length ? `<details class="checks-fold"><summary><h4>Checks (${checks.length})</h4>${icon("chevron-right")}</summary><ul class="checks">${checks.map((c) => `<li class="${checkState(c)}">${icon(CHECK_ICONS[checkState(c)])}<span>${esc(c.name || c.context || "check")}</span><span class="muted">${esc(took(c) || (c.conclusion || c.status || c.state || "").toLowerCase())}</span>${WEB_URL.test(c.detailsUrl || c.targetUrl || "") ? `<a href="${esc(c.detailsUrl || c.targetUrl)}">Details</a>` : ""}</li>`).join("")}</ul></details>` : ""}
      ${(d.reviews ?? []).length ? `<h4>Reviews</h4><ul class="reviews">${d.reviews.map((r) => `<li><b>${esc(login(r.author))}</b> <span class="muted">${esc((r.state ?? "").toLowerCase().replace(/_/g, " "))}</span>${r.body ? `<div class="md">${markdown(r.body)}</div>` : ""}</li>`).join("")}</ul>` : ""}
      ${(d.comments ?? []).length ? `<h4>Comments (${d.comments.length})</h4><ul class="reviews">${d.comments.map((c) => `<li><b>${esc(login(c.author))}</b> <span class="muted">${esc(agoText(c.createdAt))}</span><div class="md">${markdown(c.body ?? "")}</div></li>`).join("")}</ul>` : ""}</div>`;
  const code = () => `${diffToolbar(d.additions, d.deletions, `<span class="muted">${d.changedFiles} file${d.changedFiles === 1 ? "" : "s"}</span>`)}
    <div class="diff ${ui.diffView}${ui.diffWrap ? " wrap" : ""}" data-key="pr:${esc(s.url)}">${filesHtml(files, `pr:${s.url}`, { viewed: viewedOf(s.url, parseDiff(d.diff ?? "")) })}${d.truncated ? `<p class="panel-empty">This diff exceeds the size limit, so its end is cut.</p>` : ""}</div>`;
  showPanel(`<div class="pr">${head}${prTab === "code" ? code() : prTab === "timeline" ? prTimeline(d) : summary}</div>`);
}
const sum = (files, key) => files.reduce((n, f) => n + Number(f[key]), 0);

/** Editors each machine has, and the one picked there. */
const editorLists = new Map();
function editorsOn(machine) {
  if (!editorLists.has(machine)) {
    editorLists.set(machine, []);
    callOn(machine, { cmd: "editors" }).then((list) => {
      editorLists.set(machine, list);
      renderOpenIn(active);
    }, (e) => /unknown variant|can't answer/.test(String(e)) || setTimeout(() => editorLists.delete(machine), 10_000));
  }
  return editorLists.get(machine);
}
const editorFor = (machine) => {
  const list = editorsOn(machine);
  return list.find((e) => e.id === load(`gui-editor:${machine}`)) ?? list.find((e) => e.id !== "finder") ?? list[0];
};
const openIn = document.getElementById("open-in");
function renderOpenIn(tab) {
  const editor = tab && guiOn() && editorFor(tab.machine);
  openIn.hidden = !editor;
  if (!editor) return;
  const main = openIn.querySelector(".main");
  main.title = `Open in ${editor.name} (${IS_MAC ? "⌘" : "Ctrl+"}O)`;
  main.querySelector(".label").textContent = "Open";
  main.dataset.editor = editor.id;
}
function openInEditor() {
  const tab = active;
  const editor = tab && editorFor(tab.machine);
  if (!editor) return;
  callOn(tab.machine, { cmd: "open_in", tab: tab.view.id, editor: editor.id }).then(() => flash(`Opened in ${editor.name}`), (e) => flash(String(e)));
}
openIn.querySelector(".main").onclick = openInEditor;
openIn.querySelector(".pick").onclick = (e) => {
  const tab = active;
  if (!tab) return;
  const chosen = editorFor(tab.machine);
  const list = editorsOn(tab.machine);
  toggleMenu(e.currentTarget, list.length
    ? list.map((ed) => ({ label: ed.name, checked: ed.id === chosen?.id, kbd: ed.id === chosen?.id ? `${IS_MAC ? "⌘" : "Ctrl+"}O` : "", run: () => (save(`gui-editor:${tab.machine}`, ed.id), renderOpenIn(tab)) }))
    : [{ label: "No installed editors found", disabled: true }]);
};
for (const el of openIn.querySelectorAll("[data-ic]")) el.outerHTML = icon(el.dataset.ic);
