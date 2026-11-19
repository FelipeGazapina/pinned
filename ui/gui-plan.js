// #349 unit 5: the Plan surface, a Captain thread's plans in the right panel (plan.rs). gui-panel.js owns the
// panel frame, the add menu entry and the render dispatch; this file draws the surface, keeps its data current
// and owns its clicks. Loaded after gui-surfaces.js.

/** The zoom levels: the cell width and row height of one main step, in px. */
const PLAN_ZOOMS = [
  { name: "Dots", cell: 24, row: 30 },
  { name: "Steps", cell: 92, row: 52 },
  { name: "Details", cell: 132, row: 74 },
];

/** The words a step and a Captain's plan show. Every surface that names a state reads them from here. */
const PLAN_STEP_WORD = { done: "Done", running: "Running now", failed: "Failed", waiting: "Waiting for your go", pending: "Not started" };
const PLAN_CARD_WORD = { needs_you: "Needs you", fixing: "Fixing", working: "Working", idle: "Idle", finished: "Finished", closed: "Closed" };
const PLAN_ORDER = { needs_you: 0, fixing: 1, working: 2, idle: 3, finished: 4, closed: 5 };

const planAfterFix = (step) => step.state === "done" && /^after /.test(step.why ?? "");
const planStepWord = (step) => (planAfterFix(step) ? "Done, after one fix" : PLAN_STEP_WORD[step.state]);
const planMains = (plan) => plan.lists.flatMap((list) => list.steps);
/** Every step the board counts: the main steps and the steps of their fix branches. */
const planAllSteps = (plan) => planMains(plan).flatMap((step) => [step, ...step.fixes.flatMap((f) => f.steps)]);
/** `scope` prefixes every id the surface keeps (fold, picked step, fix branch): "" for a Captain's one plan, a plan's
 * `machine|tab|plan|` where one panel draws several plans whose list and step ids repeat. */
const planFixOpen = (s, step, scope = "") => s.fixOpen?.[scope + step.id] ?? step.state === "failed";
/** Who runs a step: the worker's name when the hub gives one, else Worker for a delegated step, else the Captain. */
const planRunBy = (step) => step.by ?? (step.task ? "Worker" : "Captain");
const planCost = (cost) => (cost == null ? "" : `$${cost.toFixed(2)}`);
const planClock = (ms) => new Date(ms).toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });

/** A list's tone and word, from its steps. */
function planListWord(list) {
  if (!list.steps.length) return ["pending", "Not started"];
  const states = list.steps.map((s) => s.state);
  if (states.every((x) => x === "done")) return list.steps.some(planAfterFix) ? ["done", "Done, after one fix"] : ["done", "Done"];
  if (states.includes("failed")) return ["failed", list.steps.some((s) => s.state === "failed" && s.fixes.length) ? "Failed, fix in progress" : "Failed"];
  if (states.includes("waiting")) return ["waiting", "Waiting for your go"];
  if (states.includes("running")) return ["running", "Running now"];
  if (states.includes("done")) return ["running", "Between steps"];
  return ["pending", "Not started"];
}

/** The step the footer shows when none is picked: the first waiting one, else a running fix, else the first running. */
function planLiveStep(plan) {
  for (const want of ["waiting", "failed", "running"]) {
    for (const step of planMains(plan)) {
      if (step.state !== want) continue;
      if (want !== "failed") return step;
      return step.fixes.flatMap((f) => f.steps).find((x) => x.state === "running") ?? step;
    }
  }
  return null;
}

function planFindStep(plan, id) {
  for (const list of plan.lists) {
    for (const step of list.steps) {
      if (step.id === id) return { step, list, under: null, fix: null };
      for (const fix of step.fixes) {
        const hit = fix.steps.find((x) => x.id === id);
        if (hit) return { step: hit, list, under: step, fix };
      }
    }
  }
  return null;
}

/** Cells for one list: the x of every main step, and where each open fix branch sits. */
function planLayout(list, s, Z, scope = "") {
  const pad = 16;
  let x = pad + Z.cell / 2;
  const xs = [];
  const fixes = [];
  for (const step of list.steps) {
    xs.push(x);
    const fsteps = step.fixes.flatMap((f) => f.steps);
    let gap = Z.cell;
    if (fsteps.length && planFixOpen(s, step, scope)) {
      fixes.push({ at: x, step, steps: fsteps, xs: fsteps.map((_, k) => x + (k + 0.6) * Z.cell) });
      gap = Math.max(Z.cell, (fsteps.length + 0.5) * Z.cell);
    }
    x += gap;
  }
  const last = xs.at(-1) ?? pad + Z.cell / 2;
  return { xs, fixes, last, width: last + Z.cell / 2 + pad };
}

