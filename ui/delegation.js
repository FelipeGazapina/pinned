// Delegation metrics, as pure logic: the period choices, number formats and the line under each
// breakdown row that names the setting it would tune. The totals come from the hub's
// `delegation_stats` (crates/core/src/delegate/metrics.rs); gui-delegation.js renders them.

const DELEGATION_PERIODS = [
  { days: 1, label: "24 h" },
  { days: 7, label: "7 d" },
  { days: 30, label: "30 d" },
];

/** The Delegation limits field each limit name maps to. */
const DELEGATION_LIMIT_FIELDS = { time: "Time limit", budget: "Budget per agent" };

const DELEGATION_BREAKDOWNS = [
  { key: "by_model", title: "By model" },
  { key: "by_provider", title: "By provider" },
  { key: "by_project", title: "By project" },
  { key: "by_delegator", title: "By delegator" },
  { key: "by_outcome", title: "By outcome" },
];

function delegationMinutes(ms) {
  const minutes = ms / 60000;
  if (minutes < 1) return `${Math.round(ms / 1000)} s`;
  if (minutes < 60) return `${Math.round(minutes)} min`;
  const hours = Math.floor(minutes / 60);
  const rest = Math.round(minutes % 60);
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

const delegationUsd = (n) => `$${(n ?? 0).toFixed(n >= 100 ? 0 : 2)}`;

function delegationTokens(n) {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`;
  return String(n);
}

const delegationPercent = (part, whole) => (whole ? `${Math.round((part / whole) * 100)}%` : "–");

/** What a group's spend is when only some of its tasks reported a price. */
function delegationSpend(g) {
  const total = g.cost_usd + g.box_usd;
  if (!g.priced && !g.box_usd) return "–";
  return g.priced < g.runs && !g.box_usd ? `${delegationUsd(total)}+` : delegationUsd(total);
}

/** The hints under one breakdown row, each naming the Delegation limits field it would tune:
 * a limit that stopped tasks, a failure rate of half or more, and box spend. `noun` names the
 * row's tasks, like "claude-opus-5-5" or "Captain site", or nothing for an outcome row. */
function delegationHints(g, noun) {
  const hints = [];
  const tasks = noun ? `${noun} tasks` : "tasks";
  for (const [limit, count] of Object.entries(g.limits ?? {})) {
    const field = DELEGATION_LIMIT_FIELDS[limit] ?? "Delegation limits";
    hints.push({ text: `${count} of ${g.runs} ${tasks} hit the ${limit} limit`, field });
  }
  if (g.runs >= 2 && g.failed * 2 >= g.runs) hints.push({ text: `${g.failed} of ${g.runs} ${tasks} failed`, field: "Tier" });
  if (g.box_usd > 0) hints.push({ text: `${delegationUsd(g.box_usd)} on boxes over ${delegationMinutes(g.box_ms)}`, field: "Upstash Box budget" });
  return hints;
}

/** The totals line items, in the order the tiles show them. */
function delegationTiles(t) {
  return [
    { label: "Tasks", value: String(t.runs) },
    { label: "Done", value: `${t.done} · ${delegationPercent(t.done, t.runs)}` },
    { label: "Median time", value: t.runs ? delegationMinutes(t.median_wall_ms) : "–" },
    { label: "Spend", value: delegationSpend(t), note: t.box_usd ? `${delegationUsd(t.cost_usd)} agents · ${delegationUsd(t.box_usd)} boxes` : t.priced < t.runs ? `${t.runs - t.priced} with no price` : "" },
    { label: "Tokens", value: `${delegationTokens(t.tokens_in)} in · ${delegationTokens(t.tokens_out)} out` },
    { label: "Accepted / redone", value: `${t.accepted} / ${t.redone}` },
  ];
}
