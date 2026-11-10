#!/usr/bin/env node
// Measures the RAM the web app and the desktop app hold after a day of use, on macOS.
//
//   node scripts/ram-bench/state.js synthetic /tmp/ram-state
//   node scripts/ram-bench/run.js web /tmp/ram-state [--runs 3]
//   node scripts/ram-bench/run.js app /tmp/ram-state [--runs 3]
//
// Each run opens the state's sessions, walks them with workload.js, and reads `footprint`, the
// figure Activity Monitor shows as Memory, twice: once after boot and once after the walk.
// `web` runs a hub and a node, and counts the page's renderer process in Chrome. `app` runs the desktop
// binary under a HOME of its own and counts its process plus its WebKit helpers; its window
// shows on screen for the length of the run. One JSON line per run, then a summary with the
// median of each figure. A run that walked fewer sessions than the state holds, or logged a
// page error, exits 1.
//
// `app` also loads the memory search's vector model when --models names a folder that holds it
// (default: the installed app's), since the desktop app holds it from the first prompt on. The
// folder is cloned, never downloaded; without one the summary says "vectors": "not asked".
//
// `app --pressure` reads once more after telling WebKit that memory is low, which makes every
// WebKit process on this Mac collect and drop its caches once. What is left is what the page
// really holds; the difference was garbage WebKit had not collected yet.
//
// `app --steps` reads after each part of the walk (the vector model, the threads, the terminals,
// the pages), to see which part a figure comes from.
//
// `app --scroll` first scrolls every thread for three seconds and reports the frame times.
//
// `app --threads` opens the threads one at a time before the walk and reads after each, with
// what the thread drew, to see which threads a figure comes from. Give it a short --idle.
//
// `web --floors` also reads the renderer on an empty page and on the sign-in page, which is the
// app's code and styles with no session open.
//
// `web --dump <file>` writes Chrome's own memory-infra account of the renderer after the walk.
//
// `--ablate` goes on after the last reading: it empties one holder at a time in the page (other
// threads' items, hidden terminals, caches) and prints what each gave back. It ranks what to
// cut; it measures no change. WebKit gives memory back only when told it is low, so `app`
// needs --pressure with it.
//
// Flags: --runs N (3), --repo <checkout> (this one) picks the ui/ and target/debug to run,
// --settle ms (400) is the wait on each thread, --idle s (30) the wait before the app is read,
// --trace prints the app's memory every 5 s of that wait.
// Env: PLAYWRIGHT names the playwright package, CHROME_BIN the Chrome to launch,
// RAM_BENCH_PORT the first of four ports (55210). Build first:
//   cargo build -p pinned-server
//   TAURI_CONFIG='{"identifier":"dev.personal-harness.pinned-bench"}' cargo build --bin pinned
const fs = require("fs");
const http = require("http");
const os = require("os");
const path = require("path");
const { execFileSync, spawn } = require("child_process");

