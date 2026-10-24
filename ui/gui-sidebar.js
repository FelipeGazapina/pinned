Object.assign(ICON_PATHS, {
  "panel-left-close": `<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/><path d="m16 15-3-3 3-3"/>`,
  pin: `<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>`,
  "pin-off": `<path d="M12 17v5"/><path d="M15 9.34V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H7.89"/><path d="m2 2 20 20"/><path d="M9 9v1.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h11"/>`,
  clock: `<path d="M12 6v6l4 2"/><circle cx="12" cy="12" r="10"/>`,
  "alarm-clock": `<circle cx="12" cy="13" r="8"/><path d="M12 9v4l2 2"/><path d="M5 3 2 6"/><path d="m22 6-3-3"/><path d="M6.38 18.7 4 21"/><path d="M17.64 18.67 20 21"/>`,
  "alarm-clock-off": `<path d="M6.87 6.87a8 8 0 1 0 11.26 11.26"/><path d="M19.9 14.25a8 8 0 0 0-9.15-9.15"/><path d="m22 6-3-3"/><path d="M6.26 18.67 4 21"/><path d="m2 2 20 20"/><path d="M4 4 2 6"/>`,
  "undo-2": `<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>`,
  "folder-plus": `<path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>`,
  "message-circle-question": `<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/>`,
  "message-square-plus": `<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/><path d="M12 7v6"/><path d="M9 10h6"/>`,
  pencil: `<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>`,
  "mail-open": `<path d="M21.2 8.4c.5.38.8.97.8 1.6v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V10a2 2 0 0 1 .8-1.6l8-6a2 2 0 0 1 2.4 0l8 6Z"/><path d="m22 10-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 10"/>`,
  hash: `<line x1="4" x2="20" y1="9" y2="9"/><line x1="4" x2="20" y1="15" y2="15"/><line x1="10" x2="8" y1="3" y2="21"/><line x1="16" x2="14" y1="3" y2="21"/>`,
  timer: `<line x1="10" x2="14" y1="2" y2="2"/><line x1="12" x2="15" y1="14" y2="11"/><circle cx="12" cy="14" r="8"/>`,
});

const SETTLE_AFTER_S = 3 * 86400;
const SETTLED_FIRST = 10;
const SETTLED_PAGE = 25;
const SIDEBAR_WIDTH = 256;
const MOD_KEY = IS_MAC ? "Meta" : "Control";
const MOD_LABEL = IS_MAC ? "⌘" : "Ctrl+";
/** T3 Code's project colors, Tailwind's 400 shades, picked by a hash of the project's name. */
const MONOGRAM_COLORS = ["#9ca3af", "#f87171", "#fb923c", "#fbbf24", "#facc15", "#a3e635", "#4ade80", "#34d399", "#2dd4bf",
  "#22d3ee", "#38bdf8", "#60a5fa", "#818cf8", "#a78bfa", "#c084fc", "#e879f9", "#f472b6", "#fb7185"];

/** What the user did to threads, by tab key: pins in order, manual settles and un-settles, snoozes,
 * renames, and threads kept out of auto-settle. */
const sideMarks = { pins: [], settled: {}, unsettled: {}, snoozed: {}, titles: {}, noAutoSettle: {} };
try {
  Object.assign(sideMarks, JSON.parse(load("gui-marks")) ?? {});
} catch {}
const saveMarks = () => save("gui-marks", JSON.stringify(sideMarks));
Object.assign(ui, { snoozedOpen: load("gui-snoozed-open") === "true", settledShown: SETTLED_FIRST, renaming: null, hit: 0 });
/** When this page first saw each tab, which orders the active rows newest first. */
const firstSeen = new Map();
/** Sessions closed here, newest last, for ⌘⇧T. */
const closedThreads = [];

const nowS = () => Date.now() / 1000;
const unixOf = (iso) => Date.parse(iso) / 1000 || 0;
const lastPrompt = (tab) => [...threadOf(tab).items].reverse().find((i) => i.kind === "user");
const latestUserAt = (tab) => unixOf(lastPrompt(tab)?.at) || tab.view.updated || 0;
const isPinned = (tab) => sideMarks.pins.includes(tab.key);
const canSnooze = (tab) => !["asking", "input"].includes(phaseOf(tab));
const raisedHand = (tab) => !canSnooze(tab) || (phaseOf(tab) === "failed" && (tab.view.updated ?? 0) > (sideMarks.snoozed[tab.key]?.at ?? 0));
const snoozedNow = (tab) => !!sideMarks.snoozed[tab.key] && sideMarks.snoozed[tab.key].until > nowS() && !raisedHand(tab);
const wokeNow = (tab) => !!sideMarks.snoozed[tab.key] && !snoozedNow(tab);
/** A manual settle holds until the next prompt. Auto-settle takes threads idle for 3 days, but not
 * pinned or running ones, nor one un-settled since. */
function settled(tab) {
  const at = sideMarks.settled[tab.key];
  if (at) return at >= unixOf(lastPrompt(tab)?.at);
  if (isPinned(tab) || sideMarks.noAutoSettle[tab.key] || !tab.view.updated || !["idle", "done"].includes(statusOf(tab))) return false;
  return nowS() - Math.max(tab.view.updated, sideMarks.unsettled[tab.key] ?? 0) > SETTLE_AFTER_S;
}
const sectionOf = (tab) => (snoozedNow(tab) ? "snoozed" : settled(tab) ? "settled" : isPinned(tab) ? "pinned" : "active");
const rowTabs = () => [...threadList.querySelectorAll(".thread-row")].map((r) => tabByKey(r.dataset.tab)).filter(Boolean);