// ---- data: the glance from every tabs push, the full plans fetched when the glance moves ----

const planCache = new Map(); // tab key -> { sig, plans, loading, dirty, failed }

/** The part of the glance the surface draws: a push that leaves it alone sends no call. */
const planSig = (t) => JSON.stringify([t.view.session ?? null, (t.view.plans ?? []).map((g) => [g.id, g.state, g.done, g.total, g.strips, g.finished_at ?? null])]);
const isPlanCaptain = (tab) => !!tab && tab.view.role === "orchestrator" && !tab.view.orchestrator_global;
const planGlances = (t) => [...(t.view.plans ?? [])].sort((a, b) => PLAN_ORDER[a.state] - PLAN_ORDER[b.state]);

/** One fetch of `call` into cache entry `c` for the signature `sig`. The call for a new signature starts its fetch; one in
 * flight per entry, and a signature that moves while it loads fetches once more after it. */
function planFetch(c, sig, machine, call, apply) {
  if (c.sig === sig) return c;
  if (c.loading) {
    c.dirty = true;
    return c;
  }
  c.sig = sig;
  c.loading = true;
  c.dirty = false;
  c.failed = false;
  callOn(machine, call).then(
    (reply) => {
      c.loading = false;
      apply(c, reply);
      planSettled(c);
    },
    () => {
      c.loading = false;
      c.failed = true;
      planSettled(c);
    },
  );
  return c;
}

/** The Captain's full plans for the glance it has now. */
function planRefresh(t) {
  const c = planCache.get(t.key) ?? { sig: null, plans: null, loading: false, dirty: false, failed: false };
  planCache.set(t.key, c);
  return planFetch(c, planSig(t), t.machine, { cmd: "plan", tab: t.view.id }, (c, plans) => (c.plans = plans ?? []));
}
function planSettled(c) {
  if (c.dirty) {
    c.dirty = false;
    c.sig = null;
  }
  planRerender();
}
/** The plan the surface shows: the picked one, else the most urgent plan the hub has sent. */
function planPicked(t, s) {
  const plans = planCache.get(t.key)?.plans ?? [];
  return plans.find((x) => x.id === s.plan) ?? planGlances(t).map((g) => plans.find((x) => x.id === g.id)).find(Boolean) ?? null;
}
function planRerender() {
  if (shownSurface(active)?.kind === "plan") renderPanel();
}

/** A resume (`carry_plans`) drops a tab's old plans without a word on the wire. The surface keeps the last glance it
 * drew per thread, so a session change that takes an unfinished plan away shows one line until it is dismissed.
 * A resumed session numbers its plans from p1 again, so a plan counts as the same only with its id and title. */
function planDetectDropped(t) {
  const p = panelOf(t);
  const session = t.view.session ?? null;
  const unfinished = (t.view.plans ?? []).filter((g) => !g.finished_at);
  const seen = p.seen;
  let changed = false;
  if (seen && seen.session !== session) {
    const gone = seen.plans.filter((g) => !unfinished.some((u) => u.id === g.id && u.title === g.title));
    if (gone.length) {
      p.dropped = gone.map((g) => g.title);
      changed = true;
    }
  }
  const next = { session, plans: unfinished.map((g) => ({ id: g.id, title: g.title })) };
  if (JSON.stringify(next) !== JSON.stringify(seen)) {
    p.seen = next;
    changed = true;
  }
  if (changed) savePanel(t);
}

/** A new plan id opens the surface once, on the viewed Captain, with its panel closed. The opened ids belong to one
 * session: a resume restarts plan ids at p1, so a session change starts the list over and only that session's ids stay. */
