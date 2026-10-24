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
const shellPage = () => (document.body.dataset.page in SHELL_PAGES ? document.body.dataset.page : null);

function openPage(name) {
  if (!guiOn()) setMode("gui");
  if (narrow(768)) toggleSidebar(false);
  closePalette();
  closeSideMenu();
  picker.hidden = true;
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