function markThreadSeen(tab) {
  if (!tab || !guiOn() || document.hidden) return;
  seenAt[tab.key] = tab.view.updated ?? 0;
  save("gui-seen", JSON.stringify(seenAt));
  if (wokeNow(tab)) {
    delete sideMarks.snoozed[tab.key];
    saveMarks();
  }
}

const durationLabel = (s) => (s < 60 ? `${Math.floor(s)}s` : s < 3600 ? `${Math.floor(s / 60)}m` : `${Math.floor(s / 3600)}h ${Math.floor(s / 60) % 60}m`);
function statusSlot(tab) {
  const phase = phaseOf(tab);
  if (phase === "asking") return { icon: "shield-question", text: "Approval", tone: "warning" };
  if (phase === "working") return { icon: "circle-dashed", text: "Working", tone: "info", since: unixOf(lastPrompt(tab)?.at) };
  if (phase === "input") return { icon: "message-circle-question", text: "Input", tone: "input" };
  if (phase === "failed") return tab.view.resume_at ? { icon: "circle-alert", text: "Limited", tone: "warning" } : { icon: "circle-alert", text: "Failed", tone: "error" };
  if (wokeNow(tab) && tab !== active) return { icon: "alarm-clock", text: "Woke", tone: "warning", woke: true };
  if (unseenDone(tab)) return { icon: "circle-check", text: "Done", tone: "success" };
  return null;
}

/** T3 Code's project monogram: the first letter and a later digit, last word's initial or last letter. */
function monogram(name) {
  const words = name.normalize("NFKC").trim().match(/[\p{L}\p{N}]+/gu) ?? [];
  if (!words.length) return "PR";
  const glyphs = [...words[0]];
  const second = glyphs.slice(1).find((g) => /\p{N}/u.test(g)) ?? (words.length > 1 ? [...words.at(-1)][0] : glyphs.at(-1));
  return [...`${glyphs[0]}${second}`.toUpperCase()].slice(0, 2).join("");
}
function projectIcon(name) {
  let hash = 0;
  for (const c of name.normalize("NFKC").trim().toLowerCase() || "project") hash = (hash * 31 + c.codePointAt(0)) % MONOGRAM_COLORS.length;
  const text = monogram(name);
  return `<svg class="monogram" viewBox="0 0 16 16" style="color:${MONOGRAM_COLORS[hash]}" aria-hidden="true"><text x="8" y="10.8" text-anchor="middle" fill="currentColor" font-size="8.25" font-weight="700" textLength="${[...text].length === 1 ? 6 : 12}" lengthAdjust="spacingAndGlyphs">${esc(text)}</text></svg>`;
}

function renderSidebar() {
  if (document.activeElement?.matches("#thread-list input.rename")) return;
  for (const tab of tabs) if (!firstSeen.has(tab.key)) firstSeen.set(tab.key, nowS());
  const projects = [...new Set(tabs.map(projectName))].sort();
  if (ui.project && !projects.includes(ui.project)) ui.project = "";
  const filter = document.getElementById("project-filter");
  const options = ["", ...projects].map((p) => `<option value="${esc(p)}"${p === ui.project ? " selected" : ""}>${p ? esc(p) : "All projects"}</option>`).join("");
  if (filter.dataset.built !== options) {
    filter.innerHTML = options;
    filter.dataset.built = options;
  }
  const inScope = tabs.filter((t) => !ui.project || projectName(t) === ui.project);
  const needle = ui.search.trim().toLowerCase();
  document.getElementById("search-clear").hidden = !ui.search;
  const rows = needle ? searchRows(inScope, needle) : shelfRows(inScope);
  threadList.replaceChildren(...rows);
  if (!rows.length) threadList.innerHTML = `<p class="empty">${tabs.length ? "No threads found" : "No threads yet"}</p>`;
  threadList.querySelectorAll(".thread-row").forEach((row, i) => i < 9 && (row.dataset.jump = `${MOD_LABEL}${i + 1}`));
  tickDurations();
  scheduleWake();
}

function shelfRows(list) {
  const by = { pinned: [], active: [], snoozed: [], settled: [] };
  for (const tab of list) by[sectionOf(tab)].push(tab);
  const anchor = (t) => Math.max(firstSeen.get(t.key) ?? 0, sideMarks.unsettled[t.key] ?? 0);
  by.pinned.sort((a, b) => sideMarks.pins.indexOf(a.key) - sideMarks.pins.indexOf(b.key));
  by.active.sort((a, b) => anchor(b) - anchor(a) || b.view.opened - a.view.opened);
  by.snoozed.sort((a, b) => sideMarks.snoozed[a.key].until - sideMarks.snoozed[b.key].until);
  const settledAt = (t) => sideMarks.settled[t.key] || latestUserAt(t);
  by.settled.sort((a, b) => settledAt(b) - settledAt(a));
  const rows = [...by.pinned, ...by.active].map(cardRow);
  let sink = true;
  const shelf = (kind, list, label, open, visible, action) => {
    if (!list.length) return;
    const head = document.createElement("button");
    head.className = "settled-head";
    head.classList.toggle("sink", sink);
    sink = false;
    head.dataset.shelf = kind;
    head.dataset.open = open;
    head.innerHTML = `<span>${open ? label : `${label} (${list.length})`}</span><span class="line"></span>${icon("chevron-down")}`;
    rows.push(head, ...(open ? visible : list.filter((t) => t === active)).map((t) => slimRow(t, action)));
  };
  shelf("snoozed", by.snoozed, "Snoozed", ui.snoozedOpen, by.snoozed, "wake");
  const shownSettled = by.settled.slice(0, ui.settledShown);
  if (by.settled.includes(active) && !shownSettled.includes(active)) shownSettled.push(active);
  shelf("settled", by.settled, "Settled", ui.settledOpen, shownSettled, "unsettle");
  const hidden = by.settled.length - shownSettled.length;
  if (ui.settledOpen && hidden > 0) {
    const more = document.createElement("button");
    more.className = "show-more";
    more.innerHTML = `${icon("plus")}Show ${Math.min(hidden, SETTLED_PAGE)} more`;
    rows.push(more);
  }
  return rows;
}

