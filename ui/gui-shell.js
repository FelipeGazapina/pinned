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
  "sliders-horizontal": `<line x1="21" x2="14" y1="4" y2="4"/><line x1="10" x2="3" y1="4" y2="4"/><line x1="21" x2="12" y1="12" y2="12"/><line x1="8" x2="3" y1="12" y2="12"/><line x1="21" x2="16" y1="20" y2="20"/><line x1="12" x2="3" y1="20" y2="20"/><line x1="14" x2="14" y1="2" y2="6"/><line x1="8" x2="8" y1="10" y2="14"/><line x1="16" x2="16" y1="18" y2="22"/>`,
  "arrow-down-up": `<path d="m3 16 4 4 4-4"/><path d="M7 20V4"/><path d="m21 8-4-4-4 4"/><path d="M17 4v16"/>`,
  "file-search": `<path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M4.268 21a2 2 0 0 0 1.727 1H18a2 2 0 0 0 2-2V7l-5-5H6a2 2 0 0 0-2 2v3"/><path d="m9 18-1.5-1.5"/><circle cx="5" cy="14" r="3"/>`,
  "text-search": `<path d="M21 6H3"/><path d="M10 12H3"/><path d="M10 18H3"/><circle cx="17" cy="15" r="3"/><path d="m21 19-1.9-1.9"/>`,
  link: `<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>`,
  "message-square-dashed": `<path d="M10 17H7l-4 4v-7"/><path d="M14 17h1"/><path d="M14 3h1"/><path d="M19 3a2 2 0 0 1 2 2"/><path d="M21 14v1a2 2 0 0 1-2 2"/><path d="M21 9v1"/><path d="M3 9v1"/><path d="M5 3a2 2 0 0 0-2 2"/><path d="M9 3h1"/>`,
  "folder-git-2": `<path d="M9 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v5"/><circle cx="13" cy="12" r="2"/><path d="M18 19c-2.8 0-5-2.2-5-5v8"/><circle cx="20" cy="19" r="2"/>`,
  "layers-2": `<path d="m16.02 12 5.48 3.13a1 1 0 0 1 0 1.74L13 21.74a2 2 0 0 1-2 0l-8.5-4.87a1 1 0 0 1 0-1.74L7.98 12"/><path d="M13 13.74a2 2 0 0 1-2 0L2.5 8.87a1 1 0 0 1 0-1.74L11 2.26a2 2 0 0 1 2 0l8.5 4.87a1 1 0 0 1 0 1.74Z"/>`,
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
  pageView.replaceChildren();
  delete pageView.dataset.condensed;
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
  closeDialog();
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

// As T3 Code's useEscapeToGoBack, Usage leaves a focused field too.
window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || e.defaultPrevented || e.repeat || e.isComposing || !guiOn() || !shellPage() || !paletteEl.hidden || !sideMenu.hidden || !shellDialog.hidden) return;
  if (e.target.closest?.(shellPage() === "usage" ? "#panel" : "input, textarea, select, #panel")) return;
  e.preventDefault();
  document.activeElement?.blur();
  goBack(-1) || closePage();
});

/** ⌘K, T3 Code's command palette: actions and recent threads, then projects, settings and every
 * thread once a query is typed. An item opening a `page` pushes it; Backspace on an empty query pops it. */