const argv = process.argv.slice(2);
const flag = (name, fallback) => (argv.includes(`--${name}`) ? argv[argv.indexOf(`--${name}`) + 1] : fallback);
const [kind, stateArg] = argv;
if (!["web", "app"].includes(kind) || !stateArg) {
  console.error("usage: run.js web|app <state dir> [--runs N] [--repo <checkout>] [--settle ms] [--models dir] [--ablate] [--pressure] [--steps] [--threads] [--try file.js] [--dump file] [--floors] [--scroll]");
  process.exit(2);
}
const STATE = path.resolve(stateArg);
const REPO = path.resolve(flag("repo", path.join(__dirname, "..", "..")));
const RUNS = Number(flag("runs", 3));
const SETTLE = Number(flag("settle", 400));
const IDLE = Number(flag("idle", 30));
const PORT = Number(process.env.RAM_BENCH_PORT || 55210);
const TARGET = path.join(process.env.CARGO_TARGET_DIR || path.join(REPO, "target"), "debug");
const IDENTIFIER = "dev.personal-harness.pinned-bench";
const MODELS = flag("models", path.join(os.homedir(), "Library", "Application Support", "dev.personal-harness.pinned", "models"));
const EMAIL = "bench@example.test";
const PASSWORD = "bench password 1";
const manifest = JSON.parse(fs.readFileSync(path.join(STATE, "manifest.json"), "utf8"));
const HOME = path.join(STATE, "home");
const RUN = path.join(STATE, "run");
const WORKLOAD = fs.readFileSync(path.join(__dirname, "workload.js"), "utf8");
// --try <file.js> runs that script in the page, to test an idea without a rebuild: before the
// sign-in on the web, and with the probe in the app, a few seconds after its window loads.
const TRY = flag("try") ? fs.readFileSync(flag("try"), "utf8") : "";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const median = (list) => [...list].sort((a, b) => a - b)[Math.floor(list.length / 2)];
const mb = (bytes) => Math.round(bytes / 1048576);
// A session started inside pinned must not hand its tab's identity to the bench's own sessions.
const env = (extra) => {
  const base = { ...process.env, ...extra };
  for (const key of ["PINNED_SOCK", "PINNED_TASK", "PINNED_ACCOUNT", "CLAUDE_CONFIG_DIR"]) delete base[key];
  return base;
};
const children = [];
function start(command, args, options) {
  const log = fs.openSync(path.join(RUN, `${path.basename(command)}-${children.length}.log`), "a");
  const child = spawn(command, args, { stdio: ["ignore", log, log], ...options });
  children.push(child);
  return child;
}
async function waitFor(test, what, ms = 30000) {
  for (const end = Date.now() + ms; Date.now() < end; await sleep(200)) {
    try {
      if (await test()) return;
    } catch {}
  }
  throw new Error(`timed out waiting for ${what}`);
}

/** Bytes of `footprint` per pid, with the categories that hold most of it. */
function footprint(pids) {
  const file = path.join(RUN, "footprint.json");
  try {
    execFileSync("footprint", ["-j", file, ...pids.map(String)], { stdio: "ignore" });
  } catch {}
  const read = JSON.parse(fs.readFileSync(file, "utf8"));
  return new Map(read.processes.map((p) => [p.pid, { bytes: p.footprint, categories: p.categories }]));
}
/** The median of three readings a second apart, since a collection can land between two. */
async function measure(groups) {
  const readings = [];
  for (let i = 0; i < 3; i++) {
    if (i) await sleep(1000);
    const seen = footprint(Object.values(groups).flat());
    const sums = {};
    for (const [group, pids] of Object.entries(groups)) sums[group] = pids.reduce((n, pid) => n + (seen.get(pid)?.bytes ?? 0), 0);
    readings.push(sums);
  }
  const out = {};
  for (const group of Object.keys(groups)) out[group] = mb(median(readings.map((r) => r[group])));
  return out;
}
/** The six categories of `footprint` that hold most of these processes' dirty memory, in MB. */
function categories(pids) {
  const sums = {};
  for (const { categories } of footprint(pids).values()) {
    for (const [name, sizes] of Object.entries(categories ?? {})) sums[name] = (sums[name] ?? 0) + (sizes.dirty ?? 0);
  }
  return Object.fromEntries(Object.entries(sums).sort((a, b) => b[1] - a[1]).slice(0, 6).map(([name, bytes]) => [name, mb(bytes)]));
}
/** The WebKit content, GPU and network processes of the app `owner`. macOS files them under the
 * app that is responsible for `owner`, which for a binary this script starts is the terminal's
 * app, so they are the ones that started after `owner` did. */