function rowShell(tab, cls) {
  const row = document.createElement("div");
  row.className = `thread-row ${cls}`;
  row.setAttribute("role", "button");
  row.tabIndex = 0;
  row.dataset.tab = tab.key;
  row.classList.toggle("active", tab === active);
  const ws = threadOf(tab).workspace;
  row.title = [threadTitle(tab), `${projectName(tab)} · ${machineName(tab.machine)}`, ws?.branch && `Branch ${ws.branch}`, `Model ${modelName(tab)}`, tilde(tab.view.cwd), threadOf(tab).error]
    .filter(Boolean).join("\n");
  return row;
}

const prBadge = (pr) => (pr ? `<span class="pr" data-pr="${esc(pr.url)}" title="Open pull request #${pr.number}">${icon("git-pull-request")}#${pr.number}</span>` : "");
const pinMark = (tab) => (isPinned(tab) ? `<span class="pin" role="button" data-act="unpin" title="Unpin thread">${icon("pin")}${icon("pin-off")}</span>` : "");
const titleHtml = (tab) =>
  ui.renaming === tab.key ? `<input class="rename" value="${esc(threadTitle(tab))}" aria-label="Thread title" spellcheck="false">` : esc(threadTitle(tab));
/** The branch's start shrinks with an ellipsis and its last characters stay, as T3 Code's MiddleTruncate does. */

function cardRow(tab) {
  const slot = statusSlot(tab);
  const phase = phaseOf(tab);
  const woke = !!slot?.woke;
  const row = rowShell(tab, "tall");
  row.classList.toggle("receded", tab !== active && (phase === "working" || (["asking", "done", "idle"].includes(phase) && !unseenDone(tab) && !woke)));
  row.classList.toggle("bright", unseenDone(tab) || woke || phase === "input");
  row.classList.toggle("working", phase === "working" && tab !== active);
  const ws = threadOf(tab).workspace;
  const where = ws?.branch
    ? `${ws.worktree ? icon("folder-git", "dim") : ""}<span class="branch" title="${esc(ws.branch)}">${middle(ws.branch)}</span>`
    : `<span class="branch">${icon("monitor")}<span class="head">${esc(machineName(tab.machine))}</span></span>`;
  const remote = tab.machine !== LOCAL ? `<span class="remote" title="${esc(machineName(tab.machine))}">${icon("monitor")}</span>` : "";
  const state = slot
    ? `<span class="stamp status ${slot.tone}${woke ? " woke" : ""}"${woke ? ` role="button" data-act="dismiss" title="Dismiss Woke notification"` : ""}>${icon(slot.icon)}${slot.text}${slot.since ? ` <span class="since" data-since="${slot.since}"></span>` : ""}</span>`
    : `<span class="stamp when">${esc(ago(latestUserAt(tab)))}</span>`;
  const acts = `<span class="acts">${canSnooze(tab) ? `<span role="button" data-act="snooze" title="Snooze thread">${icon("clock")}</span>` : ""}<span role="button" data-act="settle" title="Settle thread (${MOD_LABEL}⇧S)">${icon("check")}Settle</span></span>`;
  row.innerHTML = `<span class="l1">${projectIcon(projectName(tab))}${pinMark(tab)}<span class="project">${esc(projectName(tab))}</span>${tab.view.bypass ? `<span class="warn" title="Permissions skipped">⚠</span>` : ""}<span class="slot">${state}${acts}</span></span>
    <span class="l2">${titleHtml(tab)}</span>
    <span class="l3">${where}${prBadge(prOf(tab))}<span class="right">${remote}<span class="provider">${icon("claude")}</span></span></span>`;
  return row;
}

function slimRow(tab, action) {
  const row = rowShell(tab, "slim");
  const snooze = sideMarks.snoozed[tab.key];
  const time = action === "wake" ? `<span class="stamp wake-at">${esc(wakeLabel(snooze.until))}</span>` : `<span class="stamp when">${esc(ago(sideMarks.settled[tab.key] || latestUserAt(tab)))}</span>`;
  const act = action === "wake"
    ? `<span role="button" data-act="wake" title="Wake thread now">${icon("alarm-clock-off")}</span>`
    : `<span role="button" data-act="unsettle" title="Un-settle thread">${icon("undo-2")}</span>`;
  row.innerHTML = `${projectIcon(projectName(tab))}<span class="l2">${titleHtml(tab)}</span>${pinMark(tab)}${prBadge(prOf(tab))}<span class="slot">${time}<span class="acts">${act}</span></span>`;
  return row;
}

const wakeLabel = (until) => {
  const s = until - nowS();
  return s <= 0 ? "now" : s < 3600 ? `${Math.max(1, Math.ceil(s / 60))}m` : s < 86400 ? `${Math.ceil(s / 3600)}h` : `${Math.ceil(s / 86400)}d`;
};

let wakeTimer;
function scheduleWake() {
  clearTimeout(wakeTimer);
  const next = Math.min(...Object.values(sideMarks.snoozed).map((s) => s.until).filter((u) => u > nowS()));
  if (Number.isFinite(next)) wakeTimer = setTimeout(() => guiOn() && renderSidebar(), Math.min((next - nowS()) * 1000 + 50, 2 ** 31 - 1));
}

