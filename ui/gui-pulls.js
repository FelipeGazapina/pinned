Object.assign(ICON_PATHS, {
  "user-lock": `<path d="M19 16v-2a2 2 0 0 0-4 0v2"/><path d="M9.5 15H7a4 4 0 0 0-4 4v2"/><circle cx="10" cy="7" r="4"/><rect x="13" y="16" width="8" height="5" rx=".899"/>`,
  "calendar-arrow-down": `<path d="m14 17 4 4 4-4"/><path d="M16 2v3"/><path d="M18 13v8"/><path d="M21 10.354V5a2 2 0 00-2-2H5a2 2 0 00-2 2v14a2 2 0 002 2h7.343"/><path d="M3 9h18"/><path d="M8 2v3"/>`,
  "calendar-arrow-up": `<path d="m14 17 4-4 4 4"/><path d="M16 2v3"/><path d="M18 21v-8"/><path d="M21 10.343V5a2 2 0 00-2-2H5a2 2 0 00-2 2v14a2 2 0 002 2h9"/><path d="M3 9h18"/><path d="M8 2v3"/>`,
  "plug-2": `<path d="M9 2v6"/><path d="M15 2v6"/><path d="M12 17v5"/><path d="M5 8h14"/><path d="M6 11V8h12v3a6 6 0 1 1-12 0Z"/>`,
  "eye-off": `<path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/>`,
  "circle-slash": `<circle cx="12" cy="12" r="10"/><line x1="9" x2="15" y1="15" y2="9"/>`,
});

/** T3 Code's Pull requests page: the pull requests of every open project's GitHub repository, from
 * each machine's gh, grouped by the viewer's part in them. A row opens in the page's own panel. */
const PR_STATES = [
  ["all", "All", "layers"],
  ["open", "Open", "git-pull-request-arrow"],
  ["closed", "Closed", "git-pull-request-closed"],
  ["merged", "Merged", "git-merge"],
];
const PR_INVOLVEMENTS = [
  ["all", "All", "layers"],
  ["reviewing", "Reviewing", "eye"],
  ["authored", "Authored", "pen-line"],
];
const PR_SORTS = [
  ["ready", "Merge readiness", "list-checks"],
  ["blocked", "Blocked on me", "user-lock"],
  ["updated", "Recently updated", "clock"],
  ["newest", "Newest shown", "calendar-arrow-down"],
  ["oldest", "Oldest shown", "calendar-arrow-up"],
  ["largest", "Largest shown", "maximize-2"],
  ["smallest", "Smallest shown", "minimize-2"],
];
const PR_DRAFTS = [
  ["", "All", "layers"],
  ["only", "Drafts only", "git-pull-request-draft"],
  ["hide", "Hide drafts", "eye-off"],
];
const PR_REVIEWS = [
  ["", "All", "layers"],
  ["approved", "Approved", "circle-check"],
  ["changes_requested", "Changes requested", "circle-x"],
  ["review_required", "Review required", "circle-dashed"],
  ["none", "No reviews", "circle-slash"],
];
const PR_CHECK_FILTERS = [
  ["", "All", "layers"],
  ["passing", "Passing", "circle-check"],
  ["failing", "Failing", "circle-x"],
];
const PR_GROUPS = [
  ["authored", "Authored", "pen-line"],
  ["reviewing", "Review requested", "eye"],
  ["others", "Others", "users"],
];
const PR_STATE_ICON = {
  open: ["git-pull-request-arrow", "Open"],
  draft: ["git-pull-request-draft", "Draft"],
  closed: ["git-pull-request-closed", "Closed"],
  merged: ["git-merge", "Merged"],
};
const PR_CHECKS = { passing: ["circle-check", "Checks passing", "All checks have passed"], failing: ["circle-x", "Checks failing", "Some checks were not successful"], pending: ["circle-dot", "Checks pending", "Some checks haven't completed yet"] };
const PR_REVIEW = { approved: ["user-check", "Approved"], changes_requested: ["user-round-x", "Changes requested"], review_required: ["user-round", "Awaiting review"] };
const PR_PAGE = 100;
const PR_MAX = 500;
const PR_SAVED = ["state", "involvement", "sort", "host", "q", "machine", "project", "draft", "review", "checks", "author", "labels"];
const pulls = {
  state: "open", involvement: "all", sort: "ready", host: null, q: "", machine: null, project: null,
  draft: null, review: null, checks: null, author: null, labels: [],
  limit: PR_PAGE, rows: null, sent: null, more: false, error: "", loading: false, loadingMore: false, fetchedAt: 0, seq: 0,
  closing: new Set(), acting: new Map(), states: new Map(), searchOpen: false,
};
try {
  const saved = JSON.parse(load("gui-pr-page"));
  const pick = (key, list) => list.some(([v]) => v && v === saved?.[key]) && (pulls[key] = saved[key]);
  [["state", PR_STATES], ["involvement", PR_INVOLVEMENTS], ["sort", PR_SORTS], ["draft", PR_DRAFTS], ["review", PR_REVIEWS], ["checks", PR_CHECK_FILTERS]].forEach(([key, list]) => pick(key, list));
  for (const key of ["host", "q", "machine", "project", "author"]) if (typeof saved?.[key] === "string" && saved[key].length <= 200) pulls[key] = saved[key];
  if (Array.isArray(saved?.labels)) pulls.labels = saved.labels.filter((l) => typeof l === "string").slice(0, 10);
} catch {}
const savePulls = () => save("gui-pr-page", JSON.stringify(Object.fromEntries(PR_SAVED.map((key) => [key, pulls[key]]).filter(([, v]) => v !== null && v !== "" && !(Array.isArray(v) && !v.length)))));

/** T3 Code's parsePullRequestQuery: label:, author:, draft:, review: and status: come out of the
 * typed text as filters, and any other qualifier is a scoped label such as size:S. */
