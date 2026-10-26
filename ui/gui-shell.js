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
  "chart-no-axes-column": `<line x1="18" x2="18" y1="20" y2="10"/><line x1="12" x2="12" y1="20" y2="4"/><line x1="6" x2="6" y1="20" y2="14"/>`,
  "folder-plus": `<path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>`,
});

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

window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || !guiOn() || !shellPage() || !paletteEl.hidden || !sideMenu.hidden || !shellDialog.hidden || e.target.closest?.("input, textarea, select, #panel")) return;
  e.preventDefault();
  goBack(-1) || closePage();
});

/** ⌘K, T3 Code's command palette: actions and recent threads, then projects, settings and every
 * thread once a query is typed. An item opening a `page` pushes it; Backspace on an empty query pops it. */
const paletteEl = document.createElement("div");
paletteEl.id = "palette";
paletteEl.hidden = true;
paletteEl.innerHTML = `<div class="backdrop"></div><div class="popup" role="dialog" aria-label="Command palette">
  <div class="field"><button type="button" class="lead" tabindex="-1"></button><input type="text" autocomplete="off" spellcheck="false" aria-label="Search"><span class="accessory"></span></div>
  <div class="summary-slot"></div><p class="notice" role="status" hidden></p><div class="results" role="listbox"></div><footer></footer></div>`;
document.body.append(paletteEl);
const paletteInput = paletteEl.querySelector("input");
const paletteResults = paletteEl.querySelector(".results");
const palette = { views: [], hi: 0, shown: [], projects: [], linked: null };
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
        .sort((a, b) => Number(!!a.item.secondary) - Number(!!b.item.secondary) || b.rank - a.rank || (b.item.recency ?? 0) - (a.item.recency ?? 0) || a.index - b.index)
        .map((r) => r.item),
    }))
    .filter((g) => g.items.length);
}