function helpersOf(owner) {
  const script = `
import ctypes, subprocess
lib = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
responsible = lib.responsibility_get_pid_responsible_for_pid
responsible.restype = ctypes.c_int
responsible.argtypes = [ctypes.c_int]
def seconds(etime):
    days, _, clock = etime.rpartition("-")
    parts = [int(n) for n in clock.split(":")]
    return int(days or 0) * 86400 + sum(n * 60 ** i for i, n in enumerate(reversed(parts)))
rows = [line.strip().split(None, 2) for line in subprocess.run(["ps", "-axo", "pid=,etime=,comm="], capture_output=True, text=True).stdout.splitlines()]
age = {int(pid): seconds(etime) for pid, etime, comm in rows}
for pid, etime, comm in rows:
    if "com.apple.WebKit" in comm and responsible(int(pid)) == responsible(${owner}) and seconds(etime) <= age[${owner}]:
        print(pid, comm.rsplit(".", 1)[-1])
`;
  const groups = { content: [], gpu: [], network: [] };
  for (const line of execFileSync("python3", ["-c", script], { encoding: "utf8" }).trim().split("\n").filter(Boolean)) {
    const [pid, name] = line.split(" ");
    groups[name === "WebContent" ? "content" : name === "GPU" ? "gpu" : "network"].push(Number(pid));
  }
  return groups;
}

function installBin() {
  const bin = path.join(RUN, "bin");
  fs.mkdirSync(bin, { recursive: true });
  fs.copyFileSync(path.join(__dirname, "claude"), path.join(bin, "claude"));
  fs.copyFileSync(path.join(REPO, "scripts", "web-sweep", "fake-claude"), path.join(bin, "fake-claude"));
  fs.copyFileSync(path.join(REPO, "scripts", "web-sweep", "fake-gh"), path.join(bin, "gh"));
  // The node asks the login shell for PATH; this one answers with the bench's, its claude first.
  fs.writeFileSync(path.join(bin, "shell"), '#!/bin/sh\nshift\nexec /bin/sh -c "$1"\n');
  for (const f of fs.readdirSync(bin)) fs.chmodSync(path.join(bin, f), 0o755);
  return bin;
}

async function stop() {
  for (const child of children.splice(0)) {
    try {
      child.kill();
    } catch {}
  }
  if (fs.existsSync(path.join(RUN, "pg"))) {
    try {
      execFileSync("pg_ctl", ["-D", path.join(RUN, "pg"), "-m", "fast", "stop"], { stdio: "ignore" });
    } catch {}
  }
  await sleep(1500);
}

