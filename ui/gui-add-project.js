/** T3 Code's Add project in the palette (CommandPalette.tsx): pick a machine when there are several,
 * then a source: New project, a local folder browsed by path, a Git URL or a GitHub repository,
 * each cloned to a folder picked the same way. `palette.adding` holds the flow: `{ machine,
 * source, repository, remoteUrl, cloning }`. */

const isBrowseQuery = (q) => /^(\.\.?\/|\/|~\/)/.test(q);
const isRelativePath = (q) => /^\.\.?\//.test(q.trim());
const browseDir = (q) => (q.endsWith("/") ? q : q.slice(0, q.lastIndexOf("/") + 1));
const browseLeaf = (q) => (q.endsWith("/") ? "" : q.slice(q.lastIndexOf("/") + 1));
/** T3 Code's getBrowseParentPath: `~/` and `/` have none. */
function browseParent(dir) {
  const trimmed = dir.length > 1 ? dir.replace(/\/+$/, "") : dir;
  if (trimmed.startsWith("/")) {
    const segments = trimmed.split("/").filter(Boolean);
    return !segments.length ? null : segments.length === 1 ? "/" : `/${segments.slice(0, -1).join("/")}/`;
  }
  const at = trimmed.lastIndexOf("/");
  return at < 0 ? null : trimmed.slice(0, at + 1);
}
/** T3 Code's resolveProjectPathForDispatch: `./` and `../` start at `cwd`. */
function resolveRelative(value, cwd) {
  const segments = cwd.split("/").filter(Boolean);
  for (const segment of value.trim().split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") segments.pop();
    else segments.push(segment);
  }
  return `/${segments.join("/")}${value.trim().endsWith("/") && segments.length ? "/" : ""}`;
}
const cloneDirName = (repo) => {
  const bare = (repo ?? "").split(/[?#]/)[0].trim();
  const scheme = bare.indexOf("://");
  const hasHost = scheme >= 0 || /^[^/\\:]+@[^/\\:]+:/.test(bare);
  const segments = (scheme >= 0 ? bare.slice(scheme + 3) : bare).split(/[/\\:]+/).filter((s) => s.trim());
  if (hasHost && segments.length < 2) return "";
  const last = segments.at(-1)?.trim() ?? "";
  if (hasHost && segments.length === 2 && /^\d+$/.test(last)) return "";
  return last.endsWith(".git") ? last.slice(0, -4) : last;
};
const GITHUB_SHORTHAND = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})\/[A-Za-z0-9._-]+(?:\.git)?$/;
const pastedCloneUrl = (input) => (GITHUB_SHORTHAND.test(input.trim()) ? `https://github.com/${input.trim().replace(/(\.git)?$/, ".git")}` : input.trim());
const FILE_MANAGER = IS_MAC ? "Finder" : /Win/.test(navigator.platform) ? "File Explorer" : "Files";
const ADD_PROJECT_START = "~/";

/** The machine the flow adds to, and the open thread's folder there, which relative paths start at. */
const addingMachine = () => palette.adding?.machine ?? (onlineMachines().includes(active?.machine) ? active.machine : onlineMachines()[0]);
const addingCwd = () => (active && active.machine === addingMachine() ? projectOfTab(active) : null);

/** What the folder typed so far lists: `{ key, result, missing, pending }`, the result being `projects::Folders`. */
function browseState(query, pinned = "") {
  const machine = addingMachine();
  const relative = isRelativePath(query);
  if (relative && !addingCwd()) return { relativeNeedsProject: true };
  const dir = relative ? resolveRelative(browseDir(query), addingCwd()) : browseDir(query);
  const leaf = browseLeaf(query);
  const key = `${machine}:${dir}${leaf.startsWith(".") ? "." : ""}`;
  if (palette.browse?.key !== key) {
    palette.browse = { key, result: null, missing: false, pending: true };
    callOn(machine, { cmd: "browse", path: `${dir}${leaf.startsWith(".") ? "." : ""}` }).then(
      (result) => palette.browse?.key === key && Object.assign(palette.browse, { result, pending: false }),
      () => palette.browse?.key === key && Object.assign(palette.browse, { missing: true, pending: false }),
    ).then(() => paletteOpen() && palette.browse?.key === key && renderPalette());
  }
  const all = palette.browse.result?.entries ?? [];
  const filter = pinned && leaf === pinned ? "" : leaf;
  const entries = all.filter((e) => e.name.toLowerCase().startsWith(filter.toLowerCase()) && (filter.startsWith(".") || !e.name.startsWith(".")));
  return { ...palette.browse, dir, leaf, entries, exact: leaf ? (pinned ? all : entries).find((e) => e.name === leaf) ?? null : null };
}
/** The browse half of the palette: a `Directories` group under the typed path, and Add, or Clone
 * once a repository is picked, for the folder it names. */
