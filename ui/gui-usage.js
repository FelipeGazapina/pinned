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
/** `envs` holds each machine's transcripts and `limits` its claude's limits check; `selected` is null for every machine, as T3 Code's environment filter. */
const usageState = { metric: "limits", days: 30, breakdown: "model", selected: null, envs: new Map(), limits: new Map(), limitsNow: 0, refreshing: false, now: 0, seq: 0 };
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

/** T3 Code's environments are the machines: each one's scan, and how the filter labels it. */
const usageEnvs = () => [...machines.keys()];
const usageSelected = () => usageEnvs().filter((m) => !usageState.selected || usageState.selected.has(m));
const envOnline = (m) => !!machines.get(m)?.view.online;
const envReadable = (m) => envOnline(m) && !usageState.envs.get(m)?.error;
function envStatus(m) {
  const env = usageState.envs.get(m);
  if (env?.error) return /can't answer that/.test(env.error) ? "Update required" : "Unavailable";
  if (!envOnline(m)) return "Offline";
  if (!env?.loaded) return "Scanning…";
  return env.loading ? "Refreshing…" : "Ready";
}
const usagePartial = () => usageSelected().some((m) => envOnline(m) && usageState.envs.get(m)?.loading);
const usageEnvLabel = () => {
  const selected = usageSelected();
  return !usageState.selected ? "All environments" : selected.length === 1 ? machineName(selected[0]) : `${selected.length} environments`;
};

/** The window's totals, series and models over the selected machines, of one `model` when given. */
function summarize(model) {
  const { days } = usageState;
  const buckets = usageBuckets(days, usageState.now);
  const index = new Map(buckets.map((b, i) => [b, i]));
  const rows = usageSelected().flatMap((m) => usageState.envs.get(m)?.rows ?? []).filter((r) => (!model || r.model === model) && index.has(bucketOf(days, r.at)));
  const s = { buckets, rows, cost: 0, tokens: 0, input: 0, cached: 0, cacheWrite: 0, output: 0, savings: 0, premium: 0, fast: 0, unpriced: 0, unpricedTokens: 0, sessions: new Set(),
    byType: { input: 0, cache_read: 0, cache_write: 0, output: 0 }, series: buckets.map(() => ({ cost: 0, tokens: 0 })), models: new Map() };
  for (const r of rows) {
    const cost = rowCost(r);
    const tokens = rowTokens(r);
    Object.assign(s, { cost: s.cost + cost, tokens: s.tokens + tokens, input: s.input + r.input, cached: s.cached + r.cached, cacheWrite: s.cacheWrite + r.cache_write, output: s.output + r.output, savings: s.savings + r.savings, premium: s.premium + r.premium });
    if (r.speed === "fast") s.fast += cost;
    if (!r.cost) {
      s.unpriced++;
      s.unpricedTokens += tokens;
    }
    for (const k in s.byType) s.byType[k] += r.cost?.[k] ?? 0;
    s.sessions.add(`${r.machine}:${r.session}`);
    const point = s.series[index.get(bucketOf(days, r.at))];
    point.cost += cost;
    point.tokens += tokens;
    const m = s.models.get(r.model) ?? { model: r.model, cost: 0, records: 0, unpriced: 0, tokens: 0, input: 0, cached: 0, cacheWrite: 0, output: 0 };
    Object.assign(m, { cost: m.cost + cost, records: m.records + 1, unpriced: m.unpriced + (r.cost ? 0 : 1), tokens: m.tokens + tokens, input: m.input + r.input, cached: m.cached + r.cached, cacheWrite: m.cacheWrite + r.cache_write, output: m.output + r.output });
    s.models.set(r.model, m);
  }
  return s;
}
/** T3 Code's `isModelCostUnknown`: every record lacked a price, so the cost is unknown, not zero. */
const costUnknown = (m) => m.records > 0 && m.unpriced >= m.records;

/** Reads every online machine's transcripts; each machine's answer lands as it comes. */
async function fetchUsage() {
  const seq = ++usageState.seq;
  const since = usageBuckets(90)[0];
  usageState.now = Date.now() / 1000;
  await moveBrowserPrices();
  if (seq !== usageState.seq) return;
  const online = onlineMachines();
  for (const m of online) usageState.envs.set(m, { rows: [], loaded: false, ...usageState.envs.get(m), loading: true, error: "" });
  if (shellPage() === "usage") renderUsagePage();
  await Promise.all(online.map(async (m) => {
    let env;
    try {
      env = { rows: (await callOn(m, { cmd: "token_usage", since })).map((r) => ({ ...r, machine: m })), error: "" };
    } catch (e) {
      env = { rows: [], error: String(e?.message ?? e) };
    }
    if (seq !== usageState.seq) return;
    usageState.envs.set(m, { ...env, loaded: true, loading: false });
    if (shellPage() === "usage") renderUsagePage();
  }));
}

/** T3 Code's menus and selects on this page: a popup under its trigger, whose `keep` items leave it open. */
const usageMenu = document.createElement("div");
usageMenu.id = "usage-menu";
usageMenu.setAttribute("role", "menu");
usageMenu.hidden = true;
document.body.append(usageMenu);
let usageMenuItems = () => [];
/** The menu's trigger by `selector`, since a redraw of the page replaces it. */
const usageMenuAnchor = () => [...document.querySelectorAll(usageMenu.selector)].find((el) => el.getClientRects().length);
function openUsageMenu(selector, items, align = "start") {
  usageMenuAnchor()?.removeAttribute("aria-expanded");
  Object.assign(usageMenu, { selector });
  usageMenuItems = items;
  usageMenu.dataset.align = align;
  usageMenu.hidden = false;
  drawUsageMenu();
}
function drawUsageMenu() {
  const anchor = usageMenuAnchor();
  if (!anchor) return closeUsageMenu();
  anchor.setAttribute("aria-expanded", "true");
  usageMenu.innerHTML = usageMenuItems().map((item, i) => {
    if (item.sep) return "<hr>";
    if (item.note) return `<p class="note${item.notes ? " notes" : ""}">${item.note}</p>`;
    const check = item.checked === undefined ? "" : `<span class="box${item.box ? " square" : ""}">${item.checked ? icon("check") : ""}</span>`;
    return `<button type="button" role="${item.checked === undefined ? "menuitem" : "menuitemcheckbox"}" data-i="${i}"${item.checked === undefined ? "" : ` aria-checked="${item.checked}"`}${item.title ? ` title="${esc(item.title)}"` : ""}${item.disabled ? " disabled" : ""}>
      ${item.box ? check : ""}${item.icon ? icon(item.icon) : ""}<span class="label">${esc(item.label)}</span>${item.status ? `<span class="status${item.tone ? ` ${item.tone}` : ""}">${esc(item.status)}</span>` : ""}${item.box ? "" : check}</button>`;
  }).join("");
  const r = usageMenu.getBoundingClientRect();
  const a = anchor.getBoundingClientRect();
  const x = usageMenu.dataset.align === "end" ? a.right - r.width : a.left;
  usageMenu.style.left = `${Math.max(8, Math.min(x, innerWidth - r.width - 8))}px`;
  usageMenu.style.top = `${Math.max(8, Math.min(a.bottom + 4, innerHeight - r.height - 8))}px`;
}
function closeUsageMenu() {
  usageMenu.hidden = true;
  if (usageMenu.selector) usageMenuAnchor()?.removeAttribute("aria-expanded");
}
usageMenu.addEventListener("click", (e) => {
  const item = usageMenuItems()[e.target.closest("button[data-i]")?.dataset.i];
  if (!item || item.disabled) return;
  if (!item.keep) closeUsageMenu();
  item.run();
  if (item.keep && !usageMenu.hidden) drawUsageMenu();
});
window.addEventListener("mousedown", (e) => !usageMenu.hidden && !e.target.closest("#usage-menu") && !e.target.closest(usageMenu.selector) && closeUsageMenu(), true);
window.addEventListener("keydown", (e) => {
  if (usageMenu.hidden || e.key !== "Escape") return;
  e.preventDefault();
  e.stopPropagation();
  closeUsageMenu();
}, true);

/** T3 Code's tooltips and hover popovers: `data-pop` names one of the contents the last draw registered.
 * Hover shows it; a click pins it open until a click elsewhere. */
const usagePop = document.createElement("div");
usagePop.id = "usage-pop";
usagePop.setAttribute("role", "tooltip");
usagePop.hidden = true;
document.body.append(usagePop);
const usagePops = new Map();
let popSeq = 0;
function popAttr(html) {
  usagePops.set(++popSeq, html);
  usagePops.delete(popSeq - 400);
  return ` data-pop="${popSeq}"`;
}
let popHide = 0;
function showUsagePop(trigger, pinned = false) {
  clearTimeout(popHide);
  usagePop.html = usagePops.get(Number(trigger.dataset.pop)) ?? "";
  usagePop.innerHTML = usagePop.html;
  usagePop.trigger = trigger;
  usagePop.pinned = pinned;
  usagePop.hidden = false;
  const t = trigger.getBoundingClientRect();
  const r = usagePop.getBoundingClientRect();
  const above = t.top - r.height - 6 >= 8;
  usagePop.style.left = `${Math.max(8, Math.min(t.left + t.width / 2 - r.width / 2, innerWidth - r.width - 8))}px`;
  usagePop.style.top = `${above ? t.top - r.height - 6 : t.bottom + 6}px`;
}
const hideUsagePop = () => {
  usagePop.hidden = true;
  usagePop.trigger = null;
};
/** Whether `trigger` is the one the popover shows for. The page redraws as limits and usage arrive,
 * which swaps in a new trigger for the same popover, so a detached one is matched by what it says. */