async function web() {
  const [pgPort, hubPort] = [PORT, PORT + 1];
  const hub = `http://127.0.0.1:${hubPort}`;
  const server = path.join(TARGET, "pinned-server");
  const bin = installBin();
  const pg = path.join(RUN, "pg");
  execFileSync("initdb", ["-U", "pinned", "--auth=trust", "-D", pg], { stdio: "ignore", env: env({ LC_ALL: "C" }) });
  execFileSync("pg_ctl", ["-D", pg, "-l", path.join(RUN, "pg.log"), "-o", `-p ${pgPort} -c listen_addresses=127.0.0.1 -c unix_socket_directories=`, "start"], { stdio: "ignore", env: env({ LC_ALL: "C" }) });
  await waitFor(() => execFileSync("psql", ["-h", "127.0.0.1", "-p", String(pgPort), "-U", "pinned", "-d", "postgres", "-qc", "select 1"], { stdio: "ignore" }) || true, "postgres");
  execFileSync("psql", ["-h", "127.0.0.1", "-p", String(pgPort), "-U", "pinned", "-d", "postgres", "-qc", "create database pinned"], { stdio: "ignore" });
  start(server, ["hub", "--db", `postgres://pinned@127.0.0.1:${pgPort}/pinned`, "--listen", `127.0.0.1:${hubPort}`, "--ui", path.join(REPO, "ui"), "--downloads", path.join(RUN, "downloads"), "--key-file", path.join(RUN, "hub.key"), "--public-url", hub, "--signup", "open"], { env: env({}) });
  await waitFor(async () => (await fetch(hub)).ok, "the hub");
  await fetch(`${hub}/api/signup`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ email: EMAIL, password: PASSWORD }) });
  const config = path.join(RUN, "node");
  const nodeEnv = env({ HOME, SHELL: path.join(bin, "shell"), PATH: `${bin}:${process.env.PATH}`, PINNED_PASSWORD: PASSWORD });
  execFileSync(server, ["login", hub, EMAIL, "--name", "bench-node", "--config-dir", config], { stdio: "ignore", env: nodeEnv });
  for (const file of ["sessions.json", "projects.json"]) fs.copyFileSync(path.join(STATE, file), path.join(config, file));
  start(server, ["node", "--config-dir", config], { env: nodeEnv });

  const { chromium } = require(process.env.PLAYWRIGHT || "playwright");
  const executablePath = process.env.CHROME_BIN || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  // Playwright gives each launch a profile folder of its own, so no run sees another's storage.
  const browser = await chromium.launch({ executablePath, headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    const blank = argv.includes("--floors") ? await measureRenderers(browser) : undefined;
    const pageErrors = [];
    page.on("pageerror", (e) => pageErrors.push(String(e)));
    await page.goto(hub);
    await page.waitForSelector("#login [name=email]");
    const shell = argv.includes("--floors") ? await measureRenderers(browser) : undefined;
    if (TRY) await page.evaluate(TRY);
    await page.fill("#login [name=email]", EMAIL);
    await page.fill("#login [name=password]", PASSWORD);
    await page.click("#login .primary");
    await page.waitForFunction(() => document.body.dataset.hub === "online", null, { timeout: 60000 });
    await page.evaluate(WORKLOAD);
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Performance.enable");
    const browserCdp = await browser.newBrowserCDPSession();
    const read = async () => {
      // Two collections: the first frees what the second's weak references still held.
      for (let i = 0; i < 2; i++) await cdp.send("HeapProfiler.collectGarbage");
      await sleep(8000);
      const { processInfo } = await browserCdp.send("SystemInfo.getProcessInfo");
      const groups = { renderer: [], gpu: [], browser: [] };
      for (const p of processInfo) if (p.type === "renderer" || p.type === "GPU" || p.type === "browser") groups[p.type.toLowerCase()].push(p.id);
      // Chrome keeps a spare renderer and may run the service worker in one of its own; the page's is the largest.
      const sizes = footprint(groups.renderer);
      groups.renderer = [groups.renderer.reduce((a, b) => ((sizes.get(a)?.bytes ?? 0) >= (sizes.get(b)?.bytes ?? 0) ? a : b))];
      const { metrics } = await cdp.send("Performance.getMetrics");
      const metric = (name) => metrics.find((m) => m.name === name)?.value ?? 0;
      return { ...(await measure(groups)), heap: mb(metric("JSHeapUsedSize")), dom_nodes: metric("Nodes"), ...(argv.includes("--ablate") ? { categories: categories(groups.renderer) } : {}) };
    };
    const booted = await page.evaluate((sessions) => ramBench.boot(sessions), manifest.sessions);
    const boot = await read();
    const walked = await page.evaluate((settle) => ramBench.walk(settle, false), SETTLE);
    const walk = await read();
    walked.errors.push(...pageErrors);
    if (flag("dump")) fs.writeFileSync(flag("dump"), JSON.stringify(await memoryDump(browser), null, 1));
    const ablation = [];
    if (argv.includes("--ablate")) {
      for (const [name, empty] of ABLATIONS) {
        const note = await page.evaluate(empty);
        const after = await read();
        ablation.push({ name, note, renderer: after.renderer, heap: after.heap, categories: after.categories });
      }
    }
    return { primary: walk.renderer, boot, walk, booted, walked, ...(blank ? { floors: { blank, shell } } : {}), ...(ablation.length ? { ablation } : {}) };
  } finally {
    await browser.close();
  }
}

/** MB of Chrome's renderers now, with no collection asked for. */
async function measureRenderers(browser) {
  await sleep(3000);
  const cdp = await browser.newBrowserCDPSession();
  const { processInfo } = await cdp.send("SystemInfo.getProcessInfo");
  const sizes = footprint(processInfo.filter((p) => p.type === "renderer").map((p) => p.id));
  return mb(Math.max(...[...sizes.values()].map((p) => p.bytes)));
}