function tickDurations() {
  for (const el of threadList.querySelectorAll("[data-since]")) el.textContent = Number(el.dataset.since) ? durationLabel(Math.max(0, nowS() - el.dataset.since)) : "";
}
setInterval(() => guiOn() && tickDurations(), 1000);

/** Title, project, branch and PR matches first, then threads whose messages hold the text. */
function searchRows(list, needle) {
  const recent = (a, b) => latestUserAt(b) - latestUserAt(a);
  const meta = (t) => `${threadTitle(t)} ${projectName(t)} ${threadOf(t).workspace?.branch ?? ""} ${prOf(t)?.number ?? ""}`.toLowerCase();
  const named = list.filter((t) => meta(t).includes(needle)).sort(recent);
  const said = list.filter((t) => !named.includes(t)).sort(recent)
    .map((t) => [t, threadOf(t).items.find((i) => (i.kind === "user" || i.kind === "text") && i.text.toLowerCase().includes(needle))])
    .filter(([, item]) => item);
  const results = [...named.map((t) => [t, null]), ...said];
  ui.hit = Math.min(ui.hit, Math.max(0, results.length - 1));
  return results.map(([tab, item], i) => {
    const row = rowShell(tab, "result");
    row.classList.toggle("hit", i === ui.hit);
    row.classList.toggle("settled", settled(tab));
    const pr = prOf(tab);
    row.innerHTML = `${projectIcon(projectName(tab))}<span class="body"><span class="top"><span class="l2">${esc(threadTitle(tab))}</span>${pr ? `<span class="pr">#${pr.number}</span>` : ""}<span class="when">${esc(ago(latestUserAt(tab)))}</span></span>${item ? `<span class="snippet">${snippet(item.text, needle)}</span>` : ""}</span>`;
    return row;
  });
}

function snippet(text, needle) {
  const flat = text.replace(/\s+/g, " ");
  const at = flat.toLowerCase().indexOf(needle);
  const start = Math.max(0, at - 30);
  return `${start ? "…" : ""}${esc(flat.slice(start, at))}<mark>${esc(flat.slice(at, at + needle.length))}</mark>${esc(flat.slice(at + needle.length, at + needle.length + 80))}`;
}

function openResult(tab) {
  document.getElementById("thread-search").value = ui.search = "";
  ui.hit = 0;
  activate(tab);
  if (narrow(768)) toggleSidebar(false);
}

/** Settling or snoozing the open thread moves on to the next row still in the active list, as T3 Code does. */
function parkActive(tab) {
  if (tab !== active) return;
  const order = rowTabs();
  const i = order.indexOf(tab);
  const next = [...order.slice(i + 1), ...order.slice(0, i)].find((t) => t !== tab && ["pinned", "active"].includes(sectionOf(t)));
  if (next) activate(next);
}

let undoNotice = null;
let undoClaimed = false;
let noticeTimer;
const sideNotice = document.getElementById("side-notice");
function tell(html, ms = 3000, undoable = false) {
  clearTimeout(noticeTimer);
  if (!undoable) undoNotice = null;
  sideNotice.innerHTML = html;
  sideNotice.hidden = !html;
  if (html) noticeTimer = setTimeout(() => {
    undoNotice = null;
    sideNotice.hidden = true;
  }, ms);
}
/** Settle, snooze and unpin each show "⌘Z to undo" for 5 seconds; repeats of one action undo together. */
function noteUndo(action, undo) {
  if (undoNotice?.action !== action) undoNotice = { action, undos: [] };
  undoNotice.undos.push(undo);
  undoClaimed = true;
  const n = undoNotice.undos.length;
  tell(`${action} ${n} thread${n === 1 ? "" : "s"}, <button data-undo>${MOD_LABEL}Z to undo</button>`, 5000, true);
}
function undoLast() {
  if (!undoNotice) return false;
  for (const undo of undoNotice.undos.reverse()) undo();
  undoNotice = null;
  tell("");
  saveMarks();
  renderSidebar();
  return true;
}
sideNotice.onclick = (e) => e.target.closest("[data-undo]") && undoLast();

/** Runs `change` on the marks and returns a function that puts back what it touched. */
function changeMarks(key, change) {
  const before = JSON.stringify({ pins: sideMarks.pins, ...Object.fromEntries(["settled", "unsettled", "snoozed", "titles", "noAutoSettle"].map((m) => [m, sideMarks[m][key]])) });
  change();
  saveMarks();
  renderSidebar();
  return () => {
    const was = JSON.parse(before);
    sideMarks.pins = was.pins;
    for (const m of ["settled", "unsettled", "snoozed", "titles", "noAutoSettle"]) {
      if (was[m] === undefined) delete sideMarks[m][key];
      else sideMarks[m][key] = was[m];
    }
  };
}

function settle(tab) {
  if (settled(tab)) return;
  parkActive(tab);
  noteUndo("Settled", changeMarks(tab.key, () => {
    sideMarks.settled[tab.key] = nowS();
    delete sideMarks.snoozed[tab.key];
  }));
}
function unsettle(tab) {
  changeMarks(tab.key, () => {
    delete sideMarks.settled[tab.key];
    sideMarks.unsettled[tab.key] = nowS();
  });
}
function pin(tab) {
  if (isPinned(tab)) {
    noteUndo("Unpinned", changeMarks(tab.key, () => (sideMarks.pins = sideMarks.pins.filter((k) => k !== tab.key))));
    return;
  }
  changeMarks(tab.key, () => {
    sideMarks.pins = [...sideMarks.pins, tab.key];
    if (settled(tab)) unsettle(tab);
  });
}
function snooze(tab, until) {
  parkActive(tab);
  noteUndo("Snoozed", changeMarks(tab.key, () => (sideMarks.snoozed[tab.key] = { until, at: nowS() })));
}
const wake = (tab) => changeMarks(tab.key, () => delete sideMarks.snoozed[tab.key]);
function markUnread(tab) {
  seenAt[tab.key] = 0;
  save("gui-seen", JSON.stringify(seenAt));
  renderSidebar();
}

