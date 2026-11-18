#!/usr/bin/env node
// Measures how the web app responds on a phone while a busy Orchestrator streams frames at it. See
// README.md for the workload, the options and the metrics.
//
//   PLAYWRIGHT=<dir>/node_modules/playwright node scripts/mobile-lag-bench/run.js --out <dir> \
//     [--ui <dir>] [--load busy|peak|quiet] [--throttle 4,6] [--runs 3]
const fs = require("fs");
const http = require("http");
const path = require("path");
const { world, serve, accept } = require("./hub.js");
const { chromium } = require(process.env.PLAYWRIGHT || "playwright");

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const at = args.indexOf(`--${name}`);
  return at < 0 ? fallback : args[at + 1];
};
const flag = (name) => args.includes(`--${name}`);
const OUT = path.resolve(opt("out", "mobile-lag-out"));
const UI = path.resolve(opt("ui", path.join(__dirname, "../../ui")));
const THROTTLES = opt("throttle", "4,6").split(",").map(Number);
const RUNS = Number(opt("runs", "3"));
const LOAD = opt("load", "busy");
const PORT = Number(opt("port", "55140"));
const PROFILE = flag("profile");
const SHOTS = flag("shots");
const TRACE = flag("trace");
const COUNT = opt("count", "").split(",").filter(Boolean);

// Frames per second, from the hub's census (README.md). `transcript` is lines appended across the
// working tabs; a line in a tab the page watches sends a nudge.
const RATES = {
  busy: { output: 120, tabs: 0.35, usage: 2.2, transcript: 2.5, agent: 0.1 },
  // The busiest interval's whole traffic to one client.
  peak: { output: 245, tabs: 0.67, usage: 4.4, transcript: 5, agent: 0.16 },
  quiet: {},
}[LOAD];

const PHONE_UA = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Mobile/15E148 Safari/604.1";
const TYPED = "check the failing deploy and tell me which worker owns it";

const TYPES = { ".js": "text/javascript", ".css": "text/css", ".html": "text/html", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png", ".webmanifest": "application/manifest+json", ".woff2": "font/woff2" };
/** The run's hub, which takes the page's socket. */
let onSocket = null;
function staticServer() {
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    if (url.pathname === "/api/version") return res.end("0.18.7-bench");
    const file = path.join(UI, url.pathname === "/" ? "index.html" : decodeURIComponent(url.pathname));
    if (!file.startsWith(UI) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) {
      res.statusCode = 404;
      return res.end();
    }
    res.setHeader("content-type", TYPES[path.extname(file)] ?? "application/octet-stream");
    fs.createReadStream(file).pipe(res);
  });
  server.on("upgrade", (req, socket) => (req.url.startsWith("/ws/client") && onSocket ? onSocket(accept(req, socket)) : socket.destroy()));
  return new Promise((resolve) => server.listen(PORT, "127.0.0.1", () => resolve(server)));
}

// Runs in the page before its scripts: long tasks, event timings and frame times.
function collector() {
  const b = (window.__bench = { longtasks: [], events: [], frames: [], marks: {}, scrolling: false });
  new PerformanceObserver((list) => {
    for (const e of list.getEntries()) b.longtasks.push({ start: e.startTime, duration: e.duration });
  }).observe({ type: "longtask", buffered: true });
  new PerformanceObserver((list) => {
    for (const e of list.getEntries()) b.events.push({ name: e.name, start: e.startTime, delay: e.processingStart - e.startTime, processing: e.processingEnd - e.processingStart, duration: e.duration, interaction: e.interactionId });
  }).observe({ type: "event", durationThreshold: 16, buffered: true });
  const tick = (t) => {
    if (b.scrolling) b.frames.push(t);
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  b.mark = (name) => (b.marks[name] = performance.now());
  try {
    localStorage.setItem("hub-token", "bench-token");
  } catch {}
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const pct = (list, p) => {
  if (!list.length) return null;
  const sorted = [...list].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))];
};
const round = (n) => (n == null ? null : Math.round(n * 10) / 10);