/** Chrome's own account of the page's renderer, as chrome://tracing's memory-infra dumps it: MB per
 * allocator and per part of it, largest first. */
async function memoryDump(browser) {
  const cdp = await browser.newBrowserCDPSession();
  const events = [];
  cdp.on("Tracing.dataCollected", ({ value }) => events.push(...value));
  await cdp.send("Tracing.start", { traceConfig: { includedCategories: ["disabled-by-default-memory-infra"], memoryDumpConfig: { triggers: [] } } });
  await cdp.send("Tracing.requestMemoryDump", { levelOfDetail: "detailed" });
  const done = new Promise((resolve) => cdp.once("Tracing.tracingComplete", resolve));
  await cdp.send("Tracing.end");
  await done;
  const { processInfo } = await cdp.send("SystemInfo.getProcessInfo");
  const renderers = new Set(processInfo.filter((p) => p.type === "renderer").map((p) => p.id));
  const sizes = {};
  for (const event of events) {
    if (event.ph !== "v" || !renderers.has(event.pid)) continue;
    for (const [name, dump] of Object.entries(event.args?.dumps?.allocators ?? {})) {
      const size = parseInt(dump.attrs?.size?.value ?? "0", 16);
      // A page has its own renderer, and the spare one Chrome keeps holds next to nothing.
      if (size) sizes[name.replace(/0x[0-9a-f]+/gi, "*")] = Math.max(sizes[name.replace(/0x[0-9a-f]+/gi, "*")] ?? 0, size);
    }
  }
  return Object.fromEntries(Object.entries(sizes).filter(([, bytes]) => bytes >= 1048576).sort((a, b) => b[1] - a[1]).map(([name, bytes]) => [name, mb(bytes)]));
}

/** Each empties one holder in the page, after the one before it, and says how much it held. */
const ABLATIONS = [
  ["other threads' items", () => {
    let chars = 0;
    for (const tab of tabs) {
      if (tab === active) continue;
      const t = threadOf(tab);
      chars += JSON.stringify(t.items).length;
      t.items = [];
      t.history = null;
    }
    return `${Math.round(chars / 1048576)} MB of JSON`;
  }],
  ["the shown thread's items", () => {
    const t = threadOf(active);
    const chars = JSON.stringify(t.items).length;
    t.items = [];
    t.history = null;
    renderThread();
    return `${Math.round(chars / 1048576)} MB of JSON`;
  }],
  ["markdown and highlight caches", () => {
    const sizes = `${rendered.size} rendered, ${highlighted.size} highlighted`;
    rendered.clear();
    highlighted.clear();
    return sizes;
  }],
  ["hidden terminals' buffers", () => {
    let lines = 0;
    for (const tab of tabs) {
      if (tab === active) continue;
      lines += tab.term.buffer.normal.length;
      tab.term.reset();
    }
    return `${lines} lines`;
  }],
  ["every terminal", () => {
    for (const tab of tabs) {
      tab.term.dispose();
      tab.el.remove();
    }
    return `${tabs.length} terminals`;
  }],
  ["the thread and sidebar DOM", () => {
    const nodes = document.getElementsByTagName("*").length;
    for (const id of ["thread", "thread-list", "page-view", "panel-body"]) document.getElementById(id).replaceChildren();
    return `${nodes} nodes before`;
  }],
  ["every element", () => {
    const nodes = document.getElementsByTagName("*").length;
    document.body.replaceChildren();
    return `${nodes} nodes before`;
  }],
];

