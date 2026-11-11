// The state every redesign board shows, read from the boards themselves, and `ensure`, which puts
// the sweep stack into it with real sessions. ensure is idempotent: it looks each thing up by
// name first, so every screen calls it and only the first call does the work.
const fs = require("fs");
const path = require("path");
const vm = require("vm");
const { execFileSync, spawn } = require("child_process");

const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";

const mainCheckout = () => {
  const common = execFileSync("git", ["-C", __dirname, "rev-parse", "--path-format=absolute", "--git-common-dir"], { encoding: "utf8" }).trim();
  return path.dirname(common);
};
const BOARD_DIR = process.env.BOARD_DIR || path.join(mainCheckout(), ".claude/research/redesign/boards");

/** What a board's renderVals() returns for these props, as the board's own code computes it. */
function vals(board, props = {}) {
  const html = fs.readFileSync(path.join(BOARD_DIR, `${board}.dc.html`), "utf8");
  const script = html.match(/<script[^>]*data-dc-script[^>]*>([\s\S]*?)<\/script>/)[1];
  class DCLogic {
    constructor(p) {
      this.props = p;
      this.state = {};
    }
  }
  const Component = vm.runInNewContext(`${script}\nComponent`, { DCLogic });
  return new Component(props).renderVals();
}

function buildWorld() {
  const sidebar = vals("Sidebar").rows;
  const captains = vals("Orchestrator").captains;
  const phoneWorkers = vals("Phone-Thread").workers;
  const machineOf = Object.fromEntries(captains.map((c) => [c.name, c.sub.split(" · ")[0]]));
  const projects = [];
  const threads = {};
  let project = null;
  let parent = null;
  for (const r of sidebar) {
    if (r.head) {
      project = { id: r.id, name: r.name, mono: r.mono, meta: r.meta, machine: machineOf[r.name] ?? r.meta.split(" · ").at(-1) };
      projects.push(project);
      continue;
    }
    const t = { id: r.id, title: r.title, glyph: r.g, slot: r.slot, sub: r.sub ?? null, chip: r.chip ?? null, project: project.name, machine: project.machine };
    if (r.row) parent = t;
    else t.parent = parent.id;
    threads[r.id] = t;
  }
  for (const c of captains) if (!projects.some((p) => p.name === c.name)) projects.push({ id: c.name, name: c.name, mono: c.mono, meta: c.sub, machine: machineOf[c.name], offline: c.state === "Offline" });

  // Main.dc.html is static: these are its strings for the Captain and its workers.
  const captain = {
    thread: "release",
    prompt: "Release v0.17.21 once the CI passes, with its changelog.",
    branch: "release/v0.17.21",
    elapsed: "1h 12m",
    cost: 3.42,
    workers: [
      { thread: "ci", model: "claude-sonnet-5-5", label: "Sonnet 5.5", step: "Running cargo test -p pinned-server", hold: true },
      { thread: "readme", model: "claude-haiku-5-5", label: "Haiku 5.5", ask: "git push origin readme-tidy" },
      { thread: "changelog", model: "claude-haiku-5-5", label: "Haiku 5.5", step: "Wrote CHANGELOG.md with 14 entries and opened #231" },
    ],
  };
  for (const w of captain.workers) w.title = threads[w.thread].title;
  return {
    projects,
    threads,
    captain,
    phoneWorkers,
    machines: [...new Set(projects.map((p) => p.machine))],
    orchestrator: { name: "Orchestrator", machine: machineOf.pinned },
  };
}

const WORLD = buildWorld();
const T = WORLD.threads;
const HOST = WORLD.orchestrator.machine;