function popShownFor(trigger) {
  if (usagePop.hidden || !usagePop.trigger) return false;
  if (usagePop.trigger === trigger) return true;
  if (usagePop.trigger.isConnected || usagePops.get(Number(trigger.dataset.pop)) !== usagePop.html) return false;
  usagePop.trigger = trigger;
  return true;
}
document.addEventListener("pointerover", (e) => {
  if (e.pointerType === "touch") return;
  const trigger = e.target.closest?.("[data-pop]");
  if (trigger && !popShownFor(trigger)) showUsagePop(trigger);
  else if (trigger || e.target.closest?.("#usage-pop")) clearTimeout(popHide);
  else if (!usagePop.hidden && !usagePop.pinned) {
    clearTimeout(popHide);
    popHide = setTimeout(hideUsagePop, 120);
  }
});
document.addEventListener("click", (e) => {
  const trigger = e.target.closest?.("[data-pop]");
  if (trigger) return popShownFor(trigger) && usagePop.pinned ? hideUsagePop() : showUsagePop(trigger, true);
  if (!e.target.closest?.("#usage-pop")) hideUsagePop();
});
window.addEventListener("keydown", (e) => {
  if (usagePop.hidden || e.key !== "Escape") return;
  e.preventDefault();
  e.stopPropagation();
  hideUsagePop();
}, true);

/** T3 Code's shortcut hints in a control's hover title. */
const usageKeyTitle = (label, key) => `${label} (${key})`;
const metricKey = (key) => key.toUpperCase();
const periodKey = (i) => (IS_MAC ? `⇧⌘${i + 1}` : `Ctrl+Shift+${i + 1}`);
const metricOptions = () => USAGE_METRICS.map(([value, label, key]) => [value, label, usageKeyTitle(label, metricKey(key))]);
const periodOptions = () => USAGE_PERIODS.map(([value, label], i) => [value, label, usageKeyTitle(label, periodKey(i))]);

const segmented = (name, options, current, label, disabled = false) =>
  `<span class="seg" role="radiogroup" aria-label="${label}" data-seg="${name}">${options.map(([value, text, title]) => `<button type="button" role="radio" aria-checked="${value === current}" data-value="${value}"${title ? ` title="${esc(title)}"` : ""}${disabled ? " disabled" : ""}>${text}</button>`).join("")}</span>`;
const pickButton = (name, options, current, label, disabled = false) =>
  `<button type="button" class="pick" data-pick="${name}" aria-label="${label}"${disabled ? " disabled" : ""}>${options.find(([v]) => v === current)?.[1] ?? ""}${icon("chevron-down")}</button>`;

function usageRange(buckets, days) {
  const first = buckets[0];
  const last = buckets.at(-1);
  if (days === 1) return `${monthDay(first)}, ${hourText(first)} to ${monthDay(last + 3600)}, ${hourText(last + 3600)}`;
  return `${monthDay(first)} to ${monthDay(last)}`;
}

function renderUsagePage() {
  const { metric, days } = usageState;
  const limits = metric === "limits";
  const selected = usageSelected();
  const s = summarize();
  const issue = !limits && selected.some((m) => usageState.envs.get(m)?.error);
  const envButton = `<button type="button" class="crumb-menu env-filter"><span class="text">${esc(usageEnvLabel())}</span>${
    issue ? `<span class="alert" role="img" aria-label="Some environments could not report usage">${icon("circle-alert")}</span>` : icon("chevron-down")}</button>`;
  const refresh = `<button type="button" class="icon refresh${usageState.refreshing ? " spinning" : ""}" aria-label="${limits ? "Refresh limits" : "Refresh usage"}" aria-busy="${usageState.refreshing}"${
    usageState.refreshing || !selected.some(envOnline) ? " disabled" : ""}>${icon("refresh-cw")}</button>`;
  const body = !machines.size
    ? `<p class="muted note">Connect an environment to see ${limits ? "limits" : "usage"}.</p>`
    : !selected.length
      ? `<p class="muted note">Select an environment to see ${limits ? "limits" : "usage"}.</p>`
      : limits
        ? limitsHtml()
        : !selected.some((m) => usageState.envs.get(m)?.loaded) && selected.some((m) => envOnline(m))
          ? usageSkeleton()
          : !selected.some(envReadable)
            ? `<div class="env-errors">${selected.map((m) => `<p>${selected.length > 1 ? `${esc(machineName(m))}: ` : ""}${esc(usageState.envs.get(m)?.error || "Offline")}</p>`).join("")}</div>`
            : spendHtml(s);
  setPage(`${headHtml("Usage", `<span class="crumb-sep">›</span>${envButton}<span class="crumb-sep">›</span><span class="provider">${icon("claude")}<span class="text">Claude Code</span></span>
      ${limits ? "" : `<span class="range">${usageRange(s.buckets, days)}</span>`}<span class="spacer"></span>
      <span class="controls wide">${segmented("metric", metricOptions(), metric, "Usage metric")}${segmented("days", periodOptions(), days, "Usage period", limits)}${refresh}</span>
      <span class="controls narrow">${pickButton("metric", metricOptions(), metric, "Usage metric")}${pickButton("days", periodOptions(), days, "Usage period", limits)}${refresh}</span>`)}
    <div class="page-body"><div class="page-wrap usage" data-metric="${metric}"${usagePartial() ? " data-loading" : ""}>${body}</div></div>`);
  if (!limits) drawChart(pageView.querySelector(".usage .chart"), s, metric, usagePartial());
}

function spendHtml(s) {
  const { metric, days } = usageState;
  const cost = metric === "cost";
  const fmt = cost ? usd : tokensText;
  const total = cost ? usd(s.cost) : tokensText(s.tokens);
  const sessions = `${s.sessions.size.toLocaleString("en-US")} session${s.sessions.size === 1 ? "" : "s"}`;
  const unpricedShare = s.rows.length ? s.unpriced / s.rows.length : 0;
  const unit = days === 1 ? "Hourly" : "Daily";
  const active = s.tokens > 0 || s.cost > 0;
  const models = [...s.models.values()].sort((a, b) => (cost ? b.cost - a.cost || b.tokens - a.tokens : b.tokens - a.tokens || b.cost - a.cost));
  const peak = Math.max(0, ...models.map((m) => (cost ? m.cost : m.tokens)));
  const modelRows = models.map((m, i) => {
    const value = cost ? m.cost : m.tokens;
    const share = cost ? (costUnknown(m) ? "" : percentText(s.cost ? m.cost / s.cost : 0)) : percentText(s.tokens ? m.tokens / s.tokens : 0);
    return `<tr data-model="${esc(m.model)}"><td class="n">${i + 1}</td><td class="model"><button type="button"${value > 0 && peak > 0 ? "" : ` class="bare"`}>${icon("claude")}<span>${esc(m.model)}</span></button>${
      value > 0 && peak > 0 ? `<i style="width:max(8px, ${((value / peak) * 100).toFixed(2)}%)"></i>` : ""}</td>
      <td class="num fig strong">${costUnknown(m) ? `<span class="muted">Unpriced</span>` : usd(m.cost)}</td><td class="num fig share-col">${share}</td><td class="num fig">${tokensText(m.tokens)}</td></tr>`;
  }).join("");
  const periods = s.buckets.map((b, i) => [b, s.series[i]]).filter(([, p]) => p.tokens > 0 || p.cost > 0).reverse();
  const providerCols = active ? 1 : 0;
  const dayRows = periods.map(([b, p]) => `<tr><td>${days === 1 ? hourText(b) : monthDay(b)}</td>${active ? `<td class="num fig muted">${usd(p.cost)}</td>` : ""}<td class="num fig strong">${usd(p.cost)}</td><td class="num fig muted">${tokensText(p.tokens)}</td></tr>`).join("");
  const empty = (cols) => `<tr class="empty"><td colspan="${cols}">No activity in this window.</td></tr>`;
  const table = usageState.breakdown === "model"
    ? `<table class="by-model"><thead><tr><th class="n">#</th><th class="model">Model</th><th class="num">Cost</th><th class="num share-col">Share</th><th class="num">Tokens</th></tr></thead><tbody>${modelRows || empty(5)}</tbody></table>`
    : `<div class="table-region" role="region" aria-label="Usage breakdown by time" tabindex="0"><table class="by-time"><colgroup><col class="when"></colgroup><thead><tr><th>${days === 1 ? "Hour" : "Day"}</th>${active ? `<th class="num">Claude Code</th>` : ""}<th class="num">Total</th><th class="num">Tokens</th></tr></thead><tbody>${dayRows || empty(providerCols + 3)}</tbody></table></div>`;
  const premium = `<span class="aside">Premium <b>${usd(s.premium)}</b></span>`;
  return `<section class="overview">
      <div class="lead"><div class="headline"><div class="total fig">${total}</div>
        <p class="muted"><span><span class="fig">${sessions}</span>${cost ? " · API estimate" : ""}</span>${cost && unpricedShare > 0 ? `<button type="button" class="info" aria-label="Unpriced usage details"${popAttr(`API estimate excludes ${percentText(unpricedShare)} unpriced records.`)}>${icon("info")}</button>` : ""}</p></div>
        ${active ? `<div class="provider-row"><div class="line1"><span class="who"><span class="dot" style="background:${CLAUDE_COLOR}"></span>${icon("claude")}<span class="name"><span>Claude Code</span><small class="fig">${sessions}</small></span></span><b class="fig">${total}</b></div>
          <small class="line fig">${cost ? `${percentText(s.cost ? 1 : 0)} of cost · ${tokensText(s.tokens)} tokens` : `${percentText(s.tokens ? 1 : 0)} of tokens · ${usd(s.cost)}`}</small></div>` : ""}</div>
      <div class="chart-col"><h2>${unit} ${cost ? "cost" : "processed tokens"}</h2><div class="chart"></div></div></section>
    <section><h2>Totals</h2><dl class="tiles">
      <div><dt>Processed tokens</dt><dd class="fig">${tokensText(s.tokens)}</dd></div><div><dt>Cached input</dt><dd class="fig">${tokensText(s.cached)}</dd></div>
      <div><dt>Uncached input</dt><dd class="fig">${tokensText(s.input)}</dd></div><div><dt>Output</dt><dd class="fig">${tokensText(s.output)}</dd></div>
      <div><dt>Cache savings</dt><dd class="fig">${usd(s.savings)}</dd></div></dl></section>
    ${s.tokens > 0 ? `<section class="shares fig">${cost ? sharesHtml("Cost by type", costByType(s), usd) + (s.fast > 0 ? sharesHtml("Cost by speed", speedParts(s), usd, premium) : "") : sharesHtml("Tokens by type", tokensByType(s), tokensText)}</section>` : ""}
    <section class="breakdown"><div class="breakdown-head"><h2>Breakdown</h2>${segmented("breakdown", [["model", "Model"], ["day", days === 1 ? "Hour" : "Day"]], usageState.breakdown, "Usage breakdown")}</div>
      ${table}</section>`;
}