async function app() {
  const bin = installBin();
  const config = path.join(HOME, "Library", "Application Support", IDENTIFIER);
  // What an earlier run saved must not open with this one: the app's config, and WebKit's storage for it.
  for (const stale of [path.join(HOME, "Library"), path.join(os.homedir(), "Library", "WebKit", IDENTIFIER), path.join(os.homedir(), "Library", "Caches", IDENTIFIER)]) fs.rmSync(stale, { recursive: true, force: true });
  fs.mkdirSync(config, { recursive: true });
  for (const file of ["sessions.json", "projects.json"]) fs.copyFileSync(path.join(STATE, file), path.join(config, file));
  const memory = fs.existsSync(MODELS) && fs.readdirSync(MODELS).length > 0;
  // A clone shares the model's 530 MB on disk with the folder it came from.
  if (memory) execFileSync("cp", ["-c", "-R", MODELS, path.join(config, "models")]);

  // The probe runs inside the app's window and has no way back but HTTP: it posts each phase's
  // result here and waits on /go, which answers once that phase's memory is read.
  const reports = {};
  const waiting = {};
  const go = {};
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://127.0.0.1");
    const phase = url.searchParams.get("phase");
    res.setHeader("access-control-allow-origin", "*");
    if (url.pathname === "/report") {
      let body = "";
      req.on("data", (chunk) => (body += chunk));
      req.on("end", () => {
        reports[phase] = JSON.parse(body);
        waiting[phase]?.();
        res.end("ok");
      });
    } else if (url.pathname === "/go") {
      go[phase] = () => res.end("ok");
    } else res.end("");
  });
  await new Promise((resolve) => server.listen(PORT + 2, "127.0.0.1", resolve));
  const reported = (phase) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`the app never reported ${phase}`)), 600000);
    waiting[phase] = () => {
      clearTimeout(timer);
      resolve(reports[phase]);
    };
  });
  const base = `http://127.0.0.1:${PORT + 2}`;
  const ablations = argv.includes("--ablate") ? ABLATIONS : [];
  // --steps reads after each part of the walk; the last part's reading is then the walk's.
  const parts = ["memory", "threads", "terminals", "pages", "home"].filter((part) => part !== "memory" || memory);
  const threads = argv.includes("--threads") ? Array.from({ length: manifest.sessions }, (_, i) => [`thread${i}`, `() => ramBench.visit(${i}, ${SETTLE})`]) : [];
  const walk = argv.includes("--steps") ? parts.map((part) => [part === "home" ? "walk" : part, `() => ramBench.step("${part}", ${SETTLE})`]) : [["walk", `() => ramBench.walk(${SETTLE}, ${memory})`]];
  const phases = [
    ["boot", `() => ramBench.boot(${manifest.sessions})`],
    ...(argv.includes("--scroll") ? [["scroll", "() => ramBench.scroll()"]] : []),
    ...threads,
    ...walk,
    ...ablations.map(([, empty], i) => [`ablate${i}`, `async () => ({ note: await (${empty})() })`]),
  ];
  const probe = `${WORKLOAD}
${TRY}
(async () => {
  const post = (phase, result) => fetch("${base}/report?phase=" + phase, { method: "POST", body: JSON.stringify(result) });
  for (const [phase, work] of [${phases.map(([phase, work]) => `["${phase}", ${work}]`).join(", ")}]) {
    try {
      await post(phase, await work());
    } catch (e) {
      await post(phase, { failed: String(e?.stack ?? e), errors: [String(e)] });
    }
    await fetch("${base}/go?phase=" + phase);
  }
})();`;
  fs.writeFileSync(path.join(RUN, "probe.js"), probe);
  const child = start(path.join(TARGET, "pinned"), (flag("app-args") ?? "").split(" ").filter(Boolean), { env: env({ HOME, CFFIXED_USER_HOME: HOME, SHELL: path.join(bin, "shell"), PATH: `${bin}:${process.env.PATH}`, PINNED_PROBE: path.join(RUN, "probe.js") }) });
  try {
    const read = async () => {
      // WebKit has no call that collects on demand; idle seconds let its own timers run.
      for (let waited = 0; waited < IDLE; waited += 5) {
        await sleep(5000);
        if (argv.includes("--trace")) console.error(`idle ${waited + 5}s`, JSON.stringify(await measure({ app: [child.pid], ...helpersOf(child.pid) })));
      }
      const helpers = helpersOf(child.pid);
      const sums = await measure({ app: [child.pid], ...helpers });
      return { total: sums.app + sums.content + sums.gpu + sums.network, ...sums, ...(ablations.length || argv.includes("--steps") || threads.length ? { categories: categories(helpers.content) } : {}) };
    };
    const results = {};
    const readings = {};
    const pressed = {};
    for (const [phase] of phases) {
      results[phase] = await reported(phase);
      readings[phase] = await read();
      if (argv.includes("--pressure") && phase !== "boot") {
        execFileSync("notifyutil", ["-p", "org.WebKit.lowMemory"]);
        pressed[phase] = await read();
      }
      // The probe's wait on /go may land after its report does.
      await waitFor(() => go[phase], `the app to wait after ${phase}`);
      go[phase]();
    }
    if (flag("surfaces")) {
      const out = fs.openSync(flag("surfaces"), "w");
      try {
        execFileSync("footprint", ["-v", ...helpersOf(child.pid).content.map(String)], { stdio: ["ignore", out, "ignore"], maxBuffer: 1 << 30 });
      } catch {}
    }
    const ablation = ablations.map(([name], i) => {
      const reading = pressed[`ablate${i}`] ?? readings[`ablate${i}`];
      return { name, note: results[`ablate${i}`].note ?? results[`ablate${i}`].failed, content: reading.content, categories: reading.categories };
    });
    const visits = threads.map(([phase]) => ({ ...results[phase], content: readings[phase].content, graphics: readings[phase].categories?.["Owned physical footprint (unmapped) (graphics)"] }));
    const steps = argv.includes("--steps") ? parts.slice(0, -1).map((part) => ({ part, ...readings[part], ...(pressed[part] ? { pressed: pressed[part].content } : {}) })) : [];
    const walked = argv.includes("--steps") ? { visited: results.threads?.visited, vectors: results.memory?.vectors ?? "not asked", ...results.walk } : results.walk;
    return { primary: readings.walk.total, boot: readings.boot, walk: readings.walk, booted: results.boot, walked, ...(pressed.walk ? { pressed: pressed.walk } : {}), ...(steps.length ? { steps } : {}), ...(visits.length ? { visits } : {}), ...(results.scroll ? { scroll: results.scroll } : {}), ...(ablation.length ? { ablation } : {}) };
  } finally {
    server.close();
    server.closeAllConnections?.();
  }
}