// The words each thread's fake claude says, from the boards: Main and Worker-Approval for the
// Captain and its workers, Panel for the hero fix, Phone-Thread for the CI worker's step.
const README_BRIEF = "Tidy the README. Fix the install steps for Windows, drop the two stale badges, and keep every heading that the docs link to.";
const HERO_PROMPT = "The hero overlaps the nav below 400px. Fix the spacing.";
const CI_TASK = "Run the CI on main. Report what fails.";
const CHANGELOG_TASK = "Write the changelog for v0.17.21 from the merged pull requests.";
// A worker's first prompt is its title, a blank line and its task, so each matches on its task alone;
// the Captain hears of the README worker's approval as "... needs input: ...".
const CAPTAIN_TURNS = [
  { prompt: WORLD.captain.prompt, say: "I split this into three workers. I will cut the release once the CI is green and the changelog has landed." },
  { prompt: "needs input: Claude needs your permission", say: "The changelog is in and linked from #231. The CI is on its last suite. The README worker is waiting for you to allow one push." },
  { prompt: CI_TASK, tools: [["Bash", { command: "cargo fmt --check", description: "Check formatting" }, ""], ["Bash", { command: "cargo clippy --workspace", description: "Lint the workspace" }, "Finished"]], running: ["Bash", { command: "cargo test -p pinned-server", description: "Run the pinned-server tests" }] },
  { prompt: README_BRIEF, say: "The README is tidied and committed on readme-tidy. I need to push the branch so the Captain can open the pull request.", ask: { command: "git push origin readme-tidy", description: "Pushes one commit to a new remote branch. Nothing on main changes." } },
  { prompt: CHANGELOG_TASK, say: "Wrote CHANGELOG.md with 14 entries and opened #231", pr: { number: 231, repo: "example/pinned" } },
];
const LOOSE_TURNS = [
  { prompt: HERO_PROMPT, tools: [["Edit", { file_path: "src/components/Hero.css", old_string: "margin-top: 64px;", new_string: "margin-top: var(--nav-height);" }, "The file src/components/Hero.css has been updated."]], say: "The hero used a fixed top margin that ignored the taller nav on small screens. It now reads the nav height. I checked it at 390px in the preview.", pr: { number: 12, repo: "example/site" } },
  { prompt: T.updater.title, say: "The updater now retries three times with backoff before it reports a failure.", pr: { number: 214, repo: "example/pinned" } },
  { prompt: T.rate.title, running: ["Bash", { command: "cargo test webhook", description: "Run the webhook tests" }] },
];

const HERO_CSS_BEFORE = ":root {\n  --nav-height: 72px;\n}\n\n.nav {\n  height: var(--nav-height);\n}\n.hero {\n  display: grid;\n  margin-top: 64px;\n  padding: 0 24px;\n}\n\n@media (max-width: 400px) { .hero { margin-top: 64px; } }\n";
const HERO_CSS_AFTER = ":root {\n  --nav-height: 72px;\n}\n\n.nav {\n  height: var(--nav-height);\n}\n.hero {\n  display: grid;\n  margin-top: var(--nav-height);\n  padding: 0 24px;\n}\n\n@media (max-width: 400px) {\n  .hero { padding: 0 16px; }\n}\n";

const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname, "../../../target"), "debug", "pinned-server");
const GO = `http://127.0.0.1:${process.env.SWEEP_GO_PORT || 8798}`;
const homeOf = (ctx, machine) => path.join(ctx.dir, machine === HOST ? "home" : `home-${machine}`);
const git = (cwd, ...args) => execFileSync("git", ["-C", cwd, "-c", "user.name=sweep", "-c", "user.email=sweep@example.test", ...args], { stdio: "ignore" });

/** A checkout of github.com/example/<name> on `branch`, with `files` committed and `changed` left in the tree. */
function repo(home, name, branch, files = {}, changed = {}) {
  const at = path.join(home, "work", name);
  if (!fs.existsSync(path.join(at, ".git"))) {
    fs.mkdirSync(at, { recursive: true });
    git(at, "init", "-q", "-b", "main");
    git(at, "remote", "add", "origin", `https://github.com/example/${name}.git`);
    for (const [file, text] of Object.entries({ "README.md": `# ${name}\n`, ...files })) {
      fs.mkdirSync(path.dirname(path.join(at, file)), { recursive: true });
      fs.writeFileSync(path.join(at, file), text);
    }
    git(at, "add", "-A");
    git(at, "commit", "-q", "-m", "Start");
    if (branch !== "main") git(at, "checkout", "-q", "-b", branch);
    for (const [file, text] of Object.entries(changed)) fs.writeFileSync(path.join(at, file), text);
  }
  return fs.realpathSync(at);
}

