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
const usageState = { metric: "limits", days: 30, breakdown: "model", machine: "", rows: [], error: "", loaded: false, loading: false, now: 0, seq: 0 };
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

/** The window's totals, series and models, of one `model` when given. */
function summarize(model) {
  const { days, machine } = usageState;
  const buckets = usageBuckets(days, usageState.now);
  const index = new Map(buckets.map((b, i) => [b, i]));
  const rows = usageState.rows.filter((r) => (!machine || r.machine === machine) && (!model || r.model === model) && index.has(bucketOf(days, r.at)));
  const s = { buckets, rows, cost: 0, tokens: 0, input: 0, cached: 0, cacheWrite: 0, output: 0, savings: 0, premium: 0, unpriced: 0, unpricedTokens: 0, sessions: new Set(),
    byType: { input: 0, cache_read: 0, cache_write: 0, output: 0 }, series: buckets.map(() => ({ cost: 0, tokens: 0 })), models: new Map() };
  for (const r of rows) {
    const cost = rowCost(r);
    const tokens = rowTokens(r);
    Object.assign(s, { cost: s.cost + cost, tokens: s.tokens + tokens, input: s.input + r.input, cached: s.cached + r.cached, cacheWrite: s.cacheWrite + r.cache_write, output: s.output + r.output, savings: s.savings + r.savings, premium: s.premium + r.premium });
    if (!r.cost) {
      s.unpriced++;
      s.unpricedTokens += tokens;
    }
    for (const k in s.byType) s.byType[k] += r.cost?.[k] ?? 0;
    s.sessions.add(`${r.machine}:${r.session}`);
    const point = s.series[index.get(bucketOf(days, r.at))];
    point.cost += cost;
    point.tokens += tokens;
    const m = s.models.get(r.model) ?? { model: r.model, cost: 0, priced: false, tokens: 0, input: 0, cached: 0, cacheWrite: 0, output: 0, sessions: new Set() };
    Object.assign(m, { cost: m.cost + cost, priced: m.priced || !!r.cost, tokens: m.tokens + tokens, input: m.input + r.input, cached: m.cached + r.cached, cacheWrite: m.cacheWrite + r.cache_write, output: m.output + r.output });
    m.sessions.add(`${r.machine}:${r.session}`);
    s.models.set(r.model, m);
  }
  return s;
}

async function fetchUsage() {
  const seq = ++usageState.seq;
  usageState.loading = true;
  renderUsagePage();
  const since = usageBuckets(90)[0];
  const answers = await Promise.allSettled(onlineMachines().map((m) => callOn(m, { cmd: "token_usage", since, prices: usagePrices }).then((rows) => rows.map((r) => ({ ...r, machine: m })), (e) => Promise.reject(`${machineName(m)}: ${e}`))));
  if (seq !== usageState.seq) return;
  const error = answers.filter((a) => a.status === "rejected").map((a) => a.reason).join("; ");
  Object.assign(usageState, { rows: answers.flatMap((a) => (a.status === "fulfilled" ? a.value : [])), error, loaded: true, loading: false, now: Date.now() / 1000 });
  if (shellPage() === "usage") renderUsagePage();
}

const segmented = (name, options, current, disabled = false) =>
  `<span class="seg" role="radiogroup" data-seg="${name}">${options.map(([value, label]) => `<button type="button" role="radio" aria-checked="${value === current}" data-value="${value}"${disabled ? " disabled" : ""}>${label}</button>`).join("")}</span>
   <select class="seg-select" data-seg="${name}"${disabled ? " disabled" : ""}>${options.map(([value, label]) => `<option value="${value}"${value === current ? " selected" : ""}>${label}</option>`).join("")}</select>`;
function usageRange(buckets, days) {
  const first = buckets[0];
  const last = buckets.at(-1);
  if (days === 1) return `${monthDay(first)}, ${hourText(first)} to ${monthDay(last + 3600)}, ${hourText(last + 3600)}`;
  return `${monthDay(first)} to ${monthDay(last)}`;
}

