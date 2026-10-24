Object.assign(ICON_PATHS, {
  "message-square": `<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>`,
  "git-pull-request-arrow": `<circle cx="5" cy="6" r="3"/><path d="M5 9v12"/><circle cx="19" cy="18" r="3"/><path d="m15 9-3-3 3-3"/><path d="M12 6h5a2 2 0 0 1 2 2v7"/>`,
  "git-pull-request-draft": `<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M18 6V5"/><path d="M18 11v-1"/><line x1="6" x2="6" y1="9" y2="21"/>`,
  "git-pull-request-closed": `<circle cx="6" cy="6" r="3"/><path d="M6 9v12"/><path d="m21 3-6 6"/><path d="m21 9-6-6"/><path d="M18 11.5V15"/><circle cx="18" cy="18" r="3"/>`,
  "git-merge": `<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M6 21V9a9 9 0 0 0 9 9"/>`,
  "circle-x": `<circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/>`,
  "circle-dot": `<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="1"/>`,
  "user-check": `<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><polyline points="16 11 18 13 22 9"/>`,
  "user-round": `<circle cx="12" cy="8" r="5"/><path d="M20 21a8 8 0 0 0-16 0"/>`,
  "user-round-x": `<path d="M2 21a8 8 0 0 1 11.873-7"/><circle cx="10" cy="8" r="5"/><path d="m17 17 5 5"/><path d="m22 17-5 5"/>`,
  users: `<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>`,
  pencil: `<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>`,
  "refresh-cw": `<path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>`,
  "rotate-ccw": `<path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/>`,
  "triangle-alert": `<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>`,
  info: `<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>`,
  "list-filter": `<path d="M3 6h18"/><path d="M7 12h10"/><path d="M10 18h4"/>`,
  "trending-up": `<polyline points="22 7 13.5 15.5 8.5 10.5 2 17"/><polyline points="16 7 22 7 22 13"/>`,
  "trending-down": `<polyline points="22 17 13.5 8.5 8.5 13.5 2 7"/><polyline points="16 17 22 17 22 11"/>`,
  gauge: `<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>`,
  "arrow-down-up": `<path d="m3 16 4 4 4-4"/><path d="M7 20V4"/><path d="m21 8-4-4-4 4"/><path d="M17 4v16"/>`,
  "folder-plus": `<path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>`,
});

const chordLabel = (key, { shift = false, alt = false } = {}) =>
  IS_MAC ? `⌘${alt ? "⌥" : ""}${shift ? "⇧" : ""}${key}` : `Ctrl+${alt ? "Alt+" : ""}${shift ? "Shift+" : ""}${key}`;
const agoLong = (unix) => {
  const s = Date.now() / 1000 - unix;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
};

/** The app view's own pages, which take the thread's place beside the sidebar, as T3 Code's routes do. */
const SHELL_PAGES = {};
const pageView = document.getElementById("page-view");
const shellPage = () => (document.body.dataset.page in SHELL_PAGES ? document.body.dataset.page : null);

function openPage(name) {
  if (!guiOn()) setMode("gui");
  if (narrow(768)) toggleSidebar(false);
  closePalette();
  closeSideMenu();
  picker.hidden = true;
  if (!pageView.contains(document.activeElement)) document.activeElement?.blur();
  document.body.dataset.page = name;
  SHELL_PAGES[name].show();
  noteVisit();
}
function closePage() {
  if (!shellPage()) return;
  if (active) activate(active);
  else delete document.body.dataset.page;
}
for (const b of document.querySelectorAll("#sidebar footer [data-open-page]")) b.onclick = () => openPage(b.dataset.openPage);
document.querySelector("#sidebar footer .back").onclick = () => goBack(-1) || closePage();

/** Where ⌘[ and ⌘] go: the threads and pages visited, as T3 Code's router history keeps them. */
const visits = { back: [], forward: [], at: null };
const sameVisit = (a, b) => a?.page === b?.page && a?.tab === b?.tab;
function noteVisit() {
  const here = shellPage() ? { page: shellPage() } : active ? { tab: active.key } : null;
  if (!here || sameVisit(here, visits.at)) return;
  if (visits.at) visits.back.push(visits.at);
  visits.back.splice(0, visits.back.length - 50);
  visits.forward.length = 0;
  visits.at = here;
}
/** Steps back (-1) or forward (1), passing over threads closed since; false when there's nowhere to go. */
function goBack(step) {
  const from = step < 0 ? visits.back : visits.forward;
  const to = step < 0 ? visits.forward : visits.back;
  while (from.length) {
    const next = from.pop();
    if (next.tab && !tabByKey(next.tab)) continue;
    if (visits.at) to.push(visits.at);
    visits.at = next;
    if (next.page) openPage(next.page);
    else activate(tabByKey(next.tab));
    return true;
  }
  return false;
}
onGuiKey("[", () => goBack(-1));
onGuiKey("]", () => goBack(1));

window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || !shellPage() || !paletteEl.hidden || !sideMenu.hidden || e.target.closest?.("input, textarea, select, #panel")) return;
  e.preventDefault();
  goBack(-1) || closePage();
});

/** ⌘K, T3 Code's command palette: actions and recent threads, then projects, settings and every
 * thread once a query is typed. An item opening a `page` pushes it; Backspace on an empty query pops it. */