const PR_REVIEW_WORDS = { approved: "approved", changes_requested: "changes_requested", "changes-requested": "changes_requested", required: "review_required", "review-required": "review_required", none: "none" };
const PR_CHECK_WORDS = { success: "passing", passing: "passing", failure: "failing", failing: "failing" };
const PR_QUERY_TOKEN = /(?:[^\s"]|"[^"]*")+/g;
function parsePrQuery(raw) {
  const text = [];
  const filters = {};
  const names = (list) => (list.match(/(?:[^,"]|"[^"]*")+/g) ?? []).map((n) => n.replaceAll('"', "").trim().slice(0, 200).trim()).filter(Boolean).slice(0, 10);
  for (const [token] of raw.matchAll(PR_QUERY_TOKEN)) {
    const q = /^(-?)([A-Za-z][A-Za-z0-9_-]*):(.*)$/.exec(token);
    const value = q ? q[3].replaceAll('"', "").trim() : "";
    const negated = q?.[1] === "-";
    const kind = value ? q[2].toLowerCase() : "";
    if (kind === "label" && names(q[3]).length) {
      if (negated) (filters.excluded_labels ??= []).push(...names(q[3]));
      else (filters.labels ??= []).push(names(q[3]));
      continue;
    }
    if (kind === "author" && !negated) {
      filters.author = value.slice(0, 200).trim();
      continue;
    }
    if (kind === "draft" && !negated && /^(true|false)$/i.test(value)) {
      filters.draft = value.toLowerCase() === "true" ? "only" : "hide";
      continue;
    }
    if (kind === "review" && !negated && PR_REVIEW_WORDS[value.toLowerCase()]) {
      filters.review = PR_REVIEW_WORDS[value.toLowerCase()];
      continue;
    }
    if ((kind === "status" || kind === "checks") && !negated && PR_CHECK_WORDS[value.toLowerCase()]) {
      filters.checks = PR_CHECK_WORDS[value.toLowerCase()];
      continue;
    }
    if (kind && !["label", "author", "draft", "review", "status", "checks"].includes(kind) && !value.startsWith("/")) {
      const scoped = names(q[3]).map((n) => (n.includes(":") ? n : `${q[2]}:${n}`));
      if (scoped.length) {
        if (negated) (filters.excluded_labels ??= []).push(...scoped);
        else (filters.labels ??= []).push(scoped);
        continue;
      }
    }
    text.push(token);
  }
  if (filters.labels) filters.labels = filters.labels.slice(0, 10);
  if (filters.excluded_labels) filters.excluded_labels = filters.excluded_labels.slice(0, 10);
  return { text: text.join(" "), filters };
}
/** The Filters menu's own choices, which a typed qualifier of the same kind outranks. */
const menuFilters = () => Object.fromEntries([
  ["draft", pulls.draft], ["review", pulls.review], ["checks", pulls.checks], ["author", pulls.author], ["labels", pulls.labels.length ? pulls.labels.map((l) => [l]) : null],
].filter(([, v]) => v));
const prFilters = (q) => ({ ...menuFilters(), ...parsePrQuery(q).filters });

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
const projectKey = (machine, path) => `${machine}\n${path}`;
/** The projects the Project filter offers, in the sidebar's order, a thread of each for its icon. */
function prProjects() {
  const seen = new Map();
  for (const tab of tabs) {
    if (!machines.get(tab.machine)?.view.online) continue;
    const key = projectKey(tab.machine, projectOfTab(tab));
    if (!seen.has(key)) seen.set(key, { key, machine: tab.machine, path: projectOfTab(tab), name: projectName(tab), tab });
  }
  return [...seen.values()];
}
const scopedMachine = () => (openProjects().has(pulls.machine) ? pulls.machine : null);
const scopedProject = () => prProjects().find((p) => p.key === pulls.project) ?? null;
/** What the page asks for: each machine's folders, narrowed by the Server and Project filters. */
function pullScope() {
  const project = scopedProject();
  if (project) return new Map([[project.machine, [project.path]]]);
  return new Map([...openProjects()].filter(([m]) => !scopedMachine() || m === scopedMachine()).map(([m, paths]) => [m, [...paths]]));
}

async function fetchPulls(more = false) {
  const seq = ++pulls.seq;
  const sent = pulls.q;
  const { text } = parsePrQuery(sent);
  Object.assign(pulls, { loading: true, loadingMore: more, error: "" });
  renderPullsPage();
  const asks = [...pullScope()].map(([m, paths]) =>
    callOn(m, { cmd: "pull_requests", paths, state: pulls.state, involvement: pulls.involvement, query: text, filters: prFilters(sent), limit: pulls.limit }).then((found) => ({ ...found, rows: found.rows.map((r) => ({ ...r, machine: m })) })));
  const answers = await Promise.allSettled(asks);
  if (seq !== pulls.seq) return;
  const seen = new Set();
  const rows = answers.flatMap((a) => (a.status === "fulfilled" ? a.value.rows : [])).filter((r) => !seen.has(r.url) && seen.add(r.url));
  if (answers.length > 1) rows.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
  const failed = answers.find((a) => a.status === "rejected");
  const more_ = answers.some((a) => a.status === "fulfilled" && a.value.more);
  for (const r of rows) if (pulls.states.get(r.url)?.until > Date.now() && r.state !== "merged") r.state = pulls.states.get(r.url).state;
  Object.assign(pulls, { rows: failed && !rows.length ? pulls.rows : rows, sent, more: more_, error: failed ? String(failed.reason).replace(/^Error: /, "") : "", loading: false, loadingMore: false, fetchedAt: Date.now() });
  if (shellPage() === "pull-requests") renderPullsPage();
}

function setPulls(change) {
  Object.assign(pulls, { limit: PR_PAGE }, change);
  savePulls();
  fetchPulls();
}

const prSize = (r) => r.additions + r.deletions;
const prCount = new Intl.NumberFormat();
const prHosts = () => [...new Set((pulls.rows ?? []).map((r) => r.host))];
/** A host's name in the menus: GitHub, or its address once two of them share the list. */
const prHostLabel = (host) => (prHosts().length > 1 ? host : "GitHub");
/** T3 Code's matchesPullRequestFilters: the rows a filter keeps before, and besides, the hosts' answer. */
function matchesPrFilters(r, f) {
  const labels = r.labels.map((l) => l.name.trim().toLowerCase());
  const holds = (name) => labels.includes(name.trim().toLowerCase());
  const author = f.author && (/^@?me$/i.test(f.author.trim()) ? null : f.author.trim().toLowerCase());
  return (!f.draft || (r.state === "draft") === (f.draft === "only"))
    && (!f.review || (f.review === "none" ? !r.review : r.review === f.review))
    && (!f.labels || f.labels.every((group) => group.some(holds)))
    && (!f.excluded_labels || !f.excluded_labels.some(holds))
    && (!author || r.author.toLowerCase() === author);
}
const matchesPrText = (r, text) => `#${r.number} ${r.title} ${r.repo} ${r.head} ${r.author}`.toLowerCase().includes(text.trim().toLowerCase());
/** The rows shown: under the host filter, and narrowed at once by what is typed until the hosts answer it. */
function shownPulls() {
  const typed = parsePrQuery(pulls.q);
  const answered = pulls.sent === pulls.q;
  const local = { ...menuFilters(), ...typed.filters };
  const inState = (r) => pulls.state === "all" || r.state === pulls.state || (pulls.state === "open" && r.state === "draft");
  return (pulls.rows ?? []).filter((r) => inState(r) && (!pulls.host || r.host === pulls.host) && matchesPrFilters(r, local) && (!typed.text || answered || matchesPrText(r, typed.text)));
}
const prSearching = () => !!pulls.q.trim() && pulls.q !== pulls.sent;
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
  const text = parsePrQuery(pulls.q).text.trim();
  const recent = (a, b) => b.updated_at.localeCompare(a.updated_at);
  const bySize = (a, b) => Number(prSize(b) > 0) - Number(prSize(a) > 0);
  if (sort === "ready") return text ? list : [...list].sort((a, b) => PR_SORT_RANK.ready(a) - PR_SORT_RANK.ready(b) || bySize(a, b) || prSize(a) - prSize(b) || recent(a, b));
  if (sort === "blocked") {
    const role = group === "others" ? pulls.involvement : group;
    if (text || role === "all") return list;
    const rank = role === "authored" ? PR_SORT_RANK.author : PR_SORT_RANK.reviewer;
    return [...list].sort((a, b) => rank(a) - rank(b) || recent(a, b));
  }
  if (sort === "newest" || sort === "oldest") return [...list].sort((a, b) => (sort === "newest" ? -1 : 1) * a.created_at.localeCompare(b.created_at) || recent(a, b));
  if (sort === "largest" || sort === "smallest") return [...list].sort((a, b) => bySize(a, b) || (sort === "largest" ? -1 : 1) * (prSize(a) - prSize(b)) || recent(a, b));
  return list;
}

/** The pull request the page's panel shows, if it is open. */
function panelPull() {
  const p = PAGE_PANELS["pull-requests"];
  return p.open ? p.surfaces.find((s) => s.id === p.active && s.kind === "pr")?.url ?? null : null;
}

const unixOfIso = (iso) => Date.parse(iso) / 1000 || 0;
const PR_LABEL_SLOTS = ["", "wide", "wider"];
function pullLabelsHtml(labels) {
  return labels.length ? `<span class="labels">${labels.slice(0, 3).map((l, i) => {
    const left = labels.length - i - 1;
    return `<span class="label ${PR_LABEL_SLOTS[i]}" style="--label:#${/^[0-9a-f]{6}$/i.test(l.color) ? l.color : "818181"}"><span class="name">${esc(l.name)}</span>${left > 0 ? `<span class="more">+${left}</span>` : ""}</span>`;
  }).join("")}</span>` : "";
}
const pullAuthorHtml = (r) => {
  const avatar = /^https:\/\/avatars\.githubusercontent\.com\//.test(r.avatar) ? `<img src="${esc(r.avatar)}" alt="" referrerpolicy="no-referrer">` : `<span class="monogram" aria-hidden="true">${esc((r.author || "ghost").slice(0, 1).toUpperCase())}</span>`;
  const who = r.author_name && r.author_name !== r.author ? `${r.author_name} (@${r.author})` : r.author;
  return `<span class="author" title="${esc(who)}">${avatar}<span class="login">${esc(r.author || "ghost")}</span></span>`;
};
const PR_QUICK = { close: ["Close", "git-pull-request-closed"], merge: ["Merge", "git-merge"], ready: ["Ready for review", "git-pull-request-arrow"], reopen: ["Reopen", "git-pull-request-arrow"] };
const quickActions = (r) => (r.state === "merged" ? [] : r.state === "closed" ? ["reopen"] : r.state === "draft" ? ["close", "ready"] : ["close", "merge"]);
function pullRowHtml(r) {
  const [stateIcon, stateLabel] = PR_STATE_ICON[r.state];
  const checks = r.checks && PR_CHECKS[r.checks];
  const review = r.review && PR_REVIEW[r.review];
  const busy = pulls.closing.has(r.url) || pulls.acting.has(r.url) || closeSweep.keys.includes(r.url);
  const quick = quickActions(r).map((action) => {
    const [label, ic] = PR_QUICK[action];
    const stacked = action === "merge" && r.stack;
    const title = stacked ? "Open this pull request to merge its stack" : action === "close" ? "Close immediately, or drag across rows to close several" : `${label} immediately`;
    return `<button type="button" class="${action === "close" ? "danger" : ""}" data-quick="${action}" title="${title}" aria-label="${label} #${r.number}"${busy || stacked ? " disabled" : ""}>${busy ? icon("loader-circle", "spin") : icon(ic)}${label}</button>`;
  }).join("");
  const words = parsePrQuery(pulls.sent ?? "").text;
  const matched = words && pullMatchScore(r, words) <= 10 ? `<span class="matched" title="Matched in the description"><span class="sr">matched in the description</span>${icon("search")}<span class="wide">matched in the description</span></span>` : "";
  const host = prHosts().length > 1 ? `<span class="host" title="${esc(r.host)}" aria-label="${esc(r.host)}">${icon("github")}</span>` : "";
  const machine = openProjects().size > 1 ? `<span class="machine">${esc(machineName(r.machine))}</span>` : "";
  const stack = r.stack ? `<span class="stack" role="button" tabindex="0" data-stack title="View stack #${r.stack.number}, layer ${r.stack.position} of ${r.stack.size}" aria-label="Stack ${r.stack.number}, layer ${r.stack.position} of ${r.stack.size}">${icon("layers-2")}${r.stack.position}/${r.stack.size}</span>` : "";
  const stat = prSize(r) ? `<span class="stat" tabindex="0" data-breakdown role="group" aria-label="${r.additions} additions, ${r.deletions} deletions"><span class="add">+${prCount.format(r.additions)}</span> <span class="del">-${prCount.format(r.deletions)}</span></span>` : "";
  const conflict = r.state === "open" && r.conflicting ? `<span class="conflict" title="Conflicts with ${esc(r.base)}">${icon("triangle-alert")}</span>` : "";
  return `<div class="pr-row${panelPull() === r.url ? " selected" : ""}${closeSweep.keys.includes(r.url) ? " sweeping" : ""}" role="button" tabindex="0" data-url="${esc(r.url)}" data-state="${r.state}"${busy ? " data-busy" : ""}>
    <span class="main"><span class="state pr-tone" data-state="${r.state}" title="${stateLabel}">${icon(stateIcon)}${conflict}</span>
    <span class="body"><span class="line1"><span class="num">#${r.number}</span><span class="title">${esc(r.title)}</span>${checks ? `<span class="checks" role="button" tabindex="0" data-checks="${r.checks}" title="${checks[1]}" aria-label="Checks: ${checks[2]}">${icon(checks[0])}</span>` : ""}${review ? `<span class="review" data-review="${r.review}" title="${review[1]}">${icon(review[0])}</span>` : ""}
      <span class="right">${stack}${stat}</span></span>
    <span class="line2">${matched}${host}${pullAuthorHtml(r)}<span class="repo">${esc(r.repo)}</span>${machine}${pullLabelsHtml(r.labels)}<span class="when">${agoLong(unixOfIso(r.updated_at))}</span></span></span></span>
    ${quick ? `<span class="quick" role="group" aria-label="Quick actions for pull request #${r.number}">${quick}</span>` : ""}</div>`;
}

const PR_BRANCH_ART = `<svg class="branch-art" viewBox="0 0 120 72" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  <path class="rail" d="M10 58h100"/><circle cx="10" cy="58" r="5" fill="currentColor" fill-opacity="0.25"/><circle cx="110" cy="58" r="5" fill="currentColor" fill-opacity="0.25"/>
  <path d="M30 58c0-18 8-26 24-26h4"/><path class="dashed" d="M90 58c0-18-8-26-24-26h-4" stroke-dasharray="2 7"/><circle class="dashed" cx="60" cy="32" r="4"/></svg>`;
const prRefreshIcon = () => icon("refresh-cw", pulls.loading ? "spin" : "");
const clip = (q) => (q.length > 48 ? `${q.slice(0, 48)}…` : q);
function pullGhostsHtml(rows, caption = "") {
  const titles = ["60%", "40%", "50%", "66%", "40%", "60%", "50%"];
  const metas = ["40%", "33%", "40%", "25%", "33%", "40%", "33%"];
  return `<div class="pr-ghosts" role="status" aria-label="${esc(caption || "Loading pull requests")}">${caption ? `<p class="caption">${esc(caption)}</p>` : ""}${Array.from({ length: rows }, (_, i) => `<div class="ghost-row"><i class="ghost dot"></i><span class="lines"><i class="ghost" style="width:${titles[i % 7]};height:16px"></i><i class="ghost" style="width:${metas[i % 7]};height:14px"></i></span><span class="ends"><i class="ghost" style="width:48px"></i><i class="ghost" style="width:64px"></i></span></div>`).join("")}</div>`;
}
const filteredPulls = () => !!Object.keys(menuFilters()).length || pulls.state !== "open" || pulls.involvement !== "all" || !!scopedProject() || !!pulls.host;
function pullsEmptyHtml() {
  const q = pulls.q.trim();
  const checkAgain = `<button type="button" class="outline" data-pr="refresh"${pulls.loading ? " disabled" : ""}>${prRefreshIcon()}${pulls.loading ? "Checking..." : "Check again"}</button>`;
  if (!openProjects().size) return `<div class="pr-empty">${PR_BRANCH_ART}<h2>No projects in this workspace</h2><p>Add a project, and the pull requests from its repository appear here.</p><span class="buttons"><button type="button" class="primary" data-pr="add-project">${icon("plus")}Add project</button></span></div>`;
  if (prSearching()) return pullGhostsHtml(5, `Searching every host for “${clip(q)}”`);
  if (q) return `<div class="pr-empty">${PR_BRANCH_ART}<h2>Nothing matches “${esc(clip(q))}”</h2><p>The hosts were searched for it. Try fewer words, or search by number, author or branch.</p>
    <span class="buttons"><button type="button" class="outline" data-pr="clear">${icon("search")}Clear search</button>${checkAgain}</span></div>`;
  const filtered = filteredPulls();
  const more = pulls.more && pulls.limit < PR_MAX ? `<button type="button" class="outline" data-pr="more"${pulls.loading ? " disabled" : ""}>${pulls.loadingMore ? "Loading..." : "Load more pull requests"}</button>` : "";
  return `<div class="pr-empty">${PR_BRANCH_ART}<h2>${filtered ? "Nothing under these filters" : "No pull requests"}</h2><p>${filtered ? "Widen the state, involvement or project filter to see more." : "Pull requests from every project in this workspace appear here."}</p><span class="buttons">${more}${checkAgain}</span></div>`;
}

function pullsListHtml() {
  if (pulls.rows === null) {
    if (pulls.error) return `<div class="pr-empty unavailable"><span class="media">${icon("git-pull-request-arrow")}</span><h2>Could not load pull requests</h2><p>${esc(pulls.error)}</p><span class="buttons"><button type="button" class="outline" data-pr="refresh"${pulls.loading ? " disabled aria-busy=\"true\"" : ""}>${prRefreshIcon()}Retry</button></span></div>`;
    return pullGhostsHtml(7);
  }
  const rows = shownPulls();
  const warn = pulls.error ? `<p class="pr-warn"><span>${esc(pulls.error)} Showing the last pull requests loaded.</span><button type="button" class="outline xs" data-pr="refresh">Retry</button></p>` : "";
  if (!rows.length) return (pulls.loading && !pulls.q.trim() ? pullGhostsHtml(7) : pullsEmptyHtml()) + warn;
  const foot = !pulls.more ? ""
    : pulls.loading ? `<p class="pr-foot muted">${icon("loader-circle", "spin")}${pulls.loadingMore ? "Loading more" : "Updating pull requests"}</p>`
    : pulls.limit >= PR_MAX ? `<p class="pr-foot muted">Narrow your search to find more pull requests.</p>`
    : `<p class="pr-foot"><button type="button" class="outline" data-pr="more">Load more pull requests</button></p>`;
  const list = pulls.involvement !== "all" ? `<div class="pr-list">${sortPulls(rows, pulls.involvement).map(pullRowHtml).join("")}</div>`
    : PR_GROUPS.map(([key, label, ic]) => {
      const group = rows.filter((r) => r.group === key);
      if (!group.length) return "";
      return `<section class="pr-group" data-group="${key}"><h3>${icon(ic)}<span>${label}</span><span class="count">${group.length}</span><span class="rule"></span></h3><div class="pr-list">${sortPulls(group, key).map(pullRowHtml).join("")}</div></section>`;
    }).join("");
  return `<div class="pr-groups">${list}</div>${warn}${foot}`;
}

const filterCount = () => [pulls.state !== "open", pulls.involvement !== "all", pulls.host, scopedMachine(), scopedProject(), parsePrQuery(pulls.q).filters.draft ?? pulls.draft, pulls.review, pulls.checks, pulls.author, ...pulls.labels].filter(Boolean).length;
function renderPullsPage() {
  const focused = document.activeElement?.closest?.("#page-view .pr-search") ? document.activeElement.closest(".pr-search").dataset.at : null;
  const caret = focused ? [document.activeElement.selectionStart, document.activeElement.selectionEnd] : null;
  const stateLabel = PR_STATES.find(([v]) => v === pulls.state)[1];
  const involvementLabel = PR_INVOLVEMENTS.find(([v]) => v === pulls.involvement)[1];
  const searchHtml = (at) => `<label class="pr-search" data-at="${at}">${prSearching() ? icon("loader-circle", "spin") : icon("search")}<input type="search" placeholder="Search pull requests, or label:bug" aria-label="Search pull requests" value="${esc(pulls.q)}" spellcheck="false" autocomplete="off"></label>`;
  const expanded = pullsCondensed() && (pulls.searchOpen || !!pulls.q);
  const hosts = prHosts();
  const compact = `<span class="condensed"><span class="crumb-sep"${expanded ? " hidden" : ""}>/</span><button type="button" class="crumb-menu" data-pr="state" aria-label="Filter by state">${stateLabel}${icon("chevron-down")}</button><button type="button" class="crumb-menu" data-pr="involvement" aria-label="Filter by involvement">${involvementLabel}${icon("chevron-down")}</button>${hosts.length > 1 ? `<button type="button" class="crumb-menu" data-pr="host-compact" aria-label="Filter by host">${esc(pulls.host ? prHostLabel(pulls.host) : "All")}${icon("chevron-down")}</button>` : ""}
    <span class="spacer"></span>${expanded ? searchHtml("top") : `<button type="button" class="icon" data-pr="search-top" title="Search pull requests" aria-label="Search pull requests">${icon("search")}</button>`}<button type="button" class="icon" data-pr="refresh" title="Refresh pull requests" aria-label="Refresh pull requests"${pulls.loading ? " disabled" : ""}>${prRefreshIcon()}</button></span>`;
  const surfaces = PAGE_PANELS["pull-requests"].surfaces.length;
  const toggle = `<span class="panel-toggle" title="${surfaces ? `Side panel (${bindingLabel("rightPanel.toggle")})` : "Select a pull request first"}"><button type="button" class="icon" data-pr="panel" aria-label="Toggle right panel"${surfaces ? "" : " disabled"}>${icon("panel-right")}</button></span>`;
  const count = filterCount();
  const host = pulls.host ? `<button type="button" class="outline icon-only" data-pr="host" title="Filter by provider: ${esc(prHostLabel(pulls.host))}" aria-label="Filter by provider: ${esc(prHostLabel(pulls.host))}">${icon("github")}</button>`
    : `<button type="button" class="outline" data-pr="host" aria-label="Filter by provider: All">${icon("plug-2")}<span>All</span></button>`;
  setPage(`${headHtml("Pull Requests", `${compact}${toggle}`)}<div class="page-body"><div class="page-wrap pulls"${pulls.loading && pulls.rows ? " data-refreshing" : ""}${closeSweep.keys.length ? " data-sweeping" : ""}>
    <div class="pr-toolbar">${searchHtml("flow")}
      <button type="button" class="outline" data-pr="sort" aria-label="Sort pull requests">${icon("arrow-down-up")}<span>Sort</span></button>
      <button type="button" class="outline" data-pr="filters">${icon("list-filter")}<span>Filters</span>${count ? `<span class="badge">${count}</span>` : ""}</button>
      ${host}
      <button type="button" class="outline icon-only" data-pr="refresh" title="Refresh pull requests" aria-label="Refresh pull requests"${pulls.loading ? " disabled" : ""}>${prRefreshIcon()}</button></div>
    ${pullsListHtml()}</div></div>`);
  pageView.querySelector(".page-head").classList.add("pulls-head");
  pageView.querySelector(".page-head").classList.toggle("searching", expanded);
  pageView.dataset.condensed = String(pullsCondensed());
  if (focused) {
    const input = pageView.querySelector(`.pr-search[data-at="${focused}"] input`) ?? pageView.querySelector(".pr-search input");
    input.focus({ preventScroll: true });
    input.setSelectionRange(...caret);
  }
  if (!prPopover.hidden && prPopover.dataset.url && !pullRowOf(prPopover.dataset.url)) closePrPopover();
  pullsMenuRefresh();
}
/** The toolbar has scrolled under the header, which then carries the filters, as T3 Code's does. */
function pullsCondensed() {
  const body = pageView.querySelector(".page-body");
  const toolbar = pageView.querySelector(".pr-toolbar");
  return !!body && !!toolbar && body.scrollTop > toolbar.offsetTop + toolbar.offsetHeight;
}

let pullSearchTimer = 0;
pageView.addEventListener("input", (e) => {
  if (shellPage() !== "pull-requests" || !e.target.matches(".pr-search input")) return;
  pulls.q = e.target.value;
  if (!pulls.q) pulls.searchOpen = e.target.closest(".pr-search").dataset.at === "top";
  savePulls();
  clearTimeout(pullSearchTimer);
  pullSearchTimer = setTimeout(() => {
    pulls.limit = PR_PAGE;
    fetchPulls();
  }, 250);
  renderPullsPage();
});
pageView.addEventListener("focusout", (e) => {
  if (shellPage() !== "pull-requests" || !e.target.matches?.('.pr-search[data-at="top"] input') || pulls.q) return;
  setTimeout(() => {
    if (pageView.contains(document.activeElement) && document.activeElement.closest?.('.pr-search[data-at="top"]')) return;
    pulls.searchOpen = false;
    renderPullsPage();
  });
});
const pickHtml = ([value, label, ic], current, run) => ({ label, icon: ic, radio: (current ?? "") === value, run: () => run(value || null) });
const pickState = (value) => setPulls({ state: value || "all" });
const pickInvolvement = (value) => setPulls({ involvement: value || "all" });
const setHost = (host) => {
  pulls.host = host;
  savePulls();
  renderPullsPage();
};
const hostEntries = () => [{ label: "All", icon: "plug-2", radio: !pulls.host, run: () => setHost(null) }, ...prHosts().map((h) => ({ label: prHostLabel(h), icon: "github", radio: pulls.host === h, run: () => setHost(h) }))];
function filtersMenu() {
  const sub = (label, list, value, run) => {
    const current = list.find(([v]) => v === (value ?? "")) ?? list[0];
    return { label, icon: current[2], value: current[1], sub: () => [{ group: label }, ...list.map((o) => pickHtml(o, value ?? "", run))] };
  };
  const draft = prFilters(pulls.q).draft ?? null;
  const hosts = prHosts();
  const servers = [...openProjects().keys()];
  const projects = prProjects();
  return [
    sub("State", PR_STATES, pulls.state, pickState),
    sub("Involvement", PR_INVOLVEMENTS, pulls.involvement, pickInvolvement),
    "-",
    { label: "Author", icon: "user-round", value: pulls.author ?? "Anyone", sub: authorEntries, facet: "authors" },
    { label: "Labels", icon: "tag", value: pulls.labels.length ? `${pulls.labels.length} selected` : "Any", sub: labelEntries, facet: "labels" },
    sub("Draft", PR_DRAFTS, draft, (value) => {
      const typed = parsePrQuery(pulls.q).filters.draft;
      if (typed) pulls.q = [...pulls.q.matchAll(PR_QUERY_TOKEN)].map(([t]) => t).filter((t) => parsePrQuery(t).filters.draft === undefined).join(" ");
      setPulls({ draft: value });
    }),
    sub("Review", PR_REVIEWS, pulls.review, (value) => setPulls({ review: value })),
    sub("Checks", PR_CHECK_FILTERS, pulls.checks, (value) => setPulls({ checks: value })),
    ...(hosts.length > 1 ? ["-", { label: "Host", icon: pulls.host ? "github" : "plug-2", value: pulls.host ? prHostLabel(pulls.host) : "All", sub: () => [{ group: "Host" }, ...hostEntries()] }] : []),
    ...(servers.length > 1 ? ["-", { label: "Server", icon: scopedMachine() ? "monitor" : "layers", value: scopedMachine() ? machineName(scopedMachine()) : "All servers", sub: () => [{ group: "Server" },
      { label: "All servers", icon: "layers", radio: !scopedMachine(), run: () => setPulls({ machine: null }) },
      ...servers.map((m) => ({ label: machineName(m), icon: "monitor", radio: scopedMachine() === m, run: () => setPulls({ machine: m, project: null }) }))] }] : []),
    "-",
    { label: "Project", iconHtml: scopedProject() ? projectIcon(scopedProject().name, scopedProject().tab) : icon("layers"), value: scopedProject()?.name ?? "All projects", sub: () => [{ group: "Project" },
      { label: "All projects", icon: "layers", radio: !scopedProject(), run: () => setPulls({ project: null }) },
      ...projects.map((p) => ({ label: p.name, iconHtml: projectIcon(p.name, p.tab), radio: scopedProject()?.key === p.key, run: () => setPulls({ project: p.key }) }))] },
  ];
}

/** What the Author and Labels pickers know: the loaded rows of every state, counted, and what the
 * hosts find for the typed text. */
const prFacets = { rows: [], seq: 0, hosted: { labels: null, authors: null }, typed: { labels: "", authors: "" }, timer: 0 };
async function loadFacets() {
  const seq = ++prFacets.seq;
  const answers = await Promise.allSettled([...pullScope()].map(([m, paths]) => callOn(m, { cmd: "pull_requests", paths, state: "all", involvement: pulls.involvement, query: "", limit: PR_PAGE })));
  if (seq !== prFacets.seq) return;
  prFacets.rows = answers.flatMap((a) => (a.status === "fulfilled" ? a.value.rows : []));
  pullsMenuRefresh();
}
function searchFacets(kind, typed) {
  prFacets.typed[kind] = typed;
  prFacets.hosted[kind] = null;
  clearTimeout(prFacets.timer);
  prFacets.timer = setTimeout(async () => {
    const asks = [...pullScope()].map(([m, paths]) => callOn(m, { cmd: "pull_request_filter_options", paths, kind, query: typed.trim().slice(0, 100) }));
    const answers = await Promise.allSettled(asks);
    if (prFacets.typed[kind] !== typed) return;
    const failed = answers.length && answers.every((a) => a.status === "rejected");
    prFacets.hosted[kind] = { failed, found: answers.flatMap((a) => (a.status === "fulfilled" ? a.value[kind] : [])) };
    pullsMenuRefresh();
  }, 250);
  pullsMenuRefresh();
}
function facetCounts() {
  const byUrl = new Map([...prFacets.rows, ...(pulls.rows ?? [])].map((r) => [r.url, r]));
  const authors = new Map();
  const labels = new Map();
  for (const r of byUrl.values()) {
    const inState = pulls.state === "all" || r.state === pulls.state || (pulls.state === "open" && r.state === "draft");
    const key = r.author.toLowerCase();
    const held = authors.get(key);
    authors.set(key, { login: held?.login ?? r.author, avatar: held?.avatar ?? r.avatar, count: (held?.count ?? 0) + Number(inState), merged: (held?.merged ?? 0) + Number(r.state === "merged") });
    if (!inState) continue;
    for (const l of r.labels) {
      const lk = l.name.toLowerCase();
      labels.set(lk, { name: labels.get(lk)?.name ?? l.name, color: labels.get(lk)?.color ?? l.color, count: (labels.get(lk)?.count ?? 0) + 1 });
    }
  }
  return {
    authors: [...authors.values()].filter((a) => a.count > 0).sort((a, b) => b.merged - a.merged || b.count - a.count || a.login.localeCompare(b.login)),
    labels: [...labels.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name)),
  };
}
const avatarHtml = (login, avatar) => (/^https:\/\/avatars\.githubusercontent\.com\//.test(avatar ?? "") ? `<img class="avatar" src="${esc(avatar)}" alt="" referrerpolicy="no-referrer">` : `<span class="avatar monogram" aria-hidden="true">${esc((login || "?").slice(0, 1).toUpperCase())}</span>`);
function authorEntries() {
  const typed = prFacets.typed.authors.trim().toLowerCase();
  const hosted = prFacets.hosted.authors;
  const loaded = facetCounts().authors.filter((a) => !typed || a.login.toLowerCase().includes(typed));
  const seen = new Set(loaded.map((a) => a.login.toLowerCase()));
  const added = (hosted?.found ?? []).filter((a) => !seen.has(a.login.toLowerCase()) && seen.add(a.login.toLowerCase())).map((a) => ({ login: a.login, avatar: a.avatar, count: 0, merged: 0 }));
  const login = pulls.author?.toLowerCase() ?? "";
  const selected = pulls.author ? [...loaded, ...added].find((a) => a.login.toLowerCase() === login) ?? { login: pulls.author, avatar: "", merged: 0 } : null;
  const visible = [...(selected ? [selected] : []), ...[...loaded.slice(0, 15), ...added, ...loaded.slice(15)].filter((a) => a.login.toLowerCase() !== login)].slice(0, 30);
  const pending = hosted === null;
  return [
    { search: "Search authors", value: prFacets.typed.authors, busy: pending, input: (q) => searchFacets("authors", q) },
    { label: "Anyone", icon: "layers", radio: !pulls.author, run: () => setPulls({ author: null }) },
    ...visible.map((a) => ({ label: a.login, iconHtml: avatarHtml(a.login, a.avatar), aside: a.merged > 0 ? `${a.merged} merges loaded` : "", radio: a.login.toLowerCase() === login, run: () => a.login.toLowerCase() !== login && setPulls({ author: a.login }) })),
    ...(!visible.length && !pending ? [{ label: hosted?.failed ? "Couldn't search authors on the host" : "No authors found", disabled: true }] : []),
  ];
}
function labelEntries() {
  const typed = prFacets.typed.labels.trim().toLowerCase();
  const hosted = prFacets.hosted.labels;
  const loaded = facetCounts().labels.filter((l) => !typed || l.name.toLowerCase().includes(typed));
  const seen = new Set(loaded.map((l) => l.name.toLowerCase()));
  const choices = [...loaded, ...(hosted?.found ?? []).filter((l) => !seen.has(l.name.toLowerCase()) && seen.add(l.name.toLowerCase())).map((l) => ({ ...l, count: 0 }))];
  const picked = new Set(pulls.labels.map((l) => l.toLowerCase()));
  const visible = [...pulls.labels.filter((n) => !choices.some((c) => c.name.toLowerCase() === n.toLowerCase())).map((name) => ({ name, color: "", count: 0 })), ...choices];
  const pending = hosted === null;
  const toggle = (name) => {
    const on = picked.has(name.toLowerCase());
    setPulls({ labels: (on ? pulls.labels.filter((l) => l.toLowerCase() !== name.toLowerCase()) : [...pulls.labels, name]).slice(0, 10) });
  };
  return [
    { search: "Search labels", value: prFacets.typed.labels, busy: pending, input: (q) => searchFacets("labels", q) },
    ...(!visible.length && !pending ? [{ label: hosted?.failed ? "Couldn't search labels on the host" : "No labels found", disabled: true }] : []),
    ...visible.map((l) => ({ label: l.name, iconHtml: `<span class="dot" aria-hidden="true"${/^[0-9a-f]{6}$/i.test(l.color) ? ` style="--label:#${l.color}"` : ""}></span>`, aside: l.count > 0 ? String(l.count) : "", check: picked.has(l.name.toLowerCase()), keep: true, run: () => toggle(l.name) })),
  ];
}

/** T3 Code's menus on this page: radio choices with their icons, submenus that name their current
 * choice, a search field, and checkboxes that keep the menu open. Entries are functions of the
 * page's state, so a menu draws itself again when an answer arrives. */
const pullsMenu = document.createElement("div");
pullsMenu.id = "pr-menu";
pullsMenu.className = "pr-menu";
pullsMenu.hidden = true;
pullsMenu.setAttribute("role", "menu");
const pullsSubmenu = pullsMenu.cloneNode();
pullsSubmenu.id = "pr-submenu";
document.body.append(pullsMenu, pullsSubmenu);
const pullsMenuState = { anchor: null, at: null, entries: null, sub: null, subOf: -1, align: "start" };
/** The menu's button, found again when the page draws itself anew under the open menu. */
const pullsMenuAnchor = () => (pullsMenuState.anchor?.isConnected ? pullsMenuState.anchor : (pullsMenuState.anchor = pullsMenuState.at && pageView.querySelector(`[data-pr="${pullsMenuState.at}"]`)));
const pullsMenuOpen = () => !pullsMenu.hidden;
function pullsMenuItemHtml(m, i) {
  if (m === "-") return `<hr>`;
  if (m.group) return `<p class="group">${esc(m.group)}</p>`;
  if (m.search !== undefined) return `<label class="menu-search">${m.busy ? icon("loader-circle", "spin") : icon("search")}<input value="${esc(m.value ?? "")}" placeholder="${esc(m.search)}" aria-label="${esc(m.search)}" spellcheck="false" autocomplete="off"></label>`;
  const lead = m.iconHtml ?? (m.icon ? icon(m.icon) : "");
  const mark = m.check !== undefined ? `<span class="checkmark">${m.check ? icon("check") : ""}</span>` : "";
  const trail = m.sub ? `<span class="value">${esc(m.value ?? "")}</span>${icon("chevron-right", "more")}` : `${m.aside ? `<span class="aside">${esc(m.aside)}</span>` : ""}${m.radio ? icon("check", "indicator") : ""}`;
  const role = m.check !== undefined ? "menuitemcheckbox" : m.radio !== undefined ? "menuitemradio" : "menuitem";
  return `<button type="button" role="${role}" data-i="${i}"${m.radio || m.check ? ` aria-checked="true" class="checked"` : m.radio === false || m.check === false ? ` aria-checked="false"` : ""}${m.disabled ? " disabled" : ""}${m.sub ? ` data-sub aria-haspopup="menu"` : ""}>${mark}${lead}<span class="label">${esc(m.label)}</span>${trail}</button>`;
}
function drawPullsMenu(el, entries, anchorRect, side) {
  const focusedSearch = el.contains(document.activeElement) && document.activeElement.matches("input") ? [document.activeElement.selectionStart, document.activeElement.selectionEnd] : null;
  const focusedItem = el.contains(document.activeElement) ? document.activeElement.dataset?.i : undefined;
  const scroll = el.querySelector(".entries")?.scrollTop ?? 0;
  el.innerHTML = `<div class="entries">${entries.map(pullsMenuItemHtml).join("")}</div>`;
  el.querySelector(".entries").scrollTop = scroll;
  el.hidden = false;
  const w = el.offsetWidth;
  const h = el.offsetHeight;
  let x;
  let y;
  if (side === "inline-end") {
    x = anchorRect.right + w + 4 <= innerWidth - 8 ? anchorRect.right : anchorRect.left - w >= 8 ? anchorRect.left - w : innerWidth - w - 8;
    y = anchorRect.top - 5;
  } else {
    x = pullsMenuState.align === "end" ? anchorRect.right - w : anchorRect.left;
    y = anchorRect.bottom + 4;
    if (y + h > innerHeight - 8 && anchorRect.top - h - 4 >= 8) y = anchorRect.top - h - 4;
  }
  el.style.left = `${Math.max(8, Math.min(x, innerWidth - w - 8))}px`;
  el.style.top = `${Math.max(8, Math.min(y, innerHeight - h - 8))}px`;
  const input = el.querySelector(".menu-search input");
  if (focusedSearch && input) {
    input.focus();
    input.setSelectionRange(...focusedSearch);
  } else if (focusedItem !== undefined) el.querySelector(`[data-i="${focusedItem}"]`)?.focus({ preventScroll: true });
}
function openPullsMenu(anchor, entries, align = "start") {
  const again = !pullsMenu.hidden && pullsMenuState.at === anchor.dataset.pr;
  closePullsMenu();
  if (again) return;
  Object.assign(pullsMenuState, { anchor, at: anchor.dataset.pr, entries, sub: null, subOf: -1, align });
  anchor.setAttribute("aria-expanded", "true");
  drawPullsMenu(pullsMenu, entries(), anchor.getBoundingClientRect(), "bottom");
  pullsMenu.querySelector("button:not([disabled])")?.focus({ preventScroll: true });
  if (entries === filtersMenu) loadFacets();
}
function openPullsSubmenu(i) {
  const m = pullsMenuState.entries()[i];
  if (!m?.sub) return;
  if (pullsMenuState.subOf !== i && m.facet) searchFacets(m.facet, "");
  Object.assign(pullsMenuState, { sub: m.sub, subOf: i });
  for (const b of pullsMenu.querySelectorAll("[data-sub]")) b.classList.toggle("open", b.dataset.i === String(i));
  drawPullsMenu(pullsSubmenu, m.sub(), pullsMenu.querySelector(`[data-i="${i}"]`).getBoundingClientRect(), "inline-end");
  pullsSubmenu.querySelector(".menu-search input")?.focus({ preventScroll: true });
}
function closePullsSubmenu() {
  pullsSubmenu.hidden = true;
  Object.assign(pullsMenuState, { sub: null, subOf: -1 });
  for (const b of pullsMenu.querySelectorAll("[data-sub].open")) b.classList.remove("open");
}
function closePullsMenu() {
  if (pullsMenu.hidden) return;
  closePullsSubmenu();
  pullsMenu.hidden = true;
  pullsMenuAnchor()?.setAttribute("aria-expanded", "false");
  Object.assign(pullsMenuState, { anchor: null, at: null });
}
/** Draws the open menus again with the page's state, keeping them where they are. */
function pullsMenuRefresh() {
  if (pullsMenu.hidden) return;
  if (!pullsMenuAnchor()) return closePullsMenu();
  pullsMenuAnchor().setAttribute("aria-expanded", "true");
  drawPullsMenu(pullsMenu, pullsMenuState.entries(), pullsMenuAnchor().getBoundingClientRect(), "bottom");
  if (pullsMenuState.sub) {
    pullsMenu.querySelector(`[data-i="${pullsMenuState.subOf}"]`)?.classList.add("open");
    drawPullsMenu(pullsSubmenu, pullsMenuState.sub(), pullsMenu.querySelector(`[data-i="${pullsMenuState.subOf}"]`).getBoundingClientRect(), "inline-end");
  }
}
for (const el of [pullsMenu, pullsSubmenu]) {
  el.addEventListener("click", (e) => {
    const b = e.target.closest("button[data-i]");
    if (!b || b.disabled) return;
    const entries = el === pullsMenu ? pullsMenuState.entries() : pullsMenuState.sub();
    const m = entries[b.dataset.i];
    if (m.sub) return pullsMenuState.subOf === Number(b.dataset.i) || openPullsSubmenu(Number(b.dataset.i));
    if (m.keep) {
      m.run();
      return requestAnimationFrame(pullsMenuRefresh);
    }
    closePullsMenu();
    m.run();
  });
  el.addEventListener("input", (e) => {
    const entries = el === pullsMenu ? pullsMenuState.entries() : pullsMenuState.sub();
    entries.find((m) => m?.search !== undefined)?.input(e.target.value);
  });
}
pullsMenu.addEventListener("pointerover", (e) => {
  if (e.pointerType !== "mouse") return;
  const b = e.target.closest("button[data-i]");
  if (!b) return;
  if (b.hasAttribute("data-sub")) {
    if (pullsMenuState.subOf !== Number(b.dataset.i)) openPullsSubmenu(Number(b.dataset.i));
  } else closePullsSubmenu();
});
addEventListener("pointerdown", (e) => {
  if (!pullsMenu.hidden && !pullsMenu.contains(e.target) && !pullsSubmenu.contains(e.target) && !pullsMenuAnchor()?.contains(e.target)) closePullsMenu();
}, true);
addEventListener("keydown", (e) => {
  if (pullsMenu.hidden) return;
  const level = pullsSubmenu.contains(document.activeElement) ? pullsSubmenu : pullsMenu;
  const items = [...level.querySelectorAll("button[data-i]:not([disabled])")];
  const at = items.indexOf(document.activeElement);
  if (e.key === "Escape") {
    if (level === pullsSubmenu) {
      const of = pullsMenuState.subOf;
      closePullsSubmenu();
      pullsMenu.querySelector(`[data-i="${of}"]`)?.focus();
    } else {
      const anchor = pullsMenuAnchor();
      closePullsMenu();
      anchor?.focus({ preventScroll: true });
    }
  } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    items[(at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length]?.focus();
  } else if (e.key === "ArrowRight" && document.activeElement?.hasAttribute?.("data-sub")) {
    openPullsSubmenu(Number(document.activeElement.dataset.i));
    if (!pullsSubmenu.contains(document.activeElement)) pullsSubmenu.querySelector("button[data-i]:not([disabled])")?.focus();
  } else if (e.key === "ArrowLeft" && level === pullsSubmenu && !document.activeElement.matches("input")) {
    const of = pullsMenuState.subOf;
    closePullsSubmenu();
    pullsMenu.querySelector(`[data-i="${of}"]`)?.focus();
  } else return;
  e.preventDefault();
  e.stopPropagation();
}, true);

/** A row's checks, as T3 Code's checks popover lists them: opened at once, then gh's answer,
 * failing and running checks first, and the passed ones behind "Show all". */
const prPopover = document.createElement("div");
prPopover.id = "pr-popover";
prPopover.hidden = true;
prPopover.setAttribute("role", "dialog");
document.body.append(prPopover);
const CHECK_STATUS = {
  pending: ["loader-circle", "Running", "running"],
  "action-required": ["circle-dot", "Awaiting action", "pending"],
  success: ["circle-check", "Passed", "passing"],
  failure: ["circle-x", "Failed", "failing"],
  cancelled: ["circle-x", "Cancelled", "failing"],
  skipped: ["circle-slash", "Skipped", "quiet"],
  neutral: ["circle-dashed", "Neutral", "quiet"],
};
function checkStatus(c) {
  const v = (c.conclusion || c.state || "").toUpperCase();
  const s = (c.status || "").toUpperCase();
  if (["SUCCESS"].includes(v)) return "success";
  if (["FAILURE", "ERROR", "TIMED_OUT", "STARTUP_FAILURE"].includes(v)) return "failure";
  if (v === "CANCELLED") return "cancelled";
  if (v === "ACTION_REQUIRED") return "action-required";
  if (v === "SKIPPED") return "skipped";
  if (v === "NEUTRAL" || v === "STALE") return "neutral";
  return s === "COMPLETED" && !v ? "neutral" : "pending";
}
const checksView = { url: null, checks: null, error: "", all: false };
function checksBodyHtml() {
  if (checksView.error) return `<p class="note">${esc(checksView.error)}</p>`;
  if (checksView.checks === null) return `<p class="note">Loading checks…</p>`;
  if (!checksView.checks.length) return `<p class="note">No checks reported</p>`;
  const sorted = checksView.checks.map((c) => ({ ...c, status: checkStatus(c) }));
  const attention = sorted.filter((c) => ["failure", "cancelled", "action-required"].includes(c.status));
  const running = sorted.filter((c) => c.status === "pending");
  const completed = sorted.filter((c) => ["success", "skipped", "neutral"].includes(c.status));
  const collapse = attention.length + running.length > 0 && completed.length > 0;
  const shown = [...attention, ...running, ...(checksView.all || !collapse ? completed : [])];
  const details = shown.some((c) => c.detailsUrl || c.targetUrl);
  return `<ul class="checks-list${details ? " with-details" : ""}">${shown.map((c) => {
    const [ic, label, tone] = CHECK_STATUS[c.status];
    const url = c.detailsUrl || c.targetUrl || "";
    const approval = c.status === "action-required" && /\/actions\/runs\/\d+(?:\/|$)/.test(url);
    return `<li><span class="tone" data-tone="${tone}">${icon(ic, c.status === "pending" ? "spin" : "")}</span><span class="name" title="${esc(c.description || c.name || c.context || "Check")}">${esc(c.name || c.context || "Check")}</span><span class="status">${approval ? "Awaiting approval" : label}</span>${url && WEB_URL.test(url) ? `<button type="button" class="link" data-details="${esc(url)}">Details</button>` : details ? "<span></span>" : ""}</li>`;
  }).join("")}</ul>${collapse ? `<button type="button" class="ghost xs" data-show-all aria-expanded="${checksView.all}">${checksView.all ? "Show less" : "Show all"}</button>` : ""}`;
}
function drawChecks(r, at) {
  const [, , title] = PR_CHECKS[r.checks];
  prPopover.innerHTML = `<p class="title">${title}</p>${checksBodyHtml()}`;
  prPopover.hidden = false;
  placePopover(at);
}
function placePopover(at) {
  const rect = at.getBoundingClientRect();
  const w = prPopover.offsetWidth;
  const h = prPopover.offsetHeight;
  prPopover.style.left = `${Math.max(8, Math.min(rect.left, innerWidth - w - 8))}px`;
  prPopover.style.top = `${rect.bottom + 4 + h > innerHeight - 8 && rect.top - h - 4 >= 8 ? rect.top - h - 4 : rect.bottom + 4}px`;
}
const pullTab = (url) => {
  const machine = pulls.rows?.find((r) => r.url === url)?.machine;
  return machine === active?.machine ? active : tabs.find((t) => t.machine === machine);
};
async function showChecks(url, at) {
  const r = pulls.rows?.find((row) => row.url === url);
  if (!prPopover.hidden && prPopover.dataset.url === url) return closePrPopover();
  Object.assign(checksView, { url, checks: null, error: "", all: false });
  prPopover.dataset.url = url;
  prPopover.dataset.kind = "checks";
  drawChecks(r, at);
  const tab = pullTab(url);
  try {
    if (!tab) throw new Error("No thread on its machine can reach it.");
    const pr = await callOn(tab.machine, { cmd: "pull_request", tab: tab.view.id, pr: url });
    checksView.checks = pr.statusCheckRollup ?? [];
  } catch (e) {
    checksView.error = String(e).replace(/^Error: /, "");
  }
  if (checksView.url === url && !prPopover.hidden && prPopover.dataset.kind === "checks") drawChecks(r, pullRowOf(url)?.querySelector(".checks") ?? at);
}
function closePrPopover() {
  prPopover.hidden = true;
  delete prPopover.dataset.url;
  checksView.url = null;
}
prPopover.addEventListener("click", (e) => {
  const details = e.target.closest("[data-details]");
  if (details) return openOutside(details.dataset.details);
  if (e.target.closest("[data-show-all]")) {
    checksView.all = !checksView.all;
    const r = pulls.rows?.find((row) => row.url === checksView.url);
    if (r) drawChecks(r, pullRowOf(r.url)?.querySelector(".checks") ?? prPopover);
  }
});
addEventListener("pointerdown", (e) => {
  if (!prPopover.hidden && !prPopover.contains(e.target) && !e.target.closest?.(".pr-row .checks")) closePrPopover();
}, true);
addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || prPopover.hidden) return;
  e.preventDefault();
  e.stopPropagation();
  closePrPopover();
}, true);

