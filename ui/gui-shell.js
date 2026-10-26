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
document.querySelector("#sidebar footer .back").onclick = () => closePage();

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
  <div class="summary-slot"></div><p class="notice" role="status" hidden></p><div class="results" role="listbox"></div><footer></footer></div>`;
document.body.append(paletteEl);
const paletteInput = paletteEl.querySelector("input");
const paletteResults = paletteEl.querySelector(".results");
const palette = { views: [], hi: 0, shown: [], projects: [], linked: null };
const paletteOpen = () => !paletteEl.hidden;
const RECENT_THREADS = 12;

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
    if (!seen.has(`${tab.machine}:${path}`)) seen.set(`${tab.machine}:${path}`, { machine: tab.machine, path, name: path.split(/[\\/]/).filter(Boolean).pop() || "/" });
  }
  return sortedProjects([...seen.values()], (p) => tabs.filter((t) => t.machine === p.machine && projectOfTab(t) === p.path));
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