function planAutoOpen(t, session = t.view.session ?? null, plans = t.view.plans ?? []) {
  const p = panelOf(t);
  const opened = p.autoOpened?.session === session ? p.autoOpened.ids : [];
  const fresh = plans.filter((g) => !g.finished_at && !opened.includes(g.id));
  const next = { session, ids: [...opened, ...fresh.map((g) => g.id)] };
  if (JSON.stringify(next) !== JSON.stringify(p.autoOpened)) {
    p.autoOpened = next;
    savePanel(t);
  }
  if (!fresh.length || p.open) return;
  addSurface(t, p.surfaces.find((x) => x.kind === "plan") ?? { id: "plan", kind: "plan", title: "Plan" });
}

/** Every tabs push: auto-open for a new plan, and a fetch when the shown Captain's glance moved. */
function onPlanPush() {
  const t = active;
  if (!t || !guiOn()) return;
  if (isRequestThread(t)) {
    const glance = threadRequestGlance(t);
    if (!glance) return;
    planAutoOpen(t, glance.id, threadRequestPlans(t).map((e) => ({ id: planScope(e), finished_at: e.plan.finished_at })));
    if (shownSurface(t)?.kind === "plan") requestRefresh(t);
    return;
  }
  if (!isPlanCaptain(t)) return;
  planAutoOpen(t);
  if (shownSurface(t)?.kind === "plan") planRefresh(t);
}


// ---- a request thread: every plan of the request in one tree, a section per owner ----

const isRequestThread = (tab) => !!tab?.view?.request;
/** A Captain's Plan, or a request thread's. */
const hasPlanSurface = (tab) => isPlanCaptain(tab) || isRequestThread(tab);
/** The ids of one plan in a request's panel: (machine, tab, plan), the key of its fetch and the prefix of every list and step id. */
const planScope = (e) => `${e.machine}|${e.tab.id}|${e.plan.id}|`;
const planBase = (path) => String(path ?? "").split(/[\\/]/).filter(Boolean).pop() ?? "";

const requestPlanCache = new Map(); // planScope -> { sig, view, loading, dirty, failed }
const requestSnapCache = new Map(); // machine|request id -> { sig, plans, loading, dirty, failed }

const planGlanceSig = (g) => JSON.stringify([g.id, g.state, g.done, g.total, g.strips, g.finished_at ?? null]);

/** The full view of every plan the request names, and its snapshots. A plan fetches when its own glance moves; the
 * snapshots (`user_request`) when the set of plans changes. An old host that fails the call has none. */
function requestRefresh(t) {
  const glance = threadRequestGlance(t);
  if (!glance) return;
  const entries = threadRequestPlans(t);
  for (const e of entries) {
    const scope = planScope(e);
    const c = requestPlanCache.get(scope) ?? { sig: null, view: null, loading: false, dirty: false, failed: false };
    requestPlanCache.set(scope, c);
    planFetch(c, planGlanceSig(e.plan), e.machine, { cmd: "plan", tab: e.tab.id, plan: e.plan.id }, (c, plans) => (c.view = plans?.[0] ?? null));
  }
  const key = `${t.machine}|${glance.id}`;
  const c = requestSnapCache.get(key) ?? { sig: null, plans: [], loading: false, dirty: false, failed: false };
  requestSnapCache.set(key, c);
  planFetch(c, JSON.stringify([glance.closed_at ?? null, entries.map(planScope)]), t.machine, { cmd: "user_request", id: glance.id }, (c, reply) => (c.plans = reply?.plans ?? []));
}

/** The sections of a request's Plan: its own lanes under "Orchestrator", then one section per Captain of
 * `glance.captains`, in that order, and a section for a plan on a tab whose machine and path are none of them. A snapshot (a plan whose
 * Captain tab is gone) joins the section of the Captain with its machine and exact path, else the last section "Ended plans".
 * A block is { scope, glance, cache, view, ended }; `view` is null until the plan's fetch lands. */
