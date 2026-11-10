#!/usr/bin/env node
// Switches between every kind of thread on the real page and reports what each switch shows and
// how long it takes: a project's Captain, its workers and a worker's own worker, a thread without a
// project, the global orchestrator, a thread on a second machine, and the same threads after their
// node restarts and after an archive and unarchive. Each switch runs from the sidebar, the Waiting
// on you list, the command palette and a Captain report, at 1440x900 and on a 390x844 phone.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... PLAYWRIGHT=... node scripts/web-sweep/switching.js <out dir> <label>
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Starts a second node, sweep-mini, with HOME=$SWEEP_DIR/home2, and stops it at the end. Restarts
// the stack's node from its pid file, and writes the new pid back. Writes <label>-*.png and
// <label>.json, the timings and every check, into <out dir>. Exits 1 when a check fails.
const { spawn } = require("child_process");
const fs = require("fs");
const path = require("path");
const playwright = require(process.env.PLAYWRIGHT || "playwright");

const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const GO = `http://127.0.0.1:${process.env.SWEEP_GO_PORT || 8798}`;
const SERVER = path.join(process.env.CARGO_TARGET_DIR || path.join(__dirname, "../../target"), "debug/pinned-server");
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const OUT = path.resolve(process.argv[2] || path.join(__dirname, "screens"));
const LABEL = process.argv[3] || "switching";
const ROUNDS = Number(process.env.SWITCH_ROUNDS || 5);

const results = { label: LABEL, checks: [], timings: [], notes: [] };
const failed = [];
function check(ok, what, detail = "") {
  results.checks.push({ ok: !!ok, what, detail });
  console.log(`${ok ? "ok  " : "FAIL"} ${what}${detail ? ` (${detail})` : ""}`);
  if (!ok) failed.push(what);
}
const waitFor = async (what, fn, timeout = 30000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    const v = await fn();
    if (v) return v;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function nodeEnv(home) {
  return {
    ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !/^PINNED_|^CLAUDE_CONFIG_DIR$/.test(k))),
    HOME: home,
    SHELL: path.join(DIR, "bin/shell"),
    PATH: `${path.join(DIR, "bin")}:${process.env.PATH}`,
    PINNED_OPENCODE_GO_MODELS_BASE: GO,
    PINNED_OPENCODE_GO_USAGE_URL: `${GO}/zen/go/v1/usage`,
    PINNED_OPENCODE_GO_BASE: `${GO}/inference/go/anthropic`,
    PINNED_PR_WATCH_SECS: "2",
  };
}
function startNode(config, home, log) {
  const proc = spawn(SERVER, ["node", "--config-dir", config], { env: nodeEnv(home), stdio: ["ignore", fs.openSync(log, "a"), fs.openSync(log, "a")], detached: true });
  proc.unref();
  return proc.pid;
}
async function secondNode() {
  const home = path.join(DIR, "home2");
  fs.mkdirSync(path.join(home, "work"), { recursive: true });
  const repo = path.join(home, "work", "delta");
  if (!fs.existsSync(repo)) {
    require("child_process").execFileSync("git", ["init", "-q", repo]);
    require("child_process").execFileSync("git", ["-C", repo, "remote", "add", "origin", "https://github.com/example/delta.git"]);
  }
  const config = path.join(DIR, "node2");
  if (!fs.existsSync(config)) require("child_process").execFileSync(SERVER, ["login", HUB, EMAIL, "--name", "sweep-mini", "--config-dir", config], { env: { ...nodeEnv(home), PINNED_PASSWORD: PASSWORD } });
  const pid = startNode(config, home, path.join(DIR, "node2.log"));
  fs.writeFileSync(path.join(DIR, "node2.pid"), String(pid));
  return { pid, repo: fs.realpathSync(repo) };
}
async function restartNode() {
  const pid = Number(fs.readFileSync(path.join(DIR, "node.pid"), "utf8"));
  process.kill(pid, "SIGTERM");
  await waitFor("the node to exit", () => {
    try {
      process.kill(pid, 0);
      return false;
    } catch {
      return true;
    }
  });
  const fresh = startNode(path.join(DIR, "node"), path.join(DIR, "home"), path.join(DIR, "node.log"));
  fs.writeFileSync(path.join(DIR, "node.pid"), String(fresh));
  return fresh;
}

/** Appends `turns` finished turns to a session's transcript, as claude writes them, each prompt carrying `marker`. */
function growTranscript(home, cwd, session, marker, turns) {
  const file = path.join(home, ".claude", "projects", cwd.replace(/[^a-zA-Z0-9]/g, "-"), `${session}.jsonl`);
  const lines = [];
  for (let i = 1; i <= turns; i++) {
    const at = new Date(Date.now() - (turns - i) * 60000).toISOString();
    const body = `${marker} turn ${i}. ${"Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(4)}`;
    lines.push({ type: "user", uuid: `${marker}-u${i}`, timestamp: at, sessionId: session, message: { role: "user", content: `${marker} asks ${i}` } });
    lines.push({ type: "assistant", uuid: `${marker}-a${i}`, timestamp: at, sessionId: session, message: { role: "assistant", content: [{ type: "text", text: body }] } });
  }
  fs.appendFileSync(file, lines.map((l) => JSON.stringify(l)).join("\n") + "\n");
}