async function runOnce(browser, throttle, index) {
  const w = world(7);
  // Nothing runs: every agent has finished, so the page polls and streams nothing.
  if (LOAD === "quiet") for (const tab of w.tabs) Object.assign(tab, { status: "done", alive: false });
  const calls = {};
  let hub = null;
  const context = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true, userAgent: PHONE_UA });
  await context.route(/^https?:\/\/(?!127\.0\.0\.1)/, (route) => route.abort());
  onSocket = (ws) => (hub = serve(ws, w, RATES, (cmd) => (calls[cmd] = (calls[cmd] ?? 0) + 1)));
  await context.addInitScript(collector);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e.message)));
  await page.goto(`http://127.0.0.1:${PORT}/`);
  await page.waitForFunction(() => document.body.dataset.hub === "online" && typeof tabs !== "undefined" && tabs.length >= 60, null, { timeout: 60000 });
  await page.waitForFunction(() => tabs.every((t) => threadOf(t).loaded), null, { timeout: 60000 });
  const cdp = await context.newCDPSession(page);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: throttle });
  hub.start();
  await sleep(4000);
  if (TRACE) await browser.startTracing(page, { path: path.join(OUT, `${LOAD}-x${throttle}-run${index}.trace.json`), categories: ["devtools.timeline", "disabled-by-default-devtools.timeline", "disabled-by-default-devtools.timeline.stack", "v8.execute"] });
  if (PROFILE) {
    await cdp.send("Profiler.enable");
    await cdp.send("Profiler.setSamplingInterval", { interval: 200 });
    await cdp.send("Profiler.start");
  }
  if (COUNT.length)
    await page.evaluate((names) => {
      __bench.counts = {};
      for (const name of names) {
        const fn = window[name];
        if (typeof fn !== "function") continue;
        const c = (__bench.counts[name] = { calls: 0, ms: 0 });
        window[name] = function (...a) {
          c.calls++;
          const at = performance.now();
          try {
            return fn.apply(this, a);
          } finally {
            c.ms += performance.now() - at;
          }
        };
      }
    }, COUNT);
  await cdp.send("Performance.enable");
  const metrics = async () => Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));
  const metricsBefore = await metrics();
  const sentBefore = { frames: hub.sent.frames, bytes: hub.sent.bytes, kinds: { ...hub.sent.kinds } };
  const t0 = Date.now();
  await page.evaluate(() => {
    __bench.longtasks.length = 0;
    __bench.events.length = 0;
    __bench.mark("start");
  });

  const steps = {};
  const timed = async (name, fn) => {
    const at = Date.now();
    await fn();
    steps[name] = Date.now() - at;
  };
  const worker = w.tabs.find((t) => t.role === "worker");
  const other = w.tabs.filter((t) => t.role === "worker")[5];
  const key = (tab) => `${w.MACHINE}:${tab.id}`;
  // A finished worker's row folds under its Captain, so with nothing running the quiet control
  // opens the same thread the way the Captain's workers list does.
  const openRow = async (tab) => {
    const row = `.thread-row[data-tab="${key(tab)}"]`;
    if (await page.$(row)) await page.tap(row);
    else await page.evaluate((k) => (activate(tabByKey(k)), toggleSidebar(false), phoneSync()), key(tab));
  };
  const shows = (k) => page.waitForFunction((k) => active?.key === k && document.querySelector("#thread")?.childElementCount > 0, k, { timeout: 60000 });

  const shot = async (name) => SHOTS && index === 1 && (await page.screenshot({ path: path.join(OUT, `${LOAD}-x${throttle}-${name}.png`) }));
  const scroll = async (name) => {
    await page.evaluate(() => (__bench.scrolling = true));
    await timed(name, async () => {
      for (const dy of [-600, 600, -900, 900]) await cdp.send("Input.synthesizeScrollGesture", { x: 195, y: 420, yDistance: dy, speed: 1200, gestureSourceType: "touch" });
    });
    await page.evaluate(() => (__bench.scrolling = false));
  };
  const toList = async () => {
    while ((await page.evaluate(() => document.body.dataset.phoneScreen)) !== "threads") {
      const from = await page.evaluate(() => active?.key);
      await page.tap("#chat-back");
      await page.waitForFunction((from) => active?.key !== from || document.body.dataset.phoneScreen === "threads", from, { timeout: 60000 });
    }
  };

  await timed("open orchestrator", async () => {
    await page.tap('[data-phone-tab="orchestrator"]');
    await page.waitForFunction(() => document.body.dataset.phoneScreen === "orchestrator" && document.querySelector("#thread")?.childElementCount > 0, null, { timeout: 60000 });
  });
  await shot("orchestrator");
  await timed("focus composer", () => page.tap("#prompt"));
  await page.evaluate(() => __bench.mark("type"));
  await timed("type", () => page.keyboard.type(TYPED, { delay: 120 }));
  await page.evaluate(() => {
    __bench.mark("typed");
    document.activeElement.blur();
  });
  const typed = await page.$eval("#prompt", (el) => el.value);
  await scroll("scroll orchestrator");
  await timed("open list", async () => {
    await page.tap('[data-phone-tab="threads"]');
    await page.waitForFunction(() => document.body.dataset.phoneScreen === "threads", null, { timeout: 60000 });
  });
  await shot("list");
  await timed("open thread", async () => {
    await openRow(worker);
    await shows(key(worker));
  });
  await shot("thread");
  await scroll("scroll thread");
  await timed("back to captain", async () => {
    await page.tap("#chat-back");
    await shows(key(w.tabs.find((t) => t.id === worker.spawned_by)));
  });
  await timed("back to list", toList);
  await timed("switch thread", async () => {
    await openRow(other);
    await shows(key(other));
  });
  await toList();
  await timed("open needs you", async () => {
    await page.tap('[data-phone-tab="needs-you"]');
    await page.waitForFunction(() => document.body.dataset.phoneScreen === "needs-you", null, { timeout: 60000 });
  });
  await shot("needs-you");
  await timed("back to threads", async () => {
    await page.tap('[data-phone-tab="threads"]');
    await page.waitForFunction(() => document.body.dataset.phoneScreen === "threads", null, { timeout: 60000 });
  });
  await page.evaluate(() => __bench.mark("end"));
  const wall = Date.now() - t0;
  const metricsAfter = await metrics();
  const spent = (name) => round((metricsAfter[name] - metricsBefore[name]) * 1000);
  let profile = null;
  if (PROFILE) profile = (await cdp.send("Profiler.stop")).profile;
  if (TRACE) await browser.stopTracing();
  const b = await page.evaluate(() => ({ ...__bench, dom: document.getElementsByTagName("*").length }));
  const sent = { frames: hub.sent.frames - sentBefore.frames, bytes: hub.sent.bytes - sentBefore.bytes, kinds: Object.fromEntries(Object.entries(hub.sent.kinds).map(([k, v]) => [k, v - (sentBefore.kinds[k] ?? 0)])) };
  hub.stop();
  await context.close();

  const long = b.longtasks.map((t) => t.duration);
  const typing = b.events.filter((e) => ["keydown", "keypress", "input", "keyup", "beforeinput"].includes(e.name) && e.start >= b.marks.type && e.start <= b.marks.typed);
  const keyPress = new Map();
  for (const e of typing) if (e.interaction) keyPress.set(e.interaction, Math.max(keyPress.get(e.interaction) ?? 0, e.duration));
  const taps = new Map();
  for (const e of b.events.filter((e) => ["pointerdown", "pointerup", "click", "touchstart", "touchend"].includes(e.name) && e.interaction)) taps.set(e.interaction, Math.max(taps.get(e.interaction) ?? 0, e.duration));
  const interactions = new Map([...keyPress, ...taps]);
  const gaps = b.frames.slice(1).map((t, i) => t - b.frames[i]);
  const dropped = gaps.reduce((n, gap) => n + Math.max(0, Math.round(gap / (1000 / 60)) - 1), 0);
  const seconds = wall / 1000;
  const result = {
    throttle,
    run: index,
    load: LOAD,
    ui: UI,
    wallMs: wall,
    steps,
    typedOk: typed === TYPED,
    mainThread: { taskMs: spent("TaskDuration"), scriptMs: spent("ScriptDuration"), layoutMs: spent("LayoutDuration"), styleMs: spent("RecalcStyleDuration"), busyPct: round((100 * spent("TaskDuration")) / wall), layouts: metricsAfter.LayoutCount - metricsBefore.LayoutCount, styles: metricsAfter.RecalcStyleCount - metricsBefore.RecalcStyleCount },
    longTasks: { count: long.length, totalMs: round(long.reduce((a, b) => a + b, 0)), maxMs: round(Math.max(0, ...long)), busyShare: round((100 * long.reduce((a, b) => a + b, 0)) / wall) },
    keystrokes: { measured: keyPress.size, p50Ms: pct([...keyPress.values()], 50), p95Ms: pct([...keyPress.values()], 95), maxMs: Math.max(0, ...keyPress.values()), inputDelayP95Ms: round(pct(typing.filter((e) => e.name === "keydown").map((e) => e.delay), 95)) },
    taps: { measured: taps.size, maxMs: Math.max(0, ...taps.values()), values: [...taps.values()] },
    inpMs: Math.max(0, ...interactions.values()),
    scroll: { frames: b.frames.length, dropped, droppedShare: round((100 * dropped) / Math.max(1, dropped + b.frames.length)), worstGapMs: round(Math.max(0, ...gaps)) },
    received: { frames: sent.frames, bytes: sent.bytes, framesPerSec: round(sent.frames / seconds), kbPerSec: round(sent.bytes / 1024 / seconds), kinds: sent.kinds },
    calls,
    counts: b.counts,
    dom: b.dom,
    errors,
  };
  return { result, profile };
}