const paletteEl = document.createElement("div");
paletteEl.id = "palette";
paletteEl.hidden = true;
paletteEl.innerHTML = `<div class="backdrop"></div><div class="popup" role="dialog" aria-label="Command palette">
  <div class="field"><button type="button" class="lead" tabindex="-1"></button><input type="text" autocomplete="off" spellcheck="false" aria-label="Search"></div>
  <div class="results" role="listbox"></div>
  <footer><span><kbd>↑</kbd><kbd>↓</kbd>Navigate</span><span><kbd>↵</kbd>Select</span><span class="pop"><kbd>⌫</kbd>Back</span><span><kbd>Esc</kbd>Close</span></footer></div>`;
document.body.append(paletteEl);
const paletteInput = paletteEl.querySelector("input");
const paletteResults = paletteEl.querySelector(".results");
const palette = { views: [], hi: 0, shown: [], projects: [] };
const paletteOpen = () => !paletteEl.hidden;
const RECENT_THREADS = 12;
const SETTINGS_PAGES = [
  ["general", "General", "settings"],
  ["integrations", "Connectors", "zap"],
  ["projects", "Projects", "folder"],
  ["connectors", "Workflows", "bot"],
  ["account", "Account", "user-round"],
  ["machines", "Machines", "monitor"],
];

const normalize = (text) => String(text ?? "").normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase().replace(/\s+/g, " ").trim();
const startsWord = (field, token) => {
  for (let i = field.indexOf(token); i !== -1; i = field.indexOf(token, i + 1)) if (i === 0 || !/[\p{L}\p{N}]/u.test(field[i - 1])) return true;
  return false;
};
/** T3 Code's ranking: 3 exact, 2 field prefix, 1 every token starts a word, 0 mid-word; a thread's
 * title ties prefix and word-start matches so its recency decides. */
function rankItem(item, query, tokens) {
  let best = 0;
  item.terms.filter(Boolean).forEach((term, index) => {
    const field = normalize(term);
    if (!tokens.every((t) => field.includes(t))) return;
    const rank = field === query ? 3 : field.startsWith(query) ? 2 : tokens.every((t) => startsWord(field, t)) ? 1 : 0;
    const score = index === 0 && item.recency !== undefined ? 1000 + Number(rank === 3) : 1000 - index * 100 + rank;
    best = Math.max(best, (rank > 0 ? 10000 : 0) + score);
  });
  return best;
}
function filterGroups(groups, raw) {
  const query = normalize(raw);
  if (!query) return groups;
  const tokens = query.split(" ");
  return groups
    .map((g) => ({
      ...g,
      items: g.items
        .map((item, index) => ({ item, index, rank: rankItem(item, query, tokens) }))
        .filter(({ item }) => tokens.every((t) => normalize(item.terms.join(" ")).includes(t)))
        .sort((a, b) => b.rank - a.rank || (b.item.recency ?? 0) - (a.item.recency ?? 0) || a.index - b.index)
        .map((r) => r.item),
    }))
    .filter((g) => g.items.length);
}

const projectOfTab = (tab) => threadOf(tab).workspace?.project ?? tab.view.cwd;
function threadSub(tab) {
  const ws = threadOf(tab).workspace;
  return [projectName(tab), tab.machine === LOCAL ? "" : machineName(tab.machine), ws?.branch ?? "", tab === active ? "Current thread" : ""].filter(Boolean).join(" · ");
}
function threadItem(tab) {
  const t = threadOf(tab);
  const pr = prOf(tab);
  const said = t.items.filter((i) => i.kind === "user" || i.kind === "assistant").map((i) => i.text ?? "").join(" ").slice(0, 4000);
  const slot = statusSlot(tab);
  return {
    icon: "message-square",
    title: esc(threadTitle(tab)),
    status: slot ? `<span class="status" data-tone="${slot.tone}" title="${esc(slot.text)}"></span>` : "",
    sub: threadSub(tab),
    time: latestUserAt(tab),
    recency: latestUserAt(tab),
    terms: [threadTitle(tab), pr ? `#${pr.number} ${pr.url}` : "", projectName(tab), t.workspace?.branch ?? "", said, t.session ?? ""],
    run: () => activate(tab),
  };
}
const openProjectIn = (machine, path) => (machine === LOCAL ? openTab(LOCAL, path) : openRemote(machine, path, false)).catch((e) => tell(`Couldn't open a thread: ${esc(e)}`, 6000));
function paletteProjects() {
  const seen = new Map(palette.projects.map((p) => [`${p.machine}:${p.path}`, p]));
  for (const tab of tabs) {
    const path = projectOfTab(tab);
    if (!seen.has(`${tab.machine}:${path}`)) seen.set(`${tab.machine}:${path}`, { machine: tab.machine, path, name: path.split("/").filter(Boolean).pop() || "/" });
  }
  return [...seen.values()];
}
const projectSub = (p) => [p.machine === LOCAL ? "" : machineName(p.machine), tilde(p.path)].filter(Boolean).join(" · ");