const paletteEl = document.createElement("div");
paletteEl.id = "palette";
paletteEl.hidden = true;
paletteEl.innerHTML = `<div class="backdrop"></div><div class="popup" role="dialog" aria-label="Command palette">
  <div class="field"><button type="button" class="lead" tabindex="-1"></button><input type="text" autocomplete="off" spellcheck="false" aria-label="Search"><span class="accessory"></span></div>
  <div class="summary-slot"></div><div class="results" role="listbox"></div>
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
  ["archive", "Archive", "archive"],
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
    if (!seen.has(`${tab.machine}:${path}`)) seen.set(`${tab.machine}:${path}`, { machine: tab.machine, path, name: path.split(/[\\/]/).filter(Boolean).pop() || "/" });
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
    ...promptLinkItems(tab),
    pr && { icon: "copy", title: "Copy PR link", sub: pr.url, terms: ["copy pr link", "pull request url"], run: () => copyAndTell(pr.url, "PR link") },
    resume && { icon: "copy", title: "Copy thread ID", sub: threadOf(tab).session, hint: chordLabel("C", { shift: true }), terms: ["copy thread id", "resume session reference"], run: () => copyResume() },
    tab && { icon: "link", title: "Link pull request to thread", terms: ["link pull request to thread", "pr attach stack"], run: () => linkPrDialog(tab) },
    tab && { icon: "link", title: "Show linked pull requests", disabled: !linkedPrs(tab).length, terms: ["show linked pull requests", "pr stack prs"], run: () => addSurface(tab, { id: "prs", kind: "prs" }) },
    tab && { icon: "rotate-ccw", title: "Restart agent session", terms: ["restart agent session", "reset reload skills plugins mcp"], run: () => restartAgent(tab) },
    tab && { icon: "file-search", title: "Go to file", hint: chordLabel("P"), terms: ["go to file", "open file picker find quick open"], run: () => childSearch("files") },
    tab && { icon: "text-search", title: "Search project contents", hint: chordLabel("F", { shift: true }), terms: ["search project contents", "find in files grep content text search"], run: () => childSearch("content") },
    onlineMachines().length > 0 && { icon: "folder-git-2", title: "New project", page: "new-project", terms: ["new project", "create project empty repository repo git init"] },
    { icon: "folder-plus", title: "Add project", terms: ["add project", "open folder"], run: () => addProject() },
    ...onlineMachines().filter((m) => machines.get(m).view.os === "wsl").map((m) => ({ icon: "folder-plus", title: "Open WSL folder", sub: machineName(m), terms: ["open wsl folder", "add project linux directory"], run: () => addProject(m) })),
    { icon: "git-pull-request", title: "Open pull requests", terms: ["open pull requests", "prs reviews"], run: () => openPage("pull-requests") },
    { icon: "chart-column", title: "Open usage", hint: chordLabel("U"), terms: ["open usage", "cost tokens limits"], run: () => openPage("usage") },
    ...themeActionItems(),
    { icon: "square-terminal", title: "Switch to terminal view", hint: chordLabel("E"), terms: ["switch to terminal view", "mode"], run: () => setMode("terminal") },
    { icon: "settings", title: "Open settings", hint: chordLabel(","), terms: ["open settings", "settings preferences configuration"], run: () => openSettings() },
    tab && { icon: "folder", title: "Project settings", sub: project, terms: ["project settings", project], run: () => openSettings("projects") },
  ].filter(Boolean);
}
/** Claude reads its skills, plugins and MCP servers at launch, so it starts again on the thread's conversation. */
async function restartAgent(tab) {
  refit(tab);
  await callOn(tab.machine, { cmd: "restart_claude", tab: tab.view.id, cols: tab.term.cols, rows: tab.term.rows });
  tell("Agent session restarted. Skills, plugins and MCP servers load again.", 4000);
}
/** T3 Code's child searches: ⌘P's files and ⌘⇧F's contents take the palette's place, and Escape
 * there comes back to it with its query as it was. */
function childSearch(mode) {
  palette.parent = paletteView()?.query ?? "";
  openPicker(mode);
}
function backToPalette() {
  const query = palette.parent ?? "";
  delete palette.parent;
  openPalette();
  paletteInput.value = paletteView().query = query;
  renderPalette();
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
        iconHtml: projectIcon(p.name),
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
  "new-project": {
    placeholder: "Project name",
    unhighlighted: true,
    summary(query) {
      const name = query.trim();
      const machine = newProjectMachine();
      const on = onlineMachines().length > 1 ? ` on ${machineName(machine)}` : "";
      return `<div class="summary"><span class="ic">${icon("folder-git-2")}</span><span class="text"><span class="title"><span class="name">${esc(name || "New project")}</span></span>
        <span class="sub">${esc(name ? `Creates ${NEW_PROJECTS_ROOT}${projectFolderName(name)}${on}` : `Goes in ${NEW_PROJECTS_ROOT}${on}`)}</span></span></div>`;
    },
    accessory: () => `<button type="button" class="create" data-submit${!palette.views.at(-1).query.trim() || palette.creating ? " disabled" : ""}>${palette.creating ? "Creating" : "Create"}<kbd>${IS_MAC ? "⌘↵" : "Ctrl+↵"}</kbd></button>`,
    groups(query) {
      const machines = onlineMachines();
      const name = projectFolderName(query.trim());
      return [
        machines.length > 1 && { label: "Machines", items: machines.map((m) => ({ icon: "monitor", title: esc(machineName(m)), trail: m === newProjectMachine() ? icon("check", "chev") : "", terms: [], keep: true, run: () => (palette.newMachine = m) })) },
        { label: "Options", items: [{ icon: "github", title: "Create private repository on GitHub", sub: query.trim() ? name : "Your GitHub account", trail: `<span class="checkbox${palette.publish ? " on" : ""}">${palette.publish ? icon("check") : ""}</span>`, terms: [], keep: true, run: () => (palette.publish = !palette.publish) }] },
        { label: "", items: [{ icon: "folder-plus", title: "Add existing project", sub: "Open a folder or clone a repository", page: "", terms: [], run: () => addProject() }] },
      ].filter(Boolean);
    },
    submit: (query) => createProject(query),
  },
  "new-thread-in": {
    placeholder: "Search...",
    groups(query) {
      const items = [
        { icon: "square-pen", title: "No project", terms: ["no project", "without"], run: () => newThreadWithoutProject() },
        ...paletteProjects().map((p) => ({ iconHtml: projectIcon(p.name), title: esc(p.name), sub: projectSub(p), terms: [p.name, p.path], run: () => openProjectIn(p.machine, p.path) })),
      ];
      const [group] = filterGroups([{ label: "Projects", items }], query);
      group?.items.slice(0, 9).forEach((item, i) => (item.hint = chordLabel(String(i + 1))));
      return group ? [group] : [];
    },
  },
};

/** T3 Code's newProjectFolderName, which the machine applies too. */
const projectFolderName = (name) => {
  const slug = name.normalize("NFKD").replace(/[\u0300-\u036f]/g, "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+/, "").slice(0, 64).replace(/-+$/, "");
  if (!slug) return "project";
  return /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/.test(slug) ? `${slug}-project` : slug;
};
const NEW_PROJECTS_ROOT = "~/pinned/";
const newProjectMachine = () => (onlineMachines().includes(palette.newMachine) ? palette.newMachine : onlineMachines().includes(active?.machine) ? active.machine : onlineMachines()[0]);
async function createProject(query) {
  const name = query.trim();
  if (!name || palette.creating) return;
  const machine = newProjectMachine();
  const publish = palette.publish;
  palette.creating = true;
  renderPalette();
  try {
    const created = await callOn(machine, { cmd: "create_project", name });
    closePalette();
    tell(created.commit_error ? `Created ${esc(name)} without a first commit. ${esc(created.commit_error)} The project is in ${esc(tilde(created.path))}.` : `Created ${esc(name)} in ${esc(tilde(created.path))}`, 6000);
    await openProjectIn(machine, created.path);
    if (publish) {
      callOn(machine, { cmd: "publish_project", path: created.path }).then(
        (url) => tell(`Published to GitHub: ${esc(url.replace(/^https:\/\/github\.com\//, ""))}`, 6000),
        (e) => tell(`Could not create the GitHub repository. ${esc(e)}`, 8000),
      );
    }
  } catch (e) {
    tell(`Could not create the project. ${esc(e)}`, 6000);
  } finally {
    palette.creating = false;
    if (paletteOpen()) renderPalette();
  }
}

