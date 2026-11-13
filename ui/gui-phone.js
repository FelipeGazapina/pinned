/** The phone shell, at 768px and below: a tab bar over four screens (Threads, Needs you, Orchestrator,
 * More) and an open thread on its own. `body[data-phone-screen]` names the screen and is present only
 * at phone width in the app view; `body[data-phone-tabs]` is present while the tab bar shows.
 *
 * The screen is read off the state the app already keeps, so every opener (a row, the palette, a
 * card's Open, a page) lands on the right one without knowing about phones: a Needs you page is
 * Needs you, any other page is More (it was opened from there or stands for it), the thread list
 * while `ui.sidebarOpen` is Threads, and otherwise the open thread, the Orchestrator's on its tab.
 * The desktop's "close the sidebar after picking" calls are what leave Threads for a thread. */

const PHONE_MAX = 768;
const PHONE_TABS = [
  { screen: "threads", label: "Threads", icon: "phone-threads" },
  { screen: "needs-you", label: "Needs you", icon: "side-inbox" },
  { screen: "orchestrator", label: "Orchestrator", icon: "side-bot" },
  { screen: "more", label: "More", icon: "phone-more" },
];
Object.assign(ICON_PATHS, {
  "phone-threads": `<path d="M4 6h16"/><path d="M4 12h16"/><path d="M4 18h10"/>`,
  "phone-more": `<circle cx="5" cy="12" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="19" cy="12" r="1.5"/>`,
});

/** The screen for `phone` (at phone width in the app view), the `page` shown (a shell page's name or
 * "settings"), whether the thread `list` is up, and the open `tab`; null when not a phone. */
function phoneScreenOf({ phone, page, list, tab }) {
  if (!phone) return null;
  if (page === "needs-you") return "needs-you";
  if (page) return "more";
  if (list || !tab) return "threads";
  return isGlobalOrchestratorTab(tab) ? "orchestrator" : "thread";
}
/** The tab bar shows on the four tab screens, and not while the keyboard is up. */
const phoneTabsShown = (screen, typing) => !typing && PHONE_TABS.some((t) => t.screen === screen);

const PHONE_TYPING = "textarea, [contenteditable]:not([contenteditable='false']), input:not([type=checkbox], [type=radio], [type=button], [type=submit], [type=reset], [type=range], [type=color], [type=file])";
/** A touch screen raises its keyboard for a focused text field. */
const phoneTyping = () => matchMedia("(pointer: coarse)").matches && !!document.activeElement?.matches?.(PHONE_TYPING);
const phoneState = () => ({ phone: guiOn() && narrow(PHONE_MAX), page: shellPage() ?? (settingsOpen() ? "settings" : null), list: ui.sidebarOpen, tab: active });
const phoneScreen = () => phoneScreenOf(phoneState());

/** Shows `screen` with the openers the desktop uses; "thread" is the open thread. */
function phoneGo(screen) {
  if (screen === "needs-you" || screen === "more") return openPage(screen);
  if (screen === "orchestrator") return openGlobalChat();
  if (shellPage() || settingsOpen()) {
    if (active) activate(active);
    else delete document.body.dataset.page;
  }
  toggleSidebar(screen === "threads" || !active);
  phoneSync();
}

const phoneTabsEl = document.getElementById("phone-tabs");
phoneTabsEl.innerHTML = PHONE_TABS.map((t) => `<button type="button" data-phone-tab="${t.screen}">${icon(t.icon)}<span class="label">${t.label}</span>${t.screen === "needs-you" ? `<span class="badge" hidden></span>` : ""}</button>`).join("");
phoneTabsEl.addEventListener("click", (e) => {
  const tab = e.target.closest("[data-phone-tab]")?.dataset.phoneTab;
  if (!tab) return;
  if (tab === "threads" && phoneScreen() === "threads") return threadList.scrollTo({ top: 0, behavior: "smooth" });
  phoneGo(tab);
});

function drawPhoneTabs(screen) {
  for (const b of phoneTabsEl.querySelectorAll("[data-phone-tab]")) {
    if (b.dataset.phoneTab === screen) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  }
  const n = navCounts().needsYou;
  const badge = phoneTabsEl.querySelector(".badge");
  badge.hidden = !n;
  setText(badge, n ? String(n) : "");
  const tab = phoneTabsEl.querySelector('[data-phone-tab="needs-you"]');
  tab.setAttribute("aria-label", n ? `Needs you, ${n} waiting` : "Needs you");
}