function renderUsagePage() {
  const { metric, days } = usageState;
  const s = summarize();
  const machineFilter = `<span class="crumb-sep">›</span><button type="button" class="crumb-menu machine-filter" title="Machines and model prices">${esc(usageState.machine ? machineName(usageState.machine) : machines.size > 1 ? "All machines" : machineName([...machines.keys()][0] ?? LOCAL))}${icon("chevron-down")}</button>`;
  setPage(`${headHtml("Usage", `${machineFilter}<span class="crumb-sep">›</span><span class="provider">${icon("claude")}Claude Code</span>
      <span class="range">${metric === "limits" ? "" : usageRange(s.buckets, days)}</span><span class="spacer"></span>
      ${segmented("metric", USAGE_METRICS.map(([v, l]) => [v, l]), metric)}${segmented("days", USAGE_PERIODS, days, metric === "limits")}
      <button type="button" class="icon refresh" title="Refresh">${icon("refresh-cw")}</button>`)}
    <div class="page-body"><div class="page-wrap usage" data-metric="${metric}"${usageState.loading && usageState.loaded ? ` data-refreshing` : ""}>${
      metric === "limits" ? limitsHtml() : !usageState.loaded ? usageSkeleton() : (usageState.error ? `<p class="usage-warn">${icon("triangle-alert")}<span>Couldn't read usage on ${esc(usageState.error)}</span></p>` : "") + spendHtml(s)
    }</div></div>`);
  if (metric !== "limits" && usageState.loaded) drawChart(pageView.querySelector(".chart"), s, metric);
}

function spendHtml(s) {
  const { metric, days } = usageState;
  const cost = metric === "cost";
  const total = cost ? usd(s.cost) : tokensText(s.tokens);
  const sessions = `${s.sessions.size} session${s.sessions.size === 1 ? "" : "s"}`;
  const unpricedShare = s.rows.length ? s.unpriced / s.rows.length : 0;
  const unit = days === 1 ? "Hourly" : "Daily";
  const shares = (title, parts) => sharesHtml(title, parts, cost ? usd : tokensText);
  const models = [...s.models.values()].sort((a, b) => (cost ? b.cost - a.cost : 0) || b.tokens - a.tokens);
  const top = Math.max(1e-9, ...models.map((m) => (cost ? m.cost : m.tokens)));
  const modelRows = models.map((m, i) => `<tr data-model="${esc(m.model)}"><td class="n">${i + 1}</td><td class="model"><span>${esc(m.model)}</span><i style="width:min(100%, ${(((cost ? m.cost : m.tokens) / top) * 192).toFixed(1)}px)"></i></td>
    <td class="num">${m.priced ? usd(m.cost) : `<span class="muted">Unpriced</span>`}</td><td class="num share-col">${s.cost ? percentText(m.cost / s.cost) : "—"}</td><td class="num">${tokensText(m.tokens)}</td></tr>`).join("");
  const dayRows = s.buckets.map((b, i) => [b, s.series[i]]).filter(([, p]) => p.tokens > 0).reverse()
    .map(([b, p]) => `<tr><td>${days === 1 ? `${monthDay(b)}, ${hourText(b)}` : monthDay(b)}</td><td class="num">${usd(p.cost)}</td><td class="num">${usd(p.cost)}</td><td class="num">${tokensText(p.tokens)}</td></tr>`).join("");
  const table = usageState.breakdown === "model"
    ? `<table><thead><tr><th class="n">#</th><th>Model</th><th class="num">Cost</th><th class="num share-col">Share</th><th class="num">Tokens</th></tr></thead><tbody>${modelRows}</tbody></table>`
    : `<table><thead><tr><th>${days === 1 ? "Hour" : "Day"}</th><th class="num">Claude Code</th><th class="num">Total</th><th class="num">Tokens</th></tr></thead><tbody>${dayRows}</tbody></table>`;
  return `<section class="overview">
      <div class="lead"><div class="total">${total}</div>
        <p class="muted">${sessions}${cost ? " · API estimate" : ""}${cost && unpricedShare > 0 ? `<span class="info" title="API estimate excludes ${percentText(unpricedShare)} unpriced records.">${icon("info")}</span>` : ""}</p>
        <div class="provider-row"><span class="dot" style="background:${CLAUDE_COLOR}"></span>${icon("claude")}<span class="name">Claude Code<small>${sessions}</small></span><b>${total}</b>
          <small class="line">${cost ? `100.0% of cost · ${tokensText(s.tokens)} tokens` : `100.0% of tokens · ${usd(s.cost)}`}</small></div></div>
      <div class="chart-col"><h2>${unit} ${cost ? "cost" : "processed tokens"}</h2><div class="chart"></div></div></section>
    <section><h2>Totals</h2><dl class="tiles">
      <div><dt>Processed tokens</dt><dd>${tokensText(s.tokens)}</dd></div><div><dt>Cached input</dt><dd>${tokensText(s.cached)}</dd></div>
      <div><dt>Uncached input</dt><dd>${tokensText(s.input)}</dd></div><div><dt>Output</dt><dd>${tokensText(s.output)}</dd></div>
      <div><dt>Cache savings</dt><dd>${usd(s.savings)}</dd></div></dl></section>
    ${s.tokens > 0 ? shares(cost ? "Cost by type" : "Tokens by type", cost ? costByType(s) : tokensByType(s)) : ""}
    ${cost && s.premium > 0 ? shares("Cost by speed", [["Standard", s.cost - s.premium, 34], ["Fast", s.premium, 66]]) : ""}
    <section class="breakdown"><div class="breakdown-head"><h2>Breakdown</h2>${segmented("breakdown", [["model", "Model"], ["day", days === 1 ? "Hour" : "Day"]], usageState.breakdown)}</div>
      ${s.rows.length ? table : `<p class="muted empty">No activity in this window.</p>`}</section>`;
}