function actionItems() {
  const tab = active;
  const project = tab && projectName(tab);
  const pr = tab && prOf(tab);
  const resume = tab && resumeCommand(tab);
  const hasProjects = paletteProjects().length > 0;
  return [
    tab && { icon: "square-pen", title: `New thread in <b>${esc(project)}</b>`, hint: chordLabel("N", { shift: true }), terms: [`new thread in ${project}`, "chat create"], run: () => newThreadIn(tab) },
    hasProjects && { icon: "square-pen", title: "New thread in...", page: "new-thread-in", terms: ["new thread in", "project chat create"] },
    { icon: "square-pen", title: "New thread without a project", hint: chordLabel("N", { alt: true }), terms: ["new thread without a project", "chat create blank"], run: () => newThreadWithoutProject() },
    tab && { icon: "search", title: "Find in current thread", hint: chordLabel("F"), terms: ["find in current thread", "search"], run: () => openFind() },
    pr && { icon: "copy", title: "Copy PR link", sub: pr.url, terms: ["copy pr link", "pull request url"], run: () => copyAndTell(pr.url, "PR link") },
    resume && { icon: "copy", title: "Copy thread ID", sub: threadOf(tab).session, hint: chordLabel("C", { shift: true }), terms: ["copy thread id", "resume session reference"], run: () => copyResume() },
    tab && { icon: "git-pull-request", title: "Show linked pull requests", disabled: !pr, terms: ["show linked pull requests", "pr"], run: () => openSurface("pr") },
    { icon: "folder-plus", title: "Add project", terms: ["add project", "open folder"], run: () => addProject() },
    { icon: "chart-column", title: "Open usage", hint: chordLabel("U"), terms: ["open usage", "cost tokens limits"], run: () => openPage("usage") },
    { icon: "square-terminal", title: "Switch to terminal view", hint: chordLabel("E"), terms: ["switch to terminal view", "mode"], run: () => setMode("terminal") },
    { icon: "settings", title: "Open settings", hint: chordLabel(","), terms: ["open settings", "settings preferences configuration"], run: () => openSettings() },
    tab && { icon: "folder", title: "Project settings", sub: project, terms: ["project settings", project], run: () => openSettings("projects") },
  ].filter(Boolean);
}
async function openSettings(name) {
  if (!settingsOpen()) await toggleSettings();
  if (name) showPage(name);
}

const PALETTE_VIEWS = {
  root: {
    placeholder: "Search commands, projects, and threads...",
    groups(query) {
      const actionsOnly = query.startsWith(">");
      const actions = { label: "Actions", items: actionItems() };
      if (actionsOnly) return filterGroups([actions], query.slice(1));
      if (!normalize(query)) {
        const recent = [...tabs].sort((a, b) => latestUserAt(b) - latestUserAt(a)).slice(0, RECENT_THREADS).map(threadItem);
        return [actions, { label: "Recent Threads", items: recent }].filter((g) => g.items.length);
      }
      const projects = paletteProjects().map((p) => ({
        icon: "folder",
        title: esc(p.name),
        sub: projectSub(p),
        terms: [p.name, p.path, p.machine === LOCAL ? "" : machineName(p.machine)],
        run: () => {
          const latest = tabs.filter((t) => t.machine === p.machine && projectOfTab(t) === p.path).sort((a, b) => latestUserAt(b) - latestUserAt(a))[0];
          return latest ? activate(latest) : openProjectIn(p.machine, p.path);
        },
      }));
      const settings = SETTINGS_PAGES.map(([name, label, ic]) => ({ icon: ic, title: esc(label), sub: "Settings", terms: [label, "settings"], run: () => openSettings(name) }));
      const threadsGroup = { label: "Threads", items: tabs.map(threadItem) };
      return filterGroups([actions, { label: "Projects", items: projects }, { label: "Settings", items: settings }, threadsGroup], query);
    },
  },
  "new-thread-in": {
    placeholder: "Search...",
    groups(query) {
      const items = [
        { icon: "square-pen", title: "No project", terms: ["no project", "without"], run: () => newThreadWithoutProject() },
        ...paletteProjects().map((p) => ({ icon: "folder", title: esc(p.name), sub: projectSub(p), terms: [p.name, p.path], run: () => openProjectIn(p.machine, p.path) })),
      ];
      const [group] = filterGroups([{ label: "Projects", items }], query);
      group?.items.slice(0, 9).forEach((item, i) => (item.hint = chordLabel(String(i + 1))));
      return group ? [group] : [];
    },
  },
};

function openPalette(view = "root") {
  closeSideMenu();
  palette.views = [{ view, query: "" }];
  palette.hi = 0;
  paletteEl.hidden = false;
  paletteInput.value = "";
  renderPalette();
  paletteInput.focus();
  Promise.all(onlineMachines().map((m) => callOn(m, { cmd: "list_projects" }).then((list) => list.map((p) => ({ ...p, machine: m })), () => []))).then((lists) => {
    palette.projects = lists.flat();
    if (paletteOpen()) renderPalette();
  });
}
function closePalette() {
  if (!paletteOpen()) return;
  paletteEl.hidden = true;
  if (!shellPage()) focusInput();
}
const paletteView = () => palette.views.at(-1);