const projectOfTab = (tab) => threadOf(tab).workspace?.project ?? tab.view.cwd;
const machineLabel = (machine) => (machine === LOCAL ? "Local" : machineName(machine));
const metaDot = `<span class="dot">·</span>`;
/** ThreadStatusIndicators' pill, from the phase the sidebar reads too. */
function paletteStatus(tab) {
  const phase = phaseOf(tab);
  if (phase === "asking") return { label: "Pending Approval", tone: "warning" };
  if (phase === "input") return { label: "Awaiting Input", tone: "input" };
  if (phase === "working") return { label: "Working", tone: "info", pulse: true };
  if (phase === "failed") return { label: "Failed", tone: "error" };
  return unseenDone(tab) ? { label: "Completed", tone: "success" } : null;
}
/** ThreadRowLeadingStatus: the pull request's state icon, then the status dot and its label. */
function threadLead(tab) {
  const pr = prOf(tab);
  const state = pr && prStatus(tab, pr)?.state;
  const status = paletteStatus(tab);
  const mark = !pr ? "" : state
    ? `<span class="pr-state" data-state="${state}" role="img" aria-label="PR #${pr.number} ${PR_LOOKS[state].label.toLowerCase()}" title="#${pr.number} ${esc(PR_LOOKS[state].label)}">${icon(PR_LOOKS[state].icon)}</span>`
    : `<span class="pr-state" role="img" aria-label="PR #${pr.number}, status pending">${icon("git-pull-request-arrow")}</span>`;
  const pill = status ? `<span class="status" data-tone="${status.tone}"${status.pulse ? " data-pulse" : ""} role="img" aria-label="${status.label}" title="${status.label}"><i></i><span>${status.label}</span></span>` : "";
  return mark || pill ? `<span class="lead-status">${mark}${pill}</span>` : "";
}
const threadPrs = (tab) => [prOf(tab), ...linkedPrs(tab)].filter((pr, i, all) => pr && all.findIndex((o) => o?.url === pr.url) === i);
const prTerms = (tab) => threadPrs(tab).flatMap((pr) => [`#${pr.number}`, `${pr.url.match(/github\.com\/([^/]+\/[^/]+)/)?.[1] ?? ""}#${pr.number}`, pr.url]);
/** ThreadRowTrailingStatus, after the PR number a query found: a running terminal and the machine of a remote thread. */
function threadTrail(tab, query) {
  const tokens = normalize(query).split(" ").filter(Boolean);
  const hit = tokens.length && threadPrs(tab).find((pr) => tokens.every((t) => normalize(prTerms(tab).filter((term) => term.includes(`#${pr.number}`) || term === pr.url).join(" ")).includes(t)));
  const state = hit && prStatus(tab, hit)?.state;
  const parts = [
    hit && `<span class="pr-num"${state ? ` data-state="${state}"` : ""} aria-label="PR #${hit.number}">${icon(PR_LOOKS[state]?.icon ?? "git-pull-request-arrow")}#${hit.number}</span>`,
    (scriptShells.some((s) => s.tab === tab) || panelOf(tab).surfaces.some((s) => s.kind === "terminal")) && `<span class="terminal-run" role="img" aria-label="Terminal process running" title="Terminal process running">${icon("terminal")}</span>`,
    tab.machine !== LOCAL && `<span class="remote" aria-label="${esc(machineName(tab.machine))}" title="${esc(machineName(tab.machine))}">${icon("monitor")}</span>`,
  ].filter(Boolean);
  return parts.length ? `<span class="trail-status">${parts.join("")}</span>` : "";
}
/** ThreadCommandSubtitle: the project's icon, name and machine, the branch, the agent, and "Current thread". */
function threadSubHtml(tab) {
  const ws = threadOf(tab).workspace;
  return [
    `<span class="part">${projectIcon(projectName(tab), tab)}<span class="cut">${esc(projectName(tab))}</span>${metaDot}<span>${esc(machineLabel(tab.machine))}</span></span>`,
    ws?.branch && `<span class="part">${icon(ws.worktree ? "folder-git-2" : "folder")}<span class="cut">${esc(ws.branch)}</span></span>`,
    `<span class="part agent" title="Claude">${icon("claude")}</span>`,
    tab === active && `<span>Current thread</span>`,
  ].filter(Boolean).join(metaDot);
}
function threadItem(tab, query = "") {
  const t = threadOf(tab);
  const found = palette.search?.query && palette.search.matches.get(tab.key);
  return {
    icon: "message-square",
    title: esc(threadTitle(tab)),
    status: threadLead(tab),
    trail: threadTrail(tab, query),
    subHtml: threadSubHtml(tab),
    excerpt: found ? { ...found, query: palette.search.query } : null,
    time: latestUserAt(tab),
    recency: latestUserAt(tab),
    terms: [threadTitle(tab), ...prTerms(tab), projectName(tab), t.workspace?.branch ?? "", found?.snippet ?? "", t.session ?? ""],
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
/** A thread without a project runs in the home folder, T3 Code's scratch project. */
const isScratch = (p) => tilde(p.path) === "~";
const tabOfProjectAt = (p) => tabs.filter((t) => t.machine === p.machine && projectOfTab(t) === p.path).sort((a, b) => latestUserAt(b) - latestUserAt(a))[0];
/** ProjectLocationDescription: a remote project's machine icon, where it is, and its folder. */
const projectSubHtml = (p) => `<span class="part">${p.machine === LOCAL ? "" : icon("monitor")}<span>${esc(machineLabel(p.machine))}</span></span>${metaDot}<span class="cut">${esc(p.path)}</span>`;
const projectRowIcon = (p) => projectIcon(p.name, tabOfProjectAt(p));
/** The project ⌘N and "New thread in" mean: the open thread's, else the most recent one. */
const contextProject = () => (active ? { machine: active.machine, path: projectOfTab(active), name: projectName(active), tab: active } : paletteProjects()[0] ?? null);

function actionItems() {
  const tab = active;
  const here = contextProject();
  const pr = tab && prOf(tab);
  const projects = paletteProjects();
  const online = onlineMachines();
  return [
    here && { icon: "square-pen", title: `New thread in <b>${esc(here.name)}</b>`, command: "chat.newLocal", terms: ["new thread", "chat", "create", "draft"], run: () => (here.tab ? newThreadIn(here.tab) : openProjectIn(here.machine, here.path)) },
    projects.length > 0 && { icon: "square-pen", title: "New thread in...", page: "new-thread-in", terms: ["new thread", "project", "pick", "choose", "select"] },
    online.length > 0 && { icon: "message-square-dashed", title: "New thread without a project", command: "chat.newWithoutProject", terms: ["new thread", "no project", "without project", "none", "chat"], run: () => newThreadWithoutProject() },
    tab && { icon: "text-search", title: "Find in current thread", command: "chat.find", terms: ["find", "search", "messages", "plans"], run: () => openFind() },
    ...promptLinkItems(tab),
    tab && (pr || threadOf(tab).session) && { icon: "link", title: pr ? "Copy PR link" : "Copy thread ID", sub: pr ? pr.url : threadOf(tab).session, command: "thread.copyReference", terms: ["copy", "pull request", "pr link", "thread id", "reference"], run: () => copyThreadReference() },
    tab && { icon: "link-2", title: "Link pull request to thread", terms: ["link", "pull request", "pr", "attach", "stack"], run: () => linkPrDialog(tab) },
    tab && { icon: "link-2", title: "Show linked pull requests", disabled: !linkedPrs(tab).length, terms: ["pull requests", "linked", "stack", "prs"], run: () => addSurface(tab, { id: "prs", kind: "prs" }) },
    tab && { icon: "rotate-ccw", title: "Restart agent session", terms: ["restart", "reset", "reload", "agent", "session", "skills", "plugins", "mcp"], run: () => restartAgent(tab) },
    { icon: "file-search", title: "Go to file", command: "filePicker.toggle", keep: true, terms: ["go to file", "open file", "file picker", "find file", "quick open"], run: () => childSearch("files") },
    { icon: "text-search", title: "Search project contents", command: "projectSearch.toggle", keep: true, terms: ["search project", "find in files", "grep", "content search", "text search"], run: () => childSearch("content") },
    online.length > 0 && { icon: "folder-git-2", title: "New project", page: "new-project", terms: ["new project", "create project", "empty", "repository", "repo", "git init"] },
    { icon: "folder-plus", title: "Add project", keep: true, terms: ["add project", "folder", "directory", "browse", "clone", "remote", "repository", "repo", "git", "github", "url", "environment"], run: () => startAddProject() },
    ...online.filter((m) => machines.get(m).view.os === "wsl").slice(0, 1).map((m) => ({ icon: "folder-plus", title: "Open WSL folder", sub: machineName(m), keep: true, terms: ["add project", "open", "wsl", "linux", "folder", "directory"], run: () => browseForProject(m) })),
    ...themeActionItems(),
    { icon: "git-pull-request-arrow", title: "Open pull requests", terms: ["pull requests", "prs", "pr", "github", "review", "merge", "branch"], run: () => openPage("pull-requests") },
    { icon: "chart-no-axes-column", title: "Open usage", command: "usage.open", terms: ["usage", "use", "tokens", "cost", "spend", "limits", "stats", "analytics"], run: () => openPage("usage") },
    { icon: "settings", title: "Open settings", terms: ["settings", "preferences", "configuration", "keybindings"], run: () => openSettings() },
    here && { icon: "folder", title: "Project settings", sub: here.name, terms: ["project", "settings", "name", "icon", "scripts", "model", "workspace", "grouping", "checkout", "remove", "t3.json"], run: () => openSettings("projects") },
  ].filter(Boolean);
}
/** Claude reads its skills, plugins and MCP servers at launch, so it starts again on the thread's conversation. */
async function restartAgent(tab) {
  refit(tab);
  await callOn(tab.machine, { cmd: "restart_claude", tab: tab.view.id, cols: tab.term.cols, rows: tab.term.rows });
  tell("<b>Agent session will restart</b> Your next message starts a fresh session.", 4000);
}
/** T3 Code's child searches: ⌘P's files and ⌘⇧F's contents take the palette's place, and Escape
 * there comes back to it with its query as it was. */
function childSearch(mode) {
  if (!active) return tell("Open a project to search its files.");
  palette.parent = paletteView()?.query ?? "";
  paletteEl.hidden = true;
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

/** CommandPalette.logic's linked-thread search: the threads a pull request's link names, while the query is its URL. */
const linkedThreadItems = () =>
  palette.linked.threads.map((tab) => ({ icon: "message-square", title: esc(threadTitle(tab)), sub: "Linked thread", terms: [palette.linked.query, threadTitle(tab)], run: () => activate(tab) }));

const PALETTE_VIEWS = {
  root: {
    placeholder: "Search commands, projects, and threads...",
    mode: (query) => (isBrowseQuery(query) ? rootBrowseView : null),
    groups(query) {
      const actionsOnly = query.startsWith(">");
      const actions = { label: "Actions", items: actionItems() };
      if (actionsOnly) return filterGroups([actions], query.slice(1));
      const byRecent = [...tabs].sort((a, b) => latestUserAt(b) - latestUserAt(a));
      if (!normalize(query)) return [actions, { label: "Recent Threads", items: byRecent.slice(0, RECENT_THREADS).map((t) => threadItem(t)) }].filter((g) => g.items.length);
      searchThreads(query);
      const projects = paletteProjects().map((p) => ({
        iconHtml: projectRowIcon(p),
        title: esc(p.name),
        subHtml: projectSubHtml(p),
        terms: [p.name, p.path, machineLabel(p.machine)],
        run: () => (tabOfProjectAt(p) ? activate(tabOfProjectAt(p)) : openProjectIn(p.machine, p.path)),
      }));
      const linked = palette.linked && palette.linked.query === query;
      const threadsGroup = { label: "Threads", items: linked ? linkedThreadItems() : byRecent.map((t) => threadItem(t, query)) };
      return filterGroups([actions, { label: "Projects", items: projects }, { label: "Settings", items: settingsSearchItems() }, threadsGroup], query);
    },
    empty: (query) => (palette.search?.pending ? "Searching thread messages…" : query.startsWith(">") ? "No matching actions." : "No matching commands, projects, or threads."),
  },
  "new-project": {
    placeholder: "Project name",
    unhighlighted: true,
    summary(query) {
      const name = query.trim();
      const machine = newProjectMachine();
      const label = onlineMachines().length > 1 ? ` on ${machineName(machine)}` : "";
      return `<div class="summary"><span class="ic">${icon("folder-git-2")}</span><span class="text"><span class="title"><span class="name">${esc(name || "New project")}</span></span>
        <span class="sub">${esc(`${name ? `Creates ${NEW_PROJECTS_ROOT}${projectFolderName(name)}` : `Goes in ${NEW_PROJECTS_ROOT}`}${label}`)}</span></span></div>`;
    },
    accessory: () => submitButton(palette.creating ? "Creating" : "Create", palette.hi >= 0 ? `${IS_MAC ? "⌘" : "Ctrl"} Enter` : "Enter", !paletteView().query.trim() || palette.creating),
    footerAction: () => (palette.hi < 0 ? "Create" : palette.shown[palette.hi]?.toggle ? "Toggle" : "Select"),
    groups(query) {
      const machines = onlineMachines();
      const name = projectFolderName(query.trim());
      const owner = account?.github?.login ?? null;
      return [
        machines.length > 1 && { label: "Environments", items: machines.map((m) => ({ icon: "monitor", title: esc(machineName(m)), sub: m === LOCAL ? "This device" : m, trail: m === newProjectMachine() ? icon("check", "chev") : "", terms: [], keep: true, run: () => (palette.newMachine = m) })) },
        { label: "Options", items: [{ icon: "github", title: "Create private repository on GitHub", sub: query.trim() ? (owner ? `${owner}/${name}` : name) : owner ?? "Your GitHub account", trail: `<span class="checkbox${palette.publish ? " on" : ""}">${palette.publish ? icon("check") : ""}</span>`, terms: [], keep: true, toggle: true, run: () => (palette.publish = !palette.publish) }] },
        { label: "", items: [{ icon: "folder-plus", title: "Add existing project", sub: "Open a folder or clone a repository", trail: icon("chevron-right", "chev"), terms: [], keep: true, run: () => existingProjectSources(newProjectMachine()) }] },
      ].filter(Boolean);
    },
    submit: (query) => createProject(query),
  },
  "new-thread-in": {
    placeholder: "Search...",
    groups(query) {
      const all = paletteProjects();
      const at = all[0] && isScratch(all[0]) ? 0 : 1;
      const projects = all.filter((p) => !isScratch(p)).map((p, i) => ({ iconHtml: projectRowIcon(p), title: esc(p.name), subHtml: projectSubHtml(p), terms: [p.name, p.path, machineLabel(p.machine)], command: i < 9 ? `thread.jump.${i + 1}` : null, run: () => (active && active.machine === p.machine && projectOfTab(active) === p.path ? newThreadIn(active) : openProjectIn(p.machine, p.path)) }));
      const none = onlineMachines().length > 0 && { icon: "message-square-dashed", title: "No project", command: "chat.newWithoutProject", terms: ["no project", "without project", "none"], run: () => newThreadWithoutProject() };
      return filterGroups([{ label: "Projects", items: [...projects.slice(0, at), none, ...projects.slice(at)].filter(Boolean) }], query);
    },
  },
};
const submitButton = (label, keys, disabled) =>
  `<button type="button" class="create" data-submit title="${esc(`${label} (${keys})`)}"${disabled ? " disabled" : ""}>${esc(label)}<kbd>${esc(keys)}</kbd></button>`;

/** T3 Code's newProjectFolderName, which the machine applies too. */
const projectFolderName = (name) => {
  const slug = name.normalize("NFKD").replace(/[̀-ͯ]/g, "").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+/, "").slice(0, 64).replace(/-+$/, "");
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

/** Thread message search on each machine, as T3 Code's server does it: two characters at least,
 * 200ms after the last keystroke. `matches` is by tab key: `{ source, snippet }`. */
const THREAD_SEARCH_MS = 200;
let threadSearchTimer = null;
function searchThreads(query) {
  const q = query.trim();
  if (palette.search?.query === q || palette.search?.asked === q) return;
  clearTimeout(threadSearchTimer);
  if (q.length < 2) return (palette.search = null);
  palette.search = { asked: q, query: palette.search?.query ?? "", matches: palette.search?.matches ?? new Map(), pending: true };
  threadSearchTimer = setTimeout(async () => {
    const answers = await Promise.all(onlineMachines().map((m) => callOn(m, { cmd: "search_threads", query: q }).then((found) => found.map((f) => ({ ...f, machine: m })), () => [])));
    if (palette.search?.asked !== q) return;
    const byTab = new Map();
    for (const f of answers.flat()) {
      const tab = tabs.find((t) => t.machine === f.machine && t.view.id === f.tab);
      if (tab) byTab.set(tab.key, { source: f.source, snippet: f.snippet });
    }
    palette.search = { asked: q, query: q, matches: byTab, pending: false };
    if (paletteOpen() && paletteView().view === "root") renderPalette();
  }, THREAD_SEARCH_MS);
}
/** ThreadSearchMatchExcerpt: who wrote the match, then the excerpt with the query in bold. */
function threadExcerptHtml({ source, snippet, query }) {
  const at = snippet.toLowerCase().indexOf(query.toLowerCase());
  const text = at < 0 ? esc(snippet) : `${esc(snippet.slice(0, at))}<mark>${esc(snippet.slice(at, at + query.length))}</mark>${esc(snippet.slice(at + query.length))}`;
  return `<span class="excerpt"><span class="who" data-source="${source}">${source === "user" ? "You:" : "Agent:"}</span> ${text}</span>`;
}

/** Opens the palette at `mode`: the root, or "new-thread-in", "add-project" or a view's name, with
 * `query` typed and, for a pull request's link, the `linkedThreads` it names. */
function openPalette({ mode = "root", query = "", linkedThreads = null, machine = null } = {}) {
  closeSideMenu();
  palette.views = [{ view: "root", query }];
  palette.hi = 0;
  Object.assign(palette, { publish: false, newMachine: null, linked: linkedThreads && { query, threads: linkedThreads }, search: null, adding: null });
  paletteEl.hidden = false;
  paletteInput.value = query;
  if (mode === "add-project") startAddProject(machine);
  else if (mode !== "root") palette.views.push({ view: mode, query: "" });
  if (paletteView().view !== "root") paletteInput.value = paletteView().query;
  palette.hi = PALETTE_VIEWS[paletteView().view].unhighlighted ? -1 : 0;
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
  clearTimeout(threadSearchTimer);
  if (!shellPage()) focusInput();
}
const paletteView = () => palette.views.at(-1);
/** A view's spec, or the one its `mode` gives for the query, as the root browses for a typed path. */
const specFor = (view, query) => PALETTE_VIEWS[view].mode?.(query) ?? PALETTE_VIEWS[view];
const viewSpec = () => specFor(paletteView().view, paletteView().query);
const selectable = () => palette.shown.flatMap((item, i) => (item.disabled ? [] : [i]));

function renderPalette() {
  const { view, query } = paletteView();
  const spec = specFor(view, query);
  const groups = spec.groups(query);
  palette.shown = groups.flatMap((g) => g.items);
  if (palette.hi >= 0 && !selectable().includes(palette.hi)) palette.hi = spec.unhighlighted ? -1 : selectable()[0] ?? -1;
  const nested = palette.views.length > 1;
  paletteInput.placeholder = typeof spec.placeholder === "function" ? spec.placeholder(query) : spec.placeholder;
  paletteEl.querySelector(".summary-slot").innerHTML = spec.summary?.(query) ?? "";
  paletteEl.querySelector(".accessory").innerHTML = spec.accessory?.() ?? "";
  const notice = spec.notice?.(query) ?? "";
  Object.assign(paletteEl.querySelector(".notice"), { hidden: !notice, textContent: notice });
  paletteEl.dataset.nested = nested;
  paletteEl.querySelector(".lead").innerHTML = icon(nested ? "arrow-left" : spec.lead?.(query) ?? "search");
  paletteEl.querySelector(".lead").setAttribute("aria-label", nested ? "Back" : "Search");
  let n = 0;
  paletteResults.innerHTML = palette.shown.length
    ? groups.map((g) => `<div class="group" role="group">${g.label ? `<div class="label">${esc(g.label)}</div>` : ""}${g.items.map((item) => {
        const i = n++;
        const hint = item.command ? bindingLabel(item.command, item.context) : item.hint;
        const sub = item.subHtml ?? (item.sub ? esc(item.sub) : "");
        return `<div class="item${i === palette.hi ? " hi" : ""}" role="option" data-i="${i}"${item.disabled ? ` aria-disabled="true"` : ""}>
          <span class="ic">${item.iconHtml ?? icon(item.icon)}</span>
          <span class="text"><span class="title">${item.status ?? ""}<span class="name">${item.title}</span></span>${item.excerpt ? threadExcerptHtml(item.excerpt) : ""}${sub ? `<span class="sub">${sub}</span>` : ""}</span>
          ${item.trail ?? ""}${item.time ? `<span class="time">${agoLong(item.time)}</span>` : ""}${hint ? `<span class="hint">${esc(hint)}</span>` : ""}${item.page !== undefined ? icon("chevron-right", "chev") : ""}</div>`;
      }).join("")}</div>`).join("")
    : `<p class="empty">${esc(spec.empty?.(query) ?? (query.startsWith(">") ? "No matching actions." : "No matching commands, projects, or threads."))}</p>`;
  const action = spec.footerAction ? spec.footerAction(query) : "Select";
  paletteEl.querySelector("footer").innerHTML = `<span class="keys"><span><kbd>${icon("arrow-up")}</kbd><kbd>${icon("arrow-down")}</kbd>Navigate</span>${action === undefined ? "" : `<span><kbd>Enter</kbd>${esc(action)}</span>`}${nested ? `<span><kbd>Backspace</kbd>Back</span>` : ""}<span><kbd>Esc</kbd>Close</span></span>${spec.footerTrail?.(query) ?? ""}`;
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
  if (!item.keep) paletteEl.hidden = true;
  Promise.resolve(item.run()).then(() => item.keep && paletteOpen() && renderPalette(), (e) => tell(`<b>Unable to run command</b> ${esc(e?.message ?? e)}`, 6000));
}
function popPalette() {
  palette.views.pop();
  palette.adding = palette.views.some((v) => v.adding) ? palette.adding : null;
  paletteInput.value = paletteView().query;
  palette.hi = viewSpec().unhighlighted ? -1 : 0;
  renderPalette();
}
function highlight(i) {
  palette.hi = i;
  for (const el of paletteResults.querySelectorAll(".item")) el.classList.toggle("hi", Number(el.dataset.i) === i);
  paletteResults.querySelector(".item.hi")?.scrollIntoView({ block: "nearest" });
  if (viewSpec().footerAction || viewSpec().accessory) renderPalette();
}
/** CommandPalette's handleKeyDown: an app chord pressed in the palette runs its item there, ⌘1-9
 * stay in it, and any other closes it before it runs. True when the palette took the chord. */
function paletteChord(command) {
  if (command === "commandPalette.toggle") return false;
  const item = palette.shown.find((i) => i.command === command && !i.disabled);
  if (item) return (runPaletteItem(item), true);
  if (/^thread\.jump\./.test(command)) return true;
  closePalette();
  return false;
}

paletteInput.addEventListener("input", () => {
  paletteView().query = paletteInput.value;
  palette.hi = -1;
  viewSpec().onQuery?.(paletteInput.value);
  renderPalette();
});
paletteInput.addEventListener("keydown", (e) => {
  const rows = selectable();
  const spec = viewSpec();
  const primary = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if ((e.key === "ArrowDown" || e.key === "ArrowUp") && !e.metaKey && !e.ctrlKey && !e.altKey) {
    if (rows.length) highlight(palette.hi < 0 ? (e.key === "ArrowDown" ? rows[0] : rows.at(-1)) : rows[(rows.indexOf(palette.hi) + (e.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length]);
  } else if (e.key === "Enter" && (e.isComposing || e.keyCode === 229)) return;
  else if (e.key === "Enter" && spec.submit && (palette.hi < 0 || primary || spec.submitsFirst?.())) spec.submit(paletteInput.value);
  else if (e.key === "Enter") runPaletteItem(palette.shown[palette.hi >= 0 ? palette.hi : rows[0]]);
  else if (e.key === "Escape") closePalette();
  else if (e.key === "Backspace" && !paletteInput.value && palette.views.length > 1) popPalette();
  else return;
  e.preventDefault();
  e.stopPropagation();
});
paletteResults.addEventListener("mousedown", (e) => e.preventDefault());
paletteResults.addEventListener("mousemove", (e) => {
  const el = e.target.closest(".item:not([aria-disabled])");
  if (el && Number(el.dataset.i) !== palette.hi) highlight(Number(el.dataset.i));
});
paletteResults.addEventListener("click", (e) => {
  const el = e.target.closest(".item");
  if (el) runPaletteItem(palette.shown[Number(el.dataset.i)]);
});
paletteEl.querySelector(".accessory").onclick = (e) => e.target.closest("[data-submit]:not(:disabled)") && viewSpec().submit?.(paletteInput.value);
paletteEl.querySelector("footer").onclick = (e) => e.target.closest("[data-footer-act]") && viewSpec().footerRun?.();
paletteEl.querySelector(".backdrop").onclick = () => closePalette();
paletteEl.querySelector(".lead").onclick = () => (palette.views.length > 1 ? popPalette() : paletteInput.focus());

onGuiKey("k", () => (paletteOpen() ? closePalette() : openPalette()));

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
const usageState = { metric: "limits", days: 30, breakdown: "model", machine: "", rows: [], error: "", loaded: false, loading: false, now: 0, seq: 0 };
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
    const hour = new Date(now * 1000);
    hour.setMinutes(0, 0, 0);
    return Array.from({ length: 24 }, (_, i) => {
      const h = new Date(hour);
      h.setHours(h.getHours() - (23 - i));
      return h.getTime() / 1000;
    });
  }
  const day = new Date(now * 1000);
  day.setHours(0, 0, 0, 0);
  return Array.from({ length: days }, (_, i) => {
    const d = new Date(day);
    d.setDate(d.getDate() - (days - 1 - i));
    return d.getTime() / 1000;
  });
}
const localHour = (unix) => {
  const d = new Date(unix * 1000);
  d.setMinutes(0, 0, 0);
  return d.getTime() / 1000;
};
/** Rows come in quarter hours, so each falls whole into a local hour or day. */
const bucketOf = (days, at) => (days === 1 ? localHour(at) : localDay(at));

/** The window's totals, series and models, of one `model` when given. */
function summarize(model) {
  const { days, machine } = usageState;
  const buckets = usageBuckets(days, usageState.now);
  const index = new Map(buckets.map((b, i) => [b, i]));
  const rows = usageState.rows.filter((r) => (!machine || r.machine === machine) && (!model || r.model === model) && index.has(bucketOf(days, r.at)));
  const s = { buckets, rows, cost: 0, tokens: 0, input: 0, cached: 0, cacheWrite: 0, output: 0, savings: 0, premium: 0, unpriced: 0, unpricedTokens: 0, sessions: new Set(),
    byType: { input: 0, cache_read: 0, cache_write: 0, output: 0 }, series: buckets.map(() => ({ cost: 0, tokens: 0 })), models: new Map() };
  for (const r of rows) {
    const cost = rowCost(r);
    const tokens = rowTokens(r);
    Object.assign(s, { cost: s.cost + cost, tokens: s.tokens + tokens, input: s.input + r.input, cached: s.cached + r.cached, cacheWrite: s.cacheWrite + r.cache_write, output: s.output + r.output, savings: s.savings + r.savings, premium: s.premium + r.premium });
    if (!r.cost) {
      s.unpriced++;
      s.unpricedTokens += tokens;
    }
    for (const k in s.byType) s.byType[k] += r.cost?.[k] ?? 0;
    s.sessions.add(`${r.machine}:${r.session}`);
    const point = s.series[index.get(bucketOf(days, r.at))];
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
  const seq = ++usageState.seq;
  usageState.loading = true;
  renderUsagePage();
  const since = usageBuckets(90)[0];
  const answers = await Promise.allSettled(onlineMachines().map((m) => callOn(m, { cmd: "token_usage", since, prices: usagePrices }).then((rows) => rows.map((r) => ({ ...r, machine: m })), (e) => Promise.reject(`${machineName(m)}: ${e}`))));
  if (seq !== usageState.seq) return;
  const error = answers.filter((a) => a.status === "rejected").map((a) => a.reason).join("; ");
  Object.assign(usageState, { rows: answers.flatMap((a) => (a.status === "fulfilled" ? a.value : [])), error, loaded: true, loading: false, now: Date.now() / 1000 });
  if (shellPage() === "usage") renderUsagePage();
}

const segmented = (name, options, current, disabled = false) =>
  `<span class="part" role="radiogroup" data-seg="${name}">${options.map(([value, label]) => `<button type="button" role="radio" aria-checked="${value === current}" data-value="${value}"${disabled ? " disabled" : ""}>${label}</button>`).join("")}</span>
   <select class="seg-select" data-seg="${name}"${disabled ? " disabled" : ""}>${options.map(([value, label]) => `<option value="${value}"${value === current ? " selected" : ""}>${label}</option>`).join("")}</select>`;
/** Draws the page, keeping where its body was scrolled. */
function setPage(html) {
  const top = pageView.querySelector(".page-body")?.scrollTop ?? 0;
  pageView.innerHTML = html;
  const body = pageView.querySelector(".page-body");
  if (body) body.scrollTop = top;
}
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
  const machineFilter = `<span class="crumb-sep">›</span><button type="button" class="crumb-menu machine-filter" title="Machines and model prices">${esc(usageState.machine ? machineName(usageState.machine) : machines.size > 1 ? "All machines" : machineName([...machines.keys()][0] ?? LOCAL))}${icon("chevron-down")}</button>`;
  setPage(`${headHtml("Usage", `${machineFilter}<span class="crumb-sep">›</span><span class="provider">${icon("claude")}Claude Code</span>
      <span class="range">${metric === "limits" ? "" : usageRange(s.buckets, days)}</span><span class="spacer"></span>
      ${segmented("metric", USAGE_METRICS.map(([v, l]) => [v, l]), metric)}${segmented("days", USAGE_PERIODS, days, metric === "limits")}
      <button type="button" class="icon refresh" title="Refresh">${icon("refresh-cw")}</button>`)}
    <div class="page-body"><div class="page-wrap usage" data-metric="${metric}"${usageState.loading && usageState.loaded ? ` data-refreshing` : ""}>${
      metric === "limits" ? limitsHtml() : !usageState.loaded ? usageSkeleton() : (usageState.error ? `<p class="usage-warn">${icon("triangle-alert")}<span>Couldn't read usage on ${esc(usageState.error)}</span></p>` : "") + spendHtml(s)
    }</div></div>`);
  if (metric !== "limits" && usageState.loaded) drawChart(pageView.querySelector(".chart"), s, metric);
}

function spendHtml(s) {
  const { metric, days } = usageState;
  const cost = metric === "cost";
  const total = cost ? usd(s.cost) : tokensText(s.tokens);
  const sessions = `${s.sessions.size} session${s.sessions.size === 1 ? "" : "s"}`;
  const unpricedShare = s.rows.length ? s.unpriced / s.rows.length : 0;
  const unit = days === 1 ? "Hourly" : "Daily";
  const shares = (title, parts) => sharesHtml(title, parts, cost ? usd : tokensText);
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
    ${s.tokens > 0 ? shares(cost ? "Cost by type" : "Tokens by type", cost ? costByType(s) : tokensByType(s)) : ""}
    ${cost && s.premium > 0 ? shares("Cost by speed", [["Standard", s.cost - s.premium, 34], ["Fast", s.premium, 66]]) : ""}
    <section class="breakdown"><div class="breakdown-head"><h2>Breakdown</h2>${segmented("breakdown", [["model", "Model"], ["day", days === 1 ? "Hour" : "Day"]], usageState.breakdown)}</div>
      ${s.rows.length ? table : `<p class="muted empty">No activity in this window.</p>`}</section>`;
}

const costByType = (s) => [["Input", s.byType.input, 60], ["Cache read", s.byType.cache_read, 30], ["Cache write", s.byType.cache_write, 72], ["Output", s.byType.output, 100]];
const tokensByType = (s) => [["Input", s.input, 60], ["Cache read", s.cached, 30], ["Cache write", s.cacheWrite, 72], ["Output", s.output, 100]];
/** T3 Code's share bar: segments in proportion, each a mix of the text color into the background. */
function sharesHtml(title, parts, fmt) {
  const sum = parts.reduce((a, [, v]) => a + v, 0);
  if (!sum) return "";
  const ink = (mix) => `color-mix(in srgb, var(--g-fg) ${mix}%, var(--g-bg))`;
  return `<section class="share"><h2>${title}</h2><div class="share-bar">${parts.filter(([, v]) => v > 0).map(([label, v, mix]) =>
    `<span style="flex-grow:${v};background:${ink(mix)}" title="${label} · ${fmt(v)} · ${percentText(v / sum)}"></span>`).join("")}</div>
    <div class="legend">${parts.map(([label, v, mix]) => `<span><i style="background:${ink(mix)}"></i>${label}<b>${fmt(v)}</b></span>`).join("")}</div></section>`;
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
function drawChart(holder, s, metric) {
  if (!holder) return;
  const cost = metric === "cost";
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
  holder.innerHTML = `<div class="y-axis">${ticks.slice().reverse().map((t) => `<span>${t === 0 ? "0" : cost ? (t < 1000 ? usd(t).replace(/\.00$/, "") : `$${tokensText(t)}`) : tokensText(t)}</span>`).join("")}</div>
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
  const yesterday = new Date(today * 1000);
  yesterday.setDate(yesterday.getDate() - 1);
  const when = (b) => {
    if (usageState.days !== 1) return monthDay(b);
    const day = localDay(b);
    return day === today ? `${hourText(b)} today` : day === yesterday.getTime() / 1000 ? `${hourText(b)} yesterday` : `${monthDay(b)}, ${hourText(b)}`;
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
  if (e.target.closest(".machine-filter")) {
    const only = (m) => ({ label: machineName(m), checked: usageState.machine === m, run: () => setUsage({ machine: m }) });
    return openSideMenu([
      ...(machines.size > 1 ? [{ label: "All machines", checked: !usageState.machine, run: () => setUsage({ machine: "" }) }] : []),
      ...[...machines.keys()].map(only),
      { label: "Model prices", icon: "sliders-horizontal", sep: true, run: () => openPrices() },
    ], e.target.closest("button"));
  }
  const model = e.target.closest("tr[data-model]");
  if (model) openModel(model.dataset.model);
});
pageView.addEventListener("change", (e) => {
  if (shellPage() !== "usage") return;
  const seg = e.target.closest("select.seg-select");
  if (seg) setUsage({ [seg.dataset.seg]: seg.dataset.seg === "days" ? Number(seg.value) : seg.value });
});
window.addEventListener("keydown", (e) => {
  if (!guiOn() || shellPage() !== "usage" || e.repeat || e.metaKey || e.ctrlKey || e.altKey || !paletteEl.hidden || !shellDialog.hidden || e.target.closest?.("input, textarea, select, [contenteditable]")) return;
  const metric = USAGE_METRICS.find(([, , key]) => key === e.key)?.[0];
  if (metric) setUsage({ metric });
});
// ⌘⇧1-4 by key position, since Shift turns the digits into other characters on each layout.
window.addEventListener("keydown", (e) => {
  const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  const i = ["Digit1", "Digit2", "Digit3", "Digit4"].indexOf(e.code);
  if (!guiOn() || shellPage() !== "usage" || !mod || !e.shiftKey || e.altKey || i < 0 || settingsOpen()) return;
  e.preventDefault();
  if (usageState.metric !== "limits") setUsage({ days: USAGE_PERIODS[i][0] });
});

/** A model's own numbers, as T3 Code's model dialog shows them, and the prices the user sets per model,
 * which the machines apply in place of LiteLLM's list. */
let usagePrices = {};
try {
  usagePrices = JSON.parse(load("gui-usage-prices")) ?? {};
} catch {}
const PRICE_FIELDS = [
  ["input", "Input"],
  ["output", "Output"],
  ["cache_read", "Cache read"],
  ["cache_write", "Cache write"],
];

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

function openModel(model) {
  const metric = usageState.metric === "limits" ? "cost" : usageState.metric;
  const all = summarize();
  const s = summarize(model);
  const priced = s.unpriced < s.rows.length;
  const pricedTokens = s.tokens - s.unpricedTokens;
  const inputs = s.input + s.cached + s.cacheWrite;
  const stats = [
    ["Cost", priced ? usd(s.cost) : "Unpriced"],
    ["Tokens", tokensText(s.tokens)],
    priced && pricedTokens > 0 && ["Per 1M tokens", usd((s.cost / pricedTokens) * 1e6)],
    inputs > 0 && ["Cache hit", percentText(s.cached / inputs)],
  ].filter(Boolean);
  showDialog(`<div class="dialog-head"><h2>${icon("claude")}<span>${esc(model)}</span></h2><p class="muted">Claude Code${priced && all.cost ? ` · ${percentText(s.cost / all.cost)} of cost` : ""}</p></div>
    <div class="dialog-body usage"><dl class="stats">${stats.map(([label, value]) => `<div><dt>${label}</dt><dd>${value}</dd></div>`).join("")}</dl>
      <div class="chart"></div>
      <div class="dialog-shares">${priced ? sharesHtml("Cost by type", costByType(s), usd) : ""}${sharesHtml("Tokens by type", tokensByType(s), tokensText)}${
        s.premium > 0 ? sharesHtml("Cost by speed", [["Standard", s.cost - s.premium, 34], ["Fast", s.premium, 66]], usd) : ""}</div></div>
    ${s.unpricedTokens > 0 ? `<div class="dialog-foot"><span class="muted">${tokensText(s.unpricedTokens)} tokens have no known price</span><button type="button" class="primary" data-set-price>Set price</button></div>` : ""}`);
  shellDialog.dataset.kind = "model";
  drawChart(shellDialog.querySelector(".chart"), s, priced ? metric : "tokens");
  shellDialog.querySelector("[data-set-price]")?.addEventListener("click", () => openPrices(model));
}

function openPrices(initial) {
  const models = [...new Set([...Object.keys(usagePrices), ...(initial ? [initial] : [])])];
  const cell = (model, key) => `<input type="number" min="0" step="any" inputmode="decimal" data-field="${key}" aria-label="${PRICE_FIELDS.find(([k]) => k === key)[1]} price for ${esc(model || "new model")}" value="${usagePrices[model]?.[key] ?? ""}" placeholder="${key.startsWith("cache") ? "Input's" : ""}">`;
  const row = (model) => `<tr><td><input type="text" data-field="model" aria-label="Model ID" placeholder="Model ID" value="${esc(model)}" spellcheck="false"></td>${PRICE_FIELDS.map(([k]) => `<td>${cell(model, k)}</td>`).join("")}
    <td><button type="button" class="icon" data-remove title="Remove">${icon("x")}</button></td></tr>`;
  showDialog(`<div class="dialog-head"><h2><span>Custom model prices</span></h2><p class="muted">Prices are in USD per million tokens and apply to all past and future usage, on every machine this browser reads.</p></div>
    <div class="dialog-body"><div class="table-scroll"><table class="prices" aria-label="Custom model prices"><thead><tr><th>Model</th>${PRICE_FIELDS.map(([, l]) => `<th>${l}</th>`).join("")}
      <th><button type="button" class="icon" data-add title="Add model price" aria-label="Add model price">${icon("plus")}</button></th></tr></thead><tbody>${models.map(row).join("")}</tbody></table></div>
      <p class="error" hidden></p></div>
    <div class="dialog-foot"><span class="spacer"></span><button type="button" class="outline" data-cancel>Cancel</button><button type="button" class="primary" data-save>Save prices</button></div>`);
  shellDialog.dataset.kind = "prices";
  const body = shellDialog.querySelector("tbody");
  if (!models.length) body.insertAdjacentHTML("beforeend", row(""));
  shellDialog.querySelector("[data-add]").onclick = () => body.insertAdjacentHTML("beforeend", row(""));
  body.onclick = (e) => e.target.closest("[data-remove]")?.closest("tr").remove();
  shellDialog.querySelector("[data-cancel]").onclick = closeDialog;
  shellDialog.querySelector("[data-save]").onclick = () => {
    const next = {};
    const error = shellDialog.querySelector(".error");
    for (const tr of body.querySelectorAll("tr")) {
      const value = (f) => tr.querySelector(`[data-field="${f}"]`).value.trim();
      const model = value("model");
      if (!model && PRICE_FIELDS.every(([k]) => !value(k))) continue;
      const valid = (f) => value(f) === "" || (Number.isFinite(Number(value(f))) && Number(value(f)) >= 0);
      const [input, output] = [Number(value("input")), Number(value("output"))];
      if (!model || !value("input") || !value("output") || !PRICE_FIELDS.every(([k]) => valid(k))) {
        error.textContent = `${model || "A new row"} needs a model ID, input and output prices, and prices of 0 or more.`;
        error.hidden = false;
        return;
      }
      next[model] = { input, output, ...Object.fromEntries(["cache_read", "cache_write"].filter((k) => value(k)).map((k) => [k, Number(value(k))])) };
    }
    usagePrices = next;
    save("gui-usage-prices", Object.keys(next).length ? JSON.stringify(next) : null);
    closeDialog();
    fetchUsage();
  };
  shellDialog.querySelector(initial ? `tbody tr:last-child [data-field="input"]` : `tbody tr:last-child [data-field="model"]`)?.focus();
}

SHELL_PAGES.usage = {
  show() {
    usageState.now = Date.now() / 1000;
    renderUsagePage();
    if (!usageState.loading) fetchUsage();
  },
};
onGuiKey("u", () => openPage("usage"));

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