/** T3 Code's +/− hover: the change split into code, tests, mocks, generated files and docs, read
 * from the host the first time a row's count is pointed at. */
const prTip = document.createElement("div");
prTip.id = "pr-tip";
prTip.hidden = true;
prTip.setAttribute("role", "tooltip");
document.body.append(prTip);
const breakdowns = new Map();
const CHANGE_KINDS = [["code", "Code"], ["tests", "Tests"], ["mocks", "Mocks"], ["generated", "Generated"], ["docs", "Docs"]];
/** T3 Code's classifyChangedFile, by path alone. */
const GENERATED_FILES = new Set(["bun.lock", "bun.lockb", "cargo.lock", "composer.lock", "deno.lock", "flake.lock", "gemfile.lock", "go.sum", "gradle.lockfile", "mix.lock", "npm-shrinkwrap.json", "package-lock.json", "packages.lock.json", "pipfile.lock", "pnpm-lock.yaml", "podfile.lock", "poetry.lock", "pubspec.lock", "uv.lock", "yarn.lock"]);
const GENERATED_NAME = /(\.generated\.|\.gen\.|\.g\.dart$|\.pb\.go$|_pb2(_grpc)?\.py$|\.pb\.(cc|h)$|\.min\.(js|css)$|\.(js|css)\.map$)/;
const MOCK_NAME = /(\.snap$|\.(mocks?|fixtures?|stubs?|fakes?)\.[^.]+$|^(mocks?|fixtures?|stubs?|fakes?)([-_.A-Z]|$))/;
const MOCK_DIRS = new Set(["__fixtures__", "__mocks__", "__snapshots__", "fakes", "fixture", "fixtures", "mock", "mocks", "stubs", "testdata"]);
const TEST_NAME = /(\.(test|spec|e2e)\.[^.]+$|_(test|spec)\.[^.]+$|^test_.+\.py$|^conftest\.py$|^(vitest|jest)\.setup\.[^.]+$)/i;
const CLASS_TEST_NAME = /(Tests?|Spec)\.(java|kt|kts|scala|groovy|cs|php|swift|m)$/;
const TEST_DIRS = new Set(["__tests__", "__test__", "cypress", "e2e", "spec", "specs", "test", "tests"]);
function changeKind(path) {
  const parts = path.replaceAll("\\", "/").split("/");
  const name = parts.pop() ?? "";
  const dirs = parts.map((d) => d.toLowerCase());
  const inDir = (set) => dirs.some((d) => set.has(d));
  if (GENERATED_FILES.has(name.toLowerCase()) || GENERATED_NAME.test(name) || inDir(new Set(["__generated__", "generated"]))) return "generated";
  if (MOCK_NAME.test(name) || inDir(MOCK_DIRS)) return "mocks";
  if (TEST_NAME.test(name) || CLASS_TEST_NAME.test(name) || inDir(TEST_DIRS)) return "tests";
  if (/\.(md|mdx|markdown|rst|adoc|txt)$/i.test(name) && !/^(requirements.*|constraints.*|CMakeLists|robots|llms)\.txt$/i.test(name)) return "docs";
  return "code";
}
async function loadBreakdown(r) {
  if (breakdowns.has(r.url)) return;
  breakdowns.set(r.url, { pending: true });
  try {
    const files = await callOn(r.machine, { cmd: "pull_request_files", host: r.host, repo: r.repo, number: r.number });
    const split = Object.fromEntries(CHANGE_KINDS.map(([k]) => [k, { additions: 0, deletions: 0 }]));
    for (const f of files ?? []) {
      split[changeKind(f.path)].additions += f.additions;
      split[changeKind(f.path)].deletions += f.deletions;
    }
    breakdowns.set(r.url, { split: files ? split : null });
  } catch {
    breakdowns.set(r.url, { split: null });
  }
  if (prTip.dataset.url === r.url) showBreakdown(pullRowOf(r.url)?.querySelector("[data-breakdown]"));
}
function showBreakdown(stat) {
  const row = stat?.closest(".pr-row[data-url]");
  const r = row && pulls.rows?.find((x) => x.url === row.dataset.url);
  if (!r) return hideBreakdown();
  prTip.dataset.url = r.url;
  loadBreakdown(r);
  const known = breakdowns.get(r.url);
  if (!known?.split && !known?.pending) return (prTip.hidden = true);
  const line = (label, a, d) => `<span class="kind">${label}</span><span class="add">+${prCount.format(a)}</span><span class="del">-${prCount.format(d)}</span>`;
  prTip.innerHTML = known.pending ? "Loading breakdown"
    : `<div class="breakdown">${CHANGE_KINDS.filter(([k]) => known.split[k].additions || known.split[k].deletions).map(([k, label]) => line(label, known.split[k].additions, known.split[k].deletions)).join("")}<span class="sep"></span>${line("Total", r.additions, r.deletions)}</div>`;
  prTip.hidden = false;
  const rect = stat.getBoundingClientRect();
  prTip.style.left = `${Math.max(8, Math.min(rect.left + rect.width / 2 - prTip.offsetWidth / 2, innerWidth - prTip.offsetWidth - 8))}px`;
  prTip.style.top = `${rect.bottom + 4 + prTip.offsetHeight > innerHeight - 8 ? rect.top - prTip.offsetHeight - 4 : rect.bottom + 4}px`;
}
function hideBreakdown() {
  prTip.hidden = true;
  delete prTip.dataset.url;
}
pageView.addEventListener("pointerover", (e) => {
  const stat = e.target.closest?.("[data-breakdown]");
  if (stat && shellPage() === "pull-requests" && prTip.dataset.url !== stat.closest(".pr-row")?.dataset.url) showBreakdown(stat);
});
pageView.addEventListener("pointerout", (e) => {
  const stat = e.target.closest?.("[data-breakdown]");
  if (stat && !stat.contains(e.relatedTarget)) hideBreakdown();
});
pageView.addEventListener("focusin", (e) => e.target.matches?.("[data-breakdown]") && showBreakdown(e.target));
pageView.addEventListener("focusout", (e) => e.target.matches?.("[data-breakdown]") && hideBreakdown());