/** The project filter as chips: All projects, each project, and Add project at the end. */
const phoneChips = document.getElementById("phone-chips");
function drawPhoneChips() {
  const chip = (key, name) => `<button type="button" class="phone-chip" data-project="${esc(key)}" aria-pressed="${ui.project === key}">${esc(name)}</button>`;
  setHtml(phoneChips, `${chip("", "All projects")}${projectGroups().map((g) => chip(g.key, g.name)).join("")}<button type="button" class="phone-chip add" data-add-project aria-label="Add project">${icon("plus")}</button>`);
}
phoneChips.addEventListener("click", (e) => {
  if (e.target.closest("[data-add-project]")) return addProject();
  const chip = e.target.closest("[data-project]");
  if (chip) setProjectFilter(chip.dataset.project);
});

/** Search on the Threads screen is the sidebar's own field, shown under the title while in use. */
let phoneSearching = false;
function phoneSearch(on) {
  phoneSearching = on;
  sidebarEl.toggleAttribute("data-phone-search", on || !!ui.search);
  if (on) threadSearch.focus();
}
document.getElementById("phone-search").onclick = () => phoneSearch(true);
document.getElementById("phone-new").onclick = () => newThread();
threadSearch.addEventListener("blur", () => !ui.search && phoneSearch(false));

/** Projects and counts read from scripts that load after this one, the project grouping module among
 * them, so the tab bar and chips draw once the page has loaded. */
let phoneLoaded = false;
function phoneSync() {
  const screen = phoneScreen();
  const body = document.body;
  if (screen) body.dataset.phoneScreen = screen;
  else delete body.dataset.phoneScreen;
  body.toggleAttribute("data-phone-tabs", !!screen && phoneTabsShown(screen, phoneTyping()));
  if (!screen || !phoneLoaded) return;
  drawPhoneTabs(screen);
  if (screen === "threads") {
    drawPhoneChips();
    sidebarEl.toggleAttribute("data-phone-search", phoneSearching || !!ui.search);
  }
  if (shellPage() === "more") drawMore();
}

// A phone opens on the thread list.
if (narrow(PHONE_MAX)) {
  ui.sidebarOpen = true;
  document.body.dataset.sidebar = "open";
}
matchMedia(`(max-width: ${PHONE_MAX}px)`).addEventListener("change", phoneSync);
new MutationObserver(phoneSync).observe(document.body, { attributeFilter: ["data-page", "data-sidebar", "data-mode"] });
document.addEventListener("focusin", phoneSync);
document.addEventListener("focusout", () => setTimeout(phoneSync));
document.addEventListener("DOMContentLoaded", () => {
  phoneLoaded = true;
  onModelChange(phoneSync);
  phoneSync();
}, { once: true });
phoneSync();

/** A page opened from More goes back to it; the Needs you and More pages are tab screens. */
pageView.addEventListener("click", (e) => e.target.closest(".page-back") && phoneGo("more"));
document.getElementById("settings-nav").addEventListener("click", (e) => {
  if (!phoneScreen() || !e.target.closest("#settings-back")) return;
  e.stopPropagation();
  phoneGo("more");
}, true);

/* The More page: the usage card, then the pages for work and for setup, each with what it holds now
 * when that is known or cheap to learn. */
const phoneMore = { workflows: null, taste: null };

