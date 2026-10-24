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