function browseView({ clone = false, root = false } = {}) {
  const pinned = () => (clone ? cloneDirName(palette.adding.repository?.nameWithOwner ?? palette.adding.remoteUrl) : "");
  const state = (query) => browseState(query, pinned());
  const willCreate = (query, s = state(query)) => !s.relativeNeedsProject && !s.pending && query.trim().length > 0 && palette.hi < 0 && (query.endsWith("/") ? s.missing : s.exact === null);
  const label = (query) => (clone ? (willCreate(query) ? "Create & Clone" : "Clone") : willCreate(query) ? "Create & Add" : "Add");
  return {
    placeholder: root ? "Enter project path (e.g. ~/projects/my-app)" : "Enter path (e.g. ~/projects/my-app)",
    lead: () => "folder-plus",
    unhighlighted: true,
    summary: () => (clone ? repositorySummary() : ""),
    accessory() {
      const query = paletteView().query;
      return submitButton(palette.adding?.cloning ? "Cloning" : label(query), palette.hi >= 0 ? `${IS_MAC ? "⌘" : "Ctrl"} Enter` : "Enter", state(query).relativeNeedsProject || !!palette.adding?.cloning);
    },
    notice(query) {
      const s = state(query);
      return s.pending && !s.result ? "Loading folders..." : "";
    },
    empty(query) {
      if (state(query).relativeNeedsProject) return "Relative paths require an active project.";
      if (clone) return "Choose a destination path and press Enter to clone.";
      return willCreate(query) ? "Press Enter to create this folder and add it as a project." : "No matching commands, projects, or threads.";
    },
    footerAction: (query) => (state(query).relativeNeedsProject || palette.hi >= 0 ? "Select" : undefined),
    footerTrail: () => (TAURI && addingMachine() === LOCAL && !clone ? `<button type="button" class="act" data-footer-act>Open in ${FILE_MANAGER}</button>` : ""),
    footerRun: async () => {
      const path = await invoke("pick_folder", { start: addingCwd() });
      if (path) addProjectAt(LOCAL, path);
    },
    groups(query) {
      const s = state(query);
      if (s.relativeNeedsProject) return [];
      const name = pinned();
      const into = (path) => {
        paletteInput.value = paletteView().query = path;
        palette.hi = -1;
        renderPalette();
      };
      const up = browseParent(s.dir);
      const items = [
        up !== null && { icon: "corner-left-up", title: "..", terms: [], keep: true, run: () => into(name ? `${up}${name}` : up) },
        ...s.entries.map((e) => ({ icon: "folder", title: esc(e.name), terms: [], keep: true, run: () => into(`${s.dir}${e.name}/${name && e.name !== name ? name : ""}`) })),
      ].filter(Boolean);
      return items.length ? [{ label: clone ? "Select where to clone" : "Directories", items }] : [];
    },
    submit(query) {
      const s = state(query);
      if (s.relativeNeedsProject || palette.adding?.cloning) return;
      const raw = query.endsWith("/") ? s.result?.parent ?? query.trim() : s.exact?.path ?? query.trim();
      const path = isRelativePath(raw) ? resolveRelative(raw, addingCwd()) : raw;
      if (!path) return;
      return clone ? cloneTo(path) : addProjectAt(addingMachine(), path);
    },
  };
}
PALETTE_VIEWS["add-browse"] = browseView();
PALETTE_VIEWS["add-clone-to"] = browseView({ clone: true });
const rootBrowseView = browseView({ root: true });

/** T3 Code's handleAddProjectForEnvironment: a folder already a project opens its latest thread, else
 * the folder, made first when it is missing, opens as a project with a new thread. */
async function addProjectAt(machine, raw) {
  try {
    const path = await callOn(machine, { cmd: "make_folder", path: raw });
    closePalette();
    const known = tabs.filter((t) => t.machine === machine && projectOfTab(t) === path && !settled(t)).sort((a, b) => latestUserAt(b) - latestUserAt(a))[0];
    if (known) activate(known);
    else await openProjectIn(machine, path);
  } catch (e) {
    tell(`<b>Failed to add project</b> ${esc(e?.message ?? e)}`, 6000);
  }
}