// Self time and total time per function, from a V8 cpuprofile.
function summarizeProfile(profile) {
  const byId = new Map(profile.nodes.map((n) => [n.id, n]));
  const parent = new Map();
  for (const n of profile.nodes) for (const c of n.children ?? []) parent.set(c, n.id);
  const name = (n) => `${n.callFrame.functionName || "(anonymous)"} ${path.basename(n.callFrame.url || "")}:${n.callFrame.lineNumber + 1}`;
  const self = new Map();
  const total = new Map();
  profile.samples.forEach((id, i) => {
    const dt = (profile.timeDeltas[i + 1] ?? 0) / 1000;
    const leaf = byId.get(id);
    self.set(name(leaf), (self.get(name(leaf)) ?? 0) + dt);
    const seen = new Set();
    for (let at = id; at != null; at = parent.get(at)) {
      const k = name(byId.get(at));
      if (seen.has(k)) continue;
      seen.add(k);
      total.set(k, (total.get(k) ?? 0) + dt);
    }
  });
  const top = (map) => [...map].sort((a, b) => b[1] - a[1]).slice(0, 40).map(([k, v]) => [k, round(v)]);
  return { self: top(self), total: top(total) };
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const server = await staticServer();
  const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_BIN ? { executablePath: process.env.CHROME_BIN } : { channel: "chrome" }) });
  const results = [];
  try {
    for (let run = 1; run <= RUNS; run++) {
      for (const throttle of THROTTLES) {
        const { result, profile } = await runOnce(browser, throttle, run);
        results.push(result);
        const base = path.join(OUT, `${LOAD}-x${throttle}-run${run}`);
        fs.writeFileSync(`${base}.json`, JSON.stringify(result, null, 2));
        if (profile) {
          fs.writeFileSync(`${base}.cpuprofile`, JSON.stringify(profile));
          fs.writeFileSync(`${base}.profile-top.json`, JSON.stringify(summarizeProfile(profile), null, 2));
        }
        console.log(`x${throttle} run ${run}: main thread busy ${result.mainThread.busyPct}% (script ${result.mainThread.scriptMs}, layout ${result.mainThread.layoutMs} in ${result.mainThread.layouts}, style ${result.mainThread.styleMs} ms), long tasks ${result.longTasks.count} (${result.longTasks.totalMs} ms, max ${result.longTasks.maxMs}, busy ${result.longTasks.busyShare}%), keys p50 ${result.keystrokes.p50Ms} p95 ${result.keystrokes.p95Ms} max ${result.keystrokes.maxMs} ms, INP ${result.inpMs} ms, scroll dropped ${result.scroll.dropped}/${result.scroll.frames + result.scroll.dropped}, ${result.received.framesPerSec} frames/s ${result.received.kbPerSec} KB/s, typed ${result.typedOk ? "ok" : "WRONG"}, errors ${result.errors.length}`);
      }
    }
  } finally {
    await browser.close();
    server.close();
  }
  const summary = {};
  for (const throttle of THROTTLES) {
    const runs = results.filter((r) => r.throttle === throttle);
    const stat = (get) => {
      const v = runs.map(get);
      return { median: pct(v, 50), min: Math.min(...v), max: Math.max(...v) };
    };
    summary[`x${throttle}`] = {
      runs: runs.length,
      mainThreadBusyPct: stat((r) => r.mainThread.busyPct),
      mainThreadTaskMs: stat((r) => r.mainThread.taskMs),
      layoutMs: stat((r) => r.mainThread.layoutMs),
      layoutCount: stat((r) => r.mainThread.layouts),
      longTaskCount: stat((r) => r.longTasks.count),
      longTaskTotalMs: stat((r) => r.longTasks.totalMs),
      longTaskMaxMs: stat((r) => r.longTasks.maxMs),
      longTaskBusyPct: stat((r) => r.longTasks.busyShare),
      keystrokeP50Ms: stat((r) => r.keystrokes.p50Ms),
      keystrokeP95Ms: stat((r) => r.keystrokes.p95Ms),
      keystrokeMaxMs: stat((r) => r.keystrokes.maxMs),
      inpMs: stat((r) => r.inpMs),
      tapMaxMs: stat((r) => r.taps.maxMs),
      scrollDroppedPct: stat((r) => r.scroll.droppedShare),
      switchThreadMs: stat((r) => r.steps["switch thread"]),
      openThreadMs: stat((r) => r.steps["open thread"]),
      typeWallMs: stat((r) => r.steps.type),
      framesPerSec: stat((r) => r.received.framesPerSec),
      kbPerSec: stat((r) => r.received.kbPerSec),
      allTyped: runs.every((r) => r.typedOk),
      errors: runs.flatMap((r) => r.errors),
    };
  }
  fs.writeFileSync(path.join(OUT, `summary-${LOAD}.json`), JSON.stringify(summary, null, 2));
  console.log(JSON.stringify(summary, null, 1));
})();