function requestPlanSections(glance, entries, snapshots) {
  const own = [];
  const captains = glance.captains.map(() => []);
  const others = new Map();
  for (const e of entries) {
    const block = { scope: planScope(e), glance: e.plan, cache: requestPlanCache.get(planScope(e)), view: requestPlanCache.get(planScope(e))?.view ?? null, ended: false, owner: { machine: e.machine, tab: e.tab.id } };
    const at = glance.captains.findIndex((c) => c.machine === e.machine && c.path === e.tab.cwd);
    if (e.tab.request === glance.id) own.push(block);
    else if (at >= 0) captains[at].push(block);
    else {
      const key = `${e.machine}|${e.tab.cwd}`;
      if (!others.has(key)) others.set(key, { title: `Other Captain · ${planBase(e.tab.cwd)}`, blocks: [] });
      others.get(key).blocks.push(block);
    }
  }
  const live = new Set(entries.map((e) => `${e.plan.id}\n${e.plan.title}`));
  const stray = [];
  for (const snap of snapshots) {
    if (live.has(`${snap.plan.id}\n${snap.plan.title}`)) continue;
    const block = { scope: `ended|${glance.id}|${snap.machine}|${snap.path}|${snap.plan.id}|`, glance: snap.plan, cache: null, view: snap.plan, ended: true, owner: null };
    const at = glance.captains.findIndex((c) => c.machine === snap.machine && c.path === snap.path);
    (at >= 0 ? captains[at] : stray).push(block);
  }
  const sections = [];
  if (own.length) sections.push({ title: "Orchestrator", blocks: own });
  sections.push(...glance.captains.map((c, i) => ({ title: `Captain ${c.project} · ${c.machine_name}`, blocks: captains[i] })), ...others.values());
  if (stray.length) sections.push({ title: "Ended plans", blocks: stray });
  return sections;
}

/** The sections the panel draws now, from the glances and whatever the fetches have brought. */
function requestPanelModel(t) {
  const glance = threadRequestGlance(t);
  if (!glance) return null;
  const snaps = requestSnapCache.get(`${t.machine}|${glance.id}`)?.plans ?? [];
  return { glance, sections: requestPlanSections(glance, threadRequestPlans(t), snaps) };
}

// ---- render ----

function planDroppedHtml(p) {
  if (!p.dropped?.length) return "";
  const text = p.dropped.length === 1 ? `1 plan from before /resume was not carried over: ${esc(p.dropped[0])}.` : `${p.dropped.length} plans from before /resume were not carried over: ${p.dropped.map((x) => esc(x)).join(", ")}.`;
  return `<div class="pl-dropped" role="status">${icon("history")}<span>${text}</span><button type="button" class="icon" data-dismiss-dropped title="Dismiss">${icon("x")}</button></div>`;
}

function planZoomHtml(zoom) {
  return `<span class="pl-zoom" role="group" aria-label="Zoom"><button type="button" class="icon" data-zoom="-1" title="Zoom out" ${zoom === 0 ? "disabled" : ""}>${icon("minus")}</button><span class="pl-zoom-name">${PLAN_ZOOMS[zoom].name}</span><button type="button" class="icon" data-zoom="1" title="Zoom in" ${zoom === 2 ? "disabled" : ""}>${icon("plus")}</button></span>`;
}
const planProgressHtml = (done, total) => `<div class="pl-progress"><span style="width:${total ? (done / total) * 100 : 0}%"></span></div>`;

function planChipsHtml(glances, plan) {
  return `<div class="pl-chips" role="tablist">${glances
    .map((g) => `<button type="button" role="tab" class="pl-chip" data-plan="${esc(g.id)}" data-cstate="${g.state}" aria-selected="${g.id === plan.id}" title="${esc(g.title)} · ${PLAN_CARD_WORD[g.state]}"><i class="pl-cdot"></i><span class="t">${esc(g.title)}</span><span class="n">${g.finished_at ? `Finished ${planClock(g.finished_at)}` : `${g.done}/${g.total}`}</span></button>`)
    .join("")}</div>`;
}

function planStepHtml(step, x, y, Z, zoom, s, scope = "") {
  const picked = s.step === scope + step.id ? " picked" : "";
  const meta = zoom === 2 ? `<span class="pl-meta">${esc(planCost(step.cost) || (step.state === "pending" && !step.task ? "" : planRunBy(step)))}</span>` : "";
  const name = zoom ? `<span class="pl-name">${esc(step.name)}</span>` : "";
  return `<button type="button" class="pl-step${picked}" data-step="${esc(scope + step.id)}" data-state="${step.state}"${planAfterFix(step) ? ' data-fixed=""' : ""} style="left:${x - Z.cell / 2}px;top:${y - 8}px;width:${Z.cell}px" title="${esc(step.name)} · ${esc(planStepWord(step))}"><span class="pl-dot"></span>${name}${meta}</button>`;
}

