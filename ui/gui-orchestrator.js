/** The global orchestrator's chat on desktop: the Captains rail beside it, what its report cards and
 * sent messages read from the account's projects, and the key that repaints the chat when that
 * changes. The wording is pure logic in global-orchestrator.js and composer-tags.js. */

/** A value built at most once per task: a render reads it for every turn it draws. */
function perTask(build) {
  let value = null;
  return () => {
    if (value === null) {
      value = build();
      queueMicrotask(() => (value = null));
    }
    return value;
  };
}

/** The tag catalog the chat routes and paints sent messages with, as the composer reads it. */
const laneCatalog = perTask(() => composerTagCatalog());

/** When each project last reported to the orchestrator, by slug, read off the orchestrator's thread. */
function lastReports() {
  const global = tabs.find(isGlobalOrchestratorTab);
  const at = new Map();
  for (const item of global ? threadOf(global).items : []) {
    if (item.kind === "user") for (const block of reportBlocks(item.text)) at.set(tagSlug(block.project), item.at);
  }
  return at;
}

/** Every project as the rail and the report cards read it, in the sidebar's order. A project on
 * several machines is read on the one whose Captain works, else has a Captain, else is online.
 * `state` is working | idle | none | offline (that machine is away). `working` counts the
 * project's threads at work other than its Captains, `needs` holds the keys of those that wait on
 * the user, newest first, and `workers` the running workers on that machine. */
const captainFacts = perTask(() => {
  const reports = lastReports();
  const workers = runningWorkers();
  const online = (id) => machines.get(id)?.view.online === true;
  const busy = (tab) => !["done", "idle", "stopped"].includes(phaseOf(tab));
  const facts = [];
  for (const group of projectGroups(true)) {
    if (group.tabs.length && !group.tabs.some(hasProject)) continue;
    const name = groupName(group);
    const slug = tagSlug(name);
    const hosts = [...new Set(group.checkouts.map((c) => c.machine))];
    const captainOn = (id) => group.tabs.find((t) => t.machine === id && t.view.role === "orchestrator");
    const id = hosts.find((h) => captainOn(h) && busy(captainOn(h))) ?? hosts.find(captainOn) ?? hosts.find(online) ?? hosts[0];
    if (!id) continue;
    const captain = captainOn(id) ?? null;
    const checkouts = group.checkouts.filter((c) => c.machine === id);
    const state = !online(id) ? "offline" : !captain ? "none" : busy(captain) ? "working" : "idle";
    const needs = group.tabs.filter((t) => threadState(t) === "needs").sort((a, b) => (b.view.updated ?? 0) - (a.view.updated ?? 0));
    const reported = reports.get(slug);
    facts.push({
      slug,
      name,
      machine: { id, name: machineName(id) },
      path: (checkouts.find((c) => c.path.split(/[\\/]/).pop() === name) ?? checkouts[0])?.path ?? null,
      captain: captain?.key ?? null,
      state,
      workers: workers.get(slug)?.get(id) ?? 0,
      working: group.tabs.filter((t) => t.view.role !== "orchestrator" && threadState(t) === "working").length,
      needs: needs.map((t) => t.key),
      reported: reported ? dayAware(reported) : null,
      seen: state === "offline" ? lastSeen(machines.get(id)?.view.last_seen) : null,
    });
  }
  return facts;
});

/** A project's two-letter tile, in the color the sidebar's monogram gives the same name. */
function captainMonoHtml(name) {
  let hash = 0;
  for (const c of name.normalize("NFKC").trim().toLowerCase() || "project") hash = (hash * 31 + c.codePointAt(0)) % MONOGRAM_COLORS.length;
  return `<span class="captain-mono" style="--mono: ${MONOGRAM_COLORS[hash]}" aria-hidden="true">${esc(monogram(name))}</span>`;
}

/** What a report card shows of its Captain beside the report: the tile, the state after the machine
 * or as chips, and Review when one of the project's threads waits on the user. */
function reportCaptainHtml(report) {
  const mono = captainMonoHtml(report.project);
  const fact = captainFacts().find((f) => f.slug === tagSlug(report.project));
  if (!fact) return { mono, where: "", chips: "", review: "" };
  const { chips, where } = reportCaptainState(fact);
  return {
    mono,
    where,
    chips: chips.map((c) => `<span class="captain-count ${c.tone}">${esc(c.text)}</span>`).join(""),
    review: fact.needs.length ? `<button type="button" class="review" data-review-thread="${esc(fact.needs[0])}">Review</button>` : "",
  };
}