async function launch() {
  const browser = await playwright.chromium.launch({ executablePath: process.env.CHROME_BIN || undefined, args: ["--hide-scrollbars"] });
  const errors = [];
  const page = async (name, phone) => {
    const context = await browser.newContext(phone
      ? { viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true, colorScheme: "dark" }
      : { viewport: { width: 1440, height: 900 }, colorScheme: "dark" });
    const p = await context.newPage();
    p.name = name;
    p.on("pageerror", (e) => errors.push(`${name}: ${e.message}`));
    // On v0.17.22 claude's menu reader throws inside every render once a screen holds "Settings › Archive",
    // so a call that renders throws too: that counts as the page's exception, and the sweep goes on.
    const evaluate = p.evaluate.bind(p);
    p.evaluate = (fn, arg) => evaluate(fn, arg).catch((e) => {
      if (!/terminal-menu\.js/.test(e.stack ?? e.message)) throw e;
      errors.push(`${name}: ${e.message.split("\n")[0]}`);
    });
    p.on("console", (m) => m.type() === "error" && !/Failed to load resource/.test(m.text()) && errors.push(`${name} console: ${m.text()}`));
    return p;
  };
  return { browser, errors, page };
}

async function signIn(page) {
  await page.goto(HUB);
  await page.fill("#login [name=email]", EMAIL);
  await page.fill("#login [name=password]", PASSWORD);
  await page.click("#login .primary");
  await waitFor("the hub", () => page.evaluate(() => document.body.dataset.hub === "online"));
  await waitFor("both nodes", () => page.evaluate(() => [...machines.values()].filter((m) => m.view.online).length >= 2));
  await page.evaluate(() => setMode("gui"));
  // The app's login is locked; a call only the owner makes asks for the password, typed here.
  page.unlocker = setInterval(() => page.evaluate((password) => {
    const form = document.querySelector("dialog[open] form");
    if (!form?.querySelector("[name=password]")) return;
    form.querySelector("[name=password]").value = password;
    form.querySelector(".primary").click();
  }, PASSWORD).catch(() => {}), 300);
  await instrument(page);
}

/** Counts what the page asks the hub for and how often it draws, without changing either. */
const instrument = (page) => page.evaluate(() => {
  if (window.__sw) return;
  window.__sw = { frames: [], count: {}, time: {} };
  const send = hub.send.bind(hub);
  hub.send = (frame) => {
    __sw.frames.push({ t: performance.now(), type: frame.type, cmd: frame.call?.cmd ?? null, tab: frame.call?.tab ?? frame.tab ?? null });
    return send(frame);
  };
  for (const name of ["renderSidebar", "renderThread", "renderPanel", "renderGui", "renderComposer"]) {
    const f = window[name];
    window[name] = function (...args) {
      const t = performance.now();
      try {
        return f.apply(this, args);
      } finally {
        __sw.count[name] = (__sw.count[name] ?? 0) + 1;
        __sw.time[name] = (__sw.time[name] ?? 0) + performance.now() - t;
      }
    };
  }
});

/** Tab keys by name, read off the page. */
const keyOf = (page, find) => page.evaluate(find);

/** What the page shows for `key` once a switch to it settles. */
const shownFor = (page, key) => page.evaluate((key) => {
  const tab = tabByKey(key);
  return {
    active: active?.key ?? null,
    drawn: threadView.dataset.tab ?? null,
    text: threadView.innerText.slice(0, 4000),
    blank: threadView.querySelector(".blank")?.innerText ?? null,
    loading: /Loading…/.test(threadView.querySelector(".blank")?.innerText ?? ""),
    crumb: document.querySelector("#chat-head .crumb")?.innerText ?? "",
    title: tab ? threadTitle(tab) : null,
    sidebar: document.body.dataset.sidebar,
    focused: document.activeElement?.id || document.activeElement?.tagName || null,
    prompt: prompt.value,
    scrollTop: threadView.scrollTop,
    page: document.body.dataset.page ?? null,
  };
}, key);

/** Scrolls `away`'s thread up a third, switches to `to` through the sidebar and back, and returns
 * the first turn on screen before and after. */