function renderPalette() {
  const { view, query } = paletteView();
  const spec = PALETTE_VIEWS[view];
  const groups = spec.groups(query);
  palette.shown = groups.flatMap((g) => g.items);
  paletteInput.placeholder = spec.placeholder;
  paletteEl.dataset.nested = palette.views.length > 1;
  paletteEl.querySelector(".lead").innerHTML = icon(palette.views.length > 1 ? "arrow-left" : "search");
  let n = 0;
  paletteResults.innerHTML = palette.shown.length
    ? groups.map((g) => `<div class="group" role="group"><div class="label">${esc(g.label)}</div>${g.items.map((item) => {
        const i = n++;
        return `<div class="item${i === palette.hi ? " hi" : ""}" role="option" data-i="${i}"${item.disabled ? ` aria-disabled="true"` : ""}>
          <span class="ic">${icon(item.icon)}</span>
          <span class="text"><span class="title">${item.status ?? ""}<span class="name">${item.title}</span></span>${item.sub ? `<span class="sub">${esc(item.sub)}</span>` : ""}</span>
          ${item.time ? `<span class="time">${agoLong(item.time)}</span>` : ""}${item.hint ? `<span class="hint">${esc(item.hint)}</span>` : ""}${item.page ? icon("chevron-right", "chev") : ""}</div>`;
      }).join("")}</div>`).join("")
    : `<p class="empty">${query.startsWith(">") ? "No matching actions." : "No matching commands, projects, or threads."}</p>`;
  paletteResults.querySelector(".item.hi")?.scrollIntoView({ block: "nearest" });
}

function runPaletteItem(item) {
  if (!item || item.disabled) return;
  if (item.page) {
    paletteView().query = paletteInput.value;
    palette.views.push({ view: item.page, query: "" });
    paletteInput.value = "";
    palette.hi = 0;
    return renderPalette();
  }
  paletteEl.hidden = true;
  Promise.resolve(item.run()).catch((e) => tell(`Unable to run command: ${esc(e)}`, 6000));
}
function popPalette() {
  palette.views.pop();
  paletteInput.value = paletteView().query;
  palette.hi = 0;
  renderPalette();
}
function highlight(i) {
  palette.hi = i;
  for (const el of paletteResults.querySelectorAll(".item")) el.classList.toggle("hi", Number(el.dataset.i) === i);
  paletteResults.querySelector(".item.hi")?.scrollIntoView({ block: "nearest" });
}

paletteInput.addEventListener("input", () => {
  paletteView().query = paletteInput.value;
  palette.hi = -1;
  renderPalette();
});
paletteInput.addEventListener("keydown", (e) => {
  const count = palette.shown.length;
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    if (count) highlight(palette.hi < 0 ? 0 : (palette.hi + (e.key === "ArrowDown" ? 1 : -1) + count) % count);
  } else if (e.key === "Enter") runPaletteItem(palette.shown[Math.max(palette.hi, 0)]);
  else if (e.key === "Escape") closePalette();
  else if (e.key === "Backspace" && !paletteInput.value && palette.views.length > 1) popPalette();
  else return;
  e.preventDefault();
  e.stopPropagation();
});
paletteResults.addEventListener("mousedown", (e) => e.preventDefault());
paletteResults.addEventListener("mousemove", (e) => {
  const el = e.target.closest(".item");
  if (el && Number(el.dataset.i) !== palette.hi) highlight(Number(el.dataset.i));
});
paletteResults.addEventListener("click", (e) => {
  const el = e.target.closest(".item");
  if (el) runPaletteItem(palette.shown[Number(el.dataset.i)]);
});
paletteEl.querySelector(".backdrop").onclick = () => closePalette();
paletteEl.querySelector(".lead").onclick = () => (palette.views.length > 1 ? popPalette() : paletteInput.focus());

onGuiKey("k", () => (paletteOpen() ? closePalette() : openPalette()));
onGuiKey(["1", "2", "3", "4", "5", "6", "7", "8", "9"], (e) => paletteView().view === "new-thread-in" && runPaletteItem(palette.shown[Number(e.key) - 1]), { when: paletteOpen });

/** T3 Code's Usage page: cost or tokens over a window, read from claude's transcripts on each
 * machine, or how much of the subscription's limits is left. */
const USAGE_PERIODS = [
  [1, "Past 24h"],
  [7, "7 days"],
  [30, "30 days"],
  [90, "90 days"],
];
const USAGE_METRICS = [
  ["cost", "Cost", "c"],
  ["tokens", "Tokens", "t"],
  ["limits", "Limits", "l"],
];
const CLAUDE_COLOR = "#d97757";
const usageState = { metric: "limits", days: 30, breakdown: "model", machine: "", rows: [], loaded: false, loading: false, now: 0 };
try {
  const saved = JSON.parse(load("gui-usage-page"));
  if (USAGE_METRICS.some(([m]) => m === saved?.metric)) usageState.metric = saved.metric;
  if (USAGE_PERIODS.some(([d]) => d === saved?.days)) usageState.days = saved.days;
} catch {}
const saveUsagePrefs = () => save("gui-usage-page", JSON.stringify({ metric: usageState.metric, days: usageState.days }));