/** T3 Code's share-bar colors: the text color mixed into the background. */
const usageInk = (mix) => `color-mix(in oklab, var(--g-fg) ${mix}%, var(--g-bg))`;
const costByType = (s) => [["Input", s.byType.input, 60], ["Cache read", s.byType.cache_read, 30], ["Cache write", s.byType.cache_write, 72], ["Output", s.byType.output, 100]];
const tokensByType = (s) => [["Input", s.input, 60], ["Cache read", s.cached, 30], ["Cache write", s.cacheWrite, 72], ["Output", s.output, 100]];
const speedParts = (s) => [["Standard", Math.max(0, s.cost - s.fast), 34], ["Fast", s.fast, 66]];
/** T3 Code's share bar: the parts above zero in proportion, each with a tooltip, and their legend. */
function sharesHtml(title, parts, fmt, aside = "") {
  const shown = parts.filter(([, v]) => v > 0);
  const sum = shown.reduce((a, [, v]) => a + v, 0);
  if (!sum) return "";
  return `<div class="share"><div class="share-head"><h3>${title}</h3>${aside}</div>
    <div class="share-bar" role="img" aria-label="${title}: ${shown.map(([label, v]) => `${label} ${fmt(v)}`).join(", ")}">${shown.map(([label, v, mix]) =>
      `<span style="flex:${v} 1 0;background:${usageInk(mix)}"${popAttr(`${label} · ${fmt(v)} · ${percentText(v / sum)}`)}></span>`).join("")}</div>
    <div class="legend">${shown.map(([label, v, mix]) => `<span><i style="background:${usageInk(mix)}"></i>${label}<b>${fmt(v)}</b></span>`).join("")}</div></div>`;
}

const usageSkeleton = () => `<section class="overview skeleton"><div class="lead"><div class="headline"><i class="ghost" style="width:144px;height:40px"></i><i class="ghost" style="width:128px;height:16px"></i></div>
    <div class="provider-row"><div class="line1"><span class="who"><i class="ghost pill" style="width:8px;height:8px"></i><i class="ghost pill" style="width:16px;height:16px"></i><i class="ghost" style="width:80px;height:14px"></i></span><i class="ghost" style="width:56px;height:14px"></i></div><i class="ghost" style="width:144px;height:16px"></i></div></div>
    <div class="chart-col"><i class="ghost" style="width:96px;height:20px"></i><div class="ghost-chart"><i class="ghost" style="height:224px"></i><i class="ghost" style="height:16px"></i></div></div></section>
  <section><h2>Totals</h2><dl class="tiles">${["Processed tokens", "Cached input", "Uncached input", "Output", "Cache savings"].map((label) => `<div><dt>${label}</dt><dd><i class="ghost" style="width:64px;height:24px"></i></dd></div>`).join("")}</dl></section>
  <section class="shares"><div class="share"><i class="ghost" style="width:112px;height:20px"></i><i class="ghost" style="height:8px"></i><i class="ghost" style="width:288px;max-width:100%;height:16px"></i></div></section>
  <section class="breakdown"><div class="breakdown-head"><h2>Breakdown</h2><i class="ghost card" style="width:112px;height:28px"></i></div><i class="ghost" style="height:176px"></i></section>`;

/** T3 Code's Limits view: each window pooled over the accounts the selected machines sign in with,
 * from claude's own usage check, else from the statusline's report. */
const LIMIT_KIND_ORDER = { session: 0, weekly: 1, monthly: 2 };
const PACE = {
  ahead: ["trending-up", "Ahead of pace: spending faster than the window elapses"],
  on: ["gauge", "On pace with the window"],
  under: ["trending-down", "Under pace: headroom left for the rest of the window"],
};
const STATUSLINE_WINDOWS = [
  ["five_hour", "session", "Session", 300],
  ["seven_day", "weekly", "Weekly", 10080],
];
function resetsInText(resetsAt, now) {
  const left = resetsAt - now;
  if (left <= 0) return "resets now";
  if (left < 60) return "Resets momentarily";
  const d = Math.floor(left / 86400);
  const h = Math.floor((left % 86400) / 3600);
  const m = Math.floor((left % 3600) / 60);
  return `resets in ${d ? `${d}d ${h}h` : h ? `${h}h ${m}m` : `${m}m`}`;
}
/** T3 Code's `formatUpcomingTimestamp`: the time today, "tomorrow at" it, or the date and time. */
function upcomingText(unix, now) {
  const date = new Date(unix * 1000);
  const time = date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  const day = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const diff = Math.round((day(date) - day(new Date(now * 1000))) / 86400000);
  if (diff <= 0) return time;
  if (diff === 1) return `tomorrow at ${time}`;
  return `${date.toLocaleDateString(undefined, date.getFullYear() === new Date(now * 1000).getFullYear() ? { month: "numeric", day: "numeric" } : { month: "numeric", day: "numeric", year: "numeric" })} ${time}`;
}
const windowElapsed = (w, now) => (w.resets_at && w.minutes ? Math.max(0, Math.min(1, (w.minutes * 60 - (w.resets_at - now)) / (w.minutes * 60))) : null);
const windowLeft = (w) => Math.round(100 - Math.max(0, Math.min(100, w.used)));
const paceOf = (used, elapsed) => (used - elapsed * 100 > 5 ? "ahead" : used - elapsed * 100 < -5 ? "under" : "on");
const limitsOf = (m) => usageState.limits.get(m);

/** Asks each selected machine's claude for its limits; `now` moves on only when they answer, as T3 Code's does. */
async function checkLimits() {
  const machinesToCheck = usageSelected().filter(envOnline);
  for (const m of machinesToCheck) usageState.limits.set(m, { ...limitsOf(m), checking: true });
  if (shellPage() === "usage") renderUsagePage();
  await Promise.all(machinesToCheck.map(async (m) => {
    let next;
    try {
      next = { ...(await callOn(m, { cmd: "usage_limits" })), at: Date.now() / 1000 };
    } catch (e) {
      next = { ...limitsOf(m), error: String(e?.message ?? e) };
    }
    usageState.limits.set(m, { ...next, checking: false });
    usageState.limitsNow = Date.now() / 1000;
    if (shellPage() === "usage") renderUsagePage();
  }));
}