function worktree(main, branch, dir) {
  if (!fs.existsSync(dir)) git(main, "worktree", "add", "-q", "-b", branch, dir);
  return fs.realpathSync(dir);
}

function nodeEnv(ctx, home) {
  const env = {
    ...process.env, HOME: home, SHELL: path.join(ctx.dir, "bin", "shell"), PATH: `${path.join(ctx.dir, "bin")}:${process.env.PATH}`,
    PINNED_OPENCODE_GO_MODELS_BASE: GO, PINNED_OPENCODE_GO_USAGE_URL: `${GO}/zen/go/v1/usage`, PINNED_OPENCODE_GO_BASE: `${GO}/inference/go/anthropic`, PINNED_PR_WATCH_SECS: "2",
  };
  for (const key of ["PINNED_SOCK", "PINNED_TASK", "PINNED_ACCOUNT", "PINNED_TAB", "CLAUDE_CONFIG_DIR"]) delete env[key];
  return env;
}

/** Another node of the stack's account, named `name`; stack.sh stop ends it through its pid file. */
function startNode(ctx, name) {
  const home = homeOf(ctx, name);
  const config = path.join(ctx.dir, `node-${name}`);
  const env = nodeEnv(ctx, home);
  fs.mkdirSync(path.join(home, "work"), { recursive: true });
  if (!fs.existsSync(config)) execFileSync(SERVER, ["login", ctx.hub, EMAIL, "--name", name, "--config-dir", config], { env: { ...env, PINNED_PASSWORD: PASSWORD }, stdio: "ignore" });
  const log = fs.openSync(path.join(ctx.dir, `node-${name}.log`), "a");
  const proc = spawn(SERVER, ["node", "--config-dir", config], { env, detached: true, stdio: ["ignore", log, log] });
  proc.unref();
  fs.writeFileSync(path.join(ctx.dir, `node-${name}.pid`), String(proc.pid));
  return proc;
}

// Runs in the page: every machine by name, online or not.
const machinesByName = () => Object.fromEntries([...machines].map(([id, m]) => [m.view.name, { id, online: !!m.view.online }]));

async function ensureMachines(page, ctx) {
  const named = () => page.evaluate(machinesByName);
  let byName = await ctx.waitFor("the stack's node", async () => {
    const m = await named();
    return (m["sweep-node"]?.online || m[HOST]?.online) && m;
  });
  if (!byName[HOST]) {
    await page.evaluate((id) => hubAction({ cmd: "rename_machine", id: hubIdOf(id), name: "mac-mini" }), byName["sweep-node"].id);
    byName = await ctx.waitFor(`the node renamed ${HOST}`, async () => ((await named())[HOST]?.online ? named() : null));
  }
  for (const name of WORLD.machines.filter((m) => m !== HOST && !WORLD.projects.find((p) => p.machine === m)?.offline)) {
    if (byName[name]?.online) continue;
    startNode(ctx, name);
    byName = await ctx.waitFor(`${name} online`, async () => ((await named())[name]?.online ? named() : null), 60000);
  }
  const offline = WORLD.projects.find((p) => p.offline);
  // The offline machine runs once, long enough to open its project's Captain, then stops.
  if (offline && byName[offline.machine]?.online !== false) {
    if (!byName[offline.machine]?.online) startNode(ctx, offline.machine);
    byName = await ctx.waitFor(`${offline.machine} online`, async () => ((await named())[offline.machine]?.online ? named() : null), 60000);
    const cwd = repo(homeOf(ctx, offline.machine), offline.name, "main");
    await page.evaluate(async ({ machine, cwd }) => revealTab(machine, await callOn(machine, { cmd: "open_orchestrator", path: cwd })), { machine: byName[offline.machine].id, cwd });
    const pidFile = path.join(ctx.dir, `node-${offline.machine}.pid`);
    process.kill(Number(fs.readFileSync(pidFile, "utf8")), "SIGTERM");
    fs.rmSync(pidFile);
    byName = await ctx.waitFor(`${offline.machine} offline`, async () => ((await named())[offline.machine]?.online === false ? named() : null), 30000);
  }
  return Object.fromEntries(Object.entries(byName).map(([n, m]) => [n, m.id]));
}