const usd = (n) => n.toLocaleString("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 2 });
function tokensText(n) {
  for (const [size, unit] of [[1e12, "T"], [1e9, "B"], [1e6, "M"], [1e3, "K"]]) {
    if (n < size) continue;
    const v = n / size;
    return `${v.toFixed(v >= 100 ? 0 : v >= 10 ? 1 : 2).replace(/\.0+$/, "")}${unit}`;
  }
  return Math.round(n).toLocaleString("en-US");
}
const percentText = (share) => (share > 0 && share < 0.001 ? "<0.1%" : `${(share * 100).toFixed(1)}%`);
const rowCost = (r) => (r.cost ? r.cost.input + r.cost.cache_read + r.cost.cache_write + r.cost.output : 0);
const rowTokens = (r) => r.input + r.cached + r.cache_write + r.output;
const monthDay = (unix) => new Date(unix * 1000).toLocaleDateString("en-US", { month: "short", day: "numeric" });
const hourText = (unix) => new Date(unix * 1000).toLocaleTimeString("en-US", { hour: "numeric" });
const localDay = (unix) => {
  const d = new Date(unix * 1000);
  d.setHours(0, 0, 0, 0);
  return d.getTime() / 1000;
};

/** The window's buckets, oldest first: the last 24 hours by hour, or the last `days` calendar days. */
function usageBuckets(days, now = Date.now() / 1000) {
  if (days === 1) {
    const hour = Math.floor(now / 3600) * 3600;
    return Array.from({ length: 24 }, (_, i) => hour - (23 - i) * 3600);
  }
  const day = new Date(now * 1000);
  day.setHours(0, 0, 0, 0);
  return Array.from({ length: days }, (_, i) => {
    const d = new Date(day);
    d.setDate(d.getDate() - (days - 1 - i));
    return d.getTime() / 1000;
  });
}
const bucketOf = (days, hour) => (days === 1 ? hour : localDay(hour));

function summarize() {
  const { days, machine } = usageState;
  const buckets = usageBuckets(days, usageState.now);
  const index = new Map(buckets.map((b, i) => [b, i]));
  const rows = usageState.rows.filter((r) => (!machine || r.machine === machine) && index.has(bucketOf(days, r.hour)));
  const s = { buckets, rows, cost: 0, tokens: 0, input: 0, cached: 0, cacheWrite: 0, output: 0, savings: 0, premium: 0, unpriced: 0, sessions: new Set(),
    byType: { input: 0, cache_read: 0, cache_write: 0, output: 0 }, series: buckets.map(() => ({ cost: 0, tokens: 0 })), models: new Map() };
  for (const r of rows) {
    const cost = rowCost(r);
    const tokens = rowTokens(r);
    Object.assign(s, { cost: s.cost + cost, tokens: s.tokens + tokens, input: s.input + r.input, cached: s.cached + r.cached, cacheWrite: s.cacheWrite + r.cache_write, output: s.output + r.output, savings: s.savings + r.savings, premium: s.premium + r.premium });
    if (!r.cost) s.unpriced++;
    for (const k in s.byType) s.byType[k] += r.cost?.[k] ?? 0;
    s.sessions.add(`${r.machine}:${r.session}`);
    const point = s.series[index.get(bucketOf(days, r.hour))];
    point.cost += cost;
    point.tokens += tokens;
    const m = s.models.get(r.model) ?? { model: r.model, cost: 0, priced: false, tokens: 0, input: 0, cached: 0, cacheWrite: 0, output: 0, sessions: new Set() };
    Object.assign(m, { cost: m.cost + cost, priced: m.priced || !!r.cost, tokens: m.tokens + tokens, input: m.input + r.input, cached: m.cached + r.cached, cacheWrite: m.cacheWrite + r.cache_write, output: m.output + r.output });
    m.sessions.add(`${r.machine}:${r.session}`);
    s.models.set(r.model, m);
  }
  return s;
}

async function fetchUsage() {
  usageState.loading = true;
  renderUsagePage();
  const since = usageBuckets(90)[0];
  const machinesOnline = onlineMachines();
  const lists = await Promise.all(machinesOnline.map((m) => callOn(m, { cmd: "token_usage", since }).then((rows) => rows.map((r) => ({ ...r, machine: m })), () => [])));
  Object.assign(usageState, { rows: lists.flat(), loaded: true, loading: false, now: Date.now() / 1000 });
  if (shellPage() === "usage") renderUsagePage();
}

const segmented = (name, options, current, disabled = false) =>
  `<span class="seg" role="radiogroup" data-seg="${name}">${options.map(([value, label]) => `<button type="button" role="radio" aria-checked="${value === current}" data-value="${value}"${disabled ? " disabled" : ""}>${label}</button>`).join("")}</span>
   <select class="seg-select" data-seg="${name}"${disabled ? " disabled" : ""}>${options.map(([value, label]) => `<option value="${value}"${value === current ? " selected" : ""}>${label}</option>`).join("")}</select>`;
const headHtml = (title, rest = "") =>
  `<header class="page-head" data-tauri-drag-region><button type="button" class="icon side-toggle" title="Toggle sidebar (⌘B)">${icon("panel-left")}</button><h1>${title}</h1>${rest}</header>`;

function usageRange(buckets, days) {
  const first = buckets[0];
  const last = buckets.at(-1);
  if (days === 1) return `${monthDay(first)}, ${hourText(first)} to ${monthDay(last + 3600)}, ${hourText(last + 3600)}`;
  return `${monthDay(first)} to ${monthDay(last)}`;
}

function renderUsagePage() {
  const { metric, days } = usageState;
  const s = summarize();
  const remote = [...machines.keys()].length > 1;
  const machineFilter = remote
    ? `<span class="crumb-sep">›</span><select class="machine-filter" aria-label="Machine"><option value="">All machines</option>${[...machines.keys()].map((m) => `<option value="${esc(m)}"${m === usageState.machine ? " selected" : ""}>${esc(machineName(m))}</option>`).join("")}</select>`
    : "";
  pageView.innerHTML = `${headHtml("Usage", `${machineFilter}<span class="crumb-sep">›</span><span class="provider">${icon("claude")}Claude Code</span>
      <span class="range">${metric === "limits" ? "" : usageRange(s.buckets, days)}</span><span class="spacer"></span>
      ${segmented("metric", USAGE_METRICS.map(([v, l]) => [v, l]), metric)}${segmented("days", USAGE_PERIODS, days, metric === "limits")}
      <button type="button" class="icon refresh" title="Refresh">${icon("refresh-cw")}</button>`)}
    <div class="page-body"><div class="page-wrap usage" data-metric="${metric}"${usageState.loading && usageState.loaded ? ` data-refreshing` : ""}>${
      metric === "limits" ? limitsHtml() : !usageState.loaded ? usageSkeleton() : spendHtml(s)
    }</div></div>`;
  if (metric !== "limits" && usageState.loaded) drawChart(s);
}

function spendHtml(s) {
  const { metric, days } = usageState;
  const cost = metric === "cost";
  const total = cost ? usd(s.cost) : tokensText(s.tokens);
  const sessions = `${s.sessions.size} session${s.sessions.size === 1 ? "" : "s"}`;
  const unpricedShare = s.rows.length ? s.unpriced / s.rows.length : 0;
  const unit = days === 1 ? "Hourly" : "Daily";
  const shares = (title, parts) => {
    const sum = parts.reduce((a, [, v]) => a + v, 0);
    if (!sum) return "";
    const fmt = (v) => (cost ? usd(v) : tokensText(v));
    return `<section class="share"><h2>${title}</h2><div class="share-bar">${parts.filter(([, v]) => v > 0).map(([label, v, mix]) =>
      `<span style="flex-grow:${v};background:color-mix(in srgb, var(--g-fg) ${mix}%, var(--g-bg))" title="${label} · ${fmt(v)} · ${percentText(v / sum)}"></span>`).join("")}</div>
      <div class="legend">${parts.map(([label, v, mix]) => `<span><i style="background:color-mix(in srgb, var(--g-fg) ${mix}%, var(--g-bg))"></i>${label}<b>${fmt(v)}</b></span>`).join("")}</div></section>`;
  };
  const byType = cost
    ? [["Input", s.byType.input, 60], ["Cache read", s.byType.cache_read, 30], ["Cache write", s.byType.cache_write, 72], ["Output", s.byType.output, 100]]
    : [["Input", s.input, 60], ["Cache read", s.cached, 30], ["Cache write", s.cacheWrite, 72], ["Output", s.output, 100]];
  const models = [...s.models.values()].sort((a, b) => (cost ? b.cost - a.cost : 0) || b.tokens - a.tokens);
  const top = Math.max(1e-9, ...models.map((m) => (cost ? m.cost : m.tokens)));
  const modelRows = models.map((m, i) => `<tr data-model="${esc(m.model)}"><td class="n">${i + 1}</td><td class="model"><span>${esc(m.model)}</span><i style="width:min(100%, ${(((cost ? m.cost : m.tokens) / top) * 192).toFixed(1)}px)"></i></td>
    <td class="num">${m.priced ? usd(m.cost) : `<span class="muted">Unpriced</span>`}</td><td class="num share-col">${s.cost ? percentText(m.cost / s.cost) : "—"}</td><td class="num">${tokensText(m.tokens)}</td></tr>`).join("");
  const dayRows = s.buckets.map((b, i) => [b, s.series[i]]).filter(([, p]) => p.tokens > 0).reverse()
    .map(([b, p]) => `<tr><td>${days === 1 ? `${monthDay(b)}, ${hourText(b)}` : monthDay(b)}</td><td class="num">${usd(p.cost)}</td><td class="num">${usd(p.cost)}</td><td class="num">${tokensText(p.tokens)}</td></tr>`).join("");
  const table = usageState.breakdown === "model"
    ? `<table><thead><tr><th class="n">#</th><th>Model</th><th class="num">Cost</th><th class="num share-col">Share</th><th class="num">Tokens</th></tr></thead><tbody>${modelRows}</tbody></table>`
    : `<table><thead><tr><th>${days === 1 ? "Hour" : "Day"}</th><th class="num">Claude Code</th><th class="num">Total</th><th class="num">Tokens</th></tr></thead><tbody>${dayRows}</tbody></table>`;
  return `<section class="overview">
      <div class="lead"><div class="total">${total}</div>
        <p class="muted">${sessions}${cost ? " · API estimate" : ""}${cost && unpricedShare > 0 ? `<span class="info" title="API estimate excludes ${percentText(unpricedShare)} unpriced records.">${icon("info")}</span>` : ""}</p>
        <div class="provider-row"><span class="dot" style="background:${CLAUDE_COLOR}"></span>${icon("claude")}<span class="name">Claude Code<small>${sessions}</small></span><b>${total}</b>
          <small class="line">${cost ? `100.0% of cost · ${tokensText(s.tokens)} tokens` : `100.0% of tokens · ${usd(s.cost)}`}</small></div></div>
      <div class="chart-col"><h2>${unit} ${cost ? "cost" : "processed tokens"}</h2><div class="chart"></div></div></section>
    <section><h2>Totals</h2><dl class="tiles">
      <div><dt>Processed tokens</dt><dd>${tokensText(s.tokens)}</dd></div><div><dt>Cached input</dt><dd>${tokensText(s.cached)}</dd></div>
      <div><dt>Uncached input</dt><dd>${tokensText(s.input)}</dd></div><div><dt>Output</dt><dd>${tokensText(s.output)}</dd></div>
      <div><dt>Cache savings</dt><dd>${usd(s.savings)}</dd></div></dl></section>
    ${s.tokens > 0 ? shares(cost ? "Cost by type" : "Tokens by type", byType) : ""}
    ${cost && s.premium > 0 ? shares("Cost by speed", [["Standard", s.cost - s.premium, 34], ["Fast", s.premium, 66]]) : ""}
    <section class="breakdown"><div class="breakdown-head"><h2>Breakdown</h2>${segmented("breakdown", [["model", "Model"], ["day", days === 1 ? "Hour" : "Day"]], usageState.breakdown)}</div>
      ${s.rows.length ? table : `<p class="muted empty">No activity in this window.</p>`}</section>`;
}

const usageSkeleton = () => `<section class="overview"><div class="lead"><i class="ghost" style="width:144px;height:40px"></i><i class="ghost" style="width:128px;height:16px"></i></div>
  <div class="chart-col"><i class="ghost" style="height:224px;margin-left:64px"></i></div></section><section><i class="ghost" style="height:176px"></i></section>`;

function resetsInText(resetsAt, now) {
  const left = resetsAt - now;
  if (left <= 0) return "resets now";
  if (left < 60) return "Resets momentarily";
  const d = Math.floor(left / 86400);
  const h = Math.floor((left % 86400) / 3600);
  const m = Math.floor((left % 3600) / 60);
  return `resets in ${d ? `${d}d ${h}h` : h ? `${h}h ${m}m` : `${m}m`}`;
}
const LIMIT_WINDOWS = [
  ["five_hour", "Session", 300 * 60],
  ["seven_day", "Weekly", 10080 * 60],
];
const PACE = {
  ahead: ["trending-up", "Ahead of pace: spending faster than the window elapses"],
  on: ["gauge", "On pace with the window"],
  under: ["trending-down", "Under pace: headroom left for the rest of the window"],
};
function limitsHtml() {
  const now = Date.now() / 1000;
  const sources = usageState.machine ? [usageState.machine] : [...machines.keys()];
  const windows = sources.map((m) => machines.get(m)?.usage).find((u) => LIMIT_WINDOWS.some(([k]) => u?.[k]?.resets_at > now));
  if (!windows) return `<p class="muted">No provider on the selected environments reports subscription limits.</p>`;
  const cards = LIMIT_WINDOWS.filter(([k]) => windows[k]?.resets_at > now).map(([k, label, length]) => {
    const w = windows[k];
    const remaining = Math.round(100 - w.used_percentage);
    const elapsed = Math.min(1, Math.max(0, (length - (w.resets_at - now)) / length));
    const lead = w.used_percentage - elapsed * 100;
    const [paceIcon, paceLabel] = PACE[lead > 5 ? "ahead" : lead < -5 ? "under" : "on"];
    const timeLeft = Math.round((1 - elapsed) * 100);
    const at = `clamp(3px, ${timeLeft}%, calc(100% - 3px))`;
    return `<div class="limit-card" data-window="${k}"><div class="limit-text"><span class="label">${label}</span>
        <span class="figure"><b>${remaining}%</b><span class="muted">left <span class="pace" role="img" aria-label="${paceLabel}" title="${paceLabel}">${icon(paceIcon)}</span></span></span></div>
      <div class="limit-bar" title="${remaining}% left, ${resetsInText(w.resets_at, now)}, ${timeLeft}% of the window left">
        <div class="track" style="clip-path:polygon(-1px -1px, calc(${at} - 3px) -1px, calc(${at} - 3px) calc(100% + 1px), calc(${at} + 3px) calc(100% + 1px), calc(${at} + 3px) -1px, calc(100% + 1px) -1px, calc(100% + 1px) calc(100% + 1px), -1px calc(100% + 1px))">
          <span class="fill" style="width:${remaining}%;background:${CLAUDE_COLOR}"></span></div>
        <span class="tick" style="left:${at}"></span>
        <span class="inside"><b>${remaining}%</b><span class="chip">${resetsInText(w.resets_at, now).replace("resets in ", "↻ ")}</span></span></div></div>`;
  });
  return `<section class="limits"><h2>${icon("claude")}Claude Code</h2>${cards.join("")}</section>`;
}

/** Fritsch-Carlson monotone cubic through `points`, as the chart's curveMonotoneX draws it. */
function monotonePath(points) {
  const n = points.length;
  if (n < 2) return n ? `M${points[0][0]},${points[0][1]}` : "";
  const dx = [], slope = [];
  for (let i = 0; i < n - 1; i++) {
    dx.push(points[i + 1][0] - points[i][0]);
    slope.push((points[i + 1][1] - points[i][1]) / dx[i]);
  }
  const t = points.map((_, i) => (i === 0 ? slope[0] : i === n - 1 ? slope[n - 2] : slope[i - 1] * slope[i] <= 0 ? 0 : (3 * (dx[i - 1] + dx[i])) / ((2 * dx[i] + dx[i - 1]) / slope[i - 1] + (dx[i] + 2 * dx[i - 1]) / slope[i])));
  let d = `M${points[0][0]},${points[0][1]}`;
  for (let i = 0; i < n - 1; i++) {
    const h = dx[i] / 3;
    d += `C${points[i][0] + h},${points[i][1] + h * t[i]},${points[i + 1][0] - h},${points[i + 1][1] - h * t[i + 1]},${points[i + 1][0]},${points[i + 1][1]}`;
  }
  return d;
}
function niceTicks(max) {
  if (max <= 0) return [0, 1, 2, 3];
  const raw = max / 3;
  const pow = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * pow).find((s) => s >= raw);
  return [0, step, step * 2, step * 3];
}
function drawChart(s) {
  const holder = pageView.querySelector(".chart");
  if (!holder) return;
  const cost = usageState.metric === "cost";
  const values = s.series.map((p) => (cost ? p.cost : p.tokens));
  const ticks = niceTicks(Math.max(...values));
  const top = ticks.at(-1);
  const [W, H, PAD] = [960, 260, 8];
  const x = (i) => (values.length === 1 ? W / 2 : (i / (values.length - 1)) * W);
  const y = (v) => PAD + (1 - v / top) * (H - PAD);
  const points = values.map((v, i) => [x(i), y(v)]);
  const line = monotonePath(points);
  const fmt = (v) => (cost ? usd(v) : tokensText(v));
  const label = (b) => (usageState.days === 1 ? `${hourText(b)}` : monthDay(b)).toUpperCase();
  const mid = Math.floor((s.buckets.length - 1) / 2);
  holder.innerHTML = `<div class="y-axis">${ticks.slice().reverse().map((t) => `<span>${t === 0 ? "0" : cost ? `$${tokensText(t)}` : tokensText(t)}</span>`).join("")}</div>
    <div class="plot"><svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" aria-label="${esc(usageState.days === 1 ? "Hourly" : "Daily")} ${cost ? "cost" : "tokens"}">
      ${ticks.map((t) => `<line class="grid" x1="0" x2="${W}" y1="${y(t)}" y2="${y(t)}"/>`).join("")}
      <path class="area" d="${line}L${x(values.length - 1)},${H}L${x(0)},${H}Z" fill="${CLAUDE_COLOR}"/>
      <path class="line" d="${line}" stroke="${CLAUDE_COLOR}"/></svg>
      <span class="cursor" hidden></span><div class="tip" hidden></div></div>
    <div class="x-axis"><span>${label(s.buckets[0])}</span><span>${label(s.buckets[mid])}</span><span>${label(s.buckets.at(-1))}</span></div>`;
  const plot = holder.querySelector(".plot");
  const cursor = plot.querySelector(".cursor");
  const tip = plot.querySelector(".tip");
  const today = localDay(Date.now() / 1000);
  const when = (b) => {
    if (usageState.days !== 1) return monthDay(b);
    const day = localDay(b);
    return day === today ? `${hourText(b)} today` : day === today - 86400 ? `${hourText(b)} yesterday` : `${monthDay(b)}, ${hourText(b)}`;
  };
  plot.onpointermove = (e) => {
    const r = plot.getBoundingClientRect();
    const i = Math.max(0, Math.min(values.length - 1, Math.round(((e.clientX - r.left) / r.width) * (values.length - 1))));
    const px = (x(i) / W) * r.width;
    cursor.hidden = tip.hidden = false;
    cursor.style.left = `${px}px`;
    tip.innerHTML = `<p>${when(s.buckets[i])}</p><p class="row">${icon("claude")}<span>Claude Code</span><b>${fmt(values[i])}</b></p><p class="row sum"><span>Total</span><b>${fmt(values[i])}</b></p>`;
    const left = Math.min(Math.max(0, px + 12), r.width - tip.offsetWidth);
    tip.style.left = `${px + 12 + tip.offsetWidth > r.width ? Math.max(0, px - 12 - tip.offsetWidth) : left}px`;
    tip.style.top = `${Math.min(Math.max(0, e.clientY - r.top + 12), r.height - tip.offsetHeight)}px`;
  };
  plot.onpointerleave = () => (cursor.hidden = tip.hidden = true);
}