/** T3 Code's `collectLimitAccounts`: one account per signed-in email, over the machines that share it. */
function limitAccounts() {
  const accounts = new Map();
  for (const m of usageSelected()) {
    // claude's own check answers for the machine's default account; the others report through their statusline.
    const reports = machines.get(m)?.usage ?? {};
    for (const account of new Set([DEFAULT_ACCOUNT, ...Object.keys(reports)])) {
      const probe = account === DEFAULT_ACCOUNT ? limitsOf(m) : null;
      let windows = probe?.windows?.length ? probe.windows : [];
      if (!windows.length) {
        const report = reports[account] ?? {};
        windows = STATUSLINE_WINDOWS.filter(([id]) => report[id]).map(([id, kind, label, minutes]) => ({ id, kind, label, used: report[id].used_percentage, resets_at: report[id].resets_at, minutes }));
      }
      if (!windows.length) continue;
      const key = probe?.email ? `email:${probe.email.toLowerCase()}` : `machine:${m}:${account}`;
      const at = probe?.windows?.length ? probe.at : 0;
      const held = accounts.get(key);
      const environments = [...(held?.environments ?? []), account === DEFAULT_ACCOUNT ? machineName(m) : `${machineName(m)} · ${accountName(m, account)}`];
      accounts.set(key, held && held.at >= at ? { ...held, environments, plan: held.plan ?? probe?.plan } : { key, email: probe?.email, plan: probe?.plan ?? held?.plan, windows, at, environments });
    }
  }
  return [...accounts.values()];
}
/** T3 Code's `poolMember`: a spent weekly window holds the session at 100% until it can refill. */
function poolMember(account, w, now) {
  const weekly = w.kind === "session" && account.windows.find((x) => x.id === "seven_day" && x.used >= 100);
  if (!weekly) return { account, window: w, refill: w.used };
  const cutoff = Math.max(now, account.at || 0);
  let resetsAt;
  let refill = 0;
  if (weekly.resets_at > cutoff && w.resets_at > cutoff) {
    if (w.resets_at <= weekly.resets_at) [resetsAt, refill] = [weekly.resets_at, 100];
    else if (w.used >= 100) [resetsAt, refill] = [w.resets_at, 100];
    else [resetsAt, refill] = [weekly.resets_at, 100 - Math.max(0, Math.min(100, w.used))];
  }
  return { account, window: { ...w, used: 100, resets_at: resetsAt, minutes: undefined }, refill };
}
/** T3 Code's `collectLimitPools` for Claude: its windows by id, each with a column per account. */
function limitPools(accounts, now) {
  const first = accounts.flatMap((a) => a.windows).sort((x, y) => LIMIT_KIND_ORDER[x.kind] - LIMIT_KIND_ORDER[y.kind])[0];
  const resetOf = (a) => poolMember(a, a.windows.find((w) => w.id === first?.id) ?? {}, now).window.resets_at ?? Infinity;
  const sorted = [...accounts].sort((a, b) => resetOf(a) - resetOf(b) || (a.email ?? a.key).localeCompare(b.email ?? b.key));
  const byId = new Map();
  for (const a of sorted) for (const w of a.windows) byId.set(w.id, [...(byId.get(w.id) ?? []), poolMember(a, w, now)]);
  return [...byId.values()].map((members) => {
    const used = members.reduce((n, m) => n + m.window.used, 0) / members.length;
    const timed = members.flatMap((m) => (windowElapsed(m.window, now) === null ? [] : [[m.window.used, windowElapsed(m.window, now)]]));
    const resets = members.filter((m) => m.window.resets_at).map((m) => ({ member: m, at: m.window.resets_at, restores: Math.round(m.refill / members.length) })).sort((x, y) => x.at - y.at);
    return {
      ...members[0].window,
      remaining: Math.round(100 - used),
      pace: timed.length ? paceOf(timed.reduce((n, [u]) => n + u, 0) / timed.length, timed.reduce((n, [, e]) => n + e, 0) / timed.length) : null,
      columns: sorted.map((a) => members.find((m) => m.account === a) ?? { account: a, window: null }),
      resets,
    };
  }).sort((x, y) => LIMIT_KIND_ORDER[x.kind] - LIMIT_KIND_ORDER[y.kind]);
}

/** T3 Code's `redactedPlaceholder`: the email's shape, its letters scrambled until a click reveals it. */
function redactedText(value) {
  let state = 0x811c9dc5;
  for (const ch of value) state = Math.imul(state ^ ch.charCodeAt(0), 0x01000193);
  const next = () => {
    state = Math.imul(state ^ (state >>> 13), 0x85ebca6b);
    state = Math.imul(state ^ (state >>> 16), 0xc2b2ae35);
    return "abcdefghjkmnpqrstuvwxyz23456789"[Math.abs(state) % 31];
  };
  return Array.from(value, (ch) => ("@.-_".includes(ch) ? ch : next())).join("");
}
const accountInitials = (email) => {
  const [local = "", domain = ""] = email.split("@");
  return `${local[0] ?? ""}${domain[0] ?? ""}`.toUpperCase() || "?";
};
function accountChip(email) {
  let hash = 0;
  for (const ch of email) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  const hue = Math.abs(hash) % 360;
  return `<span class="account-chip" role="img" aria-label="Account ${accountInitials(email)}" style="background:oklch(0.85 0.08 ${hue});color:oklch(0.35 0.1 ${hue})">${accountInitials(email)}</span>`;
}
const limitAccountName = (a) => (a.email ? accountChip(a.email) : "Claude");

/** T3 Code's segment popover: the account, its plan and machines, then the window's figures. */
function segmentPopHtml(account, w, reset, now) {
  const timeLeft = windowElapsed(w, now) === null ? null : Math.round((1 - windowElapsed(w, now)) * 100);
  const row = (label, value) => `<div class="pop-row"><span>${label}</span><span>${value}</span></div>`;
  return `<div class="segment-pop"><div class="who"><span class="name">${icon("claude")}<span>Claude</span></span>${
    account.email ? `<button type="button" class="redacted" data-email="${esc(account.email)}" aria-label="Toggle account email visibility" title="Click to reveal email">${esc(redactedText(account.email))}</button>` : ""}</div>
    ${account.plan || account.environments.length ? `<div class="pop-rows">${account.plan ? row("Plan", esc(account.plan)) : ""}${row("Signed in", esc(account.environments.join(", ")))}</div>` : ""}
    <div class="pop-rows">${row("Left", `${windowLeft(w)}%`)}${w.resets_at ? row("Resets", `${upcomingText(w.resets_at, now)}${w.resets_at > now + 59 ? ` · ${resetsInText(w.resets_at, now).replace("resets in ", "in ")}` : ""}`) : ""}${
      timeLeft === null ? "" : row("Even pace", `${timeLeft}% left by now`)}${reset?.restores > 0 ? row("Restores", `+${reset.restores}% of pool`) : ""}</div></div>`;
}
function limitCardHtml(pool, now) {
  const many = pool.columns.length > 1;
  const refill = pool.resets.find((r) => r.restores > 0);
  const [paceIcon, paceLabel] = pool.pace ? PACE[pool.pace] : [];
  const segments = pool.columns.map((member, i) => {
    if (!member.window) return "";
    const w = member.window;
    const left = windowLeft(w);
    const elapsed = windowElapsed(w, now);
    const timeLeft = elapsed === null ? null : Math.round((1 - elapsed) * 100);
    const at = `clamp(3px, ${timeLeft}%, calc(100% - 3px))`;
    const gap = timeLeft === null ? "" : `background:linear-gradient(to right, var(--usage-track) calc(${at} - 3px), transparent calc(${at} - 3px), transparent calc(${at} + 3px), var(--usage-track) calc(${at} + 3px))`;
    const clip = timeLeft === null ? "" : `clip-path:polygon(-1px -1px, calc(${at} - 3px) -1px, calc(${at} - 3px) calc(100% + 1px), calc(${at} + 3px) calc(100% + 1px), calc(${at} + 3px) -1px, calc(100% + 1px) -1px, calc(100% + 1px) calc(100% + 1px), -1px calc(100% + 1px))`;
    const resets = w.resets_at ? resetsInText(w.resets_at, now) : "";
    const pop = popAttr(segmentPopHtml(member.account, w, pool.resets.find((r) => r.member === member), now));
    const label = `${member.account.email ? accountInitials(member.account.email) : "Claude"}: ${left}% left${resets ? `, ${resets}` : ""}${timeLeft === null ? "" : `, ${timeLeft}% of the window left`}`;
    return `<button type="button" class="segment" style="grid-column:${i + 1};grid-row:1;${gap}" aria-label="${esc(label)}"${pop}>
        <span class="clip"><span class="cut" style="${clip}"><span class="fill" style="width:${left}%;background:${CLAUDE_COLOR}"></span>${
          left < 100 && member.refill > 0 && pool.resets.some((r) => r.member === member) ? `<span class="refill" style="width:${Math.min(100 - left, member.refill)}%;--refill:${CLAUDE_COLOR}"></span>` : ""}</span></span>
        ${timeLeft === null ? "" : `<span class="tick" style="left:${at}"></span>`}<span class="index">${i + 1}</span>
        <span class="inside">${many ? `<span class="who">${limitAccountName(member.account)}</span>` : ""}<b>${left}%</b>${resets ? `<span class="chip">${resets.replace("resets in ", "↻ ")}</span>` : ""}</span></button>
      <button type="button" class="legend-row" style="grid-column:1 / -1;grid-row:${i + 2}"${pop}><span class="index-chip"><span style="background:${CLAUDE_COLOR}"></span>${i + 1}</span><span class="who">${limitAccountName(member.account)}</span><b>${left}%</b><span class="chip">${resets.replace("resets in ", "↻ ")}</span></button>`;
  }).join("");
  return `<div class="limit-card" data-window="${esc(pool.id)}"><div class="limit-text"><span class="label">${esc(pool.label)}</span>
      <span class="figure"><b>${pool.remaining}%</b><span class="muted">left${pool.pace ? ` <span class="pace" role="img" aria-label="${paceLabel}"${popAttr(paceLabel)}>${icon(paceIcon)}</span>` : ""}</span></span>
      ${refill && many ? `<span class="refill-note">↻ +${refill.restores}%</span>` : ""}</div>
    <div class="pool"><div class="segments" style="grid-template-columns:repeat(${pool.columns.length}, minmax(0, 1fr))">${segments}</div></div></div>`;
}
function limitsHtml() {
  const now = usageState.limitsNow || Date.now() / 1000;
  const pools = limitPools(limitAccounts(), now);
  const notices = usageSelected().flatMap((m) => {
    const probe = limitsOf(m);
    const text = probe?.error ? `Could not read limits: ${probe.error}` : probe?.unavailable && !machines.get(m)?.usage?.[DEFAULT_ACCOUNT]?.five_hour && !machines.get(m)?.usage?.[DEFAULT_ACCOUNT]?.seven_day ? probe.unavailable : "";
    return text ? [usageSelected().length > 1 ? `${machineName(m)} · ${text}` : text] : [];
  });
  return `<div class="limits-view">${pools.length || notices.length ? "" : `<p class="muted note">No provider on the selected environments reports subscription limits.</p>`}
    ${pools.length ? `<section class="limits"><h2>${icon("claude")}Claude</h2>${pools.map((pool) => limitCardHtml(pool, now)).join("")}</section>` : ""}
    ${notices.length ? `<div class="limit-notices" role="alert">${icon("triangle-alert")}<div>${[...new Set(notices)].map((n) => `<p>${esc(n)}</p>`).join("")}</div></div>` : ""}</div>`;
}
document.addEventListener("click", (e) => {
  const email = e.target.closest?.("#usage-pop .redacted");
  if (!email) return;
  const shown = email.textContent === email.dataset.email;
  email.textContent = shown ? redactedText(email.dataset.email) : email.dataset.email;
  email.title = shown ? "Click to reveal email" : "Click to hide email";
  email.classList.toggle("revealed", !shown);
});

