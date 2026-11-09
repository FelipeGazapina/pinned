/** Delegation metrics on the Way of working page: what the account's delegated tasks cost, how
 * long they took and how they ended, over 24 h, 7 d or 30 d. Each breakdown row says which
 * Delegation limits field it would tune. The numbers come from `delegation_stats`; the formats and
 * hints live in delegation.js. */

/** `stats` is the last answer for `days`, null until it arrives. */
const delegationUi = { days: 7, stats: null, error: null, loading: false };

async function delegationFetch() {
  const days = delegationUi.days;
  delegationUi.loading = true;
  try {
    const stats = await connectorCall("delegation_stats", { days });
    if (days !== delegationUi.days) return;
    delegationUi.stats = stats;
    delegationUi.error = null;
  } catch (e) {
    if (days !== delegationUi.days) return;
    delegationUi.error = isOlderPinned(e) ? "Update pinned on the hub to see delegation metrics." : String(e?.message ?? e);
  } finally {
    if (days === delegationUi.days) delegationUi.loading = false;
  }
  tasteRender();
}

function delegationBarHtml(value, max) {
  const share = max ? Math.max(2, Math.round((value / max) * 100)) : 0;
  return `<span class="delegation-bar" aria-hidden="true"><span style="width:${share}%"></span></span>`;
}

function delegationGroupHtml(title, key, groups) {
  if (!groups?.length) return "";
  const max = Math.max(...groups.map((g) => g.runs));
  const outcome = key === "by_outcome";
  const rows = groups
    .map((g) => {
      const hints = delegationHints(g, outcome || !g.key ? "" : g.label, key)
        .map((h) => `<li>${esc(h.text)} <span class="delegation-tune">→ ${esc(h.field)}</span></li>`)
        .join("");
      return `<li class="delegation-row">
        <div class="delegation-line"><span class="delegation-name" title="${esc(g.key)}">${esc(g.label)}</span>${delegationBarHtml(g.runs, max)}
          <span class="delegation-nums"><span>${g.runs} ${g.runs === 1 ? "task" : "tasks"}</span>${outcome ? "" : `<span>${delegationPercent(g.done, g.runs)} done</span>`}<span>${delegationMinutes(g.median_wall_ms)} median</span><span>${delegationSpend(g)}</span></span></div>
        ${hints ? `<ul class="delegation-hints">${hints}</ul>` : ""}</li>`;
    })
    .join("");
  return `<div class="delegation-group"><h3>${title}</h3><ul class="delegation-rows">${rows}</ul></div>`;
}

function delegationTaskHtml(t, value) {
  const open = t.thread && typeof tabByKey === "function" && typeof keyOf === "function" && tabByKey(keyOf(t.machine, t.thread));
  const title = open
    ? `<button type="button" class="delegation-link" data-act="delegation-open" data-machine="${esc(t.machine)}" data-tab="${esc(t.thread)}">${esc(t.title)}</button>`
    : `<span class="delegation-title">${esc(t.title)}</span>`;
  const limit = t.outcome === "limit" && t.limit ? ` · ${esc(t.limit)} limit` : "";
  const meta = [t.model, t.delegator, t.machine_name].filter(Boolean).map(esc).join(" · ");
  return `<li>${title}<span class="delegation-meta">${meta}${limit}</span><b>${value}</b></li>`;
}

function delegationListHtml(title, tasks, value) {
  if (!tasks?.length) return "";
  return `<div class="delegation-group"><h3>${title}</h3><ul class="delegation-tasks">${tasks.map((t) => delegationTaskHtml(t, value(t))).join("")}</ul></div>`;
}

/** The section, drawn by tasteRender under the way of working's controls. */
function delegationSectionHtml() {
  const periods = DELEGATION_PERIODS.map((p) => `<button type="button" data-act="delegation-days" data-days="${p.days}" aria-pressed="${p.days === delegationUi.days}">${p.label}</button>`).join("");
  const head = `<div class="delegation-head"><div class="taste-heading"><h2>Delegation metrics</h2><p>Every task the orchestrator, the Captains and other sessions delegated, on every machine of the account.</p></div>
    <div class="taste-seg" role="group" aria-label="Period">${periods}</div></div>`;
  const s = delegationUi.stats;
  let body;
  if (!s) body = `<p class="taste-quiet${delegationUi.error ? " delegation-error" : ""}">${esc(delegationUi.error ?? "Loading…")}</p>`;
  else if (!s.totals.runs) body = `<p class="taste-quiet">No delegated task ended in this period.</p>`;
  else {
    const tiles = delegationTiles(s.totals)
      .map((t) => `<div><dt>${t.label}</dt><dd>${esc(t.value)}</dd>${t.note ? `<small>${esc(t.note)}</small>` : ""}</div>`)
      .join("");
    const groups = DELEGATION_BREAKDOWNS.map((b) => delegationGroupHtml(b.title, b.key, s[b.key])).join("");
    const lists = delegationListHtml("Slowest tasks", s.slowest, (t) => delegationMinutes(t.wall_ms)) + delegationListHtml("Most expensive tasks", s.priciest, (t) => delegationUsd(t.cost_usd));
    const note = s.totals.priced < s.totals.runs ? `<p class="taste-hint">Cursor and Codex report no price, so their tasks count in time and tokens only.</p>` : "";
    body = `<dl class="delegation-tiles">${tiles}</dl>${note}<div class="delegation-grid">${groups}</div><div class="delegation-grid">${lists}</div>`;
  }
  return `<section class="taste-section delegation${delegationUi.loading && s ? " loading" : ""}">${head}${body}</section>`;
}

TASTE_ACTS["delegation-days"] = (b) => {
  const days = Number(b.dataset.days);
  if (days === delegationUi.days) return;
  delegationUi.days = days;
  tasteRender();
  delegationFetch();
};
TASTE_ACTS["delegation-open"] = (b) => revealTab(b.dataset.machine, b.dataset.tab).catch((err) => tell(`Couldn't open that thread: ${esc(err)}`, 6000));