// Runs in the page: the key of the open thread titled `title`, if any.
const keyTitled = (title) => tabs.find((t) => threadTitle(t) === title)?.key ?? null;

/** Opens a thread in `cwd`, or without a project when `cwd` is null, sends `prompt`, retitles it
 * `title`, and waits for `until` statuses. */
async function thread(page, ctx, { machine, cwd, title, prompt, until, orchestrator = false }) {
  const found = await page.evaluate(keyTitled, title);
  if (found) return found;
  const key = await page.evaluate(async ({ machine, cwd, orchestrator }) => {
    const call = orchestrator ? { cmd: "open_orchestrator", path: cwd } : cwd ? { cmd: "open_tab", path: cwd, bypass: false } : { cmd: "open_scratch", bypass: false };
    const tab = await revealTab(machine, await callOn(machine, call));
    startTab(tab);
    return tab.key;
  }, { machine, cwd, orchestrator });
  await ctx.waitFor(`${title} to start`, () => page.evaluate((k) => tabByKey(k)?.view.running === true, key));
  await page.evaluate(({ key, text, title }) => {
    const tab = tabByKey(key);
    activate(tab);
    prompt.value = text;
    send();
    if (title !== text) changeMarks(threadKey(tab), () => (sideMarks.titles[threadKey(tab)] = title));
  }, { key, text: prompt, title });
  if (until) await ctx.waitFor(`${title} to reach ${until}`, () => page.evaluate(({ key, until }) => until.includes(statusOf(tabByKey(key))), { key, until }), 30000).catch((e) => ctx.warn(e.message));
  return key;
}