function repositorySummary() {
  const { source, repository, remoteUrl, input } = palette.adding;
  return `<div class="summary-head">Repository</div><div class="summary"><span class="ic">${icon(source === "url" ? "link" : "github")}</span><span class="text"><span class="title"><span class="name">${esc(repository?.nameWithOwner ?? input)}</span></span><span class="sub">${esc(repository?.url ?? remoteUrl)}</span></span></div>`;
}
async function cloneTo(dest) {
  const { machine, remoteUrl } = palette.adding;
  palette.adding.cloning = true;
  renderPalette();
  try {
    const path = hub ? await hub.hub({ cmd: "clone", machine: hubIdOf(machine), repo: remoteUrl, dest }) : await invoke("clone_repo", { repo: remoteUrl, dest });
    closePalette();
    await openProjectIn(machine, path);
  } catch (e) {
    tell(`<b>Clone failed</b> ${esc(e?.message ?? e)}`, 8000);
  } finally {
    if (palette.adding) palette.adding.cloning = false;
    if (paletteOpen()) renderPalette();
  }
}

/** Most repositories the list shows at once; typing narrows the rest in. */
const REPOSITORY_ROWS = 50;
/** The row Enter takes for the typed text: an exact name, else the best match, unless the text
 * names some other `owner/repo`, which is looked up instead. */