async function scrollBack(page, away, to) {
  const firstTurn = () => page.evaluate(() => [...threadView.querySelectorAll(".lane > .turn")].find((el) => el.getBoundingClientRect().bottom > threadView.getBoundingClientRect().top + 1)?.dataset.turnId);
  const pick = async (key) => {
    await page.evaluate((k) => {
      toggleSidebar(true);
      document.activeElement?.blur();
      document.querySelector(`#thread-list .thread-row[data-tab="${k}"]`).click();
    }, key);
    await sleep(400);
  };
  await pick(away);
  await page.evaluate(() => {
    threadView.scrollTop = Math.floor(threadView.scrollHeight / 3);
    threadView.dispatchEvent(new Event("scroll"));
  });
  await sleep(300);
  const left = await firstTurn();
  await pick(to);
  await pick(away);
  return { left, back: await firstTurn() };
}

/** A device that opens the session after claude asked: a fresh page signs in, picks `key` in its
 * sidebar, and returns the card claude's menu shows there. */
async function lateCard(newPage, phone, key) {
  const late = await newPage(phone ? "late phone" : "late desktop", phone);
  await signIn(late);
  await late.evaluate((k) => {
    toggleSidebar(true);
    document.activeElement?.blur();
    document.querySelector(`#thread-list .thread-row[data-tab="${k}"]`).click();
  }, key);
  await waitFor("the permission card", () => late.evaluate(() => !approval.hidden), 5000).catch(() => {});
  return { page: late, ...(await cardOf(late)) };
}
const cardOf = (page) => page.evaluate(() => ({ shown: !approval.hidden, kind: approval.dataset.kind ?? null, buttons: [...approval.querySelectorAll("button")].map((b) => b.innerText.trim()).filter(Boolean) }));
const asksPermission = (card) => card.shown && card.kind === "approval" && card.buttons.includes("Approve") && card.buttons.includes("Decline");

/** Runs `how` in the page to switch to `key`, then waits for the thread to be drawn. Returns how
 * long the handler ran, how long until the thread's turns were on screen, and what the switch asked
 * the hub for in the next 600ms. */
async function timedSwitch(page, key, how, expect) {
  const r = await page.evaluate(async ({ key, how, expect }) => {
    const frames = __sw.frames.length;
    const counts = { ...__sw.count };
    const times = { ...__sw.time };
    const t0 = performance.now();
    let threw = null;
    try {
      new Function("key", how)(key);
    } catch (e) {
      threw = String(e);
    }
    const handler = performance.now() - t0;
    const drawn = () => active?.key === key && threadView.dataset.tab === key && (!expect || threadView.innerText.includes(expect));
    let shown = null;
    await new Promise((resolve) => {
      const tick = () => {
        if (drawn()) {
          shown = performance.now() - t0;
          return resolve();
        }
        if (performance.now() - t0 > 8000) return resolve();
        requestAnimationFrame(tick);
      };
      tick();
    });
    await new Promise((r) => setTimeout(r, 600));
    const asked = __sw.frames.slice(frames).filter((f) => f.t - t0 < 600).map((f) => f.cmd ?? f.type);
    const drew = Object.fromEntries(Object.entries(__sw.count).map(([k, v]) => [k, v - (counts[k] ?? 0)]));
    const spent = Object.fromEntries(Object.entries(__sw.time).map(([k, v]) => [k, Math.round((v - (times[k] ?? 0)) * 10) / 10]));
    return { handler, shown, asked, drew, spent, threw };
  }, { key, how, expect });
  return r;
}

const median = (xs) => {
  const s = xs.filter((x) => x != null).sort((a, b) => a - b);
  return s.length ? s[Math.floor(s.length / 2)] : null;
};

async function shoot(page, name) {
  await page.screenshot({ path: path.join(OUT, `${LABEL}-${name}.png`) });
}