async function seedOnce(page, ctx) {
  const ids = await ensureMachines(page, ctx);
  const home = (m) => homeOf(ctx, m);
  for (const m of new Set(Object.values(T).map((t) => t.machine))) {
    fs.writeFileSync(path.join(home(m), "fake-turns.json"), JSON.stringify([...CAPTAIN_TURNS, ...LOOSE_TURNS], null, 1));
  }
  const pinned = repo(home(T.release.machine), "pinned", WORLD.captain.branch);
  // A `claude --worktree` checkout reports to the repository that holds it, so the updater's
  // thread joins the pinned project as on the board.
  const exclude = path.join(pinned, ".git", "info", "exclude");
  if (!fs.readFileSync(exclude, "utf8").includes(".claude/")) fs.appendFileSync(exclude, ".claude/\n");
  const updater = worktree(pinned, "fix/updater-retry", path.join(pinned, ".claude", "worktrees", "updater-retry"));
  const site = repo(home(T.hero.machine), "site", "fix/hero-spacing", { "src/components/Hero.css": HERO_CSS_BEFORE }, { "src/components/Hero.css": HERO_CSS_AFTER });
  const api = repo(home(T.rate.machine), "api", "feat/rate-limit");

  const keys = {};
  keys.two = await thread(page, ctx, { machine: ids[T.two.machine], cwd: null, title: T.two.title, prompt: "Should the Captain run two workers at a time?", until: ["done"] });
  keys.rate = await thread(page, ctx, { machine: ids[T.rate.machine], cwd: api, title: T.rate.title, prompt: T.rate.title, until: ["working"] });
  keys.hero = await thread(page, ctx, { machine: ids[T.hero.machine], cwd: site, title: T.hero.title, prompt: HERO_PROMPT, until: ["done"] });
  keys.updater = await thread(page, ctx, { machine: ids[T.updater.machine], cwd: updater, title: T.updater.title, prompt: T.updater.title, until: ["done"] });

  const captainKey = await page.evaluate(keyTitled, T.release.title);
  if (captainKey) keys.release = captainKey;
  else {
    const calls = WORLD.captain.workers.map((w) => ({
      task: { ci: CI_TASK, readme: README_BRIEF, changelog: CHANGELOG_TASK }[w.thread],
      title: w.title,
      target: { providerInstanceId: "claude", model: w.model },
    }));
    fs.writeFileSync(path.join(home(T.release.machine), "delegate-calls.jsonl"), calls.map((c) => JSON.stringify(c)).join("\n"));
    keys.release = await thread(page, ctx, { machine: ids[T.release.machine], cwd: pinned, title: T.release.title, prompt: WORLD.captain.prompt, orchestrator: true });
    await ctx.waitFor("the Captain's three workers", () => page.evaluate((k) => tabs.filter((t) => t.view.spawned_by === tabByKey(k).view.id).length >= 3, keys.release), 30000);
  }
  for (const w of WORLD.captain.workers) keys[w.thread] = await page.evaluate(keyTitled, w.title);
  await ctx.waitFor("each worker in its state", () => page.evaluate((k) => ["working", "needs-input", "done"].every((s, i) => statusOf(tabByKey(k[i])) === s), [keys.ci, keys.readme, keys.changelog]), 30000).catch((e) => ctx.warn(e.message));

  if (!(await page.evaluate(() => tabs.some(isGlobalOrchestratorTab)))) {
    await page.evaluate(async (machine) => {
      await callOn(machine, { cmd: "set_global_orchestrator_settings", settings: { host: true, name: "Orchestrator" } });
      await callOn(machine, { cmd: "open_global_orchestrator" });
    }, ids[HOST]);
    await ctx.waitFor("the orchestrator's thread", () => page.evaluate(() => tabs.some(isGlobalOrchestratorTab)), 30000);
  }
  // The sidebar lists threads newest first by when the page first saw them, which follows the order
  // the machines answer in. A drag fixes it, so the rows are put in the board's order as a drag would.
  await page.evaluate((order) => {
    const marked = order.map((key) => threadKey(tabByKey(key)));
    changeMarks(marked[0], () => (sideMarks.order = marked));
  }, ["release", "updater", "hero", "rate", "two"].map((id) => keys[id]));
  // The board's shelves: earlier conversations opened again without starting, as a connector's run
  // opens one, then snoozed or settled as the row's menu does it, without the undo notice. A
  // resumed thread carries its session, which is what the hub keeps marks by.
  await page.evaluate(async ({ machine, cwd, snoozed, settled }) => {
    const titles = [...Array.from({ length: snoozed }, (_, i) => `Snoozed thread ${i + 1}`), ...Array.from({ length: settled }, (_, i) => `Settled thread ${i + 1}`)];
    for (const [i, title] of titles.entries()) {
      const session = `shelf-${i + 1}`;
      if (tabs.some((t) => t.view.session === session)) continue;
      const tab = await revealTab(machine, await callOn(machine, { cmd: "open_run", run: { folder: cwd, args: ["--resume", session], env: [] } }));
      const key = threadKey(tab);
      changeMarks(key, () => {
        sideMarks.titles[key] = title;
        if (i < snoozed) sideMarks.snoozed[key] = { until: nowS() + 3 * 86400, at: nowS() };
        else sideMarks.settled[key] = nowS();
      });
    }
  }, { machine: ids[HOST], cwd: pinned, snoozed: 2, settled: 14 });
  return { machines: ids, threads: keys };
}

/**
 * Brings the stack to the boards' shared state and fills ctx.machines (name to machine id) and
 * ctx.threads (Sidebar board row id to tab key). The first call per process seeds; every call
 * waits for this page to list the threads, and puts back the draft this browser keeps.
 */
async function ensure(page, ctx) {
  ctx.warnings ??= [];
  ctx.warn ??= (m) => ctx.warnings.push(m);
  if (!ctx.threads) Object.assign(ctx, await seedOnce(page, ctx));
  await ctx.waitFor("every seeded thread on this page", () => page.evaluate((keys) => keys.every((k) => tabByKey(k)), Object.values(ctx.threads)), 30000);
  await page.evaluate(({ two, hero }) => {
    saveDraft(tabByKey(two), "Yes, two workers at a time.");
    if (active?.key !== hero) markUnread(tabByKey(hero));
    renderSidebar();
  }, ctx.threads);
}

module.exports = { EMAIL, PASSWORD, BOARD_DIR, WORLD, vals, ensure, homeOf };