pageView.addEventListener("click", (e) => {
  if (shellPage() !== "pull-requests") return;
  const button = e.target.closest("[data-pr]");
  const act = button?.dataset.pr;
  if (act === "refresh") return fetchPulls();
  if (act === "add-project") return addProject();
  if (act === "clear") return setPulls({ q: "" });
  if (act === "panel") return togglePanel();
  if (act === "sort") return openPullsMenu(button, () => PR_SORTS.map((o) => pickHtml(o, pulls.sort, (value) => (Object.assign(pulls, { sort: value }), savePulls(), renderPullsPage()))));
  if (act === "filters") return openPullsMenu(button, filtersMenu, "end");
  if (act === "host" || act === "host-compact") return openPullsMenu(button, hostEntries);
  if (act === "state") return openPullsMenu(button, () => PR_STATES.map((o) => pickHtml(o, pulls.state, pickState)));
  if (act === "involvement") return openPullsMenu(button, () => PR_INVOLVEMENTS.map((o) => pickHtml(o, pulls.involvement, pickInvolvement)));
  if (act === "more") {
    pulls.limit = Math.min(PR_MAX, pulls.limit + PR_PAGE);
    return fetchPulls(true);
  }
  if (act === "search-top") {
    pulls.searchOpen = true;
    renderPullsPage();
    return pageView.querySelector('.pr-search[data-at="top"] input')?.focus();
  }
  const quick = e.target.closest("[data-quick]");
  const row = e.target.closest(".pr-row[data-url]");
  if (row && e.target.closest("[data-stack]")) return showStack(row.dataset.url, e.target.closest("[data-stack]"));
  if (quick?.dataset.quick === "close" && closeSweep.swept) return (closeSweep.swept = false);
  if (quick && row) return quickPull(row.dataset.url, quick.dataset.quick);
  if (row && e.target.closest(".checks")) return showChecks(row.dataset.url, e.target.closest(".checks"));
  if (row && !e.target.closest(".quick")) openPull(row.dataset.url);
});
pageView.addEventListener("keydown", (e) => {
  if (shellPage() !== "pull-requests" || (e.key !== "Enter" && e.key !== " ")) return;
  const row = e.target.closest?.(".pr-row[data-url]");
  const checks = e.target.closest?.(".checks");
  if (row && checks) {
    e.preventDefault();
    return showChecks(row.dataset.url, checks);
  }
  if (row && e.target === row) {
    e.preventDefault();
    openPull(row.dataset.url);
  }
});
/** T3 Code leaves a page on Escape even from its search field, after taking the focus out of it. */
window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || e.defaultPrevented || e.repeat || e.isComposing || shellPage() !== "pull-requests" || !guiOn() || !e.target.closest?.("#page-view input")) return;
  if (!paletteEl.hidden || !sideMenu.hidden || !shellDialog.hidden) return;
  e.preventDefault();
  e.target.blur();
  goBack(-1) || closePage();
});
pageView.addEventListener("contextmenu", (e) => {
  const num = e.target.closest(".pr-row[data-url] .num");
  if (!num || shellPage() !== "pull-requests") return;
  e.preventDefault();
  const url = num.closest(".pr-row").dataset.url;
  openSideMenu([
    { label: "Copy link", icon: "copy", run: () => navigator.clipboard?.writeText(url).catch(() => flash("Could not copy the link")) },
    { label: "Open on GitHub", run: () => openOutside(url) },
  ], { x: e.clientX, y: e.clientY });
});
pageView.addEventListener("scroll", (e) => {
  if (!e.target.matches?.(".page-body") || shellPage() !== "pull-requests") return;
  const condensed = pullsCondensed();
  if (String(condensed) === pageView.dataset.condensed) return;
  const typingTop = document.activeElement?.closest?.('.pr-search[data-at="top"]');
  if (!condensed) pulls.searchOpen = false;
  renderPullsPage();
  if (!condensed && typingTop) {
    const input = pageView.querySelector('.pr-search[data-at="flow"] input');
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  }
}, true);
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
        hint: `#${l.number} · ${l.head} · ${PR_STATE_ICON[l.state][1]}`,
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
const canCloseRow = (row) => (row.dataset.state === "open" || row.dataset.state === "draft") && !pulls.closing.has(row.dataset.url) && !pulls.acting.has(row.dataset.url);
function markCloseSweep(keys) {
  if (keys.join("\n") === closeSweep.keys.join("\n")) return;
  closeSweep.keys = keys;
  renderPullsPage();
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
  if (!button || button.disabled || !e.isPrimary || e.button !== 0 || shellPage() !== "pull-requests") return;
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
/** A row's state after an action, held until the hosts answer for it, as T3 Code's overrides are. */
const PR_AFTER = { close: "closed", reopen: "open", ready: "open" };
function overrideState(url, action) {
  const r = pulls.rows?.find((row) => row.url === url);
  if (!r || !PR_AFTER[action]) return;
  r.state = PR_AFTER[action];
  pulls.states.set(url, { state: r.state, until: Date.now() + 60000 });
}
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
      overrideState(url, "close");
      return true;
    } catch (e) {
      failures.push(`#${r?.number ?? "?"}: ${String(e).replace(/^(Error: )?(gh: )?/, "")}`);
      return false;
    } finally {
      pulls.closing.delete(url);
    }
  }));
  const closed = outcomes.filter(Boolean).length;
  const title = failures.length ? `Closed ${closed} of ${batch.length} pull requests` : `Closed ${closed} pull request${closed === 1 ? "" : "s"}`;
  flash([title, ...failures.slice(0, 3)].join(" · "));
  fetchPulls();
}