/** T3 Code's chart geometry: a 960 by 260 view, four tick steps, and room above the top gridline. */
const CHART_W = 960;
const CHART_H = 260;
const CHART_TOP = 8;
function niceScale(peak) {
  if (peak <= 0) return { max: 0, ticks: [0] };
  const raw = peak / 4;
  const magnitude = 10 ** Math.floor(Math.log10(raw));
  const normalized = raw / magnitude;
  const step = (normalized > 5 ? 10 : normalized > 2 ? 5 : normalized > 1 ? 2 : 1) * magnitude;
  const max = Math.ceil(peak / step) * step;
  const ticks = [];
  for (let v = 0; v <= max + step * 1e-6; v += step) ticks.push(v);
  return { max, ticks };
}
/** T3 Code's monotone curve: averaged tangents, flattened at turns and limited so the curve can't overshoot. */
function monotonePath(points) {
  const n = points.length;
  if (n < 2) return "";
  const slopes = points.slice(1).map(([x, y], i) => (x === points[i][0] ? 0 : (y - points[i][1]) / (x - points[i][0])));
  const t = points.map((_, i) => (i === 0 ? slopes[0] : i === n - 1 ? slopes[n - 2] : slopes[i - 1] * slopes[i] <= 0 ? 0 : (slopes[i - 1] + slopes[i]) / 2));
  for (let i = 0; i < n - 1; i++) {
    if (slopes[i] === 0) {
      t[i] = t[i + 1] = 0;
      continue;
    }
    const a = t[i] / slopes[i];
    const b = t[i + 1] / slopes[i];
    const m = a * a + b * b;
    if (m > 9) {
      t[i] = (3 / Math.sqrt(m)) * a * slopes[i];
      t[i + 1] = (3 / Math.sqrt(m)) * b * slopes[i];
    }
  }
  const f = (v) => v.toFixed(2);
  let d = `M${f(points[0][0])},${f(points[0][1])}`;
  for (let i = 0; i < n - 1; i++) {
    const [x0, y0] = points[i];
    const [x1, y1] = points[i + 1];
    const dx = (x1 - x0) / 3;
    d += ` C${f(x0 + dx)},${f(y0 + t[i] * dx)} ${f(x1 - dx)},${f(y1 - t[i + 1] * dx)} ${f(x1)},${f(y1)}`;
  }
  return d;
}
function drawChart(holder, s, metric, loading = false) {
  if (!holder) return;
  const cost = metric === "cost";
  const values = s.series.map((p) => (cost ? p.cost : p.tokens));
  const peak = Math.max(0, ...values);
  const active = peak > 0 || s.rows.length > 0;
  const placeholder = peak === 0 && loading;
  const scale = placeholder ? { max: 4, ticks: [0, 1, 2, 3, 4] } : niceScale(peak);
  const y = (v) => (scale.max === 0 ? CHART_H : CHART_H - (v / scale.max) * (CHART_H - CHART_TOP));
  const step = values.length < 2 ? 0 : CHART_W / (values.length - 1);
  const line = active ? monotonePath(values.map((v, i) => [i * step, y(v)])) : "";
  const fmt = (v) => (cost ? usd(v) : tokensText(v));
  const label = (b) => (usageState.days === 1 ? hourText(b) : monthDay(b));
  const mid = Math.floor(s.buckets.length / 2);
  const kind = `${usageState.days === 1 ? "Hourly" : "Daily"} ${cost ? "cost" : "processed tokens"} by provider`;
  holder.innerHTML = `<div class="y-axis">${placeholder ? "" : scale.ticks.map((t) => `<span style="top:${((y(t) / CHART_H) * 100).toFixed(3)}%">${t === 0 ? "0" : fmt(t)}</span>`).join("")}</div>
    <div class="plot"><svg viewBox="0 0 ${CHART_W} ${CHART_H}" preserveAspectRatio="none" role="img" aria-label="${kind}">
      ${scale.ticks.map((t) => `<line class="grid" x1="0" x2="${CHART_W}" y1="${y(t)}" y2="${y(t)}"/>`).join("")}
      ${line ? `<path class="area series" d="${line} L${CHART_W},${CHART_H} L0,${CHART_H} Z" fill="${CLAUDE_COLOR}"/><path class="line series" d="${line}" stroke="${CLAUDE_COLOR}"/>` : ""}
      <line class="cursor" y1="${CHART_TOP}" y2="${CHART_H}" visibility="hidden"/></svg><div class="tip" hidden></div></div>
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
    if (!r.width || !values.length) return;
    const px = Math.min(r.width, Math.max(0, e.clientX - r.left));
    const py = Math.min(r.height, Math.max(0, e.clientY - r.top));
    const i = Math.min(values.length - 1, Math.max(0, Math.round((px / r.width) * (values.length - 1))));
    cursor.setAttribute("x1", i * step);
    cursor.setAttribute("x2", i * step);
    cursor.setAttribute("visibility", "visible");
    tip.hidden = false;
    tip.innerHTML = `<p>${when(s.buckets[i])}</p>${active ? `<p class="row"><span>${icon("claude")}Claude Code</span><b${loading ? ` class="muted"` : ""}>${fmt(values[i])}</b></p>` : ""}<p class="row sum"><span>Total</span><b${loading ? ` class="muted"` : ""}>${fmt(values[i])}</b></p>`;
    const [w, h] = [tip.offsetWidth, tip.offsetHeight];
    const left = px + 12 + w <= r.width ? px + 12 : px - 12 - w;
    const top = py + 12 + h <= r.height ? py + 12 : py - 12 - h;
    tip.style.left = `${Math.min(Math.max(0, left), Math.max(0, r.width - w))}px`;
    tip.style.top = `${Math.min(Math.max(0, top), Math.max(0, r.height - h))}px`;
  };
  plot.onpointerleave = () => {
    cursor.setAttribute("visibility", "hidden");
    tip.hidden = true;
  };
}

function setUsage(change) {
  const toLimits = change.metric === "limits" && usageState.metric !== "limits";
  Object.assign(usageState, change);
  if (toLimits || (change.selected !== undefined && usageState.metric === "limits")) checkLimits();
  saveUsagePrefs();
  renderUsagePage();
}
/** Refresh re-reads the transcripts, or on Limits checks the limits again, spinning until it's done. */
async function refreshUsage() {
  if (usageState.refreshing) return;
  usageState.refreshing = true;
  try {
    await (usageState.metric === "limits" ? checkLimits() : fetchUsage());
  } finally {
    usageState.refreshing = false;
    if (shellPage() === "usage") renderUsagePage();
  }
}
function envFilterMenu() {
  openUsageMenu("#page-view .env-filter", () => {
    const envs = usageEnvs();
    const status = usageState.metric !== "limits";
    const selected = usageSelected();
    const failed = selected.filter((m) => usageState.envs.get(m)?.error);
    return [
      { label: "All environments", box: true, checked: !usageState.selected, keep: true, run: () => setUsage({ selected: usageState.selected ? null : new Set() }) },
      { sep: true },
      ...envs.map((m) => ({ label: machineName(m), box: true, checked: !usageState.selected || usageState.selected.has(m), keep: true,
        status: status ? envStatus(m) : "", tone: usageState.envs.get(m)?.error ? "error" : "",
        run: () => {
          const next = new Set(usageSelected());
          if (next.has(m)) next.delete(m);
          else next.add(m);
          setUsage({ selected: next.size === envs.length ? null : next });
        } })),
      ...(envs.length ? [] : [{ note: "No environments connected." }]),
      ...(status && usagePartial() ? [{ note: "Totals are partial while selected environments scan." }] : []),
      ...(status && failed.length ? [{ notes: true, note: failed.map((m) => `<span>${esc(machineName(m))}: ${esc(usageState.envs.get(m).error)}</span>`).join("") }] : []),
      { sep: true },
      { label: "Model prices", icon: "sliders-horizontal", run: () => openPrices() },
    ];
  });
}
pageView.addEventListener("click", (e) => {
  if (shellPage() !== "usage") return;
  const seg = e.target.closest(".seg button");
  if (seg) {
    const name = seg.parentElement.dataset.seg;
    return setUsage({ [name]: name === "days" ? Number(seg.dataset.value) : seg.dataset.value });
  }
  const pick = e.target.closest(".pick");
  if (pick) {
    const [options, current] = pick.dataset.pick === "days" ? [periodOptions(), usageState.days] : [metricOptions(), usageState.metric];
    const selector = `#page-view .pick[data-pick="${pick.dataset.pick}"]`;
    if (!usageMenu.hidden && usageMenu.selector === selector) return closeUsageMenu();
    return openUsageMenu(selector, () => options.map(([value, label, title]) => ({ label, title, checked: value === current, run: () => setUsage({ [pick.dataset.pick]: value }) })), "end");
  }
  if (e.target.closest(".refresh")) return refreshUsage();
  if (e.target.closest(".env-filter")) return usageMenu.hidden ? envFilterMenu() : closeUsageMenu();
  const model = e.target.closest("tr[data-model]");
  if (model) openModel(model.dataset.model);
});
/** T3 Code's page keys: C, T and L pick the metric and ⌘⇧1-4 the period, except in a field, a dialog or a popup. */
window.addEventListener("keydown", (e) => {
  if (!guiOn() || shellPage() !== "usage" || e.defaultPrevented || e.repeat || e.isComposing || !paletteEl.hidden || dialogOpen() || !usageMenu.hidden || !sideMenu.hidden || settingsOpen()) return;
  if (e.target.closest?.('input, textarea, select, [contenteditable="true"], [role="dialog"], [aria-modal="true"]')) return;
  const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  const period = ["Digit1", "Digit2", "Digit3", "Digit4"].indexOf(e.code);
  const metric = !mod && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && USAGE_METRICS.find(([, , key]) => key === e.key.toLowerCase())?.[0];
  if (metric) setUsage({ metric });
  else if (mod && e.shiftKey && !e.altKey && period >= 0) {
    if (usageState.metric !== "limits") setUsage({ days: USAGE_PERIODS[period][0] });
  } else return;
  e.preventDefault();
  e.stopPropagation();
}, true);