/** The tags of a sent message painted as the composer painted them. */
function orchestratorTagsHtml(html) {
  const catalog = laneCatalog();
  return tagsInHtml(html, { project: new Set(catalog.projects.map((p) => p.slug)), machine: new Set(catalog.machines.map((m) => m.slug)) });
}

/** "Routed to Captain ..." under a sent message. Nothing records where the orchestrator sent it,
 * so it reads the message's tags against today's catalog, and drops out once a tag stops resolving. */
function orchestratorRoutedHtml(text) {
  const line = routedLine(text, laneCatalog());
  return line ? `<p class="routed">${esc(line)}</p>` : "";
}

/** Everything the orchestrator's chat draws from outside its own thread, for renderThread's key. */
const globalLaneKey = () =>
  JSON.stringify([globalSettings.group_reports, laneCatalog().projects, captainFacts().map((f) => [f.slug, f.state, f.working, f.needs.length, f.needs[0] ?? null])]);

threadView.addEventListener("click", (e) => {
  const review = tabByKey(e.target.closest("[data-review-thread]")?.dataset.reviewThread);
  if (review) activate(review);
});

/** The Captains rail: a row per project with its Captain's state, opening that Captain, or starting
 * one where the project has none. */
function railHtml(facts) {
  const rows = facts.map((f) => {
    const start = f.state === "none" && f.path;
    const label = f.captain ? `Open the Captain of ${f.name}` : start ? `Start a Captain for ${f.name} on ${f.machine.name}` : `${f.machine.name} is offline`;
    const data = [`data-captain-project="${esc(f.slug)}"`, `data-captain-machine="${esc(f.machine.id)}"`, f.captain && `data-captain-tab="${esc(f.captain)}"`, start && `data-captain-path="${esc(f.path)}"`].filter(Boolean).join(" ");
    return `<button type="button" class="captain-row" data-state="${f.state}" ${data} title="${esc(label)}"${f.captain || start ? "" : " disabled"}>${captainMonoHtml(f.name)}<span class="text">
      <span class="top"><span class="name">${esc(f.name)}</span><span class="state">${CAPTAIN_STATE_LABELS[f.state]}</span>${start ? `<span class="start">Start one</span>` : ""}</span>
      <span class="sub">${esc(captainRailSub(f))}</span></span></button>`;
  });
  return `<div class="rail-head"><h2>Captains</h2><button type="button" class="all" data-rail-projects>All projects</button></div>${rows.join("") || `<p class="rail-empty">No projects on the account's machines yet.</p>`}`;
}

const chatEl = document.getElementById("chat");
const captainsRail = Object.assign(document.createElement("aside"), { id: "captains-rail", hidden: true });
captainsRail.setAttribute("aria-label", "Captains");
document.getElementById("chat-head").after(captainsRail);

/** The board's rail width and the lane it leaves the chat; below that lane the rail steps aside. */
const RAIL_WIDTH = 296;
const RAIL_LANE = 680;
function renderRail() {
  const thread = getComputedStyle(threadView);
  const lane = chatEl.clientWidth - RAIL_WIDTH - parseFloat(thread.paddingLeft) - parseFloat(thread.paddingRight);
  const on = guiOn() && !!active && isGlobalOrchestratorTab(active) && !narrow(768) && lane >= RAIL_LANE;
  chatEl.toggleAttribute("data-rail", on);
  captainsRail.hidden = !on;
  if (on) setHtml(captainsRail, railHtml(captainFacts()));
}
onModelChange(renderRail);
new ResizeObserver(renderRail).observe(chatEl);
visibleInterval(renderRail, 30000);

captainsRail.addEventListener("click", (e) => {
  if (e.target.closest("[data-rail-projects]")) return openGlobalProjects();
  const row = e.target.closest(".captain-row");
  if (!row || row.disabled) return;
  const captain = tabByKey(row.dataset.captainTab);
  if (captain) return activate(captain);
  if (row.dataset.captainPath) openOrchestrator(row.dataset.captainMachine, row.dataset.captainPath);
});