const costByType = (s) => [["Input", s.byType.input, 60], ["Cache read", s.byType.cache_read, 30], ["Cache write", s.byType.cache_write, 72], ["Output", s.byType.output, 100]];
const tokensByType = (s) => [["Input", s.input, 60], ["Cache read", s.cached, 30], ["Cache write", s.cacheWrite, 72], ["Output", s.output, 100]];
/** T3 Code's share bar: segments in proportion, each a mix of the text color into the background. */
function sharesHtml(title, parts, fmt) {
  const sum = parts.reduce((a, [, v]) => a + v, 0);
  if (!sum) return "";
  const ink = (mix) => `color-mix(in srgb, var(--g-fg) ${mix}%, var(--g-bg))`;
  return `<section class="share"><h2>${title}</h2><div class="share-bar">${parts.filter(([, v]) => v > 0).map(([label, v, mix]) =>
    `<span style="flex-grow:${v};background:${ink(mix)}" title="${label} · ${fmt(v)} · ${percentText(v / sum)}"></span>`).join("")}</div>
    <div class="legend">${parts.map(([label, v, mix]) => `<span><i style="background:${ink(mix)}"></i>${label}<b>${fmt(v)}</b></span>`).join("")}</div></section>`;
}

const usageSkeleton = () => `<section class="overview"><div class="lead"><i class="ghost" style="width:144px;height:40px"></i><i class="ghost" style="width:128px;height:16px"></i></div>
  <div class="chart-col"><i class="ghost" style="height:224px;margin-left:64px"></i></div></section><section><i class="ghost" style="height:176px"></i></section>`;