function openPalette(view = "root") {
  closeSideMenu();
  palette.views = [{ view, query: "" }];
  palette.hi = 0;
  Object.assign(palette, { publish: false, newMachine: null });
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
  paletteEl.querySelector(".summary-slot").innerHTML = spec.summary?.(query) ?? "";
  paletteEl.querySelector(".accessory").innerHTML = spec.accessory?.() ?? "";
  paletteEl.dataset.nested = palette.views.length > 1;
  paletteEl.querySelector(".lead").innerHTML = icon(palette.views.length > 1 ? "arrow-left" : "search");
  let n = 0;
  paletteResults.innerHTML = palette.shown.length
    ? groups.map((g) => `<div class="group" role="group"><div class="label">${esc(g.label)}</div>${g.items.map((item) => {
        const i = n++;
        return `<div class="item${i === palette.hi ? " hi" : ""}" role="option" data-i="${i}"${item.disabled ? ` aria-disabled="true"` : ""}>
          <span class="ic">${item.iconHtml ?? icon(item.icon)}</span>
          <span class="text"><span class="title">${item.status ?? ""}<span class="name">${item.title}</span></span>${item.sub ? `<span class="sub">${esc(item.sub)}</span>` : ""}</span>
          ${item.time ? `<span class="time">${agoLong(item.time)}</span>` : ""}${item.trail ?? ""}${item.hint ? `<span class="hint">${esc(item.hint)}</span>` : ""}${item.page !== undefined ? icon("chevron-right", "chev") : ""}</div>`;
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
    palette.hi = PALETTE_VIEWS[item.page].unhighlighted ? -1 : 0;
    return renderPalette();
  }
  if (item.keep) {
    item.run();
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
  } else if (e.key === "Enter" && PALETTE_VIEWS[paletteView().view].submit && (palette.hi < 0 || e.metaKey || e.ctrlKey)) {
    if (!e.isComposing) PALETTE_VIEWS[paletteView().view].submit(paletteInput.value);
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
paletteEl.querySelector(".accessory").onclick = (e) => e.target.closest("[data-submit]") && PALETTE_VIEWS[paletteView().view].submit?.(paletteInput.value);
paletteEl.querySelector(".backdrop").onclick = () => closePalette();
paletteEl.querySelector(".lead").onclick = () => (palette.views.length > 1 ? popPalette() : paletteInput.focus());

onGuiKey("k", () => (paletteOpen() ? closePalette() : openPalette()));
onGuiKey(["1", "2", "3", "4", "5", "6", "7", "8", "9"], (e) => paletteView().view === "new-thread-in" && runPaletteItem(palette.shown[Number(e.key) - 1]), { when: paletteOpen });

/** Draws the page, keeping where its body was scrolled. */
function setPage(html) {
  const top = pageView.querySelector(".page-body")?.scrollTop ?? 0;
  pageView.innerHTML = html;
  const body = pageView.querySelector(".page-body");
  if (body) body.scrollTop = top;
}
const headHtml = (title, rest = "") =>
  `<header class="page-head" data-tauri-drag-region><button type="button" class="icon side-toggle" title="Toggle sidebar (⌘B)">${icon("panel-left")}</button><h1>${title}</h1>${rest}</header>`;

pageView.addEventListener("click", (e) => {
  if (e.target.closest(".side-toggle")) toggleSidebar(true);
});

const shellDialog = document.createElement("div");
shellDialog.id = "shell-dialog";
shellDialog.hidden = true;
document.body.append(shellDialog);
const dialogOpen = () => !shellDialog.hidden;
function showDialog(html) {
  delete shellDialog.dataset.kind;
  shellDialog.innerHTML = `<div class="backdrop"></div><div class="popup" role="dialog" aria-modal="true">${html}<button type="button" class="icon close" title="Close" aria-label="Close">${icon("x")}</button></div>`;
  shellDialog.hidden = false;
  shellDialog.querySelector(".backdrop").onclick = shellDialog.querySelector(".close").onclick = closeDialog;
}
function closeDialog() {
  shellDialog.hidden = true;
  shellDialog.replaceChildren();
}
window.addEventListener("keydown", (e) => {
  if (!dialogOpen() || e.key !== "Escape") return;
  e.preventDefault();
  e.stopPropagation();
  closeDialog();
}, true);

/** T3 Code's Pull requests page: the pull requests of every open project's GitHub repository, from
 * each machine's gh, grouped by the viewer's part in them. A row opens in the side panel. */
const PR_STATES = [
  ["all", "All"],
  ["open", "Open"],
  ["closed", "Closed"],
  ["merged", "Merged"],
];
const PR_INVOLVEMENTS = [
  ["all", "All"],
  ["reviewing", "Reviewing"],
  ["authored", "Authored"],
];
const PR_SORTS = [
  ["ready", "Merge readiness"],
  ["blocked", "Blocked on me"],
  ["updated", "Recently updated"],
  ["newest", "Newest shown"],
  ["oldest", "Oldest shown"],
  ["largest", "Largest shown"],
  ["smallest", "Smallest shown"],
];
const PR_GROUPS = [
  ["authored", "Authored", "pencil"],
  ["reviewing", "Review requested", "eye"],
  ["others", "Others", "users"],
];
const PR_STATE_ICON = {
  open: ["git-pull-request-arrow", "#059669", "Open"],
  draft: ["git-pull-request-draft", "#71717a", "Draft"],
  closed: ["git-pull-request-closed", "#dc2626", "Closed"],
  merged: ["git-merge", "#7c3aed", "Merged"],
};
const PR_CHECKS = { passing: ["circle-check", "#059669", "Checks passing"], failing: ["circle-x", "#dc2626", "Checks failing"], pending: ["circle-dot", "#d97706", "Checks pending"] };
const PR_REVIEW = { approved: ["user-check", "#059669", "Approved"], changes_requested: ["user-round-x", "#d97706", "Changes requested"], review_required: ["user-round", "", "Awaiting review"] };
const PR_PAGE = 100;
const PR_MAX = 500;
const pulls = { state: "open", involvement: "all", sort: "ready", host: null, q: "", limit: PR_PAGE, rows: null, more: false, error: "", loading: false, fetchedAt: 0, seq: 0, closing: new Set() };
try {
  const saved = JSON.parse(load("gui-pr-page"));
  for (const [key, list] of [["state", PR_STATES], ["involvement", PR_INVOLVEMENTS], ["sort", PR_SORTS]]) if (list.some(([v]) => v === saved?.[key])) pulls[key] = saved[key];
  if (typeof saved?.host === "string") pulls.host = saved.host;
} catch {}

/** The open projects' folders, per machine. */
function openProjects() {
  const byMachine = new Map();
  for (const tab of tabs) {
    if (!machines.get(tab.machine)?.view.online) continue;
    if (!byMachine.has(tab.machine)) byMachine.set(tab.machine, new Set());
    byMachine.get(tab.machine).add(projectOfTab(tab));
  }
  return byMachine;
}

async function fetchPulls() {
  const seq = ++pulls.seq;
  const asked = pulls.q;
  Object.assign(pulls, { loading: true, error: "" });
  renderPullsPage();
  const asks = [...openProjects()].map(([m, paths]) =>
    callOn(m, { cmd: "pull_requests", paths: [...paths], state: pulls.state, involvement: pulls.involvement, query: pulls.q, limit: pulls.limit }).then((found) => ({ ...found, rows: found.rows.map((r) => ({ ...r, machine: m })) })));
  const answers = await Promise.allSettled(asks);
  if (seq !== pulls.seq) return;
  const seen = new Set();
  const rows = answers.flatMap((a) => (a.status === "fulfilled" ? a.value.rows : [])).filter((r) => !seen.has(r.url) && seen.add(r.url));
  if (answers.length > 1) rows.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  const failed = answers.find((a) => a.status === "rejected");
  const more = answers.some((a) => a.status === "fulfilled" && a.value.more);
  Object.assign(pulls, { rows: failed && !rows.length && pulls.rows === null ? null : rows, answered: asked, more, error: failed ? String(failed.reason) : "", loading: false, fetchedAt: Date.now() });
  if (shellPage() === "pull-requests") renderPullsPage();
}

const savePulls = () => save("gui-pr-page", JSON.stringify({ state: pulls.state, involvement: pulls.involvement, sort: pulls.sort, host: pulls.host }));
function setPulls(change) {
  Object.assign(pulls, { limit: PR_PAGE }, change);
  savePulls();
  fetchPulls();
}

const prSize = (r) => r.additions + r.deletions;
const prHosts = () => [...new Set((pulls.rows ?? []).map((r) => r.host))];
/** The rows under the host filter, which applies only while the list spans more than one host. */
const shownPulls = () => (pulls.host && prHosts().length > 1 ? pulls.rows.filter((r) => r.host === pulls.host) : pulls.rows);
/** The search's own words, without qualifiers such as label:bug. */
const searchWords = (q) => q.split(/\s+/).filter((w) => w && !/^-?[\w-]+:/.test(w)).join(" ");
/** T3 Code's scorePullRequestMatch: 10 means nothing GitHub showed matched, so the description did. */
function pullMatchScore(r, query) {
  const needle = query.trim().toLowerCase();
  if (!needle) return 0;
  const number = needle.replace(/^#/, "");
  if (/^\d+$/.test(number)) return String(r.number) === number ? 100 : 0;
  const title = r.title.toLowerCase();
  const terms = needle.split(/\s+/).filter(Boolean);
  if (title === needle) return 90;
  if (title.includes(needle)) return 80;
  if (terms.length > 1 && terms.every((t) => title.includes(t))) return 70;
  if (r.head.toLowerCase().includes(needle)) return 60;
  if (r.author.toLowerCase().includes(needle)) return 50;
  if (r.repo.toLowerCase().includes(needle)) return 40;
  if (terms.some((t) => title.includes(t))) return 30;
  return 10;
}
const PR_SORT_RANK = {
  ready: (r) => (r.conflicting ? 4 : r.state === "merged" || r.state === "closed" ? 3 : r.state === "draft" ? 2 : r.checks === "passing" && r.review === "approved" ? 0 : r.checks === "passing" ? 1 : 2),
  author: (r) => (r.state === "merged" || r.state === "closed" ? 6 : r.conflicting ? 0 : r.review === "changes_requested" ? 1 : r.checks === "failing" ? 2 : r.state === "draft" ? 3 : r.checks === "passing" && r.review === "approved" ? 5 : 4),
  reviewer: (r) => (r.state === "open" || r.state === "draft" ? 0 : 1),
};
/** T3 Code's orders within a group; "updated" keeps GitHub's, and a search keeps its relevance. */
function sortPulls(list, group) {
  const { sort } = pulls;
  const recent = (a, b) => b.updated_at.localeCompare(a.updated_at);
  const bySize = (a, b) => Number(prSize(b) > 0) - Number(prSize(a) > 0);
  if (sort === "ready") return pulls.q.trim() ? list : [...list].sort((a, b) => PR_SORT_RANK.ready(a) - PR_SORT_RANK.ready(b) || bySize(a, b) || prSize(a) - prSize(b) || recent(a, b));
  if (sort === "blocked") {
    const role = group === "others" ? pulls.involvement : group;
    if (pulls.q.trim() || role === "all") return list;
    const rank = role === "authored" ? PR_SORT_RANK.author : PR_SORT_RANK.reviewer;
    return [...list].sort((a, b) => rank(a) - rank(b) || recent(a, b));
  }
  if (sort === "newest" || sort === "oldest") return [...list].sort((a, b) => (sort === "newest" ? -1 : 1) * a.created_at.localeCompare(b.created_at) || recent(a, b));
  if (sort === "largest" || sort === "smallest") return [...list].sort((a, b) => bySize(a, b) || (sort === "largest" ? -1 : 1) * (prSize(a) - prSize(b)) || recent(a, b));
  return list;
}

const unixOfIso = (iso) => Date.parse(iso) / 1000 || 0;
function pullRowHtml(r) {
  const [stateIcon, stateColor, stateLabel] = PR_STATE_ICON[r.state];
  const checks = r.checks && PR_CHECKS[r.checks];
  const review = r.review && PR_REVIEW[r.review];
  const mark = ([ic, color, label], cls) => `<span class="${cls}" title="${label}" style="${color ? `color:${color}` : ""}">${icon(ic)}</span>`;
  const labels = r.labels.slice(0, 3).map((l) => `<span class="label" style="--label:#${/^[0-9a-f]{6}$/i.test(l.color) ? l.color : "818181"}">${esc(l.name)}</span>`).join("") + (r.labels.length > 3 ? `<span class="more">+${r.labels.length - 3}</span>` : "");
  const open = r.state === "open" || r.state === "draft";
  const quick = [
    open && `<button type="button" class="danger" data-quick="close" title="Close immediately, or drag across rows to close several"${pulls.closing.has(r.url) ? " disabled" : ""}>${pulls.closing.has(r.url) ? `<span class="spinner small"></span>` : ""}Close</button>`,
    r.state === "open" && `<button type="button" data-quick="merge"${r.stack ? ` disabled title="Open this pull request to merge its stack"` : ""}>Merge</button>`,
    r.state === "draft" && `<button type="button" data-quick="ready">Ready for review</button>`,
    r.state === "closed" && `<button type="button" data-quick="reopen">Reopen</button>`,
  ].filter(Boolean).join("");
  const words = searchWords(pulls.answered ?? "");
  const matched = words && pullMatchScore(r, words) <= 10 ? `<span class="matched" title="Matched in the description"><span class="sr">matched in the description</span>${icon("search")}<span class="wide">matched in the description</span></span>` : "";
  const host = prHosts().length > 1 ? `<span class="host" title="${esc(r.host)}" aria-label="${esc(r.host)}">${icon("github")}</span>` : "";
  const stack = r.stack ? `<span class="stack" role="button" tabindex="0" data-stack title="View stack #${r.stack.number}, layer ${r.stack.position} of ${r.stack.size}" aria-label="Stack ${r.stack.number}, layer ${r.stack.position} of ${r.stack.size}">${icon("layers-2")}${r.stack.position}/${r.stack.size}</span>` : "";
  return `<div class="pr-row${active && threadOf(active).prUrl === r.url ? " selected" : ""}${closeSweep.keys.includes(r.url) ? " sweeping" : ""}" role="button" tabindex="0" data-url="${esc(r.url)}" data-state="${r.state}">
    <span class="state" title="${stateLabel}" style="color:${stateColor}">${icon(stateIcon)}${open && r.state === "open" && r.conflicting ? `<span class="conflict" title="Conflicts with ${esc(r.base)}">${icon("triangle-alert")}</span>` : ""}</span>
    <span class="body"><span class="line1"><span class="num">#${r.number}</span><span class="title">${esc(r.title)}</span>${checks ? mark(checks, "checks") : ""}${review ? mark(review, "review") : ""}
      <span class="right">${stack}${prSize(r) ? `<span class="stat"><span class="add">+${r.additions}</span> <span class="del">-${r.deletions}</span></span>` : ""}</span></span>
    <span class="line2">${matched}${host}${/^https:\/\/avatars\.githubusercontent\.com\//.test(r.avatar) ? `<img src="${esc(r.avatar)}" alt="" referrerpolicy="no-referrer">` : ""}<span class="login">${esc(r.author)}</span><span class="repo">${esc(r.repo)}</span>${labels}<span class="when">${agoLong(unixOfIso(r.updated_at))}</span></span></span>
    ${quick ? `<span class="quick">${quick}</span>` : ""}</div>`;
}

function pullsEmptyHtml() {
  const art = `<span class="art">${icon("git-pull-request")}</span>`;
  if (!openProjects().size) return `<div class="pr-empty">${art}<h2>No projects in this workspace</h2><p>Add a project, and the pull requests from its repository appear here.</p><button type="button" class="outline" data-pr="add-project">Add project</button></div>`;
  if (pulls.q.trim()) return `<div class="pr-empty">${art}<h2>Nothing matches “${esc(pulls.q.trim())}”</h2><p>The hosts were searched for it. Try fewer words, or search by number, author or branch.</p>
    <span class="buttons"><button type="button" class="outline" data-pr="clear">Clear search</button><button type="button" class="outline" data-pr="refresh">${pulls.loading ? "Checking..." : "Check again"}</button></span></div>`;
  if (pulls.state !== "open" || pulls.involvement !== "all" || shownPulls() !== pulls.rows) return `<div class="pr-empty">${art}<h2>Nothing under these filters</h2><p>Widen the state, involvement, host or project filter to see more.</p></div>`;
  return `<div class="pr-empty">${art}<h2>No pull requests</h2><p>Pull requests from every project in this workspace appear here.</p></div>`;
}

function pullsListHtml() {
  if (pulls.rows === null) {
    if (pulls.error) return `<div class="pr-empty"><h2>Could not load pull requests</h2><p>${esc(pulls.error)}</p><span class="buttons"><button type="button" class="outline" data-pr="refresh">Retry</button></span></div>`;
    return `<div class="pr-list" aria-label="Loading pull requests">${Array.from({ length: 7 }, () => `<div class="pr-row ghost-row"><i class="ghost"></i><i class="ghost"></i></div>`).join("")}</div>`;
  }
  const warn = pulls.error ? `<p class="pr-warn">${icon("triangle-alert")}<span>${esc(pulls.error)} Showing the last pull requests loaded.</span><button type="button" class="outline" data-pr="refresh">Retry</button></p>` : "";
  const rows = shownPulls();
  if (!rows.length) return warn + pullsEmptyHtml();
  const foot = !pulls.more ? ""
    : pulls.limit >= PR_MAX ? `<p class="pr-foot muted">Narrow your search to find more pull requests.</p>`
    : `<p class="pr-foot"><button type="button" class="outline" data-pr="more"${pulls.loading ? " disabled" : ""}>${pulls.loading ? "Loading more" : "Load more pull requests"}</button></p>`;
  if (pulls.involvement !== "all") return `${warn}<div class="pr-list">${sortPulls(rows, pulls.involvement).map(pullRowHtml).join("")}</div>${foot}`;
  return warn + PR_GROUPS.map(([key, label, ic]) => {
    const list = rows.filter((r) => r.group === key);
    if (!list.length) return "";
    return `<section class="pr-group" data-group="${key}"><h3>${icon(ic)}<span>${label}</span><span class="count">${list.length}</span><span class="rule"></span></h3><div class="pr-list">${sortPulls(list, key).map(pullRowHtml).join("")}</div></section>`;
  }).join("") + foot;
}

function renderPullsPage() {
  const focused = document.activeElement?.matches?.("#page-view .pr-search input");
  const caret = focused ? document.activeElement.selectionStart : null;
  const stateLabel = PR_STATES.find(([v]) => v === pulls.state)[1];
  const involvementLabel = PR_INVOLVEMENTS.find(([v]) => v === pulls.involvement)[1];
  const filterLabel = [stateLabel, involvementLabel, shownPulls() !== pulls.rows && pulls.host].filter(Boolean).join(" · ");
  const compact = `<span class="condensed"><button type="button" class="crumb-menu" data-pr="state">${stateLabel}${icon("chevron-down")}</button><button type="button" class="crumb-menu" data-pr="involvement">${involvementLabel}${icon("chevron-down")}</button>
    <span class="spacer"></span><button type="button" class="icon" data-pr="search-top" title="Search pull requests" aria-label="Search pull requests">${icon("search")}</button><button type="button" class="icon" data-pr="refresh" title="Refresh pull requests" aria-label="Refresh pull requests">${icon("refresh-cw")}</button></span>`;
  setPage(`${headHtml("Pull Requests", compact)}<div class="page-body"><div class="page-wrap pulls"${pulls.loading && pulls.rows ? " data-refreshing" : ""}${closeSweep.keys.length ? " data-sweeping" : ""}>
    <div class="pr-toolbar"><label class="pr-search">${icon("search")}<input type="search" placeholder="Search pull requests, or label:bug" aria-label="Search pull requests" value="${esc(pulls.q)}" spellcheck="false" autocomplete="off"></label>
      <button type="button" class="outline" data-pr="sort" title="Sort">${icon("arrow-down-up")}<span>Sort</span></button>
      <button type="button" class="outline" data-pr="filters" title="Filters">${icon("list-filter")}<span>${esc(filterLabel)}</span></button>
      <button type="button" class="outline icon-only" data-pr="refresh" title="Refresh pull requests" aria-label="Refresh pull requests">${icon("refresh-cw")}</button></div>
    ${pullsListHtml()}</div></div>`);
  if (focused) {
    const input = pageView.querySelector(".pr-search input");
    input.focus();
    input.setSelectionRange(caret, caret);
  }
}

let pullSearchTimer = 0;
pageView.addEventListener("input", (e) => {
  if (shellPage() !== "pull-requests" || !e.target.matches(".pr-search input")) return;
  pulls.q = e.target.value;
  clearTimeout(pullSearchTimer);
  pullSearchTimer = setTimeout(fetchPulls, 300);
});
pageView.addEventListener("click", (e) => {
  if (shellPage() !== "pull-requests") return;
  const act = e.target.closest("[data-pr]")?.dataset.pr;
  if (act === "refresh") return fetchPulls();
  if (act === "add-project") return addProject();
  if (act === "clear") return setPulls({ q: "" });
  if (act === "sort") return openSideMenu(PR_SORTS.map(([value, label]) => ({ label, checked: pulls.sort === value, run: () => (Object.assign(pulls, { sort: value }), savePulls(), renderPullsPage()) })), e.target.closest("button"));
  const pick = (key, list) => () => list.map(([value, label]) => ({ label, checked: pulls[key] === value, run: () => setPulls({ [key]: value }) }));
  if (act === "filters") {
    const hosts = prHosts();
    const setHost = (host) => (Object.assign(pulls, { host }), savePulls(), renderPullsPage());
    return openSideMenu([
      { label: "State", icon: "git-pull-request", children: pick("state", PR_STATES) },
      { label: "Involvement", icon: "users", children: pick("involvement", PR_INVOLVEMENTS) },
      hosts.length > 1 && { label: "Host", icon: "github", children: () => [{ label: "All hosts", checked: shownPulls() === pulls.rows, run: () => setHost(null) }, ...hosts.map((h) => ({ label: h, checked: pulls.host === h, run: () => setHost(h) }))] },
    ].filter(Boolean), e.target.closest("button"));
  }
  if (act === "state") return openSideMenu(pick("state", PR_STATES)(), e.target.closest("button"));
  if (act === "involvement") return openSideMenu(pick("involvement", PR_INVOLVEMENTS)(), e.target.closest("button"));
  if (act === "more") {
    pulls.limit = Math.min(PR_MAX, pulls.limit + PR_PAGE);
    return fetchPulls();
  }
  if (act === "search-top") {
    pageView.querySelector(".page-body").scrollTop = 0;
    return pageView.querySelector(".pr-search input").focus();
  }
  const quick = e.target.closest("[data-quick]");
  const row = e.target.closest(".pr-row[data-url]");
  if (row && e.target.closest("[data-stack]")) return showStack(row.dataset.url, e.target.closest("[data-stack]"));
  if (quick?.dataset.quick === "close" && closeSweep.swept) return (closeSweep.swept = false);
  if (quick && row) return quickPull(row.dataset.url, quick.dataset.quick);
  if (row && e.target.closest(".checks")) return showChecks(row.dataset.url, e.target.closest(".checks"));
  if (row) openPull(row.dataset.url);
});
pageView.addEventListener("keydown", (e) => {
  const row = e.target.closest?.(".pr-row[data-url]");
  if (row && e.target === row && (e.key === "Enter" || e.key === " ") && shellPage() === "pull-requests") {
    e.preventDefault();
    openPull(row.dataset.url);
  }
});
pageView.addEventListener("contextmenu", (e) => {
  const num = e.target.closest(".pr-row[data-url] .num");
  if (!num || shellPage() !== "pull-requests") return;
  e.preventDefault();
  const url = num.closest(".pr-row").dataset.url;
  openSideMenu([
    { label: "Copy link", icon: "copy", run: () => copyAndTell(url, "PR link") },
    { label: "Copy number", icon: "copy", run: () => copyAndTell(num.textContent, `PR ${num.textContent} number`) },
    { label: "Open on GitHub", icon: "external-link", run: () => openOutside(url) },
  ], { x: e.clientX, y: e.clientY });
});
pageView.addEventListener("scroll", (e) => {
  if (!e.target.matches?.(".page-body")) return;
  const toolbar = pageView.querySelector(".pr-toolbar");
  pageView.dataset.condensed = !!toolbar && e.target.scrollTop > toolbar.offsetTop + toolbar.offsetHeight;
}, true);
/** The pull request's checks, from gh, as T3 Code's checks popover lists them. */
/** A tab on the machine that found the pull request, whose gh can reach it. */
const pullTab = (url) => {
  const machine = pulls.rows?.find((r) => r.url === url)?.machine;
  return machine === active?.machine ? active : tabs.find((t) => t.machine === machine);
};
async function showChecks(url, at) {
  const tab = pullTab(url);
  if (!tab) return openOutside(`${url}/checks`);
  try {
    const pr = await callOn(tab.machine, { cmd: "pull_request", tab: tab.view.id, pr: url });
    const checks = (pr.statusCheckRollup ?? []).map((c) => {
      const result = (c.conclusion || c.state || c.status || "").toLowerCase();
      const ic = /success|neutral|skipped/.test(result) ? "circle-check" : /failure|error|cancel|timed_out|action_required/.test(result) ? "circle-x" : "circle-dot";
      return { label: c.name || c.context || "Check", icon: ic, hint: result.replace(/_/g, " "), run: () => openOutside(c.detailsUrl || c.targetUrl || `${url}/checks`) };
    });
    openSideMenu(checks.length ? checks : [{ label: "No checks", disabled: true, run() {} }], at);
  } catch (e) {
    tell(`Couldn't read the checks: ${esc(e)}`, 6000);
  }
}
/** T3 Code's stack popover: the layers of the pull request's GitHub stack, top first, then its base. */
async function showStack(url, at) {
  const r = pulls.rows?.find((row) => row.url === url);
  const head = { label: `Stack #${r.stack.number}`, disabled: true, run() {} };
  openSideMenu([head, { label: "Loading stack…", disabled: true, run() {} }], at);
  try {
    const stack = await callOn(r.machine, { cmd: "pull_request_stack", repo: r.repo, number: r.number });
    if (sideMenu.hidden) return;
    if (!stack) return openSideMenu([head, { label: "This pull request is no longer in a stack.", disabled: true, run() {} }], at);
    const base = `https://github.com/${r.repo}/pull/`;
    openSideMenu([
      { ...head, label: `Stack #${stack.number}` },
      ...[...stack.layers].reverse().map((l) => ({
        iconHtml: `<span class="pr-tone" data-state="${l.state}">${icon(PR_STATE_ICON[l.state][0])}</span>`,
        label: l.title || l.head,
        hint: `#${l.number} · ${l.head} · ${PR_STATE_ICON[l.state][2]}`,
        trail: l.number === r.number ? icon("check") : "",
        run: () => openPull(`${base}${l.number}`),
      })),
      { label: `↳ ${stack.base}`, disabled: true, run() {} },
    ], at);
  } catch (e) {
    if (!sideMenu.hidden) openSideMenu([head, { label: String(e), disabled: true, run() {} }], at);
  }
}

/** T3 Code's close sweep: pressing a row's Close and dragging over more rows of its group closes
 * every open one passed over, once the press ends. Rows are known by URL, since a refresh may
 * draw the list again under the press. */
const closeSweep = { origin: null, x: 0, y: 0, keys: [], swept: false };
const pullRowOf = (url) => [...pageView.querySelectorAll(".pr-row[data-url]")].find((row) => row.dataset.url === url);
const closeSweepRows = (origin) => [...(origin.closest(".pr-group") ?? origin.closest(".pr-list")).querySelectorAll(".pr-row[data-url]")];
const canCloseRow = (row) => (row.dataset.state === "open" || row.dataset.state === "draft") && !pulls.closing.has(row.dataset.url);
function markCloseSweep(keys) {
  closeSweep.keys = keys;
  for (const row of pageView.querySelectorAll(".pr-row[data-url]")) row.classList.toggle("sweeping", keys.includes(row.dataset.url));
  pageView.querySelector(".pulls")?.toggleAttribute("data-sweeping", keys.length > 0);
}
function closeSweepTo(y) {
  const origin = pullRowOf(closeSweep.origin);
  if (!origin) return;
  const rows = closeSweepRows(origin);
  const scroller = pageView.querySelector(".page-body").getBoundingClientRect();
  const at = Math.min(Math.max(y, scroller.top), scroller.bottom - 1);
  let target = rows[0];
  for (const row of rows) if (row.getBoundingClientRect().top <= at) target = row;
  const [from, to] = [rows.indexOf(origin), rows.indexOf(target)].sort((a, b) => a - b);
  markCloseSweep(rows.slice(from, to + 1).filter(canCloseRow).map((row) => row.dataset.url));
}
pageView.addEventListener("pointerdown", (e) => {
  const button = e.target.closest('[data-quick="close"]');
  if (!button || !e.isPrimary || e.button !== 0 || shellPage() !== "pull-requests") return;
  Object.assign(closeSweep, { origin: button.closest(".pr-row").dataset.url, x: e.clientX, y: e.clientY, keys: [], swept: false });
});
window.addEventListener("pointermove", (e) => {
  if (!closeSweep.origin) return;
  if (!closeSweep.keys.length && Math.hypot(e.clientX - closeSweep.x, e.clientY - closeSweep.y) < 6) return;
  closeSweepTo(e.clientY);
});
window.addEventListener("pointerup", () => {
  const { origin, keys } = closeSweep;
  closeSweep.origin = null;
  if (!origin || !keys.length) return;
  closeSweep.swept = true;
  markCloseSweep([]);
  closePulls(keys);
});
window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || !closeSweep.origin) return;
  e.preventDefault();
  e.stopPropagation();
  closeSweep.origin = null;
  markCloseSweep([]);
}, true);
async function closePulls(urls) {
  const batch = urls.filter((url) => !pulls.closing.has(url));
  if (!batch.length) return;
  batch.forEach((url) => pulls.closing.add(url));
  renderPullsPage();
  const failures = [];
  const outcomes = await Promise.all(batch.map(async (url) => {
    const r = pulls.rows.find((row) => row.url === url);
    const tab = pullTab(url);
    try {
      if (!tab) throw new Error("no thread on its machine to reach it");
      await callOn(tab.machine, { cmd: "pr_action", tab: tab.view.id, pr: url, action: "close" });
      return true;
    } catch (e) {
      failures.push(`#${r?.number ?? "?"}: ${String(e).replace(/^Error: /, "")}`);
      return false;
    } finally {
      pulls.closing.delete(url);
    }
  }));
  const closed = outcomes.filter(Boolean).length;
  const title = failures.length ? `Closed ${closed} of ${batch.length} pull requests` : `Closed ${closed} pull request${closed === 1 ? "" : "s"}`;
  tell(esc([title, ...failures.slice(0, 3)].join(". ")), failures.length ? 8000 : 4000);
  fetchPulls();
}