function planListHtml(list, s, zoom, scope = "") {
  const Z = PLAN_ZOOMS[zoom];
  const folded = (s.folded ?? []).includes(scope + list.id);
  const [tone, word] = planListWord(list);
  const fixSteps = list.steps.filter((x) => x.fixes.length);
  const fixOn = fixSteps.some((x) => planFixOpen(s, x, scope));
  const fixCount = fixSteps.reduce((n, x) => n + x.fixes.length, 0);
  const chip = fixSteps.length
    ? `<button type="button" class="pl-fixchip" data-fixchip="${esc(scope + list.id)}" aria-pressed="${fixOn}" title="${fixOn ? "Hide" : "Show"} the fix branch">${icon("git-branch")}${fixCount} ${fixCount === 1 ? "fix" : "fixes"}</button>`
    : "";
  const bar = `<span class="pl-bar">${list.steps.map((x) => `<i data-state="${x.state}"></i>`).join("")}</span>`;
  const head = `<div class="pl-list-head"><button type="button" class="pl-fold" data-fold="${esc(scope + list.id)}" aria-expanded="${!folded}">${icon(folded ? "chevron-right" : "chevron-down")}<span class="pl-list-name">${esc(list.name)}</span></button>${bar}<span class="pl-word" data-tone="${tone}">${word}</span>${chip}</div>`;
  if (folded) return `<section class="pl-list folded">${head}</section>`;
  const { xs, fixes, last, width } = planLayout(list, s, Z, scope);
  const dotY = zoom === 0 ? Z.row / 2 : 12;
  const height = Z.row + (fixes.length ? Z.row : 0);
  let doneTo = -1;
  while (doneTo + 1 < list.steps.length && list.steps[doneTo + 1].state === "done") doneTo++;
  let svg = `<line class="pl-base" x1="0" y1="${dotY}" x2="${last}" y2="${dotY}"/>`;
  if (doneTo >= 0) svg += `<line class="pl-fill" x1="0" y1="${dotY}" x2="${xs[doneTo]}" y2="${dotY}"/>`;
  let steps = "";
  for (const f of fixes) {
    const fy = Z.row + dotY;
    const r = 7;
    const rx = f.xs.at(-1) + Z.cell * 0.5;
    const healed = f.steps.every((x) => x.state === "done");
    svg += `<path class="pl-u${healed ? " healed" : ""}" d="M${f.at},${dotY} V${fy - r} Q${f.at},${fy} ${f.at + r},${fy} H${rx - r} Q${rx},${fy} ${rx},${fy - r} V${dotY}"/>`;
    f.steps.forEach((st, k) => (steps += planStepHtml(st, f.xs[k], fy, Z, zoom, s, scope)));
  }
  list.steps.forEach((st, i) => (steps += planStepHtml(st, xs[i], dotY, Z, zoom, s, scope)));
  const track = `<div class="pl-track" data-z="${zoom}" style="height:${height}px"><div class="pl-canvas" style="width:${width}px;height:${height}px"><svg width="${width}" height="${height}" aria-hidden="true">${svg}</svg>${steps}</div></div>`;
  return `<section class="pl-list${fixes.length ? " has-fix" : ""}">${head}${track}</section>`;
}

function planFootHtml(plan, s, owner = null) {
  const live = planLiveStep(plan);
  const hit = (s.step && planFindStep(plan, s.step)) || (live && planFindStep(plan, live.id));
  if (!hit) return "";
  const { step, list, under, fix } = hit;
  const why = step.why && !planAfterFix(step) ? `<p class="pl-why">${esc(step.why)}</p>` : "";
  const branch = under ? `Fix ${esc(fix.id)}, under “${esc(under.name)}”` : "Main line";
  const diff = step.task && step.branch && owner ? ` <button type="button" class="outline pl-open" data-see-diff="${esc(step.task)}" data-diff-machine="${esc(owner.machine)}" data-diff-tab="${esc(owner.tab)}">${icon("git-branch")}See the diff</button>` : "";
  return `<div class="pl-foot" data-state="${step.state}"><p class="pl-foot-state"><span class="pl-dot"></span>${s.step ? planStepWord(step) : `Live now · ${planStepWord(step)}`}</p>
      <p class="pl-foot-name">${esc(step.name)}</p>${step.what ? `<p class="pl-foot-what">${esc(step.what)}</p>` : ""}${why}
      <dl class="pl-facts"><dt>List</dt><dd>${esc(list.name)}</dd><dt>Run by</dt><dd>${esc(planRunBy(step))}</dd><dt>Branch</dt><dd>${branch}</dd><dt>Cost</dt><dd>${esc(planCost(step.cost)) || "None yet"}</dd></dl>
      ${step.task ? `<button type="button" class="outline pl-open" data-open-task="${esc(step.task)}">${icon("arrow-up-right")}Open thread</button>${diff}` : ""}</div>`;
}

