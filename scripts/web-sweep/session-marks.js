#!/usr/bin/env node
// Proves the sidebar's marks live on the account's hub. Two browsers signed in to one account, A
// and B, each its own Chrome profile: a rename, a pin and an archive made from A's row menu show in
// B at once, with no reload, at 1440x900 and on a 390x844 phone. A third browser, C, that kept
// marks in localStorage brings them up on its first sign-in, where the hub keeps any field it
// already has, and then forgets them. An agent session in A renames a thread and archives another
// through act's mark_sessions and archive_session, as Claude Code calls pinned's MCP server, and
// both browsers show it at once. Its delete_session waits for the user's approval, and the thread
// leaves both sidebars only once A approves.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/session-marks.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes session-marks-live.png, session-marks-agent.png and session-marks-pending.png (1440x900),
// and session-marks-phone.png (390x844), into scripts/web-sweep/screens/.
const { spawn } = require("child_process");
const fs = require("fs");
const os = require("os");
const path = require("path");

const HUB = `http://127.0.0.1:${process.env.SWEEP_HUB_PORT || 8797}`;
const DIR = process.env.SWEEP_DIR || path.join(process.env.TMPDIR || "/tmp", "pinned-web-sweep");
const HOME = path.join(DIR, "home");
const SCREENS = path.join(__dirname, "screens");
const EMAIL = "sweep@example.test";
const PASSWORD = "sweep password 1";
const STORED_MARKS = ["gui-marks", "gui-seen", "gui-archived"];

const check = (ok, what) => {
  if (!ok) throw new Error(what);
};
const waitFor = async (what, fn, timeout = 20000) => {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`timed out waiting for ${what}`);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const chromium = process.env.CHROME_BIN || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
/** Every Chrome this script started, closed in the `finally` whether or not its launch finished. */
const browsers = [];