function resetsInText(resetsAt, now) {
  const left = resetsAt - now;
  if (left <= 0) return "resets now";
  if (left < 60) return "Resets momentarily";
  const d = Math.floor(left / 86400);
  const h = Math.floor((left % 86400) / 3600);
  const m = Math.floor((left % 3600) / 60);
  return `resets in ${d ? `${d}d ${h}h` : h ? `${h}h ${m}m` : `${m}m`}`;
}
const LIMIT_WINDOWS = [
  ["five_hour", "Session", 300 * 60],
  ["seven_day", "Weekly", 10080 * 60],
];
const PACE = {
  ahead: ["trending-up", "Ahead of pace: spending faster than the window elapses"],
  on: ["gauge", "On pace with the window"],
  under: ["trending-down", "Under pace: headroom left for the rest of the window"],
};
function limitsHtml() {
  const now = Date.now() / 1000;
  const sources = usageState.machine ? [usageState.machine] : [...machines.keys()];
  const windows = sources.map((m) => machines.get(m)?.usage).find((u) => LIMIT_WINDOWS.some(([k]) => u?.[k]?.resets_at > now));
  if (!windows) return `<p class="muted">No provider on the selected environments reports subscription limits.</p>`;
  const cards = LIMIT_WINDOWS.filter(([k]) => windows[k]?.resets_at > now).map(([k, label, length]) => {
    const w = windows[k];
    const remaining = Math.round(100 - w.used_percentage);
    const elapsed = Math.min(1, Math.max(0, (length - (w.resets_at - now)) / length));
    const lead = w.used_percentage - elapsed * 100;
    const [paceIcon, paceLabel] = PACE[lead > 5 ? "ahead" : lead < -5 ? "under" : "on"];
    const timeLeft = Math.round((1 - elapsed) * 100);
    const at = `clamp(3px, ${timeLeft}%, calc(100% - 3px))`;
    return `<div class="limit-card" data-window="${k}"><div class="limit-text"><span class="label">${label}</span>
        <span class="figure"><b>${remaining}%</b><span class="muted">left <span class="pace" role="img" aria-label="${paceLabel}" title="${paceLabel}">${icon(paceIcon)}</span></span></span></div>
      <div class="limit-bar" title="${remaining}% left, ${resetsInText(w.resets_at, now)}, ${timeLeft}% of the window left">
        <div class="track" style="clip-path:polygon(-1px -1px, calc(${at} - 3px) -1px, calc(${at} - 3px) calc(100% + 1px), calc(${at} + 3px) calc(100% + 1px), calc(${at} + 3px) -1px, calc(100% + 1px) -1px, calc(100% + 1px) calc(100% + 1px), -1px calc(100% + 1px))">
          <span class="fill" style="width:${remaining}%;background:${CLAUDE_COLOR}"></span></div>
        <span class="tick" style="left:${at}"></span>
        <span class="inside"><b>${remaining}%</b><span class="chip">${resetsInText(w.resets_at, now).replace("resets in ", "↻ ")}</span></span></div></div>`;
  });
  return `<section class="limits"><h2>${icon("claude")}Claude Code</h2>${cards.join("")}</section>`;
}