/** The body of the surface for the active Captain: dropped line, switcher, header, the plan's lists, footer. */
function planBodyHtml(t, s) {
  const p = panelOf(t);
  const dropped = planDroppedHtml(p);
  const glances = planGlances(t);
  if (!glances.length) return `${dropped}<div class="pl-empty"><p class="t">The Captain is writing its plan</p><p>Each part of the work shows up here as a list of steps.</p></div>`;
  const c = planRefresh(t);
  if (!c.plans) return `${dropped}<p class="pl-loading">${c.failed ? "Could not load the plan." : "Loading the plan…"}</p>`;
  const plan = planPicked(t, s);
  if (!plan) return `${dropped}<div class="pl-empty"><p class="t">The Captain is writing its plan</p><p>Each part of the work shows up here as a list of steps.</p></div>`;
  const zoom = s.zoom ?? 1;
  const chips = glances.length > 1 ? planChipsHtml(glances, plan) : "";
  const ms = planAllSteps(plan);
  const done = ms.filter((x) => x.state === "done").length;
  const head = `<div class="pl-head"><span class="pl-count">${done} of ${ms.length} done</span>${planZoomHtml(zoom)}</div>${planProgressHtml(done, ms.length)}`;
  const at = plan.closed ? `Closed: ${esc(plan.closed)}` : plan.finished_at ? `Finished at ${planClock(plan.finished_at)}` : `Plan, from your message at ${planClock(plan.asked_at)}`;
  const root = `<p class="pl-root">${icon("message-circle")}<span><b>${esc(plan.title)}</b><span class="pl-root-at">${at}</span></span></p>`;
  const lists = plan.lists.map((l) => planListHtml(l, s, zoom)).join("");
  return `${dropped}${chips}${head}<div class="pl-scroll">${root}<div class="pl-lists">${lists}</div></div>${planFootHtml(plan, s, { machine: t.machine, tab: t.view.id })}`;
}


const planPlural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** The footer of a request's panel: the picked step of whichever plan holds it, else the plan's live step, first plan first. */
function requestFootHtml(blocks, s) {
  const hold = s.step && blocks.find((b) => b.view && s.step.startsWith(b.scope) && planFindStep(b.view, s.step.slice(b.scope.length)));
  if (hold) return planFootHtml(hold.view, { ...s, step: s.step.slice(hold.scope.length) }, hold.owner);
  const live = blocks.find((b) => b.view && planLiveStep(b.view));
  return live ? planFootHtml(live.view, { ...s, step: null }, live.owner) : "";
}

function requestBlockHtml(b, s, zoom, many) {
  const title = b.ended || many ? `<p class="pl-plan"><b>${esc(b.view?.title ?? b.glance.title)}</b>${b.ended ? ` <span class="pl-ended">ended</span>` : ""}</p>` : "";
  if (!b.view) return `${title}<p class="pl-loading">${b.cache?.failed ? "Could not load the plan." : "Loading the plan…"}</p>`;
  return `${title}<div class="pl-lists">${b.view.lists.map((l) => planListHtml(l, s, zoom, b.scope)).join("")}</div>`;
}