async function launch(name, errors) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "session-marks-"));
  const proc = spawn(chromium, ["--headless=new", "--remote-debugging-port=0", "--remote-allow-origins=*", "--no-first-run",
    "--no-default-browser-check", "--disable-gpu", "--hide-scrollbars", "--use-mock-keychain", `--user-data-dir=${profile}`, "about:blank"], { stdio: ["ignore", "pipe", "pipe"] });
  browsers.push({ proc, profile });
  const ws = await new Promise((resolve, reject) => {
    const fail = setTimeout(() => reject(new Error(`${name}'s Chromium never said its DevTools port`)), 15000);
    let buf = "";
    const on = (chunk) => { buf += chunk; const m = buf.match(/DevTools listening on (ws:\/\/\S+)/); if (m) { clearTimeout(fail); resolve(m[1]); } };
    proc.stderr.on("data", on);
    proc.stdout.on("data", on);
    proc.on("exit", () => reject(new Error(`${name}'s Chromium exited early: ${buf.slice(-300)}`)));
  });
  const base = `http://${ws.match(/^ws:\/\/(\S+?)\//)[1]}`;
  const target = await (await fetch(`${base}/json/new?about:blank`, { method: "PUT" })).json();
  const socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = () => reject(new Error(`${name}'s DevTools socket did not open`)); });
  let id = 0;
  const pending = new Map();
  const listeners = new Set();
  socket.addEventListener("message", (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) {
      const { resolve, reject } = pending.get(m.id);
      pending.delete(m.id);
      m.error ? reject(new Error(m.error.message)) : resolve(m.result);
    } else listeners.forEach((l) => l(m));
  });
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    pending.set(++id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (fn, arg) => {
    const r = await send("Runtime.evaluate", { expression: `(${fn})(${JSON.stringify(arg ?? null)})`, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(`${name}: ${r.exceptionDetails.exception?.description || r.exceptionDetails.text}`);
    return r.result.value;
  };
  await send("Runtime.enable");
  await send("Page.enable");
  listeners.add((m) => m.method === "Runtime.exceptionThrown" && errors.push(`${name}: ${m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text}`));
  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });
  let scale = 1;
  const size = async (width, height, phone = false) => {
    scale = phone ? 3 : 1;
    await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: scale, mobile: phone });
    await send("Emulation.setTouchEmulationEnabled", { enabled: phone, maxTouchPoints: phone ? 5 : 1 });
  };
  await size(1440, 900);
  const pointOf = async (selector) => {
    const at = await evaluate((s) => {
      const el = document.querySelector(s);
      if (!el) return null;
      el.scrollIntoView({ block: "nearest" });
      const r = el.getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    }, selector);
    check(at, `${name} has no ${selector}`);
    return at;
  };
  const mouse = async ({ x, y }, button = "left") => {
    await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
    await send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button, buttons: button === "right" ? 2 : 1, clickCount: 1 });
    await send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button, buttons: 0, clickCount: 1 });
  };
  return {
    name,
    send,
    evaluate,
    size,
    async goto(url) {
      const loaded = new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error(`${name} timed out loading ${url}`)), 20000);
        const on = (m) => m.method === "Page.loadEventFired" && (clearTimeout(timer), listeners.delete(on), resolve());
        listeners.add(on);
      });
      await send("Page.navigate", { url });
      await loaded;
    },
    click: async (selector, button) => mouse(await pointOf(selector), button),
    async tap(selector) {
      const { x, y } = await pointOf(selector);
      await send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
      await send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    },
    /** Clicks the open row menu's item named `label`. */
    async menuPick(label) {
      await waitFor(`${name}'s row menu`, () => evaluate(() => !document.getElementById("side-menu").hidden));
      const item = await evaluate((l) => {
        const b = [...document.querySelectorAll("#side-menu button")].find((e) => e.querySelector(".label")?.textContent === l);
        if (!b) return { missing: [...document.querySelectorAll("#side-menu .label")].map((e) => e.textContent) };
        const r = b.getBoundingClientRect();
        return { disabled: b.disabled, x: r.left + r.width / 2, y: r.top + r.height / 2 };
      }, label);
      check(!item.missing, `${name}'s row menu has no "${label}": ${JSON.stringify(item.missing)}`);
      check(!item.disabled, `${name}'s "${label}" is disabled`);
      await mouse(item);
    },
    type: (text) => send("Input.insertText", { text }),
    async enter() {
      await send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: "\r" });
      await send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    },
    // Captured at CSS pixels, so the phone's 3x screen does not triple the committed file. On a desktop
    // the pointer rests on empty chat first, so no hover state lands in the shot.
    async screenshot(file) {
      if (scale === 1) await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: 1380, y: 660 });
      const { cssLayoutViewport: { clientWidth: width, clientHeight: height } } = await send("Page.getLayoutMetrics");
      const shot = await send("Page.captureScreenshot", { format: "png", clip: { x: 0, y: 0, width, height, scale: 1 / scale } });
      fs.writeFileSync(file, Buffer.from(shot.data, "base64"));
    },
  };
}

async function signIn(page) {
  await page.evaluate(({ email, password }) => {
    const form = document.querySelector("#login");
    form.querySelector("[name=email]").value = email;
    form.querySelector("[name=password]").value = password;
    form.querySelector(".primary").click();
  }, { email: EMAIL, password: PASSWORD });
  await waitFor(`${page.name} on the hub`, () => page.evaluate(() => document.body.dataset.hub === "online"));
  await waitFor(`the sweep node in ${page.name}`, () => page.evaluate(() => [...machines.values()].some((m) => m.view.online && m.view.name === "sweep-node")));
  // A reload would drop this, so each later check of it proves the page never reloaded.
  await page.evaluate(() => {
    setMode("gui");
    window.sweepNeverReloaded = true;
  });
  return page.evaluate(() => [...machines.keys()].find((k) => machines.get(k).view.name === "sweep-node"));
}

const row = (key) => `.thread-row[data-tab="${key}"]`;
const cardTitle = (page, key) => page.evaluate((s) => document.querySelector(s)?.querySelector(".l2")?.textContent ?? null, row(key));
const listed = (page, key) => page.evaluate((s) => !!document.querySelector(s), row(key));
const notReloaded = async (page) => check(await page.evaluate(() => window.sweepNeverReloaded === true), `${page.name} reloaded`);