function snoozePresets(now = new Date()) {
  const time = (d) => d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const at = (days, hour) => {
    const d = new Date(now);
    d.setDate(d.getDate() + days);
    d.setHours(hour, 0, 0, 0);
    return d;
  };
  const list = [["In 1 hour", new Date(+now + 3600e3)], ["In 3 hours", new Date(+now + 3 * 3600e3)]];
  if (at(0, 18) - now > 3600e3) list.push(["This evening", at(0, 18)]);
  const tomorrow = at(1, 9);
  list.push(["Tomorrow", tomorrow]);
  const monday = at((1 - now.getDay() + 7) % 7 || 7, 9);
  if (+monday !== +tomorrow) list.push(["Next week", monday, `${monday.toLocaleDateString([], { weekday: "short" })} ${time(monday)}`]);
  return list.map(([label, date, when]) => ({ label, when: when ?? time(date), until: date / 1000 }));
}

const snoozeItems = (tab) => [
  ...snoozePresets().map((p) => ({ label: p.label, hint: p.when, run: () => snooze(tab, p.until) })),
  { label: "Custom…", sep: true, form: (menu) => customSnooze(menu, tab) },
];

function customSnooze(menu, tab) {
  const soon = new Date(Date.now() + 3600e3);
  const local = new Date(soon - soon.getTimezoneOffset() * 60e3).toISOString().slice(0, 16);
  menu.innerHTML = `<form class="custom-snooze"><label>Snooze until<input type="datetime-local" value="${local}" required></label><p class="error"></p><button class="primary">Snooze</button></form>`;
  const form = menu.querySelector("form");
  form.querySelector("input").focus();
  form.onsubmit = (e) => {
    e.preventDefault();
    const until = Date.parse(form.querySelector("input").value) / 1000;
    if (!(until > nowS())) return (form.querySelector(".error").textContent = "Pick a time in the future.");
    closeSideMenu();
    snooze(tab, until);
  };
}

async function copyAndTell(text, what) {
  try {
    await navigator.clipboard.writeText(text);
    tell(`Copied ${esc(what)}`);
  } catch {
    tell(`Couldn't copy ${esc(what)}`);
  }
}
const resumeCommand = (tab) => threadOf(tab).session && `claude --resume ${threadOf(tab).session}`;

function rowMenuItems(tab) {
  const ws = threadOf(tab).workspace;
  const project = projectName(tab);
  const session = threadOf(tab).session;
  return [
    { label: ws?.branch ? `New thread on ${ws.branch}` : `New thread in ${project}`, icon: "message-square-plus", run: () => newThreadIn(tab) },
    isPinned(tab) ? { label: "Unpin thread", icon: "pin-off", hint: `${MOD_LABEL}⇧P`, run: () => pin(tab) } : { label: "Pin thread", icon: "pin", hint: `${MOD_LABEL}⇧P`, run: () => pin(tab) },
    settled(tab) ? { label: "Un-settle thread", icon: "circle-check", run: () => unsettle(tab) } : { label: "Settle thread", icon: "circle-check", hint: `${MOD_LABEL}⇧S`, run: () => settle(tab) },
    sideMarks.snoozed[tab.key] && snoozedNow(tab)
      ? { label: "Wake thread", icon: "clock", run: () => wake(tab) }
      : { label: "Snooze", icon: "clock", disabled: !canSnooze(tab), children: () => snoozeItems(tab) },
    { label: "Rename thread", icon: "pencil", sep: true, run: () => startRename(tab) },
    { label: "Mark unread", icon: "mail-open", disabled: phaseOf(tab) !== "done", run: () => markUnread(tab) },
    { label: ui.project === project ? "Show all projects" : `Filter by ${project}`, icon: "folder", run: () => setProjectFilter(ui.project === project ? "" : project) },
    { label: "Auto-settle behavior", icon: "timer", children: () => [
      { label: "Enabled", checked: !sideMarks.noAutoSettle[tab.key], run: () => changeMarks(tab.key, () => delete sideMarks.noAutoSettle[tab.key]) },
      { label: "Disabled", checked: !!sideMarks.noAutoSettle[tab.key], run: () => changeMarks(tab.key, () => (sideMarks.noAutoSettle[tab.key] = true)) },
    ] },
    { label: "Copy", icon: "copy", sep: true, children: () => [
      { label: "Path", icon: "folder", run: () => copyAndTell(tab.view.cwd, "path") },
      ...(ws?.branch ? [{ label: "Branch", icon: "git-branch", run: () => copyAndTell(ws.branch, "branch") }] : []),
      { label: "Session ID", icon: "hash", disabled: !session, run: () => copyAndTell(session, "session ID") },
      { label: "Resume command", icon: "terminal", hint: `${MOD_LABEL}⇧C`, disabled: !session, run: () => copyAndTell(resumeCommand(tab), "resume command") },
    ] },
    { label: "Close thread", icon: "x", sep: true, danger: true, run: () => closeTab(tab) },
  ];
}

const sideMenu = document.createElement("div");
sideMenu.id = "side-menu";
sideMenu.hidden = true;
document.body.append(sideMenu);