(async () => {
  fs.mkdirSync(OUT, { recursive: true });
  const { browser, errors, page: newPage } = await launch();
  const mini = await secondNode();
  let exit = 0;
  try {
    const page = await newPage("desktop", false);
    await signIn(page);
    const ids = await page.evaluate(() => Object.fromEntries([...machines.entries()].filter(([, m]) => m.view.online).map(([id, m]) => [m.view.name, id])));
    const node = ids["sweep-node"];
    const remote = ids["sweep-mini"];
    const work = path.join(DIR, "home", "work");
    const alpha = fs.realpathSync(path.join(work, "alpha"));
    const beta = fs.realpathSync(path.join(work, "beta"));

    // A Captain in alpha that starts two workers, one still at it; one worker starts its own worker.
    const captain = await page.evaluate(async ({ node, alpha }) => {
      const tab = await revealTab(node, await callOn(node, { cmd: "open_orchestrator", path: alpha }));
      await startTab(tab);
      return tab.key;
    }, { node, alpha });
    await waitFor("the Captain to run", () => page.evaluate((k) => tabByKey(k)?.view.running, captain));
    const sendTo = (key, text) => page.evaluate(({ key, text }) => {
      activate(tabByKey(key));
      prompt.value = text;
      return send();
    }, { key, text });
    const delegate = async (from, calls) => {
      const before = await page.evaluate(() => tabs.length);
      fs.writeFileSync(path.join(DIR, "home", "delegate-calls.jsonl"), calls.map((c) => JSON.stringify(c)).join("\n"));
      await sendTo(from, "start the workers");
      await waitFor("the workers", () => page.evaluate((n) => tabs.length >= n, before + calls.length));
    };
    await delegate(captain, [
      { task: "Run the CI on main and report what fails. [hold]", title: "Run the CI on main", target: { providerInstanceId: "claude", model: "claude-sonnet-5-5" } },
      { task: "Write the changelog for v0.17.21.", title: "Changelog for v0.17.21", target: { providerInstanceId: "claude", model: "claude-haiku-5-5" } },
    ]);
    const captainId = await page.evaluate((k) => tabByKey(k).view.id, captain);
    const [held, worker] = await waitFor("both workers' sessions", () => page.evaluate((id) => {
      const kids = tabs.filter((t) => t.view.spawned_by === id).sort((a, b) => a.view.opened - b.view.opened);
      return kids.length === 2 && kids.every((t) => t.view.session) ? kids.map((t) => t.key) : null;
    }, captainId));
    await waitFor("the changelog worker to finish", () => page.evaluate((k) => tabByKey(k).view.status === "done", worker));
    // The Captain reads the queued calls too, so it takes the worker's report before the worker gets them.
    await waitFor("the Captain to take the worker's report", () => page.evaluate(async (k) => {
      const tab = tabByKey(k);
      const page = await callOn(tab.machine, { cmd: "thread", tab: tab.view.id, from: 0 });
      return tab.view.status === "done" && !tab.view.queued && page.items.some((i) => i.kind === "user" && /^\[pinned\] Delegated task .*"Changelog for v0\.17\.21".* finished/.test(i.text));
    }, captain));
    await delegate(worker, [{ task: "Check the release notes' links.", title: "Check the links", target: { providerInstanceId: "claude", model: "claude-haiku-5-5" } }]);
    const workerId = await page.evaluate((k) => tabByKey(k).view.id, worker);
    const grandchild = await waitFor("the worker's worker", () => page.evaluate((id) => tabs.find((t) => t.view.spawned_by === id && t.view.session)?.key, workerId));

    // A thread with no project, a plain beta thread that asks a question, the global orchestrator, and a thread on sweep-mini.
    const loose = await page.evaluate(async (node) => (await openScratchTab(node)).key, node);
    const asking = await page.evaluate(async ({ node, beta }) => (await openRemote(node, beta, false)).key, { node, beta });
    const global = await page.evaluate(async (node) => {
      const tab = await revealTab(node, await callOn(node, { cmd: "open_global_orchestrator" }));
      await startTab(tab);
      return tab.key;
    }, node);
    const far = await page.evaluate(async ({ remote, repo }) => (await openRemote(remote, repo, false)).key, { remote, repo: mini.repo });
    const named = { captain, held, worker, grandchild, loose, asking, global, far };
    for (const [name, key] of Object.entries(named)) {
      await waitFor(`${name} to run`, () => page.evaluate((k) => tabByKey(k)?.view.running, key));
      if (!["held", "worker", "grandchild"].includes(name)) await sendTo(key, name === "asking" ? "asks" : `hello from ${name}`);
    }
    await waitFor("every session", () => page.evaluate((keys) => keys.every((k) => tabByKey(k)?.view.session), Object.values(named)));
    await waitFor("beta's question", () => page.evaluate((k) => !!tabByKey(k).view.question, asking));
    // Long transcripts, so a switch has turns to draw and a place to scroll back to.
    const views = await page.evaluate((keys) => Object.fromEntries(keys.map(([n, k]) => [n, { session: tabByKey(k).view.session, cwd: tabByKey(k).view.cwd, machine: tabByKey(k).machine }])), Object.entries(named));
    for (const [name, v] of Object.entries(views)) growTranscript(v.machine === remote ? path.join(DIR, "home2") : path.join(DIR, "home"), v.cwd, v.session, `${name}-marker`, name === "global" ? 6 : 40);
    // A Captain's report in the global orchestrator's thread, which links to the Captain's tab.
    const captainTab = await page.evaluate((k) => ({ id: tabByKey(k).view.id, machine: tabByKey(k).machine }), captain);
    fs.appendFileSync(
      path.join(DIR, "home", ".claude", "projects", views.global.cwd.replace(/[^a-zA-Z0-9]/g, "-"), `${views.global.session}.jsonl`),
      `${JSON.stringify({ type: "user", uuid: "report-1", timestamp: new Date().toISOString(), sessionId: views.global.session, message: { role: "user", content: `[pinned] Captain alpha (sweep-node) [${captainTab.machine} ${captainTab.id}]: CI is green on main.` } })}\n`,
    );
    for (const [name, key] of Object.entries(named)) {
      await page.evaluate((k) => activate(tabByKey(k)), key);
      await waitFor(`${name}'s long transcript`, () => page.evaluate((k) => threadOf(tabByKey(k)).items.some((i) => i.kind === "user" && /-marker asks/.test(i.text)), key));
    }
    results.threads = Object.fromEntries(Object.entries(named).map(([n, k]) => [n, k]));
    // Thirty more threads, so the sidebar a switch redraws is the size of a working day's.
    await page.evaluate(async ({ node, beta }) => {
      for (let i = 0; i < 30; i++) await callOn(node, { cmd: "open_tab", path: beta, bypass: false });
      await refreshTabs(node);
    }, { node, beta });

    // A press on a row takes the focus off the prompt, which a synthetic click does not, so each click blurs first.
    const rowClick = `const row = document.querySelector('#thread-list .thread-row[data-tab="' + key + '"]'); if (!row) throw new Error('no row for ' + key); document.activeElement?.blur(); row.click();`;
    const globalClick = `document.activeElement?.blur(); document.querySelector('#thread-list [data-global-open]').click();`;
    const markers = Object.fromEntries(Object.keys(named).map((n) => [n, `${n}-marker asks`]));
    const kinds = ["captain", "worker", "grandchild", "held", "loose", "asking", "far", "global"];
    for (const name of kinds) {
      const runs = [];
      for (let i = 0; i < ROUNDS; i++) {
        const base = name === "loose" ? captain : loose;
        await page.evaluate((k) => activate(tabByKey(k)), base);
        await sleep(150);
        runs.push(await timedSwitch(page, named[name], name === "global" ? globalClick : rowClick, markers[name]));
      }
      const shown = await shownFor(page, named[name]);
      check(shown.active === named[name] && shown.drawn === named[name], `desktop sidebar: ${name} is the thread shown`, `${shown.active} drew ${shown.drawn}`);
      check(shown.text.includes(markers[name]) && !Object.entries(markers).some(([n, m]) => n !== name && shown.text.includes(m)), `desktop sidebar: ${name}'s own transcript, nothing of another thread's`);
      check(!shown.loading, `desktop sidebar: ${name} is not stuck on Loading…`);
      results.timings.push({ size: "desktop", via: "sidebar", kind: name, handler: median(runs.map((r) => r.handler)), shown: median(runs.map((r) => r.shown)), sidebar: median(runs.map((r) => r.spent.renderSidebar)), thread: median(runs.map((r) => r.spent.renderThread)), asked: runs.at(-1).asked, drew: runs.at(-1).drew });
    }
    await shoot(page, "desktop-captain");
    // A thread this page has not drawn yet, as after a reload: its transcript comes from the machine.
    for (const name of ["captain", "worker", "loose"]) {
      await page.evaluate(({ k, base }) => {
        activate(tabByKey(base));
        threads.delete(k);
      }, { k: named[name], base: name === "loose" ? captain : loose });
      await sleep(200);
      const r = await timedSwitch(page, named[name], rowClick, markers[name]);
      results.timings.push({ size: "desktop", via: "first visit", kind: name, handler: r.handler, shown: r.shown, sidebar: r.spent.renderSidebar, thread: r.spent.renderThread, asked: r.asked, drew: r.drew });
    }
    await page.evaluate(() => openGlobalProjects());
    await waitFor("the projects page", () => page.evaluate(() => !!document.querySelector(".gp-machine:not([disabled])")));
    const chip = await timedSwitch(page, captain, `document.querySelector('.gp-machine:not([disabled])').click();`, markers.captain);
    check(chip.shown != null, "desktop: the projects page's Captain chip opens the Captain", JSON.stringify({ shown: chip.shown }));

    await page.evaluate((k) => activate(tabByKey(k)), loose);
    const waitingRun = await timedSwitch(page, asking, `ui.waitingOpen = true; renderSidebar(); document.querySelector('#thread-list .waiting-row[data-waiting="' + key + '"]').click();`, markers.asking);
    check(waitingRun.shown != null, "desktop: the Waiting on you row opens beta's question thread", JSON.stringify(waitingRun));
    await page.evaluate((k) => activate(tabByKey(k)), loose);
    const paletteRun = await timedSwitch(page, worker, `openPalette({ query: threadTitle(tabByKey(key)) }); runPaletteItem(palette.shown.find((i) => i.title === esc(threadTitle(tabByKey(key))) && i.icon === "message-square"));`, markers.worker).catch((e) => ({ error: String(e) }));
    check(paletteRun.shown != null, "desktop: the palette opens the worker", JSON.stringify(paletteRun));

    const kept = await scrollBack(page, captain, worker);
    check(kept.left && kept.back === kept.left, "desktop: switching away and back keeps the Captain scrolled where it was", `left at ${kept.left}, came back to ${kept.back}`);
    await shoot(page, "desktop-scroll-back");

    await page.evaluate((k) => activate(tabByKey(k)), captain);
    await page.fill("#prompt", "draft for the Captain");
    await page.evaluate((k) => {
      document.activeElement?.blur();
      document.querySelector(`#thread-list .thread-row[data-tab="${k}"]`).click();
    }, worker);
    const onWorker = await shownFor(page, worker);
    check(onWorker.prompt === "", "desktop: the Captain's draft stays out of the worker's composer", JSON.stringify(onWorker.prompt));
    check(onWorker.focused === "prompt", "desktop: the prompt has the focus after a sidebar switch", onWorker.focused);
    await page.fill("#prompt", "draft for the worker");
    await page.evaluate((k) => document.querySelector(`#thread-list .thread-row[data-tab="${k}"]`).click(), captain);
    check((await shownFor(page, captain)).prompt === "draft for the Captain", "desktop: the Captain's draft comes back");
    await page.evaluate((k) => document.querySelector(`#thread-list .thread-row[data-tab="${k}"]`).click(), worker);
    check((await shownFor(page, worker)).prompt === "draft for the worker", "desktop: the worker's draft comes back");
    await page.evaluate(() => {
      prompt.value = "";
      prompt.dispatchEvent(new Event("input"));
    });
    await page.evaluate((k) => activate(tabByKey(k)), captain);
    await page.evaluate(() => {
      prompt.value = "";
      prompt.dispatchEvent(new Event("input"));
    });

    const race = await page.evaluate(async ({ a, b }) => {
      const tb = tabByKey(b);
      threads.delete(tb.key);
      const call = hub.request.bind(hub);
      hub.request = (frame) => (frame.call?.tab === tb.view.id ? new Promise((r) => setTimeout(r, 900)).then(() => call(frame)) : call(frame));
      activate(tb);
      activate(tabByKey(a));
      await new Promise((r) => setTimeout(r, 2000));
      hub.request = call;
      return { active: active.key, drawn: threadView.dataset.tab, text: threadView.innerText.slice(0, 3000) };
    }, { a: captain, b: held });
    check(race.active === captain && race.drawn === captain && race.text.includes(markers.captain) && !race.text.includes(markers.held), "desktop: A→B→A before B loads ends on A's transcript", `${race.active} drew ${race.drawn}`);

    const cdp = await page.context().newCDPSession(page);
    const listeners = async () => {
      const out = {};
      for (const [name, expr] of Object.entries({ window: "window", document: "document", thread: "threadView", prompt: "prompt", list: "threadList", socket: "hub.ws" })) {
        const { result } = await cdp.send("Runtime.evaluate", { expression: expr });
        out[name] = (await cdp.send("DOMDebugger.getEventListeners", { objectId: result.objectId })).listeners.length;
      }
      Object.assign(out, await page.evaluate(() => ({ attached: hub.attached.size, waiters: tabListed.length, views: viewListeners.length, pending: hub.pending.size })));
      return out;
    };
    const before = await listeners();
    const cycle = [captain, worker, grandchild, loose, asking, far, held];
    const watched = await page.evaluate(async (cycle) => {
      const from = __sw.frames.length;
      for (let i = 0; i < 40; i++) {
        activate(tabByKey(cycle[i % cycle.length]));
        await new Promise((r) => setTimeout(r, 40));
      }
      await new Promise((r) => setTimeout(r, 1500));
      const frames = __sw.frames.slice(from);
      const by = {};
      for (const f of frames) by[f.cmd ?? f.type] = (by[f.cmd ?? f.type] ?? 0) + 1;
      return by;
    }, cycle);
    const after = await listeners();
    check(JSON.stringify(before) === JSON.stringify(after), "desktop: 40 switches leave the listeners and hub subscriptions as they were", `${JSON.stringify(before)} → ${JSON.stringify(after)}`);
    results.notes.push({ what: "hub calls during 40 switches", by: watched });

    const phone = await newPage("phone", true);
    await signIn(phone);
    for (const name of ["captain", "worker", "loose", "far"]) {
      const runs = [];
      for (let i = 0; i < 3; i++) {
        await phone.evaluate((k) => {
          activate(tabByKey(k));
          toggleSidebar(true);
        }, name === "loose" ? captain : loose);
        await sleep(150);
        runs.push(await timedSwitch(phone, named[name], rowClick, markers[name]));
      }
      const shown = await shownFor(phone, named[name]);
      check(shown.active === named[name] && shown.drawn === named[name] && shown.sidebar === "closed", `phone sidebar: ${name} is shown and the sidebar closed`, `${shown.drawn} sidebar ${shown.sidebar}`);
      check(shown.focused !== "prompt", `phone sidebar: picking ${name} leaves the keyboard down`, `focus on ${shown.focused}`);
      results.timings.push({ size: "phone", via: "sidebar", kind: name, handler: median(runs.map((r) => r.handler)), shown: median(runs.map((r) => r.shown)), sidebar: median(runs.map((r) => r.spent.renderSidebar)), thread: median(runs.map((r) => r.spent.renderThread)), asked: runs.at(-1).asked, drew: runs.at(-1).drew });
    }
    await shoot(phone, "phone-worker");
    const phoneKept = await scrollBack(phone, captain, worker);
    check(phoneKept.left && phoneKept.back === phoneKept.left, "phone: switching away and back keeps the Captain scrolled where it was", `left at ${phoneKept.left}, came back to ${phoneKept.back}`);
    await shoot(phone, "phone-scroll-back");
    await phone.evaluate((k) => {
      activate(tabByKey(k));
      toggleSidebar(true);
    }, loose);
    await sleep(200);
    await phone.evaluate((k) => {
      openPalette({ query: threadTitle(tabByKey(k)) });
      runPaletteItem(palette.shown.find((i) => i.title === esc(threadTitle(tabByKey(k))) && i.icon === "message-square"));
    }, captain).catch((e) => results.notes.push({ what: "phone palette", error: String(e) }));
    await sleep(500);
    const viaPalette = await shownFor(phone, captain);
    check(viaPalette.active === captain && viaPalette.sidebar === "closed", "phone: a thread picked in the palette is not left under the open sidebar", `sidebar ${viaPalette.sidebar}`);
    await shoot(phone, "phone-palette");

    await page.evaluate((k) => activate(tabByKey(k)), global);
    await waitFor("the report row", () => page.evaluate(() => !!threadView.querySelector("[data-report-tab]")));
    const report = await timedSwitch(page, captain, `threadView.querySelector('[data-report-tab]').click();`, markers.captain);
    check(report.shown != null, "desktop: the Captain report row opens the Captain", JSON.stringify(report));

    // The worker's worker asks to run a command while another thread's answer shows "Settings › Archive",
    // a line v0.17.22's menu reader threw on in every render.
    await sendTo(grandchild, "/ask");
    await waitFor("the worker's worker to ask", () => page.evaluate((k) => /permission/.test(tabByKey(k).view.message ?? ""), grandchild));
    await sendTo(loose, "Settings › Archive");
    await waitFor("the line on the thread's screen", () => page.evaluate((k) => screenLines(tabByKey(k)).some((l) => l.includes("echo: Settings › Archive")), loose));
    for (const phoneSize of [false, true]) {
      const late = await lateCard(newPage, phoneSize, grandchild);
      check(asksPermission(late), `${phoneSize ? "phone" : "desktop"}: a permission prompt opened from the sidebar after it was asked shows Approve and Decline`, JSON.stringify(late));
      await shoot(late.page, `${phoneSize ? "phone" : "desktop"}-permission-late`);
      await late.page.context().close();
    }
    for (const [size, at] of [["desktop", page], ["phone", phone]]) {
      await at.evaluate((k) => activate(tabByKey(k)), captain);
      await timedSwitch(at, grandchild, `toggleSidebar(true); ${rowClick}`, markers.grandchild);
      const card = await cardOf(at);
      check(asksPermission(card), `${size}: the permission card is there after switching away and back`, JSON.stringify(card));
      await shoot(at, `${size}-permission-switch-back`);
    }

    await page.evaluate((k) => activate(tabByKey(k)), worker);
    await phone.evaluate((k) => {
      activate(tabByKey(k));
      toggleSidebar(false);
    }, grandchild);
    await page.fill("#prompt", "draft kept over the restart");
    const sessions = await page.evaluate((keys) => Object.fromEntries(Object.entries(keys).map(([n, k]) => [n, tabByKey(k).view.session])), named);
    await restartNode();
    const renamed = await waitFor("the node's tabs back under new ids", () => page.evaluate((sessions) => {
      const out = {};
      for (const [n, s] of Object.entries(sessions)) {
        const tab = tabs.find((t) => t.view.session === s);
        if (!tab) return null;
        out[n] = tab.key;
      }
      return out;
    }, sessions), 60000);
    check(renamed.worker !== named.worker, "restart: the worker came back under a new tab id", `${named.worker} → ${renamed.worker}`);
    await sleep(1500);
    const afterRestart = await shownFor(page, renamed.worker);
    check(afterRestart.active === renamed.worker && afterRestart.text.includes(markers.worker), "desktop restart: the worker stays on screen once its new tab is listed", `active ${afterRestart.active}`);
    check(afterRestart.prompt === "draft kept over the restart", "desktop restart: the worker's draft is still in the composer", JSON.stringify(afterRestart.prompt));
    const phoneAfter = await shownFor(phone, renamed.grandchild);
    check(phoneAfter.active === renamed.grandchild, "phone restart: the worker's worker stays on screen", `active ${phoneAfter.active}`);
    await shoot(page, "desktop-after-restart");
    await shoot(phone, "phone-after-restart");
    // Right after the restart each thread is starting: picking one shows its transcript, not a blank pane.
    for (const name of ["captain", "grandchild", "loose", "asking"]) {
      const r = await timedSwitch(page, renamed[name], rowClick, markers[name]);
      check(r.shown != null, `desktop restart: ${name} shows its transcript under its new id`, JSON.stringify({ shown: r.shown }));
    }
    const nested = await page.evaluate((r) => ({ worker: tabByKey(r.worker).view.spawned_by === tabByKey(r.captain).view.id, grandchild: tabByKey(r.grandchild).view.spawned_by === tabByKey(r.worker).view.id }), renamed);
    await sendTo(renamed.grandchild, "/ask");
    await waitFor("the restored worker's worker to ask", () => page.evaluate((k) => /permission/.test(tabByKey(k).view.message ?? ""), renamed.grandchild));
    await phone.evaluate((k) => activate(tabByKey(k)), renamed.loose);
    await timedSwitch(phone, renamed.grandchild, `toggleSidebar(true); ${rowClick}`, markers.grandchild);
    check(asksPermission(await cardOf(phone)), "phone restart: the permission card shows on the worker's worker under its new id", JSON.stringify(await cardOf(phone)));
    await shoot(phone, "phone-permission-after-restart");
    check(nested.worker && nested.grandchild, "restart: the workers still nest under their new parents' ids", JSON.stringify(nested));
    await page.evaluate((k) => activate(tabByKey(k)), renamed.global);
    await waitFor("the report row after the restart", () => page.evaluate(() => !!threadView.querySelector("[data-report-tab]")));
    const staleReport = await timedSwitch(page, renamed.captain, `threadView.querySelector('[data-report-tab]').click();`, markers.captain);
    check(staleReport.shown != null && staleReport.shown < 1500, "desktop restart: a report from before the restart still opens its Captain", JSON.stringify({ shown: staleReport.shown }));
    await sleep(2500);
    const notice = await page.evaluate(() => document.getElementById("side-notice").hidden ? "" : document.getElementById("side-notice").innerText);
    results.notes.push({ what: "notice after the stale report click", notice });
    await shoot(page, "desktop-report-after-restart");
    await phone.evaluate((k) => activate(tabByKey(k)), renamed.global);
    await waitFor("the report row on the phone", () => phone.evaluate(() => !!threadView.querySelector("[data-report-tab]")));
    const phoneReport = await timedSwitch(phone, renamed.captain, `threadView.querySelector('[data-report-tab]').click();`, markers.captain);
    check(phoneReport.shown != null && phoneReport.shown < 1500, "phone restart: a report from before the restart still opens its Captain", JSON.stringify({ shown: phoneReport.shown }));
    await sleep(2500);
    await shoot(phone, "phone-report-after-restart");

    await page.evaluate(async (k) => {
      sideSettings.confirmArchive = false;
      await archiveThread(tabByKey(k));
    }, renamed.loose);
    await waitFor("the loose thread to close", () => page.evaluate((s) => !tabs.some((t) => t.view.session === s), sessions.loose));
    await page.evaluate(() => openSettings("archive"));
    await waitFor("the archive page", () => page.evaluate(() => !!document.querySelector("[data-unarchive]")));
    await page.evaluate(() => document.querySelector("[data-unarchive]").click());
    const back2 = await waitFor("the unarchived thread", () => page.evaluate((s) => tabs.find((t) => t.view.session === s)?.key, sessions.loose), 20000);
    await waitFor("the unarchived transcript", () => page.evaluate((k) => active?.key === k && threadView.innerText.includes("loose-marker asks"), back2), 10000).catch(() => {});
    const unarchived = await shownFor(page, back2);
    check(unarchived.active === back2 && unarchived.text.includes(markers.loose) && !unarchived.page, "desktop: the unarchived thread opens with its transcript", JSON.stringify({ active: unarchived.active, page: unarchived.page }));
    await shoot(page, "desktop-unarchived");

    check(!errors.length, "no exception on any page", errors.slice(0, 5).join(" | "));
    results.errors = errors;
  } catch (e) {
    console.error(e);
    results.crash = String(e.stack ?? e);
    exit = 1;
  } finally {
    fs.writeFileSync(path.join(OUT, `${LABEL}.json`), JSON.stringify(results, null, 2));
    try {
      process.kill(mini.pid, "SIGTERM");
    } catch {}
    await browser.close();
  }
  console.log("\ntimings (ms, median):");
  for (const t of results.timings) console.log(`${t.size.padEnd(8)}${t.via.padEnd(12)}${t.kind.padEnd(11)} handler ${t.handler?.toFixed(1).padStart(6)}  shown ${t.shown?.toFixed(1).padStart(7)}  sidebar ${String(t.sidebar).padStart(5)}  thread ${String(t.thread).padStart(5)}  asked ${t.asked.join(",")}`);
  if (failed.length) console.log(`\n${failed.length} failed:\n- ${failed.join("\n- ")}`);
  process.exit(exit || (failed.length ? 1 : 0));
})();