function setUsage(change) {
  Object.assign(usageState, change);
  saveUsagePrefs();
  renderUsagePage();
}
pageView.addEventListener("click", (e) => {
  if (e.target.closest(".side-toggle")) return toggleSidebar(true);
  if (shellPage() !== "usage") return;
  const seg = e.target.closest(".seg button");
  if (seg) {
    const name = seg.parentElement.dataset.seg;
    return setUsage({ [name]: name === "days" ? Number(seg.dataset.value) : seg.dataset.value });
  }
  if (e.target.closest(".refresh")) return fetchUsage();
});
pageView.addEventListener("change", (e) => {
  if (shellPage() !== "usage") return;
  if (e.target.matches(".machine-filter")) return setUsage({ machine: e.target.value });
  const seg = e.target.closest("select.seg-select");
  if (seg) setUsage({ [seg.dataset.seg]: seg.dataset.seg === "days" ? Number(seg.value) : seg.value });
});
window.addEventListener("keydown", (e) => {
  if (shellPage() !== "usage" || e.repeat || e.metaKey || e.ctrlKey || e.altKey || !paletteEl.hidden || e.target.closest?.("input, textarea, select, [contenteditable]")) return;
  const metric = USAGE_METRICS.find(([, , key]) => key === e.key)?.[0];
  if (metric) setUsage({ metric });
});
onGuiKey(["1", "!", "2", "@", "3", "#", "4", "$"], (e) => {
  if (usageState.metric === "limits") return;
  const i = ["1!", "2@", "3#", "4$"].findIndex((k) => k.includes(e.key));
  setUsage({ days: USAGE_PERIODS[i][0] });
}, { shift: true, when: () => shellPage() === "usage" });

SHELL_PAGES.usage = {
  show() {
    usageState.now = Date.now() / 1000;
    renderUsagePage();
    if (!usageState.loading) fetchUsage();
  },
};
onGuiKey("u", () => openPage("usage"));