/** A menu at a point or under an element. An item with children swaps the menu for them. */
function openSideMenu(items, at) {
  const show = (list, back) => {
    sideMenu.replaceChildren();
    if (back) {
      const b = document.createElement("button");
      b.className = "back";
      b.innerHTML = `${icon("chevron-right")}${esc(back)}`;
      b.onclick = () => show(items);
      sideMenu.append(b);
    }
    for (const item of list) {
      if (item.sep && sideMenu.children.length) sideMenu.append(Object.assign(document.createElement("hr")));
      const b = document.createElement("button");
      b.className = `${item.danger ? "danger" : ""}${item.children ? " sub" : ""}`;
      b.disabled = !!item.disabled;
      b.innerHTML = `<span class="ic">${item.checked ? icon("check") : item.icon ? icon(item.icon) : ""}</span><span class="label">${esc(item.label)}</span>${item.hint ? `<span class="hint">${esc(item.hint)}</span>` : ""}${item.children ? icon("chevron-right") : ""}`;
      b.onclick = () => {
        if (item.children) return show(item.children(), item.label);
        if (item.form) return item.form(sideMenu);
        closeSideMenu();
        item.run();
      };
      sideMenu.append(b);
    }
    place();
  };
  const place = () => {
    const r = sideMenu.getBoundingClientRect();
    const anchor = at instanceof Element && at.getBoundingClientRect();
    const x = anchor ? anchor.right - r.width : at.x;
    const y = anchor ? anchor.bottom + 4 : at.y;
    sideMenu.style.left = `${Math.max(8, Math.min(x, innerWidth - r.width - 8))}px`;
    sideMenu.style.top = `${Math.max(8, Math.min(y, innerHeight - r.height - 8))}px`;
  };
  sideMenu.hidden = false;
  show(items);
}
const closeSideMenu = () => (sideMenu.hidden = true);
window.addEventListener("mousedown", (e) => !sideMenu.hidden && !e.target.closest("#side-menu") && closeSideMenu(), true);
window.addEventListener("keydown", (e) => {
  if (sideMenu.hidden || e.key !== "Escape") return;
  closeSideMenu();
  e.preventDefault();
  e.stopPropagation();
}, true);

function startRename(tab, where = "row") {
  if (where === "header") {
    const title = document.querySelector("#chat-head .crumb .title");
    if (!title) return;
    title.innerHTML = `<input class="rename" value="${esc(threadTitle(tab))}" aria-label="Thread title" spellcheck="false">`;
    return editTitle(tab, title.querySelector("input"));
  }
  ui.renaming = tab.key;
  renderSidebar();
  const input = threadList.querySelector("input.rename");
  if (input) editTitle(tab, input);
}

/** Enter or blur keeps the trimmed title, Esc keeps the old one; an empty title is refused. */
function editTitle(tab, input) {
  input.focus();
  input.select();
  let done = false;
  const finish = (keep) => {
    if (done) return;
    const title = input.value.trim();
    if (keep && !title) {
      tell("Thread title cannot be empty");
      if (document.activeElement === input) return;
    }
    done = true;
    ui.renaming = null;
    input.onblur = null;
    input.blur();
    if (keep && title && title !== threadTitle(tab)) {
      changeMarks(tab.key, () => {
        if (title === (tab.view.title || firstPrompt(tab))) delete sideMarks.titles[tab.key];
        else sideMarks.titles[tab.key] = title;
      });
    }
    renderSidebar();
    if (tab === active) renderThread();
  };
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      finish(true);
    } else if (e.key === "Escape") {
      e.preventDefault();
      finish(false);
    }
  };
  input.onblur = () => finish(true);
  input.onclick = (e) => e.stopPropagation();
  input.ondblclick = (e) => e.stopPropagation();
}

threadList.addEventListener("click", (e) => {
  const head = e.target.closest(".settled-head");
  if (head) {
    const key = head.dataset.shelf === "snoozed" ? "snoozedOpen" : "settledOpen";
    ui[key] = !ui[key];
    save(head.dataset.shelf === "snoozed" ? "gui-snoozed-open" : "gui-settled-open", String(ui[key]));
    return renderSidebar();
  }
  if (e.target.closest(".show-more")) {
    ui.settledShown += SETTLED_PAGE;
    return renderSidebar();
  }
  const row = e.target.closest(".thread-row");
  const tab = row && tabByKey(row.dataset.tab);
  if (!tab || e.target.closest("input.rename")) return;
  if (row.classList.contains("result")) return openResult(tab);
  const act = e.target.closest("[data-act]")?.dataset.act;
  if (act === "snooze") return openSideMenu(snoozeItems(tab), e.target.closest("[data-act]"));
  if (act) return { settle, unsettle, wake, unpin: pin, dismiss: wake }[act](tab);
  activate(tab);
  const pr = e.target.closest("[data-pr]");
  if (pr) openPr(tab, pr.dataset.pr);
  if (narrow(768)) toggleSidebar(false);
});
threadList.addEventListener("dblclick", (e) => {
  const row = e.target.closest(".thread-row:not(.result)");
  if (row && e.target.closest(".l2") && !e.target.closest("input")) startRename(tabByKey(row.dataset.tab));
});
threadList.addEventListener("contextmenu", (e) => {
  const row = e.target.closest(".thread-row");
  const tab = row && tabByKey(row.dataset.tab);
  if (!tab || e.target.closest("input")) return;
  e.preventDefault();
  openSideMenu(rowMenuItems(tab), { x: e.clientX, y: e.clientY });
});
threadList.addEventListener("keydown", (e) => {
  const row = e.target.closest(".thread-row");
  if (row && e.target === row && (e.key === "Enter" || e.key === " ")) {
    e.preventDefault();
    row.click();
  }
});
threadList.addEventListener("mousemove", (e) => {
  const row = e.target.closest(".thread-row.result");
  const i = [...threadList.querySelectorAll(".thread-row.result")].indexOf(row);
  if (row && i !== ui.hit) {
    ui.hit = i;
    threadList.querySelectorAll(".thread-row.result").forEach((r, j) => r.classList.toggle("hit", j === i));
  }
});