function repositoryPick(rows, query) {
  const name = query.trim().replace(/^https:\/\/github\.com\//, "").replace(/\.git$/, "").toLowerCase();
  const exact = rows.find((i) => i.fullName.toLowerCase() === name);
  return exact ?? (GITHUB_SHORTHAND.test(name) ? null : rows[0] ?? null);
}
/** Highlights that row, so Enter picks what the list shows; none means Enter looks the text up. */
function highlightRepository(query) {
  const rows = PALETTE_VIEWS["add-repository"].groups(query).flatMap((g) => g.items);
  palette.hi = rows.indexOf(repositoryPick(rows, query));
}
/** Lists the connected account's repositories once for the GitHub and Box sources. */
function loadGithubRepos(flow) {
  Object.assign(flow, { repos: null, reposError: "" });
  connectorCall("github_repos").then(
    (repos) => (flow.repos = repos),
    (e) => Object.assign(flow, { repos: [], reposError: String(e?.message ?? e) }),
  ).then(() => {
    if (palette.adding !== flow || !paletteOpen() || paletteView().view !== "add-repository") return;
    highlightRepository(paletteView().query);
    renderPalette();
  });
}
function chooseRepository(flow, input, repository) {
  if (flow.source === "box") confirmBox({ input, repository, remoteUrl: repository.url });
  else confirmClone({ input, repository, remoteUrl: repository.url });
}

/** The repository step: a Git URL goes on as typed; GitHub and Box search the account's
 * repositories, and any other `owner/repo` is looked up. */
PALETTE_VIEWS["add-repository"] = {
  placeholder: () => (palette.adding.source === "url" ? "Enter Git clone URL" : "Search your repositories, or enter owner/repo"),
  lead: () => (palette.adding.source === "url" ? "link" : "github"),
  unhighlighted: true,
  accessory() {
    const flow = palette.adding;
    const label = flow.looking ? "Working" : flow.source === "url" ? "Continue" : palette.hi >= 0 ? "Select" : "Lookup";
    return submitButton(label, "Enter", (palette.hi < 0 && !paletteView().query.trim()) || flow.looking);
  },
  footerAction: () => (palette.adding.source === "url" ? "Continue" : palette.hi >= 0 ? "Select" : "Lookup"),
  onQuery: (query) => palette.adding.source !== "url" && highlightRepository(query),
  empty(query) {
    const { source, repos, reposError } = palette.adding;
    if (source === "url") return "Enter a Git clone URL and press Enter to continue.";
    if (!repos) return "Loading your repositories...";
    if (reposError) return `Could not list your repositories (${reposError}). Enter owner/repo and press Enter to look it up.`;
    if (!query.trim()) return "No repositories on this GitHub account. Enter owner/repo and press Enter to look it up.";
    return "No repository matches. Enter owner/repo and press Enter to look it up on GitHub.";
  },
  groups(query) {
    const flow = palette.adding;
    if (flow.source === "url" || !flow.repos?.length) return [];
    const items = flow.repos.map((r) => ({
      icon: "github",
      title: esc(r.full_name),
      sub: [r.private ? "Private" : "", r.description].filter(Boolean).join(" · "),
      fullName: r.full_name,
      terms: [r.full_name, r.description],
      keep: true,
      run: () => chooseRepository(flow, r.full_name, { nameWithOwner: r.full_name, url: r.clone_url }),
    }));
    return filterGroups([{ label: "Your repositories", items }], query).map((g) => ({ ...g, items: g.items.slice(0, REPOSITORY_ROWS) }));
  },
  async submit(query) {
    const typed = query.trim();
    const flow = palette.adding;
    if (flow.looking) return;
    if (palette.hi >= 0) return runPaletteItem(palette.shown[palette.hi]);
    if (!typed) return;
    if (flow.source === "url") return confirmClone({ input: typed, repository: null, remoteUrl: pastedCloneUrl(typed) });
    flow.looking = true;
    renderPalette();
    try {
      const repository = await lookupGithub(typed);
      if (palette.adding !== flow) return;
      chooseRepository(flow, typed, repository);
    } catch (e) {
      tell(`<b>Repository lookup failed</b> ${esc(e?.message ?? e)}`, 6000);
    } finally {
      flow.looking = false;
      if (paletteOpen()) renderPalette();
    }
  },
};
/** A repo picked for a box: size and budget next, then the box boots around it. */
function confirmBox({ input, repository, remoteUrl }) {
  Object.assign(palette.adding, { input, repository, remoteUrl, size: "medium" });
  palette.views.push({ view: "add-box-budget", query: "", adding: true });
  paletteInput.value = paletteView().query;
  palette.hi = -1;
  renderPalette();
}
const BOX_SIZE_CHOICES = [["small", "Small"], ["medium", "Medium"], ["large", "Large"]];
/** What an hour on a box costs in USD, by size: the hub's `box_usd_per_hour`. A ceiling, since Upstash bills a box's active CPU time. */
const BOX_RATE_BY_SIZE = { small: 0.1, medium: 0.2, large: 0.4 };
const boxRate = (size) => BOX_RATE_BY_SIZE[size] ?? BOX_RATE_BY_SIZE.medium;
/** The most a box may spend, in USD. At least an hour on its size: $0.10 Small, $0.20 Medium, $0.40 Large. */
const parseBoxBudget = (query, size) => {
  const n = Number(query.trim().replace(/^\$/, ""));
  return Number.isFinite(n) && n >= boxRate(size) ? Math.round(n * 100) / 100 : null;
};
const boxHoursText = (usd, size) => {
  const hours = usd / boxRate(size);
  return hours < 10 ? `${+hours.toFixed(1)}h` : `${Math.round(hours)}h`;
};
PALETTE_VIEWS["add-box-budget"] = {
  placeholder: "Max spend in USD (e.g. 2)",
  lead: () => "server",
  unhighlighted: true,
  summary(query) {
    const { repository, input, size } = palette.adding;
    const usd = parseBoxBudget(query, size);
    const funds = usd == null ? "Name the most this box may spend" : `$${usd.toFixed(2)} ≈ ${boxHoursText(usd, size)} at $${boxRate(size).toFixed(2)}/h`;
    return `<div class="summary-head">Box session</div><div class="summary"><span class="ic">${icon("server")}</span><span class="text"><span class="title"><span class="name">${esc(repository?.nameWithOwner ?? input)}</span></span><span class="sub">${esc(`${size} box · ${funds}`)}</span></span></div>`;
  },
  accessory() {
    const usd = parseBoxBudget(paletteView().query, palette.adding.size);
    const label = palette.adding.creating ? "Creating box" : usd == null ? "Create box" : `Create box · $${usd.toFixed(2)}`;
    return submitButton(label, "Enter", usd == null || palette.adding.creating);
  },
  footerAction: () => "Create box",
  empty: () => "The box shuts down and deletes the session when the money runs out.",
  groups() {
    return [{ label: "Size", items: BOX_SIZE_CHOICES.map(([id, label]) => ({ icon: "server", title: label, trail: palette.adding.size === id ? icon("check", "chev") : "", terms: [], keep: true, run: () => { palette.adding.size = id; renderPalette(); } })) }];
  },
  submit: (query) => createBox(query),
};
async function createBox(query) {
  const flow = palette.adding;
  const usd = parseBoxBudget(query, flow.size);
  if (usd == null || flow.creating) return;
  if (!hub) return tell("<b>Sign in first</b> Boxes belong to an account.", 6000);
  flow.creating = true;
  renderPalette();
  try {
    const st = await hub.hub({ cmd: "open_box", repo: flow.repository?.url ?? flow.remoteUrl, size: flow.size ?? "medium", budget_usd: usd });
    Object.assign(flow, { creating: false, box: st.id, machine: st.machine });
    palette.views.push({ view: "add-box-wait", query: "", adding: true });
    renderPalette();
    watchBox(flow);
  } catch (e) {
    flow.creating = false;
    tell(`<b>Box failed to start</b> ${esc(e?.message ?? e)}`, 8000);
    if (paletteOpen()) renderPalette();
  }
}
PALETTE_VIEWS["add-box-wait"] = {
  placeholder: "",
  lead: () => "server",
  unhighlighted: true,
  summary() {
    const { repository, input } = palette.adding;
    return `<div class="summary-head">Box is booting</div><div class="summary"><span class="ic">${icon("server")}</span><span class="text"><span class="title"><span class="name">${esc(repository?.nameWithOwner ?? input)}</span></span><span class="sub">This takes a few minutes; the thread opens itself.</span></span></div>`;
  },
  accessory: () => submitButton("Booting", "", true),
  groups: () => [],
  empty: () => "Leave the palette open; the box thread opens on its own.",
  submit: () => {},
};
/** Polls a booting box until its session tab opens, then reveals it. */
async function watchBox(flow) {
  for (let i = 0; i < 120; i++) {
    await new Promise((r) => setTimeout(r, 3000));
    if (!hub || palette.adding !== flow || !paletteOpen()) return;
    let st = null;
    try {
      st = await hub.hub({ cmd: "box_status", machine: flow.machine });
    } catch {
      return;
    }
    if (st.phase === "ending") {
      palette.views.pop();
      tell("<b>Box ended while booting</b> Try again.", 6000);
      if (paletteOpen()) renderPalette();
      return;
    }
    if (st.phase === "running" && st.tab) {
      closePalette();
      try {
        await revealTab(flow.machine, st.tab);
        refreshBoxInfo();
      } catch (e) {
        tell(`<b>Box is up, open its thread</b> ${esc(e?.message ?? e)}`, 8000);
      }
      return;
    }
  }
}
/** The account's own repositories answer first; any other public one is asked of GitHub. */
async function lookupGithub(typed) {
  const name = typed.replace(/^https:\/\/github\.com\//, "").replace(/\.git$/, "");
  if (!GITHUB_SHORTHAND.test(name)) throw new Error("Enter a repository as owner/repo.");
  const mine = await connectorCall("github_repos").catch(() => []);
  const own = mine.find((r) => r.full_name.toLowerCase() === name.toLowerCase());
  if (own) return { nameWithOwner: own.full_name, url: `https://github.com/${own.full_name}.git` };
  const res = await fetch(`https://api.github.com/repos/${name}`).catch(() => null);
  if (!res?.ok) throw new Error(res?.status === 404 ? `${name} was not found on GitHub.` : "GitHub did not answer.");
  const found = await res.json();
  return { nameWithOwner: found.full_name, url: found.clone_url };
}
function confirmClone({ input, repository, remoteUrl }) {
  Object.assign(palette.adding, { input, repository, remoteUrl });
  const name = cloneDirName(repository?.nameWithOwner ?? remoteUrl);
  palette.views.push({ view: "add-clone-to", query: `${ADD_PROJECT_START}${name}` });
  paletteInput.value = paletteView().query;
  palette.hi = -1;
  renderPalette();
}

/** Sources: New project, Local folder, Git URL, and the GitHub repositories once GitHub is connected. */
PALETTE_VIEWS["add-sources"] = {
  placeholder: "Search...",
  lead: () => "folder-plus",
  groups(query) {
    const machine = palette.adding.machine;
    const ready = !!palette.adding.github;
    const boxed = !!palette.adding.upstash;
    const github = { icon: "github", title: "GitHub repository", sub: "Search your GitHub repositories", terms: ["clone", "remote", "repository", "repo", "git", "GitHub", ...(ready ? [] : ["setup required"])], keep: true, run: () => pickSource("github") };
    const box = { icon: "server", title: "Box", sub: "Open a GitHub repo on a new Upstash Box", terms: ["box", "upstash", "cloud", "remote", "github", ...(boxed ? [] : ["setup required"])], keep: true, run: () => pickSource("box") };
    const items = [
      { icon: "folder-git-2", title: "New project", sub: "Start a new Git repository from a name", terms: ["new project", "create", "empty", "repository", "git init"], keep: true, run: () => newProjectOn(machine) },
      { icon: "folder-plus", title: "Local folder", sub: "Browse a folder on disk", terms: ["local", "folder", "directory", "browse"], keep: true, run: () => browseForProject(machine) },
      { icon: "link", title: "Git URL", sub: "Clone from a remote URL", terms: ["clone", "remote", "repository", "repo", "git", "Git URL"], keep: true, run: () => pickSource("url") },
      ready ? github : { ...github, disabled: true, trail: `<button type="button" class="setup" data-setup title="Connect GitHub under Settings → Connectors">Setup Required</button>` },
      boxed ? box : { ...box, disabled: true, trail: `<button type="button" class="setup" data-setup title="Connect Upstash Box under Settings → Connectors">Setup Required</button>` },
    ];
    return filterGroups([{ label: "Sources", items }], query);
  },
};
function pickSource(source) {
  Object.assign(palette.adding, { source, repository: null, remoteUrl: "", input: "" });
  if (source !== "url") loadGithubRepos(palette.adding);
  palette.views.push({ view: "add-repository", query: "" });
  paletteInput.value = "";
  palette.hi = -1;
  renderPalette();
}
function newProjectOn(machine) {
  palette.newMachine = machine;
  palette.views.push({ view: "new-project", query: "", adding: true });
  paletteInput.value = "";
  palette.hi = -1;
  renderPalette();
}

/** Environments, when there is more than one machine: picking one opens its Sources. */
PALETTE_VIEWS["add-environments"] = {
  placeholder: "Search...",
  lead: () => "folder-plus",
  groups(query) {
    const items = onlineMachines().sort((a, b) => Number(b === LOCAL) - Number(a === LOCAL) || machineName(a).localeCompare(machineName(b))).map((m) => (
      { icon: "monitor", title: esc(machineName(m)), sub: m === LOCAL ? "This device" : m, terms: [machineName(m), m, m === LOCAL ? "this device" : ""], keep: true, run: () => showSources(m) }));
    return filterGroups([{ label: "Environments", items }], query);
  },
};
function showSources(machine) {
  palette.adding = { machine, github: palette.adding?.github ?? null, upstash: palette.adding?.upstash ?? null };
  palette.views.push({ view: "add-sources", query: "", adding: true });
  paletteInput.value = "";
  palette.hi = 0;
  renderPalette();
}
/** Asks once whether GitHub is connected, for the Sources' GitHub repository row. */
function beginAdding(machine) {
  palette.adding = { machine, github: null, upstash: null };
  connectorCall("get_settings").then((s) => {
    if (!palette.adding) return;
    palette.adding.github = s?.github ?? null;
    palette.adding.upstash = s?.upstash ?? null;
    if (paletteOpen() && paletteView().view === "add-sources") renderPalette();
  }, () => {});
}
/** CommandPalette's openAddProjectFlow: Environments when there are several machines, else that one's Sources. */
function startAddProject(machine = null) {
  beginAdding(machine);
  if (!machine && !onlineMachines().length) return closePalette(), openSettings("machines");
  if (machine || onlineMachines().length === 1) return showSources(machine ?? onlineMachines()[0]);
  palette.views.push({ view: "add-environments", query: "", adding: true });
  paletteInput.value = "";
  palette.hi = selectable()[0] ?? 0;
  renderPalette();
}
/** New project's "Add existing project": back to the Sources it came from, or that machine's. */
function existingProjectSources(machine) {
  palette.views.pop();
  const sources = paletteView().view === "add-sources";
  if (sources && palette.adding?.machine === machine) {
    paletteInput.value = paletteView().query;
    palette.hi = 0;
    return renderPalette();
  }
  if (sources) palette.views.pop();
  beginAdding(machine);
  showSources(machine);
}
/** Local folder, and Open WSL folder: the path to browse starts at `~/`. */
function browseForProject(machine) {
  palette.adding = { ...(palette.adding ?? {}), machine };
  palette.views.push({ view: "add-browse", query: ADD_PROJECT_START, adding: true });
  paletteInput.value = ADD_PROJECT_START;
  palette.hi = -1;
  renderPalette();
}
paletteResults.addEventListener("click", (e) => {
  if (!e.target.closest("[data-setup]")) return;
  e.stopPropagation();
  closePalette();
  openSettings("integrations");
}, true);