(async () => {
  const results = [];
  let failed = false;
  for (let run = 1; run <= RUNS; run++) {
    fs.rmSync(RUN, { recursive: true, force: true });
    fs.mkdirSync(RUN, { recursive: true });
    let result;
    try {
      result = await (kind === "web" ? web() : app());
    } catch (e) {
      result = { failed: String(e?.stack ?? e) };
    }
    await stop();
    const errors = [...(result.booted?.errors ?? []), ...(result.walked?.errors ?? [])];
    const ok = !result.failed && !result.walked?.failed && result.walked?.visited === manifest.sessions && result.walked?.loaded === manifest.sessions && errors.length === 0;
    failed ||= !ok;
    console.log(JSON.stringify({ run, kind, ok, ...result }));
    if (ok) results.push(result);
  }
  if (results.length) {
    const field = (pick) => median(results.map(pick));
    const groups = Object.keys(results[0].walk);
    console.log(
      JSON.stringify({
        summary: kind,
        state: manifest.kind,
        sessions: manifest.sessions,
        runs: results.length,
        failed_runs: RUNS - results.length,
        primary_mb: field((r) => r.primary),
        primary_range_mb: [Math.min(...results.map((r) => r.primary)), Math.max(...results.map((r) => r.primary))],
        boot_mb: Object.fromEntries(groups.map((g) => [g, field((r) => r.boot[g])])),
        walk_mb: Object.fromEntries(groups.map((g) => [g, field((r) => r.walk[g])])),
        visited: field((r) => r.walked.visited),
        vectors: results[0].walked.vectors,
        items: field((r) => r.walked.items),
      }),
    );
  }
  process.exit(failed ? 1 : 0);
})();
