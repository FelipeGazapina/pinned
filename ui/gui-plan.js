// #349 units 5-6: the Plan surface, a Captain thread's plans in the right panel (plan.rs), and the Captains surface
// on the Orchestrator thread, one card per Captain. gui-panel.js owns the panel frame, the add menu entry and the
// render dispatch; this file draws the surfaces, keeps their data current and owns their clicks. Loaded after
// gui-surfaces.js.

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
/** The states the Captains header counts, in its order. */
const CAPTAIN_COUNTED = ["needs_you", "fixing", "working", "idle"];
/** What each letter of a glance's strips means: `x` is a done step that took a fix. */
const PLAN_DOT_STATE = { d: "done", x: "done", r: "running", f: "failed", w: "waiting", p: "pending" };

const planAfterFix = (step) => step.state === "done" && /^after /.test(step.why ?? "");
const planStepWord = (step) => (planAfterFix(step) ? "Done, after one fix" : PLAN_STEP_WORD[step.state]);
const planMains = (plan) => plan.lists.flatMap((list) => list.steps);
const planFixOpen = (s, step) => s.fixOpen?.[step.id] ?? step.state === "failed";
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
function planLayout(list, s, Z) {
  const pad = 16;
  let x = pad + Z.cell / 2;
  const xs = [];
  const fixes = [];
  for (const step of list.steps) {
    xs.push(x);
    const fsteps = step.fixes.flatMap((f) => f.steps);
    let gap = Z.cell;
    if (fsteps.length && planFixOpen(s, step)) {
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
/** The Orchestrator's own thread, where the Captains surface shows. */
const isGlobalThread = (tab) => tab?.view?.orchestrator_global === true;
const planGlances = (t) => [...(t.view.plans ?? [])].sort((a, b) => PLAN_ORDER[a.state] - PLAN_ORDER[b.state]);

/** The Captain's full plans for the glance it has now. The call for a new glance starts its fetch; one in flight
 * per tab, and a glance that moves while it loads fetches once more after it. */
function planRefresh(t) {
  const c = planCache.get(t.key) ?? { sig: null, plans: null, loading: false, dirty: false, failed: false };
  planCache.set(t.key, c);
  const sig = planSig(t);
  if (c.sig === sig) return c;
  if (c.loading) {
    c.dirty = true;
    return c;
  }
  c.sig = sig;
  c.loading = true;
  c.dirty = false;
  c.failed = false;
  callOn(t.machine, { cmd: "plan", tab: t.view.id }).then(
    (plans) => {
      c.loading = false;
      c.plans = plans ?? [];
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

/** The fold, the picked plan and the picked step belong to one Captain session: a session change (a /resume) or a
 * different Captain starts them over, so an old pick never lands on the new session's plan with the same id. */
function planKeep(s, t) {
  const session = t.view.session ?? null;
  if (s.keep?.key === t.key && s.keep?.session === session) return;
  s.keep = { key: t.key, session };
  delete s.folded;
  delete s.fixOpen;
  delete s.plan;
  delete s.step;
}

/** The Captain whose plan the surface draws now: the thread's own, or on the Orchestrator's Captains surface the one
 * in full view. */
function planSubject(s, t) {
  if (!isGlobalThread(t)) return t;
  return (s.focus && tabByKey(s.focus.key)) ?? null;
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
 * session: a resume restarts plan ids at p1, so a session change starts the list over and only that session's ids stay.
 * A plan the resume carried over keeps its id and title, and counts as opened already: it does not reopen the surface. */
function planAutoOpen(t) {
  const p = panelOf(t);
  const session = t.view.session ?? null;
  const plans = t.view.plans ?? [];
  const carried = p.seen && p.seen.session !== session ? plans.filter((g) => p.seen.plans.some((u) => u.id === g.id && u.title === g.title)).map((g) => g.id) : [];
  const opened = p.autoOpened?.session === session ? p.autoOpened.ids : carried;
  const fresh = plans.filter((g) => !g.finished_at && !opened.includes(g.id));
  const next = { session, ids: [...opened, ...fresh.map((g) => g.id)] };
  if (JSON.stringify(next) !== JSON.stringify(p.autoOpened)) {
    p.autoOpened = next;
    savePanel(t);
  }
  if (!fresh.length || p.open) return;
  addSurface(t, p.surfaces.find((x) => x.kind === "plan") ?? { id: "plan", kind: "plan", title: "Plan" });
}

/** Every tabs push. On a Captain: auto-open for a new plan, the dropped line, and a fetch when the shown glance moved.
 * On the Orchestrator: the Captains surface redraws from the glances, and fetches only the cards that are unfolded. */
function onPlanPush() {
  const t = active;
  if (!t || !guiOn()) return;
  if (isGlobalThread(t)) return planRerender();
  if (!isPlanCaptain(t)) return;
  planAutoOpen(t);
  planDetectDropped(t);
  if (shownSurface(t)?.kind === "plan") planRefresh(t);
}

// ---- the Orchestrator's Captains: one card per Captain, the words and order from its plans and its own wait ----

/** A Captain's card: its state, the word for it, and the plan the card leads with. The most urgent of its plans and its
 * own wait for the user decide the state. The rail's word is this word too. */
function captainCard(t) {
  const plans = planGlances(t);
  const state = threadState(t) === "needs" ? "needs_you" : (plans[0]?.state ?? "idle");
  const lead = plans.find((g) => g.state === state) ?? plans[0] ?? null;
  const ends = plans.map((g) => g.finished_at).filter((at) => at != null);
  const word = state === "finished" && ends.length ? `Finished at ${planClock(Math.max(...ends))}` : PLAN_CARD_WORD[state];
  return { state, word, plans, now: lead?.now ?? "", done: lead?.done ?? 0, total: lead?.total ?? 0 };
}

/** Every Captain tab with its card, and the project and machine its card names. */
function captainRows() {
  const facts = captainFacts();
  return tabs.filter(isPlanCaptain).map((t) => ({
    tab: t,
    card: captainCard(t),
    name: facts.find((f) => f.captain === t.key)?.name ?? projectName(t),
    machine: machineName(t.machine),
  }));
}

/** One plan's dots: a group per list, a dot per step, as the glance's strips spell them. */
const planStripHtml = (g) => `<span class="pl-dots">${g.strips.map((x) => `<span class="pl-grp">${[...x].map((ch) => `<i class="pl-sd" data-state="${PLAN_DOT_STATE[ch] ?? "pending"}"></i>`).join("")}</span>`).join("")}</span>`;

/** The rail row's dots: one strip per unfinished plan of its Captain. */
function planRailStripHtml(t) {
  const live = planGlances(t).filter((g) => !g.finished_at);
  return live.length ? `<span class="pl-railstrip">${live.map(planStripHtml).join("")}</span>` : "";
}

/** The Captains surface as a whole: the header counts, the cards by urgency, the plain Captains, or the full plan. */
function captainsView(s) {
  const rows = captainRows();
  const focus = s.focus ? rows.find((r) => r.tab.key === s.focus.key) : null;
  if (focus) return `<div class="pl" data-view="full">${captainFullHtml(focus, s)}</div>`;
  if (!rows.length) return `<div class="pl" data-view="captains"><div class="pl-empty"><p class="t">No Captain yet</p><p>A project's Captain keeps its plan here.</p></div></div>`;
  const withPlan = rows.filter((r) => r.card.plans.length).sort((a, b) => PLAN_ORDER[a.card.state] - PLAN_ORDER[b.card.state]);
  const plain = rows.filter((r) => !r.card.plans.length);
  const count = CAPTAIN_COUNTED.map((k) => [k, rows.filter((r) => r.card.state === k).length])
    .filter(([, n]) => n)
    .map(([k, n]) => `${n} ${PLAN_CARD_WORD[k].toLowerCase()}`)
    .join(" · ");
  const cards = withPlan.map((r) => captainCardHtml(r, s)).join("");
  const rest = plain.length ? `<p class="pl-sec">No plan running</p>${plain.map(captainPlainHtml).join("")}` : "";
  return `<div class="pl" data-view="captains"><div class="pl-head"><span class="pl-count">${count}</span></div><div class="pl-scroll pl-cards">${cards}${rest}</div></div>`;
}

/** A card: the Captain's tile, name, machine, the now line, the word, done of total, a strip per unfinished plan, and
 * the tree in place when unfolded. Unfolded for this session only: a resume folds it again. */
function captainCardHtml(r, s) {
  const { tab, card } = r;
  const open = s.unfolded?.[tab.key] === (tab.view.session ?? "");
  const strips = card.plans
    .filter((g) => !g.finished_at)
    .map((g) => `<div class="pl-strip"><span class="t" title="${esc(g.title)}">${esc(g.title)}</span>${planStripHtml(g)}</div>`)
    .join("");
  const body = open ? planInPlaceHtml(r) : "";
  return `<section class="pl-card" data-cstate="${card.state}" data-open="${open}"><button type="button" class="pl-card-head" data-card="${esc(tab.key)}" aria-expanded="${open}">${captainMonoHtml(r.name)}<span class="text"><span class="top"><span class="name">${esc(r.name)}</span><span class="machine">${esc(r.machine)}</span></span><span class="sub">${esc(card.now)}</span></span><span class="end"><span class="pl-word" data-tone="${card.state}">${card.word}</span><span class="n">${card.done}/${card.total}</span></span>${icon(open ? "chevron-up" : "chevron-down", "chev")}</button>${strips}${body}</section>`;
}

/** A Captain with no plan: a short row under No plan running. Its thread opens from its own button. */
const captainPlainHtml = (r) => `<div class="pl-idle">${captainMonoHtml(r.name)}<span class="name">${esc(r.name)}</span><span class="machine">${esc(r.machine)}</span><span class="pl-word" data-tone="${r.card.state}">${r.card.word}</span><button type="button" class="ghost" data-open-thread="${esc(r.tab.key)}">${icon("arrow-up-right")}Open thread</button></div>`;

/** The tree in place of an unfolded card: its unfinished plans stacked at the Dots zoom, without step names. Fetches
 * that Captain's plans once per glance change. Its controls are the full plan and the thread. */
function planInPlaceHtml(r) {
  const c = planRefresh(r.tab);
  const actions = `<div class="pl-card-actions"><button type="button" class="outline" data-full="${esc(r.tab.key)}">${icon("maximize-2")}Open the full plan</button><button type="button" class="ghost" data-open-thread="${esc(r.tab.key)}">${icon("arrow-up-right")}Open thread</button></div>`;
  if (!c.plans) return `<div class="pl-card-body"><p class="pl-loading">${c.failed ? "Could not load the plan." : "Loading the plan…"}</p>${actions}</div>`;
  const live = r.card.plans
    .filter((g) => !g.finished_at)
    .map((g) => c.plans.find((p) => p.id === g.id))
    .filter(Boolean);
  const trees = live.map((p) => `<div class="pl-mini"><p class="pl-mini-title">${esc(p.title)}</p><div class="pl-lists">${p.lists.map(planDotsListHtml).join("")}</div></div>`).join("");
  return `<div class="pl-card-body">${trees || `<p class="pl-loading">Every plan is finished.</p>`}${actions}</div>`;
}

/** A list in the tree in place: its name, its word and its track at the Dots zoom. It has no controls. */
function planDotsListHtml(list) {
  const [tone, word] = planListWord(list);
  return `<section class="pl-list"><div class="pl-list-head"><span class="pl-list-name">${esc(list.name)}</span><span class="pl-word" data-tone="${tone}">${word}</span></div>${planTrackHtml(list, {}, 0)}</section>`;
}

/** The whole plan of one Captain on the Orchestrator's surface, with All Captains back and Open thread. */
function captainFullHtml(r, s) {
  const lead = `<div class="pl-fullhead"><button type="button" class="ghost pl-back" data-back>${icon("chevron-left")}All Captains</button>${captainMonoHtml(r.name)}<span class="name">${esc(r.name)}</span><span class="machine">${esc(r.machine)}</span><button type="button" class="ghost" data-open-thread="${esc(r.tab.key)}">${icon("arrow-up-right")}Open thread</button></div>`;
  return planBodyHtml(r.tab, s, { captains: true, lead });
}

// ---- render ----

function planDroppedHtml(p) {
  if (!p.dropped?.length) return "";
  const text = p.dropped.length === 1 ? `1 plan from before /resume was not carried over: ${esc(p.dropped[0])}.` : `${p.dropped.length} plans from before /resume were not carried over: ${p.dropped.map((x) => esc(x)).join(", ")}.`;
  return `<div class="pl-dropped" role="status">${icon("history")}<span>${text}</span><button type="button" class="icon" data-dismiss-dropped title="Dismiss">${icon("x")}</button></div>`;
}

function planChipsHtml(glances, plan) {
  return `<div class="pl-chips" role="tablist">${glances
    .map((g) => `<button type="button" role="tab" class="pl-chip" data-plan="${esc(g.id)}" data-cstate="${g.state}" aria-selected="${g.id === plan.id}" title="${esc(g.title)} · ${PLAN_CARD_WORD[g.state]}"><i class="pl-cdot"></i><span class="t">${esc(g.title)}</span><span class="n">${g.finished_at ? `Finished ${planClock(g.finished_at)}` : `${g.done}/${g.total}`}</span></button>`)
    .join("")}</div>`;
}

function planStepHtml(step, x, y, Z, zoom, s) {
  const picked = s.step === step.id ? " picked" : "";
  const meta = zoom === 2 ? `<span class="pl-meta">${esc(planCost(step.cost) || (step.state === "pending" && !step.task ? "" : planRunBy(step)))}</span>` : "";
  const name = zoom ? `<span class="pl-name">${esc(step.name)}</span>` : "";
  return `<button type="button" class="pl-step${picked}" data-step="${esc(step.id)}" data-state="${step.state}"${planAfterFix(step) ? ' data-fixed=""' : ""} style="left:${x - Z.cell / 2}px;top:${y - 8}px;width:${Z.cell}px" title="${esc(step.name)} · ${esc(planStepWord(step))}"><span class="pl-dot"></span>${name}${meta}</button>`;
}

/** A list's track: the dots on the base line, the open fix branches and the steps, at one zoom level. */
function planTrackHtml(list, s, zoom) {
  const Z = PLAN_ZOOMS[zoom];
  const { xs, fixes, last, width } = planLayout(list, s, Z);
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
    f.steps.forEach((st, k) => (steps += planStepHtml(st, f.xs[k], fy, Z, zoom, s)));
  }
  list.steps.forEach((st, i) => (steps += planStepHtml(st, xs[i], dotY, Z, zoom, s)));
  return `<div class="pl-track" data-z="${zoom}" style="height:${height}px"><div class="pl-canvas" style="width:${width}px;height:${height}px"><svg width="${width}" height="${height}" aria-hidden="true">${svg}</svg>${steps}</div></div>`;
}

function planListHtml(list, s, zoom) {
  const Z = PLAN_ZOOMS[zoom];
  const folded = (s.folded ?? []).includes(list.id);
  const [tone, word] = planListWord(list);
  const fixSteps = list.steps.filter((x) => x.fixes.length);
  const fixOn = fixSteps.some((x) => planFixOpen(s, x));
  const fixCount = fixSteps.reduce((n, x) => n + x.fixes.length, 0);
  const chip = fixSteps.length
    ? `<button type="button" class="pl-fixchip" data-fixchip="${esc(list.id)}" aria-pressed="${fixOn}" title="${fixOn ? "Hide" : "Show"} the fix branch">${icon("git-branch")}${fixCount} ${fixCount === 1 ? "fix" : "fixes"}</button>`
    : "";
  const bar = `<span class="pl-bar">${list.steps.map((x) => `<i data-state="${x.state}"></i>`).join("")}</span>`;
  const head = `<div class="pl-list-head"><button type="button" class="pl-fold" data-fold="${esc(list.id)}" aria-expanded="${!folded}">${icon(folded ? "chevron-right" : "chevron-down")}<span class="pl-list-name">${esc(list.name)}</span></button>${bar}<span class="pl-word" data-tone="${tone}">${word}</span>${chip}</div>`;
  if (folded) return `<section class="pl-list folded">${head}</section>`;
  const { fixes } = planLayout(list, s, Z);
  return `<section class="pl-list${fixes.length ? " has-fix" : ""}">${head}${planTrackHtml(list, s, zoom)}</section>`;
}

function planFootHtml(plan, s, what = true) {
  const live = planLiveStep(plan);
  const hit = (s.step && planFindStep(plan, s.step)) || (live && planFindStep(plan, live.id));
  if (!hit) return "";
  const { step, list, under, fix } = hit;
  const why = step.why && !planAfterFix(step) ? `<p class="pl-why">${esc(step.why)}</p>` : "";
  const branch = under ? `Fix ${esc(fix.id)}, under “${esc(under.name)}”` : "Main line";
  return `<div class="pl-foot" data-state="${step.state}"><p class="pl-foot-state"><span class="pl-dot"></span>${s.step ? planStepWord(step) : `Live now · ${planStepWord(step)}`}</p>
      <p class="pl-foot-name">${esc(step.name)}</p>${what && step.what ? `<p class="pl-foot-what">${esc(step.what)}</p>` : ""}${why}
      <dl class="pl-facts"><dt>List</dt><dd>${esc(list.name)}</dd><dt>Run by</dt><dd>${esc(planRunBy(step))}</dd><dt>Branch</dt><dd>${branch}</dd><dt>Cost</dt><dd>${esc(planCost(step.cost)) || "None yet"}</dd></dl>
      ${step.task ? `<button type="button" class="outline pl-open" data-open-task="${esc(step.task)}">${icon("arrow-up-right")}Open thread</button>` : ""}</div>`;
}

/** The body of the surface for one Captain: dropped line, switcher, header, the plan's lists, footer. On the
 * Captains surface's full view `captains` drops the dropped line and the footer's "what", and `lead` goes first. */
function planBodyHtml(t, s, { captains = false, lead = "" } = {}) {
  planKeep(s, t);
  const dropped = captains ? "" : planDroppedHtml(panelOf(t));
  const empty = `<div class="pl-empty"><p class="t">The Captain is writing its plan</p><p>Each part of the work shows up here as a list of steps.</p></div>`;
  const glances = planGlances(t);
  if (!glances.length) return `${lead}${dropped}${empty}`;
  const c = planRefresh(t);
  if (!c.plans) return `${lead}${dropped}<p class="pl-loading">${c.failed ? "Could not load the plan." : "Loading the plan…"}</p>`;
  const plan = planPicked(t, s);
  if (!plan) return `${lead}${dropped}${empty}`;
  const zoom = s.zoom ?? 1;
  const chips = glances.length > 1 ? planChipsHtml(glances, plan) : "";
  const ms = planMains(plan);
  const done = ms.filter((x) => x.state === "done").length;
  const head = `<div class="pl-head"><span class="pl-count">${done} of ${ms.length} done</span><span class="pl-zoom" role="group" aria-label="Zoom"><button type="button" class="icon" data-zoom="-1" title="Zoom out" ${zoom === 0 ? "disabled" : ""}>${icon("minus")}</button><span class="pl-zoom-name">${PLAN_ZOOMS[zoom].name}</span><button type="button" class="icon" data-zoom="1" title="Zoom in" ${zoom === 2 ? "disabled" : ""}>${icon("plus")}</button></span></div>
      <div class="pl-progress"><span style="width:${ms.length ? (done / ms.length) * 100 : 0}%"></span></div>`;
  const at = plan.closed ? `Closed: ${esc(plan.closed)}` : plan.finished_at ? `Finished at ${planClock(plan.finished_at)}` : `Plan, from your message at ${planClock(plan.asked_at)}`;
  const root = `<p class="pl-root">${icon("message-circle")}<span><b>${esc(plan.title)}</b><span class="pl-root-at">${at}</span></span></p>`;
  const lists = plan.lists.map((l) => planListHtml(l, s, zoom)).join("");
  return `${lead}${dropped}${chips}${head}<div class="pl-scroll">${root}<div class="pl-lists">${lists}</div></div>${planFootHtml(plan, s, !captains)}`;
}

function renderPlanPanel(s) {
  const t = active;
  if (isGlobalThread(t)) return showPanel(captainsView(s));
  if (!isPlanCaptain(t)) return showPanel(`<div class="pl"><div class="pl-empty"><p class="t">No plan here</p><p>Only a project's Captain keeps a plan.</p></div></div>`);
  planDetectDropped(t);
  showPanel(`<div class="pl" data-view="plan">${planBodyHtml(t, s)}</div>`);
}

// ---- clicks ----

function onPlanClick(e) {
  const thread = active;
  const s = thread && shownSurface(thread);
  if (!s || s.kind !== "plan" || !e.target.closest(".pl")) return;
  const q = (sel) => e.target.closest(sel);
  let el;
  if ((el = q("[data-card]"))) {
    const key = el.dataset.card;
    const session = tabByKey(key)?.view.session ?? "";
    s.unfolded = { ...(s.unfolded ?? {}) };
    if (s.unfolded[key] === session) delete s.unfolded[key];
    else s.unfolded[key] = session;
  } else if ((el = q("[data-full]"))) s.focus = { key: el.dataset.full };
  else if (q("[data-back]")) s.focus = null;
  else if ((el = q("[data-open-thread]"))) return activate(tabByKey(el.dataset.openThread));
  else if (q(".pl-card-body")) return;
  else {
    const t = planSubject(s, thread);
    if (!t) return;
    planKeep(s, t);
    if ((el = q("[data-zoom]"))) s.zoom = Math.max(0, Math.min(2, (s.zoom ?? 1) + Number(el.dataset.zoom)));
    else if ((el = q("[data-fold]"))) {
      const id = el.dataset.fold;
      s.folded = (s.folded ?? []).includes(id) ? s.folded.filter((x) => x !== id) : [...(s.folded ?? []), id];
    } else if ((el = q("[data-fixchip]"))) {
      const list = planPicked(t, s)?.lists.find((l) => l.id === el.dataset.fixchip);
      const steps = (list?.steps ?? []).filter((x) => x.fixes.length);
      const on = !steps.some((x) => planFixOpen(s, x));
      s.fixOpen = { ...(s.fixOpen ?? {}) };
      for (const step of steps) s.fixOpen[step.id] = on;
    } else if ((el = q("[data-step]"))) s.step = s.step === el.dataset.step ? null : el.dataset.step;
    else if ((el = q("[data-plan]"))) {
      s.plan = el.dataset.plan;
      s.step = null;
    } else if (q("[data-dismiss-dropped]")) panelOf(t).dropped = null;
    else if ((el = q("[data-open-task]"))) return openDelegated(el.dataset.openTask);
    else return;
    if (t !== thread) savePanel(t);
  }
  savePanel(thread);
  planRerender();
}

panelBody.addEventListener("click", onPlanClick);
onModelChange(onPlanPush);