const moreHints = {
  ideas: () => (typeof ideasOpenText === "function" ? ideasOpenText() : ""),
  reviews: () => (navCounts().reviews > 0 ? `${navCounts().reviews} to review` : ""),
  projects: () => {
    const catalog = globalProjectsCatalog();
    const n = catalog.machines.length;
    return catalog.projects.length ? `${catalog.projects.length} on ${n} ${n === 1 ? "machine" : "machines"}` : "";
  },
  taste: () => {
    const taste = tasteUi.saved ?? phoneMore.taste;
    if (!taste) return "";
    const id = tasteProfile(taste.captains);
    return TASTE_PROFILES.find((p) => p.id === id)?.label ?? "Custom";
  },
  machines: () => (navCounts().machinesTotal ? `${navCounts().machinesOnline} online` : ""),
  workflows: () => (phoneMore.workflows ? `${phoneMore.workflows} on` : ""),
  settings: () => {
    const permission = window.Notification && navigator.serviceWorker ? Notification.permission : null;
    return { granted: "Notifications on", default: "Notifications off", denied: "Notifications blocked" }[permission] ?? "";
  },
  terminal: () => "Keys row, paste, select",
  update: () => {
    if (document.body.dataset.update) return `${document.body.dataset.update} ready`;
    const behind = machinesToUpdate().length;
    return behind ? `${behind} ${behind === 1 ? "machine" : "machines"} to update` : document.body.dataset.version ?? "";
  },
};
const MORE_LISTS = [
  [
    { id: "ideas", label: "Ideas", hint: moreHints.ideas, open: () => openPage("ideas") },
    { id: "pull-requests", label: "Pull requests", hint: moreHints.reviews, open: () => openPage("pull-requests") },
    { id: "projects", label: "Projects", hint: moreHints.projects, open: () => openGlobalProjects() },
    { id: "taste", label: "Way of working", hint: moreHints.taste, open: () => openPage("taste") },
    { id: "skills", label: "Skills", open: () => openPage("skills") },
  ],
  [
    { id: "machines", label: "Machines", hint: moreHints.machines, tone: "ok", open: () => openSettings("machines") },
    { id: "accounts", label: "Accounts and connectors", open: () => openSettings("accounts") },
    { id: "workflows", label: "Workflows", hint: moreHints.workflows, open: () => openSettings("connectors") },
    { id: "settings", label: "Settings", hint: moreHints.settings, open: () => openSettings() },
    { id: "terminal", label: "Terminal view", hint: moreHints.terminal, open: () => setMode("terminal") },
    { id: "archive", label: "Archive", open: () => openSettings("archive") },
    { id: "update", label: "Check for updates", hint: moreHints.update, tone: () => (document.body.dataset.update ? "new" : ""), open: () => runUpdate(true) },
  ],
];
const moreRows = MORE_LISTS.flat();

/** The usage card's head and its 5-hour and 7-day bars, from what the machines report. */
function moreUsageHtml() {
  const usage = usageNow();
  const bar = (name, percent) => {
    const shown = Math.round(percent);
    return `<span class="more-bar"><span class="name">${name}</span><span class="track"><span style="width: ${Math.min(100, Math.max(0, shown))}%"></span></span><span class="pct">${shown}%</span></span>`;
  };
  const meta = usage ? `<span class="meta">${esc(usage.provider)} · ${esc(resetsText(usage.resetsAt / 1000))}</span>` : "";
  const bars = usage ? `${bar("5 hours", usage.percent)}${usage.week ? bar("7 days", usage.week.percent) : ""}` : "";
  return `<button type="button" class="more-usage" data-more="usage"><span class="more-usage-head"><span class="label">Usage</span>${meta}</span>${bars}</button>`;
}

function moreListHtml(list) {
  return `<div class="more-list">${list.map((row) => {
    const hint = row.hint?.() ?? "";
    const tone = typeof row.tone === "function" ? row.tone() : row.tone;
    return `<button type="button" class="more-row" data-more="${row.id}"><span class="label">${esc(row.label)}</span>${hint ? `<span class="hint"${tone ? ` data-tone="${tone}"` : ""}>${esc(hint)}</span>` : ""}${icon("chevron-right", "chev")}</button>`;
  }).join("")}</div>`;
}

function drawMore() {
  const body = pageView.querySelector(".more-wrap .more-body");
  if (body) setHtml(body, `${moreUsageHtml()}${MORE_LISTS.map(moreListHtml).join("")}`);
}

function showMore() {
  setPage(`${headHtml("More")}<div class="page-body"><div class="more-wrap"><div class="more-title"><h1>More</h1></div><div class="more-body"></div></div></div>`);
  drawMore();
  connectorCall("get_settings").then((s) => {
    phoneMore.workflows = (s?.connectors ?? []).filter((c) => c.enabled).length;
    drawMore();
  }, () => {});
  if (!tasteUi.saved) connectorCall("get_tastes").then((t) => {
    phoneMore.taste = t;
    drawMore();
  }, () => {});
}
pageView.addEventListener("click", (e) => {
  const button = shellPage() === "more" && e.target.closest("[data-more]");
  if (!button) return;
  if (button.dataset.more === "usage") return openPage("usage");
  moreRows.find((r) => r.id === button.dataset.more)?.open();
});
SHELL_PAGES.more = { show: showMore };