// Runs in the page: the sidebar from the top, each element as a token.
const sidebarTokens = () => [...document.querySelectorAll("#thread-list > *")].map((el) => {
  if (el.matches(".side-heading")) return `heading ${el.textContent}`;
  if (el.matches(".project-head")) return `project ${el.querySelector(".name")?.textContent}`;
  if (el.matches(".thread-row")) return el.dataset.tab;
  return el.className;
});

/** The thread names `page`'s Settings › Archive lists once it lists `thread`'s conversation by `name`,
 * reached as a user clicks there: Settings, then Archive once the page Settings opened on has drawn.
 * openSettings("archive") is not used: General, drawn async, lands over the page opened after it. */
async function archiveLists(page, thread, name) {
  await page.click("#gui-settings");
  await waitFor(`${page.name}'s Settings page to draw`, () => page.evaluate(() => settingsOpen() && document.querySelector("#settings-page h2")?.textContent === document.querySelector("#settings-nav .links .active span")?.textContent));
  await page.click('#settings-nav [data-page="archive"]');
  const shown = () => page.evaluate(() => [...document.querySelectorAll("#archive-settings .archived label")].map((l) => l.textContent));
  const kept = () => page.evaluate(({ s, n }) => archived.some((a) => a.session === s && a.name === n), { s: thread.session, n: name });
  try {
    await waitFor(`${page.name}'s Archive page to list "${name}"`, async () => (await kept()) && (await shown()).includes(name)).catch(async (e) => {
      const state = await page.evaluate(async () => ({ open: settingsOpen(), page, heading: document.querySelector("#settings-page h2")?.textContent, archived, hub: (await hub.hub({ cmd: "session_marks" })).filter((r) => r.archived) }));
      throw new Error(`${e.message}; it lists ${JSON.stringify(await shown())}; ${JSON.stringify(state)}`);
    });
    await sleep(500);
    const list = await shown();
    check(list.includes(name) && (await kept()), `${page.name}'s Archive page dropped "${name}" again: it lists ${JSON.stringify(list)}`);
    return list;
  } finally {
    if (await page.evaluate(() => settingsOpen())) await page.click("#settings-back");
    await waitFor(`${page.name} to leave Settings`, () => page.evaluate(() => !settingsOpen()));
  }
}

// Every tool result the fake claude sessions wrote, newest last.
function toolResults(name) {
  const files = [];
  const walk = (dir) => fs.existsSync(dir) && fs.readdirSync(dir, { withFileTypes: true }).forEach((e) => {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) walk(p);
    else if (p.endsWith(".jsonl")) files.push(p);
  });
  walk(path.join(HOME, ".claude", "projects"));
  const uses = new Map();
  const results = [];
  for (const file of files) {
    for (const line of fs.readFileSync(file, "utf8").split("\n").filter(Boolean)) {
      const entry = JSON.parse(line);
      for (const part of Array.isArray(entry.message?.content) ? entry.message.content : []) {
        if (part.type === "tool_use") uses.set(`${file}:${part.id}`, part.name);
        if (part.type === "tool_result" && uses.get(`${file}:${part.tool_use_id}`) === `mcp__pinned__${name}`) results.push(part);
      }
    }
  }
  return results;
}

// One turn of the active thread that makes these pinned tool calls, as Claude Code makes them.
async function callThrough(page, calls) {
  fs.writeFileSync(path.join(HOME, "taste-calls.jsonl"), calls.map((c) => JSON.stringify(c)).join("\n"));
  const done = `Done: ${calls.map((c) => c.name).join(", ")}.`;
  const before = await page.evaluate((d) => document.getElementById("thread").innerText.split(d).length, done);
  await page.evaluate((t) => { prompt.value = t; send(); }, `call ${calls.map((c) => c.name).join(" ")}`);
  await waitFor(done, () => page.evaluate(({ d, n }) => document.getElementById("thread").innerText.split(d).length > n, { d: done, n: before }), 30000);
  return calls.map((c) => {
    const result = toolResults(c.name).at(-1);
    check(result && !result.is_error, `${c.name} failed: ${JSON.stringify(result)}`);
    return JSON.parse(result.content);
  });
}
const act = async (page, action, args) => (await callThrough(page, [{ name: "act", input: { action, args } }]))[0];