// Files dropped on a row go to that thread, which opens first.
const carriesFiles = (e) => e.dataTransfer?.types.includes("Files");
threadList.addEventListener("dragover", (e) => {
  const row = carriesFiles(e) && e.target.closest(".thread-row");
  threadList.querySelectorAll(".drop").forEach((r) => r !== row && r.classList.remove("drop"));
  if (!row) return;
  e.preventDefault();
  row.classList.add("drop");
});
threadList.addEventListener("dragleave", (e) => !threadList.contains(e.relatedTarget) && threadList.querySelectorAll(".drop").forEach((r) => r.classList.remove("drop")));
threadList.addEventListener("drop", (e) => {
  const row = carriesFiles(e) && e.target.closest(".thread-row");
  const tab = row && tabByKey(row.dataset.tab);
  if (!tab) return;
  e.preventDefault();
  e.stopPropagation();
  row.classList.remove("drop");
  delete terms.dataset.dropping;
  activate(tab);
  attachFiles(tab, [...e.dataTransfer.files]);
});
/** The row under a point the app's window reports in physical pixels, for a drop from Finder. */
function threadAt(position) {
  const row = guiOn() && position && document.elementFromPoint(position.x / devicePixelRatio, position.y / devicePixelRatio)?.closest("#thread-list .thread-row");
  const tab = row && tabByKey(row.dataset.tab);
  if (tab && tab !== active) activate(tab);
  return tab;
}

const threadSearch = document.getElementById("thread-search");
function setSearch(value) {
  threadSearch.value = ui.search = value;
  ui.hit = 0;
  renderSidebar();
}
threadSearch.oninput = () => setSearch(threadSearch.value);
threadSearch.onkeydown = (e) => {
  if (e.isComposing || e.keyCode === 229) return;
  if (e.key === "Escape" && ui.search) {
    e.preventDefault();
    e.stopPropagation();
    return setSearch("");
  }
  const results = threadList.querySelectorAll(".thread-row.result");
  if (!results.length || !["ArrowDown", "ArrowUp", "Enter"].includes(e.key)) return;
  e.preventDefault();
  if (e.key === "Enter") return openResult(tabByKey(results[ui.hit].dataset.tab));
  ui.hit = (ui.hit + (e.key === "ArrowDown" ? 1 : -1) + results.length) % results.length;
  results.forEach((r, i) => r.classList.toggle("hit", i === ui.hit));
  results[ui.hit].scrollIntoView({ block: "nearest" });
};
document.getElementById("search-clear").onclick = () => {
  setSearch("");
  threadSearch.focus();
};

function setProjectFilter(project) {
  ui.project = project;
  ui.settledShown = SETTLED_FIRST;
  save("gui-project", ui.project);
  renderSidebar();
}
document.getElementById("project-filter").onchange = (e) => setProjectFilter(e.target.value);

const projectCount = () => new Set(tabs.map((t) => `${t.machine}:${threadOf(t).workspace?.project ?? t.view.cwd}`)).size;
/** A new session in the thread's folder, on its machine, without the picker. */
async function newThreadIn(tab) {
  if (!tab) return toggleProjects();
  if (narrow(768)) toggleSidebar(false);
  if (pickerOpen()) closeProjects();
  const path = threadOf(tab).workspace?.project ?? tab.view.cwd;
  try {
    if (tab.machine === LOCAL) await openTab(LOCAL, path);
    else await openRemote(tab.machine, path, false);
  } catch (e) {
    tell(`Couldn't open a thread: ${esc(e)}`, 6000);
  }
}
/** T3 Code's chat.new: with more than one project the picker asks where, otherwise the thread opens here. */
function newThread(here = false) {
  if (narrow(768)) toggleSidebar(false);
  if (active && (here || projectCount() <= 1)) return newThreadIn(active);
  toggleProjects();
}
function newThreadWithoutProject() {
  const machine = active?.machine ?? onlineMachines()[0];
  if (!machine) return toggleProjects();
  if (pickerOpen()) closeProjects();
  openTab(machine, null).catch((e) => tell(`Couldn't open a thread: ${esc(e)}`, 6000));
}
async function addProject() {
  if (narrow(768)) toggleSidebar(false);
  await showProjects({ machine: active?.machine });
  if (pickerOpen() && picker.querySelector(".choices")) browseFolders();
}
document.getElementById("new-thread").onclick = (e) => newThread(e.shiftKey);
document.getElementById("add-project").onclick = () => addProject();

/** Called as a tab closes, so ⌘⇧T can bring its conversation back. */
function rememberClosed(tab) {
  const t = threadOf(tab);
  closedThreads.push({ machine: tab.machine, cwd: tab.view.cwd, session: t.session, title: sideMarks.titles[tab.key], pinned: isPinned(tab) });
  closedThreads.splice(0, closedThreads.length - 20);
  for (const m of ["settled", "unsettled", "snoozed", "titles", "noAutoSettle"]) delete sideMarks[m][tab.key];
  sideMarks.pins = sideMarks.pins.filter((k) => k !== tab.key);
  saveMarks();
}