/** Fritsch-Carlson monotone cubic through `points`, as the chart's curveMonotoneX draws it. */
function monotonePath(points) {
  const n = points.length;
  if (n < 2) return n ? `M${points[0][0]},${points[0][1]}` : "";
  const dx = [], slope = [];
  for (let i = 0; i < n - 1; i++) {
    dx.push(points[i + 1][0] - points[i][0]);
    slope.push((points[i + 1][1] - points[i][1]) / dx[i]);
  }
  const t = points.map((_, i) => (i === 0 ? slope[0] : i === n - 1 ? slope[n - 2] : slope[i - 1] * slope[i] <= 0 ? 0 : (3 * (dx[i - 1] + dx[i])) / ((2 * dx[i] + dx[i - 1]) / slope[i - 1] + (dx[i] + 2 * dx[i - 1]) / slope[i])));
  let d = `M${points[0][0]},${points[0][1]}`;
  for (let i = 0; i < n - 1; i++) {
    const h = dx[i] / 3;
    d += `C${points[i][0] + h},${points[i][1] + h * t[i]},${points[i + 1][0] - h},${points[i + 1][1] - h * t[i + 1]},${points[i + 1][0]},${points[i + 1][1]}`;
  }
  return d;
}
function niceTicks(max) {
  if (max <= 0) return [0, 1, 2, 3];
  const raw = max / 3;
  const pow = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * pow).find((s) => s >= raw);
  return [0, step, step * 2, step * 3];
}
function drawChart(holder, s, metric) {
  if (!holder) return;
  const cost = metric === "cost";
  const values = s.series.map((p) => (cost ? p.cost : p.tokens));
  const ticks = niceTicks(Math.max(...values));
  const top = ticks.at(-1);
  const [W, H, PAD] = [960, 260, 8];
  const x = (i) => (values.length === 1 ? W / 2 : (i / (values.length - 1)) * W);
  const y = (v) => PAD + (1 - v / top) * (H - PAD);
  const points = values.map((v, i) => [x(i), y(v)]);
  const line = monotonePath(points);
  const fmt = (v) => (cost ? usd(v) : tokensText(v));
  const label = (b) => (usageState.days === 1 ? `${hourText(b)}` : monthDay(b)).toUpperCase();
  const mid = Math.floor((s.buckets.length - 1) / 2);
  holder.innerHTML = `<div class="y-axis">${ticks.slice().reverse().map((t) => `<span>${t === 0 ? "0" : cost ? (t < 1000 ? usd(t).replace(/\.00$/, "") : `$${tokensText(t)}`) : tokensText(t)}</span>`).join("")}</div>
    <div class="plot"><svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" aria-label="${esc(usageState.days === 1 ? "Hourly" : "Daily")} ${cost ? "cost" : "tokens"}">
      ${ticks.map((t) => `<line class="grid" x1="0" x2="${W}" y1="${y(t)}" y2="${y(t)}"/>`).join("")}
      <path class="area" d="${line}L${x(values.length - 1)},${H}L${x(0)},${H}Z" fill="${CLAUDE_COLOR}"/>
      <path class="line" d="${line}" stroke="${CLAUDE_COLOR}"/></svg>
      <span class="cursor" hidden></span><div class="tip" hidden></div></div>
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
    const i = Math.max(0, Math.min(values.length - 1, Math.round(((e.clientX - r.left) / r.width) * (values.length - 1))));
    const px = (x(i) / W) * r.width;
    cursor.hidden = tip.hidden = false;
    cursor.style.left = `${px}px`;
    tip.innerHTML = `<p>${when(s.buckets[i])}</p><p class="row">${icon("claude")}<span>Claude Code</span><b>${fmt(values[i])}</b></p><p class="row sum"><span>Total</span><b>${fmt(values[i])}</b></p>`;
    const left = Math.min(Math.max(0, px + 12), r.width - tip.offsetWidth);
    tip.style.left = `${px + 12 + tip.offsetWidth > r.width ? Math.max(0, px - 12 - tip.offsetWidth) : left}px`;
    tip.style.top = `${Math.min(Math.max(0, e.clientY - r.top + 12), r.height - tip.offsetHeight)}px`;
  };
  plot.onpointerleave = () => (cursor.hidden = tip.hidden = true);
}

function setUsage(change) {
  Object.assign(usageState, change);
  saveUsagePrefs();
  renderUsagePage();
}
pageView.addEventListener("click", (e) => {
  if (shellPage() !== "usage") return;
  const seg = e.target.closest(".seg button");
  if (seg) {
    const name = seg.parentElement.dataset.seg;
    return setUsage({ [name]: name === "days" ? Number(seg.dataset.value) : seg.dataset.value });
  }
  if (e.target.closest(".refresh")) return fetchUsage();
  if (e.target.closest(".machine-filter")) {
    const only = (m) => ({ label: machineName(m), checked: usageState.machine === m, run: () => setUsage({ machine: m }) });
    return openSideMenu([
      ...(machines.size > 1 ? [{ label: "All machines", checked: !usageState.machine, run: () => setUsage({ machine: "" }) }] : []),
      ...[...machines.keys()].map(only),
      { label: "Model prices", icon: "sliders-horizontal", sep: true, run: () => openPrices() },
    ], e.target.closest("button"));
  }
  const model = e.target.closest("tr[data-model]");
  if (model) openModel(model.dataset.model);
});
pageView.addEventListener("change", (e) => {
  if (shellPage() !== "usage") return;
  const seg = e.target.closest("select.seg-select");
  if (seg) setUsage({ [seg.dataset.seg]: seg.dataset.seg === "days" ? Number(seg.value) : seg.value });
});
window.addEventListener("keydown", (e) => {
  if (!guiOn() || shellPage() !== "usage" || e.repeat || e.metaKey || e.ctrlKey || e.altKey || !paletteEl.hidden || !shellDialog.hidden || e.target.closest?.("input, textarea, select, [contenteditable]")) return;
  const metric = USAGE_METRICS.find(([, , key]) => key === e.key)?.[0];
  if (metric) setUsage({ metric });
});
// ⌘⇧1-4 by key position, since Shift turns the digits into other characters on each layout.
window.addEventListener("keydown", (e) => {
  const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  const i = ["Digit1", "Digit2", "Digit3", "Digit4"].indexOf(e.code);
  if (!guiOn() || shellPage() !== "usage" || !mod || !e.shiftKey || e.altKey || i < 0 || settingsOpen()) return;
  e.preventDefault();
  if (usageState.metric !== "limits") setUsage({ days: USAGE_PERIODS[i][0] });
});

/** A model's own numbers, as T3 Code's model dialog shows them, and the prices the user sets per model,
 * which the machines apply in place of LiteLLM's list. */
let usagePrices = {};
try {
  usagePrices = JSON.parse(load("gui-usage-prices")) ?? {};
} catch {}
const PRICE_FIELDS = [
  ["input", "Input"],
  ["output", "Output"],
  ["cache_read", "Cache read"],
  ["cache_write", "Cache write"],
];

function openModel(model) {
  const metric = usageState.metric === "limits" ? "cost" : usageState.metric;
  const all = summarize();
  const s = summarize(model);
  const priced = s.unpriced < s.rows.length;
  const pricedTokens = s.tokens - s.unpricedTokens;
  const inputs = s.input + s.cached + s.cacheWrite;
  const stats = [
    ["Cost", priced ? usd(s.cost) : "Unpriced"],
    ["Tokens", tokensText(s.tokens)],
    priced && pricedTokens > 0 && ["Per 1M tokens", usd((s.cost / pricedTokens) * 1e6)],
    inputs > 0 && ["Cache hit", percentText(s.cached / inputs)],
  ].filter(Boolean);
  showDialog(`<div class="dialog-head"><h2>${icon("claude")}<span>${esc(model)}</span></h2><p class="muted">Claude Code${priced && all.cost ? ` · ${percentText(s.cost / all.cost)} of cost` : ""}</p></div>
    <div class="dialog-body usage"><dl class="stats">${stats.map(([label, value]) => `<div><dt>${label}</dt><dd>${value}</dd></div>`).join("")}</dl>
      <div class="chart"></div>
      <div class="dialog-shares">${priced ? sharesHtml("Cost by type", costByType(s), usd) : ""}${sharesHtml("Tokens by type", tokensByType(s), tokensText)}${
        s.premium > 0 ? sharesHtml("Cost by speed", [["Standard", s.cost - s.premium, 34], ["Fast", s.premium, 66]], usd) : ""}</div></div>
    ${s.unpricedTokens > 0 ? `<div class="dialog-foot"><span class="muted">${tokensText(s.unpricedTokens)} tokens have no known price</span><button type="button" class="primary" data-set-price>Set price</button></div>` : ""}`);
  shellDialog.dataset.kind = "model";
  drawChart(shellDialog.querySelector(".chart"), s, priced ? metric : "tokens");
  shellDialog.querySelector("[data-set-price]")?.addEventListener("click", () => openPrices(model));
}

function openPrices(initial) {
  const models = [...new Set([...Object.keys(usagePrices), ...(initial ? [initial] : [])])];
  const cell = (model, key) => `<input type="number" min="0" step="any" inputmode="decimal" data-field="${key}" aria-label="${PRICE_FIELDS.find(([k]) => k === key)[1]} price for ${esc(model || "new model")}" value="${usagePrices[model]?.[key] ?? ""}" placeholder="${key.startsWith("cache") ? "Input's" : ""}">`;
  const row = (model) => `<tr><td><input type="text" data-field="model" aria-label="Model ID" placeholder="Model ID" value="${esc(model)}" spellcheck="false"></td>${PRICE_FIELDS.map(([k]) => `<td>${cell(model, k)}</td>`).join("")}
    <td><button type="button" class="icon" data-remove title="Remove">${icon("x")}</button></td></tr>`;
  showDialog(`<div class="dialog-head"><h2><span>Custom model prices</span></h2><p class="muted">Prices are in USD per million tokens and apply to all past and future usage, on every machine this browser reads.</p></div>
    <div class="dialog-body"><div class="table-scroll"><table class="prices" aria-label="Custom model prices"><thead><tr><th>Model</th>${PRICE_FIELDS.map(([, l]) => `<th>${l}</th>`).join("")}
      <th><button type="button" class="icon" data-add title="Add model price" aria-label="Add model price">${icon("plus")}</button></th></tr></thead><tbody>${models.map(row).join("")}</tbody></table></div>
      <p class="error" hidden></p></div>
    <div class="dialog-foot"><span class="spacer"></span><button type="button" class="outline" data-cancel>Cancel</button><button type="button" class="primary" data-save>Save prices</button></div>`);
  shellDialog.dataset.kind = "prices";
  const body = shellDialog.querySelector("tbody");
  if (!models.length) body.insertAdjacentHTML("beforeend", row(""));
  shellDialog.querySelector("[data-add]").onclick = () => body.insertAdjacentHTML("beforeend", row(""));
  body.onclick = (e) => e.target.closest("[data-remove]")?.closest("tr").remove();
  shellDialog.querySelector("[data-cancel]").onclick = closeDialog;
  shellDialog.querySelector("[data-save]").onclick = () => {
    const next = {};
    const error = shellDialog.querySelector(".error");
    for (const tr of body.querySelectorAll("tr")) {
      const value = (f) => tr.querySelector(`[data-field="${f}"]`).value.trim();
      const model = value("model");
      if (!model && PRICE_FIELDS.every(([k]) => !value(k))) continue;
      const valid = (f) => value(f) === "" || (Number.isFinite(Number(value(f))) && Number(value(f)) >= 0);
      const [input, output] = [Number(value("input")), Number(value("output"))];
      if (!model || !value("input") || !value("output") || !PRICE_FIELDS.every(([k]) => valid(k))) {
        error.textContent = `${model || "A new row"} needs a model ID, input and output prices, and prices of 0 or more.`;
        error.hidden = false;
        return;
      }
      next[model] = { input, output, ...Object.fromEntries(["cache_read", "cache_write"].filter((k) => value(k)).map((k) => [k, Number(value(k))])) };
    }
    usagePrices = next;
    save("gui-usage-prices", Object.keys(next).length ? JSON.stringify(next) : null);
    closeDialog();
    fetchUsage();
  };
  shellDialog.querySelector(initial ? `tbody tr:last-child [data-field="input"]` : `tbody tr:last-child [data-field="model"]`)?.focus();
}

SHELL_PAGES.usage = {
  show() {
    usageState.now = Date.now() / 1000;
    renderUsagePage();
    if (!usageState.loading) fetchUsage();
  },
};
onGuiKey("u", () => openPage("usage"));