const errors = [];
(async () => {
  try {
    const A = await launch("A", errors);
    await A.goto(HUB);
    const node = await signIn(A);
    const alpha = fs.realpathSync(path.join(HOME, "work", "alpha"));
    const threads = [];
    for (const ask of ["login form", "release notes", "old spike"]) {
      const key = await A.evaluate(async ({ machine, dir }) => (await openTab(machine, dir)).key, { machine: node, dir: alpha });
      await waitFor(`claude running for "${ask}"`, () => A.evaluate((k) => tabByKey(k)?.view.running === true, key), 30000);
      await A.evaluate(({ k, t }) => {
        activate(tabByKey(k));
        prompt.value = t;
        send();
      }, { k: key, t: ask });
      // Claude's hooks name the session, so the node knows it from the first prompt on.
      await waitFor(`"${ask}" answered in a Claude session`, () => A.evaluate(({ k, t }) => !!tabByKey(k).view.session && !turnLive(tabByKey(k)) && document.getElementById("thread").innerText.includes(`echo: ${t}`), { k: key, t: ask }), 30000);
      threads.push(await A.evaluate((k) => ({ key: k, id: tabByKey(k).view.id, session: tabByKey(k).view.session }), key));
    }
    const [t1, t2, t3] = threads;
    const B = await launch("B", errors);
    await B.goto(HUB);
    await signIn(B);
    await waitFor("B to list A's three threads", () => B.evaluate((list) => list.every(({ key, session }) => tabByKey(key)?.view.session === session && !!document.querySelector(`.thread-row[data-tab="${key}"]`)), threads), 30000);
    console.log(`1. A opened three threads in alpha, sessions ${threads.map((t) => t.session).join(", ")}; B lists the same three`);

    await A.click(row(t1.key), "right");
    await A.menuPick("Rename thread");
    await waitFor("A's rename input", () => A.evaluate(() => !!document.activeElement?.matches("#thread-list input.rename")));
    await A.type("Fix login");
    await A.enter();
    await waitFor('A to show "Fix login"', async () => (await cardTitle(A, t1.key)) === "Fix login");
    await A.click(row(t2.key), "right");
    await A.menuPick("Pin thread");
    await waitFor("A to pin thread 2", () => A.evaluate((k) => sectionOf(tabByKey(k)) === "pinned", t2.key));
    t3.title = await A.evaluate((k) => threadTitle(tabByKey(k)), t3.key);
    await A.click(row(t3.key), "right");
    await A.menuPick("Archive thread");
    if (await A.evaluate(() => document.getElementById("side-dialog").open)) await A.click("#side-dialog .primary");
    await waitFor("A to archive thread 3", async () => !(await listed(A, t3.key)));

    await waitFor('B to show "Fix login" on thread 1', async () => (await cardTitle(B, t1.key)) === "Fix login");
    await waitFor("B to pin thread 2", () => B.evaluate((s) => !!document.querySelector(`${s} .pin`), row(t2.key)));
    const tokens = await B.evaluate(sidebarTokens);
    const firstProject = tokens.findIndex((t) => t.startsWith("heading ") || t.startsWith("project "));
    check(await B.evaluate((k) => sectionOf(tabByKey(k)) === "pinned", t2.key) && tokens.indexOf(t2.key) >= 0 && tokens.indexOf(t2.key) < firstProject,
      `B does not list thread 2 in Pinned, above the projects: ${JSON.stringify(tokens)}`);
    await waitFor("thread 3 to leave B's sidebar", async () => !(await listed(B, t3.key)));
    const archivedB = await archiveLists(B, t3, t3.title);
    await B.click(row(t1.key));
    await waitFor("B to open thread 1", () => B.evaluate((k) => active?.key === k, t1.key));
    await notReloaded(B);
    await B.screenshot(path.join(SCREENS, "session-marks-live.png"));
    await B.size(390, 844, true);
    // B was opened on a desktop, where the sidebar starts open; a phone shows the thread first.
    if (await B.evaluate(() => document.body.dataset.sidebar === "open")) {
      await B.tap(row(t1.key));
      await waitFor("the phone's sidebar to close on the tapped thread", () => B.evaluate(() => document.body.dataset.sidebar === "closed"));
    }
    await B.tap("#chat-sidebar-toggle");
    await waitFor("the phone's sidebar", () => B.evaluate(() => document.body.dataset.sidebar === "open" && !document.body.classList.contains("sidebar-animating")));
    check(await cardTitle(B, t1.key) === "Fix login" && await B.evaluate((s) => !!document.querySelector(`${s} .pin`), row(t2.key)), "the phone lost the rename or the pin");
    await B.screenshot(path.join(SCREENS, "session-marks-phone.png"));
    await B.size(1440, 900);
    await notReloaded(B);
    console.log(`2. B, never reloaded: thread 1 reads "Fix login", thread 2 is pinned above the projects (${JSON.stringify(tokens.slice(0, firstProject + 1))}), thread 3 left the sidebar, and Settings › Archive lists ${JSON.stringify(archivedB)}`);

    const C = await launch("C", errors);
    const seeded = {
      "gui-marks": JSON.stringify({ titles: { [`session:${t2.session}`]: "Migrated name" }, pins: [], order: [], settled: {}, unsettled: {}, snoozed: {}, noAutoSettle: {} }),
      "gui-seen": "{}",
    };
    const { identifier } = await C.send("Page.addScriptToEvaluateOnNewDocument", {
      source: `if (location.origin === ${JSON.stringify(HUB)}) for (const [k, v] of Object.entries(${JSON.stringify(seeded)})) localStorage.setItem(k, v);`,
    });
    await C.goto(HUB);
    await C.send("Page.removeScriptToEvaluateOnNewDocument", { identifier });
    check(await C.evaluate((k) => sideMarks.titles[k] === "Migrated name", `session:${t2.session}`), "C's page did not load the seeded gui-marks");
    await signIn(C);
    await waitFor('A to show "Migrated name" on thread 2', async () => (await cardTitle(A, t2.key)) === "Migrated name");
    await waitFor('B to show "Migrated name" on thread 2', async () => (await cardTitle(B, t2.key)) === "Migrated name");
    await waitFor("C to forget its stored marks", () => C.evaluate((keys) => keys.every((k) => localStorage.getItem(k) === null), STORED_MARKS));
    const hubRow2 = (await A.evaluate(() => hub.hub({ cmd: "session_marks" }))).find((r) => r.session === t2.session);
    check(hubRow2?.title === "Migrated name" && hubRow2.pin != null, `the hub's row for thread 2: ${JSON.stringify(hubRow2)}`);
    await notReloaded(A);
    await notReloaded(B);
    console.log(`3. C brought "Migrated name" up for thread 2; A and B show it with no reload; C's storage holds none of ${STORED_MARKS.join(", ")}; the hub's row ${JSON.stringify(hubRow2)}`);

    const agent = await A.evaluate(async ({ machine, dir }) => {
      const tab = await openTab(machine, dir);
      activate(tab);
      return tab.key;
    }, { machine: node, dir: alpha });
    await waitFor("claude running for the agent", () => A.evaluate((k) => tabByKey(k)?.view.running === true, agent), 30000);
    const renamed = await act(A, "mark_sessions", { changes: [{ session: t1.session, mark: { title: "Renamed by agent" } }] });
    check(renamed.outcome === "done", `mark_sessions answered ${JSON.stringify(renamed)}`);
    const archived = await act(A, "archive_session", { machine: node, tab: t2.id });
    check(archived.outcome === "done", `archive_session answered ${JSON.stringify(archived)}`);
    await waitFor('B to show "Renamed by agent" on thread 1', async () => (await cardTitle(B, t1.key)) === "Renamed by agent");
    await waitFor("thread 2 to leave B's sidebar", async () => !(await listed(B, t2.key)));
    const archivedAgent = await archiveLists(B, t2, "Migrated name");
    await notReloaded(B);
    check(await A.evaluate((k) => active?.key === k && !settingsOpen(), agent), "A does not show the agent's thread");
    for (const fold of ["#thread details.work:not([open]) > summary", "#thread details.tool:not([open]) > summary"]) {
      while (await A.evaluate((s) => !!document.querySelector(s), fold)) await A.click(fold);
    }
    await waitFor("the agent's two act calls unfolded", () => A.evaluate(() => [...document.querySelectorAll("#thread details.work details.tool")].filter((d) => d.open && /mark_sessions|archive_session/.test(d.innerText)).length === 2));
    await sleep(400);
    const steps = await A.evaluate(() => [...document.querySelectorAll("#thread details.work .steps")].map((e) => e.innerText.replace(/\s+/g, " ").trim().slice(0, 160)));
    await A.screenshot(path.join(SCREENS, "session-marks-agent.png"));
    console.log(`4. the agent's mark_sessions answered ${JSON.stringify(renamed)}; archive_session answered ${JSON.stringify(archived)}; its thread shows ${JSON.stringify(steps)}; B, never reloaded, shows "Renamed by agent", no longer lists thread 2, and Settings › Archive lists ${JSON.stringify(archivedAgent)}`);

    const asked = await act(A, "delete_session", { machine: node, tab: t1.id });
    check(asked.outcome === "pending" && asked.id, `delete_session answered ${JSON.stringify(asked)}`);
    await sleep(3000);
    check((await listed(A, t1.key)) && (await listed(B, t1.key)), "thread 1 left a sidebar before the user approved its delete");
    const waiting = (await A.evaluate(() => hub.hub({ cmd: "confirmations" }))).find((c) => c.id === asked.id);
    check(waiting && !waiting.decision && waiting.action === "delete_session", `confirmations lists ${JSON.stringify(waiting)}`);
    await B.screenshot(path.join(SCREENS, "session-marks-pending.png"));
    const decided = await A.evaluate((id) => hub.hub({ cmd: "decide_confirmation", id, approve: true }), asked.id);
    check(decided.outcome === "done", `the approval came to ${JSON.stringify(decided)}`);
    await waitFor("thread 1 to leave A's sidebar", async () => !(await listed(A, t1.key)));
    await waitFor("thread 1 to leave B's sidebar", async () => !(await listed(B, t1.key)));
    // Long enough for a browser that still held thread 1's marks to send them back.
    await sleep(2000);
    const left = (await A.evaluate(() => hub.hub({ cmd: "session_marks" }))).find((r) => r.session === t1.session);
    check(!left, `session_marks still has thread 1's row: ${JSON.stringify(left)}`);
    await notReloaded(A);
    await notReloaded(B);
    console.log(`5. delete_session answered pending ${asked.id}; 3 s later thread 1 was still in A and B and confirmations listed it waiting; approved, it came to "${decided.outcome}", thread 1 left A and B with no reload, and session_marks has no row for ${t1.session}`);

    check(!errors.length, `page errors:\n${errors.slice(0, 5).join("\n")}`);
    console.log("6. no page exceptions in A, B or C");
    console.log("session marks: every check passed");
  } finally {
    for (const { proc, profile } of browsers) {
      if (proc.exitCode === null && proc.signalCode === null) {
        const exited = new Promise((resolve) => proc.once("exit", () => resolve(true)));
        proc.kill("SIGTERM");
        if (!(await Promise.race([exited, sleep(5000)]))) {
          proc.kill("SIGKILL");
          await exited;
        }
      }
      fs.rmSync(profile, { recursive: true, force: true, maxRetries: 5 });
    }
    console.log(`7. closed ${browsers.length} Chrome processes and removed their profiles`);
  }
})().catch((error) => {
  console.error(error.stack || String(error));
  if (errors.length) console.error(`page errors:\n${errors.join("\n")}`);
  process.exit(1);
});