/** A model's own numbers, as T3 Code's model dialog shows them. */
function openModel(model) {
  const metric = usageState.metric === "limits" ? "cost" : usageState.metric;
  const all = summarize();
  const s = summarize(model);
  const m = s.models.get(model) ?? { records: 0, unpriced: 0 };
  const unknown = costUnknown(m);
  const pricedTokens = s.tokens - s.unpricedTokens;
  const inputs = s.input + s.cached + s.cacheWrite;
  const stats = [
    ["Cost", unknown ? "Unpriced" : usd(s.cost)],
    ["Tokens", tokensText(s.tokens)],
    !unknown && pricedTokens > 0 && ["Per 1M tokens", usd((s.cost / pricedTokens) * 1e6)],
    inputs > 0 && ["Cache hit", percentText(s.cached / inputs)],
  ].filter(Boolean);
  showDialog(`<div class="dialog-head"><h2>${icon("claude")}<span>${esc(model)}</span></h2><p class="muted">Claude Code${unknown ? "" : ` · ${percentText(all.cost ? s.cost / all.cost : 0)} of cost`}</p></div>
    <div class="dialog-body usage"><dl class="stats">${stats.map(([label, value]) => `<div><dt>${label}</dt><dd>${value}</dd></div>`).join("")}</dl>
      <div class="chart"></div>
      <div class="dialog-shares">${unknown ? "" : sharesHtml("Cost by type", costByType(s), usd)}${sharesHtml("Tokens by type", tokensByType(s), tokensText)}${
        s.fast > 0 ? sharesHtml("Cost by speed", speedParts(s), usd, `<span class="aside">Premium <b>${usd(s.premium)}</b></span>`) : ""}</div></div>
    ${s.unpricedTokens > 0 ? `<div class="dialog-foot"><span class="muted">${tokensText(s.unpricedTokens)} tokens have no known price</span><button type="button" class="primary" data-set-price>Set price</button></div>` : ""}`);
  shellDialog.dataset.kind = "model";
  drawChart(shellDialog.querySelector(".chart"), s, unknown ? "tokens" : metric);
  shellDialog.querySelector("[data-set-price]")?.addEventListener("click", () => openPrices(model));
}

/** T3 Code's custom model prices: per machine, in USD per million tokens, with "Map to" counting one
 * model's usage as another's. Each machine keeps its own; the dialog edits the ones "Apply to" picks. */