/** T3 Code's page keeps a right panel of its own, holding only the pull requests opened from it. */
PAGE_PANELS["pull-requests"] = { open: false, surfaces: [], active: null, max: false, width: 0 };
/** The row of the panel's pull request stays selected, and the header's toggle follows the panel. */
function syncPullsPanel() {
  if (!onPullsPage()) return;
  const url = panelPull();
  for (const row of pageView.querySelectorAll(".pr-row[data-url]")) row.classList.toggle("selected", row.dataset.url === url);
  const toggle = pageView.querySelector(".pulls-head .panel-toggle");
  const any = PAGE_PANELS["pull-requests"].surfaces.length > 0;
  if (!toggle) return;
  toggle.title = any ? `Side panel (${bindingLabel("rightPanel.toggle")})` : "Select a pull request first";
  toggle.querySelector("button").disabled = !any;
}
new MutationObserver(syncPullsPanel).observe(document.getElementById("panel-tabs"), { subtree: true, childList: true });
new MutationObserver(syncPullsPanel).observe(document.body, { attributes: true, attributeFilter: ["data-panel"] });
function openPull(url) {
  if (!active) return openOutside(url);
  const p = PAGE_PANELS["pull-requests"];
  p.width ||= Math.floor(innerWidth / 2);
  addSurface(active, { id: `pr:${url}`, kind: "pr", url });
  renderPullsPage();
}
const QUICK_DONE = { merge: "Merge requested", ready: "Marked ready for review", close: "Pull request closed", reopen: "Pull request reopened" };
const QUICK_FAILED = { merge: "Could not merge this pull request", ready: "Could not mark this ready for review", close: "Could not close this pull request", reopen: "Could not reopen this pull request" };
/** T3 Code's resolvePullRequestMergeMethod: the method last picked, if the repository allows it, or the first it allows. */
function quickMergeMethod(url) {
  const allowed = pulls.rows?.find((r) => r.url === url)?.merge_methods ?? [];
  if (!allowed.length) throw new Error("No merge method is available for this repository.");
  return allowed.includes(mergeMethod()) ? mergeMethod() : allowed[0];
}
async function quickPull(url, action) {
  const tab = pullTab(url);
  if (!tab) return openOutside(url);
  pulls.acting.set(url, action);
  renderPullsPage();
  try {
    await callOn(tab.machine, { cmd: "pr_action", tab: tab.view.id, pr: url, action: action === "merge" ? quickMergeMethod(url) : action });
    overrideState(url, action);
    flash(QUICK_DONE[action]);
    fetchPulls();
  } catch (e) {
    flash(`${QUICK_FAILED[action]} · ${String(e).replace(/^(Error: )?(gh: )?/, "")}`);
  } finally {
    pulls.acting.delete(url);
    renderPullsPage();
  }
}
/** Shift alone shows every row's quick actions, as T3 Code's speed mode does. */
for (const type of ["keydown", "keyup"]) window.addEventListener(type, (e) => (document.body.dataset.shiftHeld = e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey));
window.addEventListener("blur", () => delete document.body.dataset.shiftHeld);
const onPullsPage = () => shellPage() === "pull-requests";
onGuiKey("f", () => {
  if (pullsCondensed()) {
    pulls.searchOpen = true;
    renderPullsPage();
  }
  const input = pageView.querySelector(`.pr-search[data-at="${pullsCondensed() ? "top" : "flow"}"] input`);
  input?.focus();
  input?.select();
}, { when: onPullsPage });
onGuiKey("c", () => {
  const url = panelPull();
  if (url) navigator.clipboard?.writeText(url).then(() => flash(`PR link copied · ${url}`), () => flash("Failed to copy PR link"));
}, { shift: true, when: () => onPullsPage() && !!panelPull() });
onGuiKey("w", (e) => e.defaultPrevented || closeSurface(), { when: () => onPullsPage() && PAGE_PANELS["pull-requests"].open });
onGuiKey("b", () => PAGE_PANELS["pull-requests"].surfaces.length && togglePanel(), { alt: true, when: onPullsPage });
for (const [key, shift] of [["d", false], ["j", true]]) onGuiKey(key, () => {}, { shift, when: onPullsPage });
/** The panel's "+" offers nothing here, as T3 Code's page panel adds no surface of its own. */
document.getElementById("panel-add").addEventListener("click", (e) => {
  if (!onPullsPage()) return;
  e.stopImmediatePropagation();
  toggleMenu(e.currentTarget, [
    ["Browser", "globe", "B", "Only available in the desktop app."], ["Terminal", "square-terminal", "T", "Available when a project is open."],
    ["Files", "files", "F", "Available when a project is open."], ["Diff", "file-diff", "D", "Available for Git repositories."],
    ["Pull request", "git-pull-request", "P", "No pull request on this branch yet."], ["Linked pull requests", "link-2", "L", "No linked pull requests available."],
    ["Device", "smartphone", "M", "Available from a thread."],
  ].map(([label, ic, kbd, hint]) => ({ label, icon: ic, kbd, hint, disabled: true, run() {} })));
}, true);
setInterval(() => onPullsPage() && !document.hidden && fetchPulls(), 5 * 60 * 1000);
document.addEventListener("visibilitychange", () => onPullsPage() && !document.hidden && Date.now() - pulls.fetchedAt > 10000 && fetchPulls());

SHELL_PAGES["pull-requests"] = {
  show() {
    pulls.searchOpen = false;
    renderPullsPage();
    renderPanel();
    fetchPulls();
  },
};