/** The body of a request thread's surface: header with the zoom, one root, then a section per owner. */
function requestPlanHtml(t, s) {
  requestRefresh(t);
  const model = requestPanelModel(t);
  const none = `<div class="pl-empty"><p class="t">No plan yet</p><p>Each part of the work shows up here as a list of steps.</p></div>`;
  if (!model) return none;
  const blocks = model.sections.flatMap((x) => x.blocks);
  if (!blocks.length) return none;
  const zoom = s.zoom ?? 1;
  const views = blocks.map((b) => b.view).filter(Boolean);
  const lists = views.reduce((n, v) => n + v.lists.length, 0);
  const steps = views.flatMap(planAllSteps);
  const done = steps.filter((x) => x.state === "done").length;
  const fixes = views.flatMap(planMains).reduce((n, x) => n + x.fixes.length, 0);
  const summary = [planPlural(lists, "list"), `${done} of ${planPlural(steps.length, "step")}`, fixes ? planPlural(fixes, "fix branch").replace("branchs", "branches") : ""].filter(Boolean).join(", ");
  const head = `<div class="pl-head"><span class="pl-title">Plan</span><span class="pl-count">${esc(summary)}</span>${planZoomHtml(zoom)}</div>${planProgressHtml(done, steps.length)}`;
  const root = `<p class="pl-root-row">Plan, from your request at ${planClock(model.glance.opened_at)}</p>`;
  const body = model.sections
    .map((sec) => `<section class="pl-sec-group"><h3 class="pl-sec">${esc(sec.title)}</h3>${sec.blocks.map((b) => requestBlockHtml(b, s, zoom, sec.blocks.length > 1)).join("") || `<p class="pl-loading">No plan from this Captain yet.</p>`}</section>`)
    .join("");
  return `${head}<div class="pl-scroll">${root}${body}</div>${requestFootHtml(blocks, s)}`;
}

function renderPlanPanel(s) {
  const t = active;
  if (isRequestThread(t)) return showPanel(`<div class="pl" data-view="plan">${requestPlanHtml(t, s)}</div>`);
  if (!isPlanCaptain(t)) return showPanel(`<div class="pl"><div class="pl-empty"><p class="t">No plan here</p><p>Only a project's Captain keeps a plan.</p></div></div>`);
  planDetectDropped(t);
  showPanel(`<div class="pl" data-view="plan">${planBodyHtml(t, s)}</div>`);
}

// ---- clicks ----

function onPlanClick(e) {
  const t = active;
  const s = t && shownSurface(t);
  if (!s || s.kind !== "plan" || !e.target.closest(".pl")) return;
  const q = (sel) => e.target.closest(sel);
  let el;
  if ((el = q("[data-zoom]"))) s.zoom = Math.max(0, Math.min(2, (s.zoom ?? 1) + Number(el.dataset.zoom)));
  else if ((el = q("[data-fold]"))) {
    const id = el.dataset.fold;
    s.folded = (s.folded ?? []).includes(id) ? s.folded.filter((x) => x !== id) : [...(s.folded ?? []), id];
  } else if ((el = q("[data-fixchip]"))) {
    const id = el.dataset.fixchip;
    const owner = isRequestThread(t) ? requestPanelModel(t)?.sections.flatMap((x) => x.blocks).find((b) => b.view && id.startsWith(b.scope)) : null;
    const scope = owner?.scope ?? "";
    const list = (owner?.view ?? planPicked(t, s))?.lists.find((l) => scope + l.id === id);
    const steps = (list?.steps ?? []).filter((x) => x.fixes.length);
    const on = !steps.some((x) => planFixOpen(s, x, scope));
    s.fixOpen = { ...(s.fixOpen ?? {}) };
    for (const step of steps) s.fixOpen[scope + step.id] = on;
  } else if ((el = q("[data-step]"))) s.step = s.step === el.dataset.step ? null : el.dataset.step;
  else if ((el = q("[data-plan]"))) {
    s.plan = el.dataset.plan;
    s.step = null;
  } else if (q("[data-dismiss-dropped]")) panelOf(t).dropped = null;
  else if ((el = q("[data-open-task]"))) return openDelegated(el.dataset.openTask);
  else if ((el = q("[data-see-diff]"))) return planSeeDiff(el.dataset.diffMachine, el.dataset.diffTab, el.dataset.seeDiff);
  else return;
  savePanel(t);
  planRerender();
}

/** The worker's own diff surface, on its branch against its base, in front. `open_task` finds or opens the worker's tab,
 * the same call Open thread makes; it runs on this click and never while drawing. */
async function planSeeDiff(machine, tab, task) {
  try {
    const created = await callOn(machine, { cmd: "open_task", tab, task });
    await refreshTabs(machine);
    const worker = tabByKey(keyOf(machine, created));
    if (!worker) return;
    setDiffState(worker, { scope: "base", base: null });
    addSurface(worker, { id: "diff", kind: "diff" });
    activate(worker);
    await startTab(worker);
  } catch (e) {
    showComposerNote(String(e), true);
  }
}

panelBody.addEventListener("click", onPlanClick);
onModelChange(onPlanPush);