const PRICE_FIELDS = [
  ["input", "Input", false],
  ["output", "Output", false],
  ["cache_read", "Cache read", true],
  ["cache_write", "Cache write", true],
];
const priceForm = (price) => Object.fromEntries(PRICE_FIELDS.map(([k]) => [k, price?.[k] === undefined || price?.[k] === null ? "" : String(price[k])]));
/** T3 Code's `isUsagePriceEdit`: digits and a point, or an exponent; deleting a run is always allowed. */
function isPriceEdit(previous, next) {
  if (/^\s*(?:\d*\.?\d*|(?:\d+\.?\d*|\.\d+)e[+-]?\d*)\s*$/i.test(next)) return true;
  if (next.length >= previous.length) return false;
  let start = 0;
  while (start < next.length && next[start] === previous[start]) start++;
  return next.slice(start) === previous.slice(previous.length - (next.length - start));
}
/** Blank cache prices use the input rate; explicit zero means free. */
function parsePriceForm(form) {
  const price = {};
  for (const [k, , optional] of PRICE_FIELDS) {
    const raw = form[k].trim();
    if (!raw) {
      if (optional) continue;
      return null;
    }
    if (!/^(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(raw) || !Number.isFinite(Number(raw)) || Number(raw) < 0) return null;
    price[k] = Number(raw);
  }
  return price;
}
const modelPrice = (target, model) => (target.prices && Object.hasOwn(target.prices, model) ? target.prices[model] : undefined);
const modelAlias = (target, model) => (target.aliases && Object.hasOwn(target.aliases, model) ? target.aliases[model] : undefined);
function aliasCell(targets, model) {
  if (targets.some((t) => t.aliases === null)) return { value: "", placeholder: "Unavailable" };
  const values = targets.map((t) => modelAlias(t, model) ?? "");
  if (values.some((v) => v !== values[0])) return { value: "", placeholder: "Mixed" };
  return { value: values[0] ?? "", placeholder: "None" };
}
function priceCell(targets, model, field) {
  const optional = PRICE_FIELDS.find(([k]) => k === field)[2];
  if (targets.some((t) => t.prices === null)) return { value: "", placeholder: "Unavailable" };
  const values = targets.map((t) => (modelPrice(t, model) ? priceForm(modelPrice(t, model))[field] : null));
  if (values.some((v) => v !== values[0])) return { value: "", placeholder: "Mixed" };
  return { value: values[0] ?? "", placeholder: values[0] === null || values[0] === undefined ? "Automatic" : optional ? "Input rate" : "0.00" };
}
const emptyDraft = (d) => d.isNew && !d.model.trim() && !(d.alias ?? "").trim() && Object.values(d.values).every((v) => !v.trim());
/** T3 Code's `usagePriceTableChanges`: what saving `drafts` writes to one machine, as `set_usage_prices`
 * takes it, and the rows it can't save there. */
function priceChanges(target, drafts) {
  const patch = { prices: {}, aliases: {} };
  const errors = new Map();
  for (const draft of drafts) {
    if (emptyDraft(draft)) continue;
    const model = draft.model.trim();
    const currentAlias = modelAlias(target, model);
    if (draft.removed) {
      if (modelPrice(target, model)) patch.prices[model] = null;
      if (currentAlias !== undefined) patch.aliases[model] = null;
      continue;
    }
    const alias = (draft.alias ?? currentAlias ?? "").trim();
    if (alias) {
      if (!model) errors.set(draft.id, "Enter a model ID.");
      else if (alias === model) errors.set(draft.id, "Map to a different model.");
      else {
        if (alias !== currentAlias) patch.aliases[model] = alias;
        if (modelPrice(target, model)) patch.prices[model] = null;
      }
      continue;
    }
    if (currentAlias !== undefined) {
      patch.aliases[model] = null;
      if (!modelPrice(target, model) && Object.values(draft.values).every((v) => !v.trim())) continue;
    }
    const original = priceForm(modelPrice(target, model));
    const form = { ...original, ...draft.values };
    const parsed = model ? parsePriceForm(form) : null;
    if (!parsed) {
      const missing = PRICE_FIELDS.find(([k, , optional]) => !optional && !form[k].trim());
      errors.set(draft.id, !model ? "Enter a model ID." : missing ? `${missing[1]} is required on ${target.label}.` : "Use non-negative numbers for prices.");
      continue;
    }
    const next = priceForm(parsed);
    if (PRICE_FIELDS.some(([k]) => original[k] !== next[k])) patch.prices[model] = parsed;
  }
  return { patch, errors, empty: !Object.keys(patch.prices).length && !Object.keys(patch.aliases).length };
}

/** The open prices dialog: its machines, what's been typed, and the last save's results. */
const priceDialog = { targets: [], selected: null, drafts: [], pending: false, attempt: null, next: 0, focus: null };
async function loadPriceTargets() {
  priceDialog.targets = await Promise.all(usageEnvs().map(async (machine) => {
    const target = { machine, label: machineName(machine), prices: null, aliases: null, unavailable: null };
    if (!envOnline(machine)) return { ...target, unavailable: "Offline" };
    try {
      const book = await callOn(machine, { cmd: "usage_prices" });
      return { ...target, prices: book.prices ?? {}, aliases: book.aliases ?? {} };
    } catch (e) {
      return { ...target, unavailable: /can't answer that/.test(String(e)) ? "Update server to edit prices" : "Prices not loaded" };
    }
  }));
}
const priceSelected = () => priceDialog.targets.filter((t) => !priceDialog.selected || priceDialog.selected.has(t.machine));

async function openPrices(initial) {
  Object.assign(priceDialog, { selected: usageState.selected && new Set(usageState.selected), drafts: [], pending: false, attempt: null, focus: null });
  showDialog(`<div class="dialog-head"><h2><span>Custom model prices</span></h2><p class="muted">Prices and mappings apply to all past and future usage on the environments you select.</p></div><div class="dialog-body prices-body"><p class="muted">Loading prices…</p></div>`);
  shellDialog.dataset.kind = "prices";
  await loadPriceTargets();
  if (shellDialog.dataset.kind !== "prices") return;
  const selected = priceSelected();
  if (initial && !selected.some((t) => Object.hasOwn(t.prices ?? {}, initial) || Object.hasOwn(t.aliases ?? {}, initial))) {
    priceDialog.drafts = [{ id: "new:initial", model: initial, isNew: true, values: {} }];
    priceDialog.focus = { id: "new:initial", field: "input" };
  }
  drawPrices();
}

function priceRows() {
  const selected = priceSelected();
  const custom = [...new Set(selected.flatMap((t) => [...Object.keys(t.prices ?? {}), ...Object.keys(t.aliases ?? {})]))].sort();
  const newModels = new Set(priceDialog.drafts.filter((d) => d.isNew).map((d) => d.model.trim()));
  const rows = [
    ...custom.filter((m) => !priceDialog.attempt || !newModels.has(m)).map((model) => priceDialog.drafts.find((d) => d.id === `model:${model}`) ?? { id: `model:${model}`, model, isNew: false, values: {} }),
    ...priceDialog.drafts.filter((d) => d.isNew),
  ];
  const staged = priceDialog.drafts.filter((d) => !emptyDraft(d));
  const errors = new Map(selected.filter((t) => !t.unavailable && t.prices).flatMap((t) => [...priceChanges(t, staged).errors]));
  for (const d of staged) {
    if (!d.isNew) continue;
    if (!d.model.trim()) errors.set(d.id, "Enter a model ID.");
    else if (!priceDialog.attempt && (custom.includes(d.model.trim()) || priceDialog.drafts.some((o) => o.id !== d.id && o.model.trim() === d.model.trim()))) errors.set(d.id, "This model already has a row. Edit its prices there.");
  }
  return { selected, custom, rows, staged, errors };
}

function drawPrices() {
  const body = shellDialog.querySelector(".prices-body");
  if (!body) return;
  const { selected, custom, rows, staged, errors } = priceRows();
  const failed = priceDialog.attempt?.destinations.filter((d) => priceDialog.attempt.results.get(d.machine)?.status === "failed") ?? [];
  const locked = priceDialog.pending || failed.length > 0;
  const hasChanges = staged.length > 0;
  const destination = selected.length === 1 ? selected[0].label : `${selected.length} environments`;
  const usedModels = [...new Set(selected.flatMap((t) => usageState.envs.get(t.machine)?.rows.map((r) => r.model) ?? []))];
  const models = [...new Set([...custom, ...usedModels])].sort();
  const dis = locked ? " disabled" : "";
  const row = (r) => {
    const alias = aliasCell(selected, r.model);
    const mapped = (r.alias ?? alias.value).trim();
    const shownError = errors.has(r.id) && (r.model.trim() || Object.values(r.values).some((v) => v));
    const model = r.isNew
      ? `<input type="text" data-field="model" value="${esc(r.model)}" aria-label="New model ID"${shownError && r.model.trim() ? ` aria-invalid="true"` : ""} list="usage-price-models" placeholder="Model ID" autocomplete="off" spellcheck="false"${dis}>`
      : `<span class="model-id${r.removed ? " removed" : ""}">${esc(r.model)}</span>`;
    const cells = r.removed
      ? `<td colspan="5"><span class="muted">Automatic pricing after saving</span></td>`
      : `<td><input type="text" data-field="alias" value="${esc(r.alias ?? alias.value)}" aria-label="Map ${esc(r.model || "new model")} to model" list="usage-alias-models" placeholder="${r.isNew ? "Optional" : alias.placeholder}" autocomplete="off" spellcheck="false"${dis}></td>${
        mapped
          ? `<td colspan="4"><span class="muted counted">Counted as ${esc(mapped)}</span></td>`
          : PRICE_FIELDS.map(([k, label, optional]) => {
              const cell = priceCell(selected, r.model, k);
              return `<td><input type="text" class="mono" inputmode="decimal" data-field="${k}" value="${esc(r.values[k] ?? (r.isNew ? "" : cell.value))}" aria-label="${label} price for ${esc(r.model || "new model")}" placeholder="${r.isNew ? (optional ? "Input rate" : "0.00") : cell.placeholder}" autocomplete="off"${dis}></td>`;
            }).join("")}`;
    const [act, tip] = r.removed ? [`Undo reset for ${r.model}`, "Undo reset"] : r.isNew ? ["Remove new model", "Remove row"] : [`Reset price for ${r.model} to automatic`, "Reset to automatic"];
    return `<tr data-row="${esc(r.id)}"><td>${model}${shownError ? `<p role="alert" class="row-error">${esc(errors.get(r.id))}</p>` : ""}</td>${cells}
      <td><button type="button" class="icon" data-reset aria-label="${esc(act)}"${popAttr(tip)}${dis}>${icon(r.isNew ? "x" : "rotate-ccw")}</button></td></tr>`;
  };
  const table = !selected.length
    ? `<p class="muted">${priceDialog.targets.length ? "Select an environment to see and change its model prices." : "Connect an environment to set model prices."}</p>`
    : `<div class="table-scroll"><table class="prices" aria-label="Custom model prices"><colgroup><col class="model"><col class="alias">${PRICE_FIELDS.map(() => "<col>").join("")}<col class="act"></colgroup>
        <thead><tr><th>Model ID</th><th>Map to</th>${PRICE_FIELDS.map(([, label]) => `<th>${label}</th>`).join("")}<th><button type="button" class="icon" data-add aria-label="Add model price"${dis}>${icon("plus")}</button></th></tr></thead>
        <tbody>${rows.length ? rows.map(row).join("") : `<tr class="none"><td colspan="7">${selected.some((t) => t.prices === null) ? "Some environment prices are unavailable." : "No custom prices or mappings. Add a row to set one."}</td></tr>`}</tbody></table></div>
      <datalist id="usage-price-models">${models.filter((m) => !custom.includes(m)).map((m) => `<option value="${esc(m)}">`).join("")}</datalist>
      <datalist id="usage-alias-models">${models.filter((m) => selected.every((t) => !Object.hasOwn(t.aliases ?? {}, m))).map((m) => `<option value="${esc(m)}">`).join("")}</datalist>
      <p class="muted hint">Blank cache rates use the input price. Enter 0 for free tokens. A mapped model’s usage counts as the model it maps to.${selected.length > 1 ? " Mixed cells keep each environment’s rate until you edit them." : ""}</p>`;
  const status = priceDialog.attempt
    ? `<div role="status" class="save-status">${priceDialog.attempt.destinations.map((d) => {
        const result = priceDialog.attempt.results.get(d.machine);
        return `<div><span class="muted">${esc(d.label)}</span><span class="${result?.status === "failed" ? "failed" : "muted"}">${result?.status === "failed" ? `Not saved · ${esc(result.error)}` : result?.status === "saved" ? "Saved" : "Saving…"}</span></div>`;
      }).join("")}</div>`
    : "";
  const focused = document.activeElement?.closest?.("#shell-dialog [data-row] input, #shell-dialog .apply-to");
  const keep = priceDialog.focus ?? (focused?.dataset.field ? { id: focused.closest("[data-row]").dataset.row, field: focused.dataset.field, start: focused.selectionStart, end: focused.selectionEnd } : null);
  priceDialog.focus = null;
  body.innerHTML = `<div class="apply-row"><div class="apply"><span id="usage-prices-apply-label">Apply to</span><button type="button" class="outline apply-to" aria-labelledby="usage-prices-apply-label usage-prices-selection"${priceDialog.pending || hasChanges ? " disabled" : ""}><span id="usage-prices-selection">${esc(!priceDialog.selected ? "All environments" : destination)}</span>${icon("chevron-down")}</button></div><span class="muted unit">USD / million tokens</span></div>
    ${table}${status}`;
  let foot = shellDialog.querySelector(".dialog-foot");
  if (!foot) {
    shellDialog.querySelector(".dialog-body").insertAdjacentHTML("afterend", `<div class="dialog-foot"></div>`);
    foot = shellDialog.querySelector(".dialog-foot");
  }
  foot.innerHTML = `<span class="muted">${hasChanges ? `Changes apply to ${esc(destination)}` : ""}</span><span class="buttons">${hasChanges ? `<button type="button" class="ghost" data-discard${priceDialog.pending ? " disabled" : ""}>${failed.length ? "Discard pending changes" : "Discard changes"}</button>` : ""}<button type="button" class="primary" data-save${
    priceDialog.pending || (!hasChanges && !failed.length) || (!failed.length && (errors.size > 0 || !selected.length)) ? " disabled" : ""}>${priceDialog.pending ? "Saving…" : failed.length ? "Retry failed saves" : "Save changes"}</button></span>`;
  if (keep) {
    const input = shellDialog.querySelector(`[data-row="${CSS.escape(keep.id)}"] [data-field="${keep.field}"]`);
    input?.focus();
    if (input && keep.start !== undefined) input.setSelectionRange(keep.start, keep.end);
  }
}

function updateDraft(draft) {
  priceDialog.drafts = priceDialog.drafts.some((d) => d.id === draft.id) ? priceDialog.drafts.map((d) => (d.id === draft.id ? draft : d)) : [...priceDialog.drafts, draft];
  priceDialog.attempt = null;
}
function updateOrDropDraft(draft) {
  if (!draft.isNew && !Object.keys(draft.values).length && draft.alias === undefined && !draft.removed) priceDialog.drafts = priceDialog.drafts.filter((d) => d.id !== draft.id);
  else updateDraft(draft);
}
const rowDraft = (id) => priceRows().rows.find((r) => r.id === id);
const matchesSaved = (row, cell, value) => !row.isNew && cell.placeholder !== "Mixed" && cell.placeholder !== "Unavailable" && value === cell.value;

async function savePrices(retry) {
  if (priceDialog.pending) return;
  const { selected, staged } = priceRows();
  const failed = priceDialog.attempt?.destinations.filter((d) => priceDialog.attempt.results.get(d.machine)?.status === "failed") ?? [];
  const destinations = retry ? failed.map((d) => priceDialog.targets.find((t) => t.machine === d.machine) ?? { ...d, prices: null, aliases: null, unavailable: "Environment removed" }) : selected;
  const edits = retry ? priceDialog.attempt.drafts : staged;
  if (!destinations.length || !edits.length) return;
  priceDialog.pending = true;
  const previous = retry ? priceDialog.attempt.results : new Map();
  priceDialog.attempt = { drafts: edits, destinations: retry ? priceDialog.attempt.destinations : destinations.map((t) => ({ machine: t.machine, label: t.label })), results: new Map([...previous, ...destinations.map((t) => [t.machine, { status: "saving" }])]) };
  drawPrices();
  let anyFailed = false;
  await Promise.all(destinations.map(async (target) => {
    const plan = priceChanges(target, edits);
    let result;
    const unavailable = target.unavailable ?? [...plan.errors.values()][0];
    if (unavailable) result = { status: "failed", error: unavailable };
    else if (plan.empty) result = { status: "saved" };
    else {
      try {
        const book = await callOn(target.machine, { cmd: "set_usage_prices", ...plan.patch });
        Object.assign(target, { prices: book.prices ?? {}, aliases: book.aliases ?? {} });
        result = { status: "saved" };
      } catch {
        result = { status: "failed", error: "Could not save. Try again." };
      }
    }
    if (result.status === "failed") anyFailed = true;
    priceDialog.attempt.results.set(target.machine, result);
    if (shellDialog.dataset.kind === "prices") drawPrices();
  }));
  priceDialog.pending = false;
  if (!anyFailed) priceDialog.drafts = [];
  if (shellDialog.dataset.kind === "prices") drawPrices();
  fetchUsage();
}

shellDialog.addEventListener("input", (e) => {
  if (shellDialog.dataset.kind !== "prices") return;
  const input = e.target.closest("[data-row] input[data-field]");
  if (!input) return;
  const row = rowDraft(input.closest("[data-row]").dataset.row);
  const field = input.dataset.field;
  const { selected } = priceRows();
  if (field === "model") updateDraft({ ...row, model: input.value });
  else if (field === "alias") {
    const { alias: _, ...rest } = row;
    updateOrDropDraft(matchesSaved(row, aliasCell(selected, row.model), input.value) ? rest : { ...rest, alias: input.value });
  } else {
    const before = row.values[field] ?? (row.isNew ? "" : priceCell(selected, row.model, field).value);
    if (!isPriceEdit(before, input.value)) {
      input.value = before;
      return;
    }
    const values = { ...row.values, [field]: input.value };
    if (matchesSaved(row, priceCell(selected, row.model, field), input.value)) delete values[field];
    updateOrDropDraft({ ...row, values });
  }
  drawPrices();
});
shellDialog.addEventListener("click", (e) => {
  if (shellDialog.dataset.kind !== "prices") return;
  if (e.target.closest("[data-add]")) {
    const id = `new:${priceDialog.next++}`;
    updateDraft({ id, model: "", isNew: true, values: {} });
    priceDialog.focus = { id, field: "model" };
    return drawPrices();
  }
  const reset = e.target.closest("[data-reset]");
  if (reset) {
    const row = rowDraft(reset.closest("[data-row]").dataset.row);
    hideUsagePop();
    if (row.isNew) priceDialog.drafts = priceDialog.drafts.filter((d) => d.id !== row.id);
    else if (row.removed) updateOrDropDraft({ ...row, removed: false });
    else updateDraft({ ...row, removed: true });
    return drawPrices();
  }
  if (e.target.closest("[data-discard]")) {
    Object.assign(priceDialog, { drafts: [], attempt: null });
    return drawPrices();
  }
  const save = e.target.closest("[data-save]");
  if (save) return savePrices(save.textContent === "Retry failed saves");
  if (e.target.closest(".apply-to")) {
    if (!usageMenu.hidden) return closeUsageMenu();
    openUsageMenu("#shell-dialog .apply-to", () => {
      const pick = (ids) => {
        Object.assign(priceDialog, { selected: ids, attempt: null });
        drawPrices();
      };
      return [
        { label: "All environments", box: true, checked: !priceDialog.selected, keep: true, run: () => pick(priceDialog.selected ? null : new Set()) },
        { sep: true },
        ...priceDialog.targets.map((t) => ({ label: t.label, box: true, checked: !priceDialog.selected || priceDialog.selected.has(t.machine), keep: true, status: t.unavailable ?? "",
          run: () => {
            const next = new Set(priceSelected().map((s) => s.machine));
            if (next.has(t.machine)) next.delete(t.machine);
            else next.add(t.machine);
            pick(next.size === priceDialog.targets.length ? null : next);
          } })),
      ];
    });
  }
});

/** Prices this browser once kept for every machine move to each machine's own list. */
async function moveBrowserPrices() {
  let legacy = {};
  try {
    legacy = JSON.parse(load("gui-usage-prices")) ?? {};
  } catch {}
  if (!Object.keys(legacy).length) return;
  const moved = await Promise.allSettled(onlineMachines().map((m) => callOn(m, { cmd: "set_usage_prices", prices: legacy, aliases: {} })));
  if (moved.length && moved.every((r) => r.status === "fulfilled")) save("gui-usage-prices", null);
}

SHELL_PAGES.usage = {
  show() {
    usageState.now = Date.now() / 1000;
    renderUsagePage();
    if (!usageState.envs.size || onlineMachines().some((m) => !usageState.envs.get(m)?.loading)) fetchUsage();
    if (usageState.metric === "limits") checkLimits();
  },
};
onGuiKey("u", () => openPage("usage"));