function openPull(url) {
  if (!active) return openOutside(url);
  openPr(active, url);
  renderPullsPage();
}
async function quickPull(url, action) {
  const tab = pullTab(url);
  if (!tab) return openOutside(url);
  try {
    await callOn(tab.machine, { cmd: "pr_action", tab: tab.view.id, pr: url, action });
    fetchPulls();
  } catch (e) {
    tell(`Couldn't ${action === "ready" ? "mark it ready" : action} the pull request: ${esc(e)}`, 6000);
  }
}
for (const type of ["keydown", "keyup"]) window.addEventListener(type, (e) => e.key === "Shift" && (document.body.dataset.shiftHeld = e.type === "keydown"));
window.addEventListener("blur", () => delete document.body.dataset.shiftHeld);
onGuiKey("f", () => pageView.querySelector(".pr-search input")?.focus(), { when: () => shellPage() === "pull-requests" });
onGuiKey("c", () => copyAndTell(threadOf(active).prUrl, "PR link"), { shift: true, when: () => shellPage() === "pull-requests" && !!active && !!pulls.rows?.some((r) => r.url === threadOf(active).prUrl) });
setInterval(() => shellPage() === "pull-requests" && !document.hidden && fetchPulls(), 5 * 60 * 1000);
document.addEventListener("visibilitychange", () => shellPage() === "pull-requests" && !document.hidden && Date.now() - pulls.fetchedAt > 10000 && fetchPulls());

SHELL_PAGES["pull-requests"] = {
  show() {
    renderPullsPage();
    fetchPulls();
  },
};