async function reopenClosed() {
  const last = closedThreads.at(-1);
  if (!last) return tell("No closed thread to reopen");
  if (!machines.get(last.machine)?.view.online) return tell(`${esc(machineName(last.machine))} is offline`);
  closedThreads.pop();
  try {
    const args = last.session ? ["--resume", last.session] : [];
    const id = await callOn(last.machine, { cmd: "run", run: { folder: last.cwd, args, env: [] } });
    await refreshTabs(last.machine);
    const tab = tabByKey(keyOf(last.machine, id));
    if (last.title) sideMarks.titles[tab.key] = last.title;
    if (last.pinned) sideMarks.pins.push(tab.key);
    saveMarks();
    activate(tab);
  } catch (e) {
    tell(`Couldn't reopen the thread: ${esc(e)}`, 6000);
  }
}

function copyResume() {
  if (!active) return;
  const command = resumeCommand(active);
  if (command) copyAndTell(command, command);
  else tell("This thread has no session yet");
}

function jumpToRow(n) {
  const tab = rowTabs()[n - 1];
  if (tab) activate(tab);
}

function toggleSidebar(open = !ui.sidebarOpen) {
  ui.sidebarOpen = open;
  if (!narrow(768)) save("gui-sidebar", String(open));
  document.body.dataset.sidebar = open ? "open" : "closed";
}
document.getElementById("sidebar-toggle").innerHTML = icon("panel-left-close");
document.getElementById("add-project").innerHTML = icon("folder-plus");
for (const id of ["sidebar-toggle", "chat-sidebar-toggle"]) document.getElementById(id).onclick = () => toggleSidebar();
onGuiKey("b", () => toggleSidebar());
onGuiKey("n", () => newThread());
onGuiKey("o", () => newThread(), { shift: true });
onGuiKey("n", () => newThread(true), { shift: true });
onGuiKey("n", () => newThreadWithoutProject(), { alt: true });
onGuiKey("s", () => active && (settled(active) ? unsettle(active) : settle(active)), { shift: true });
onGuiKey("p", () => active && pin(active), { shift: true });
onGuiKey("c", () => copyResume(), { shift: true });
onGuiKey("t", () => reopenClosed(), { shift: true });
onGuiKey(["1", "2", "3", "4", "5", "6", "7", "8", "9"], (e) => jumpToRow(Number(e.key)));
document.getElementById("workspace").addEventListener("click", (e) => {
  if (e.target.id === "workspace" && narrow(768) && ui.sidebarOpen) toggleSidebar(false);
});

// ⌘Z undoes the notice's action, even from the composer, until another key or an edit there.
window.addEventListener("keydown", (e) => {
  if (!["Meta", "Control", "Shift", "Alt"].includes(e.key) && !(e.key.toLowerCase() === "z" && (IS_MAC ? e.metaKey : e.ctrlKey))) undoClaimed = false;
  if (!guiOn() || !undoNotice || e.repeat || e.shiftKey || e.altKey || e.key.toLowerCase() !== "z") return;
  if (!(IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey) || e.target.closest?.("#terms")) return;
  if (e.target.closest?.("input, textarea, [contenteditable]") && !(undoClaimed && ui.sidebarOpen)) return;
  e.preventDefault();
  e.stopPropagation();
  undoLast();
}, true);
window.addEventListener("input", () => (undoClaimed = false), true);

// Holding ⌘ for a moment shows which row each ⌘1-9 opens.
let hintTimer;
function showJumpHints(on) {
  clearTimeout(hintTimer);
  if (!on) return delete document.body.dataset.jumpHints;
  hintTimer = setTimeout(() => guiOn() && (document.body.dataset.jumpHints = ""), 200);
}
window.addEventListener("keydown", (e) => !(e.repeat && e.key === MOD_KEY) && showJumpHints(e.key === MOD_KEY || (/^[1-9]$/.test(e.key) && "jumpHints" in document.body.dataset)), true);
window.addEventListener("keyup", (e) => e.key === MOD_KEY && showJumpHints(false), true);
window.addEventListener("blur", () => showJumpHints(false));

const sidebarEl = document.getElementById("sidebar");
function setSidebarWidth(px) {
  const width = Math.round(Math.max(208, Math.min(px, innerWidth - 640)));
  document.documentElement.style.setProperty("--sidebar-width", `${width}px`);
  return width;
}
if (Number(load("gui-sidebar-width"))) setSidebarWidth(Number(load("gui-sidebar-width")));
const sidebarRail = document.getElementById("sidebar-resize");
sidebarRail.onmousedown = (down) => {
  down.preventDefault();
  const left = sidebarEl.getBoundingClientRect().left;
  document.body.classList.add("resizing");
  drag(down, {
    move: (e) => setSidebarWidth(e.clientX - left),
    drop: (e) => save("gui-sidebar-width", String(setSidebarWidth(e.clientX - left))),
  });
  addEventListener("mouseup", () => document.body.classList.remove("resizing"), { once: true });
};
sidebarRail.ondblclick = () => save("gui-sidebar-width", String(setSidebarWidth(SIDEBAR_WIDTH)));

/** The header's project crumb opens a new thread there; a double-click on the title renames it. */
function crumbHtml(tab) {
  const name = projectName(tab);
  return `<button class="project" data-new-here title="New thread in ${esc(name)}">${projectIcon(name)}<span>${esc(name)}</span></button><span class="sep">/</span><span class="title" title="${esc(threadTitle(tab))}">${esc(threadTitle(tab))}</span>`;
}
const crumbEl = document.querySelector("#chat-head .crumb");
crumbEl.addEventListener("click", (e) => e.target.closest("[data-new-here]") && newThreadIn(active));
crumbEl.addEventListener("dblclick", (e) => active && e.target.closest(".title") && !e.target.closest("input") && startRename(active, "header"));
document.getElementById("chat-head").addEventListener("contextmenu", (e) => {
  if (!active || e.target.closest("input, button:not([data-new-here])")) return;
  e.preventDefault();
  openSideMenu(rowMenuItems(active), { x: e.clientX, y: e.clientY });
});
