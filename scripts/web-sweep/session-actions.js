#!/usr/bin/env node
// Proves an agent runs the session actions through act, and that Agent activity shows them. An agent
// session in the sweep node stops, restarts and moves to another model a started thread, renames and
// pins it through mark_sessions, archives another, and asks to delete a third; the delete waits in
// Waiting on you, in a sentence naming the thread. Agent activity is screenshotted at 1440x900 and
// on a 390x844 phone, then the two are put side by side.
//
//   SWEEP_DIR=... scripts/web-sweep/stack.sh start
//   SWEEP_DIR=... node scripts/web-sweep/session-actions.js
//   SWEEP_DIR=... scripts/web-sweep/stack.sh stop
//
// Writes session-actions-desktop.png, session-actions-phone.png and session-actions-side-by-side.png
// into scripts/web-sweep/screens/.
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
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "session-actions-"));
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


/** The two screenshots next to each other on one dark canvas, the desktop's and the phone's, drawn
 * and captured by a Chrome of its own. */
async function sideBySide(desktop, phone, out) {
  const S = await launch("S", errors);
  const data = (file) => `data:image/png;base64,${fs.readFileSync(file).toString("base64")}`;
  const [width, height, gap] = [1440 + 32 + 390, 900, 32];
  await S.size(width, height);
  await S.goto("about:blank");
  await S.evaluate(async ({ sources, gap }) => {
    document.body.style.cssText = `margin:0;background:#18181b;display:flex;gap:${gap}px;align-items:flex-start`;
    document.body.innerHTML = sources.map((src) => `<img src="${src}" style="display:block">`).join("");
    await Promise.all([...document.images].map((img) => img.decode()));
  }, { sources: [data(desktop), data(phone)], gap });
  await S.screenshot(out);
}

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
      await waitFor(`"${ask}" answered in a Claude session`, () => A.evaluate(({ k, t }) => !!tabByKey(k).view.session && !turnLive(tabByKey(k)) && document.getElementById("thread").innerText.includes(`echo: ${t}`), { k: key, t: ask }), 30000);
      threads.push(await A.evaluate((k) => ({ key: k, id: tabByKey(k).view.id, session: tabByKey(k).view.session }), key));
    }
    const [t1, t2, t3] = threads;
    console.log(`1. A opened three started threads in alpha, sessions ${threads.map((t) => t.session).join(", ")}`);

    const agent = await A.evaluate(async ({ machine, dir }) => {
      const tab = await openTab(machine, dir);
      activate(tab);
      return tab.key;
    }, { machine: node, dir: alpha });
    await waitFor("claude running for the agent", () => A.evaluate((k) => tabByKey(k)?.view.running === true, agent), 30000);

    const asks = [
      ["mark_sessions", { changes: [{ session: t1.session, mark: { title: "Fix login" } }, { session: t1.session, mark: { pin: 0 } }] }],
      ["stop_turn", { tab: t1.id }],
      ["set_bypass", { tab: t1.id, on: false, cols: 120, rows: 32 }],
      ["restart_claude", { tab: t2.id, cols: 120, rows: 32 }],
      ["set_model", { tab: t2.id, model: "claude-sonnet-5-5", cols: 120, rows: 32 }],
      ["archive_session", { machine: node, tab: t3.id }],
    ];
    const answers = [];
    for (const [action, args] of asks) answers.push(await act(A, action, args));
    answers.forEach((answer, at) => check(answer.outcome === "done", `${asks[at][0]} answered ${JSON.stringify(answer)}`));
    const [renamed, , , restarted] = answers;
    console.log(`2. the agent's ${asks.map(([action]) => action).join(", ")} all answered done (mark_sessions ${JSON.stringify(renamed.result)}; restart_claude ${JSON.stringify(restarted.result)})`);

    const [asked] = await callThrough(A, [{ name: "act", input: { action: "delete_session", args: { machine: node, tab: t2.id }, request_id: "sweep-delete" } }]);
    check(asked.outcome === "pending" && asked.id, `delete_session answered ${JSON.stringify(asked)}`);
    await waitFor("thread 3 to leave A's sidebar", () => A.evaluate((k) => !document.querySelector(`.thread-row[data-tab="${k}"]`), t3.key));
    check(await A.evaluate((k) => !!document.querySelector(`.thread-row[data-tab="${k}"]`), t2.key), "thread 2 left the sidebar before the user approved its delete");
    console.log(`3. delete_session answered pending ${asked.id}; thread 3 is archived, thread 2 is still listed`);

    await A.click("#gui-settings");
    await waitFor("A's Settings page to draw", () => A.evaluate(() => settingsOpen() && document.querySelector("#settings-page h2")?.textContent === document.querySelector("#settings-nav .links .active span")?.textContent));
    await A.click('#settings-nav [data-page="agent-activity"]');
    await waitFor("Agent activity to list the acts", () => A.evaluate((n) => document.querySelectorAll("#agent-activity .agent-audit-row").length >= n, asks.length + 1), 30000);
    const page = await A.evaluate(() => ({
      waiting: [...document.querySelectorAll("#agent-activity .agent-confirm .title")].map((e) => e.textContent),
      heads: [...document.querySelectorAll("#agent-activity .agent-audit-row .head b")].map((e) => e.textContent),
    }));
    const named = (heads, text) => heads.some((h) => h === text);
    check(page.waiting.length === 1 && /^Delete “.+” for good$/.test(page.waiting[0]), `Waiting on you reads ${JSON.stringify(page.waiting)}`);
    for (const re of [/^Rename and pin a conversation$/, /^Stop the turn of “/, /^Ask for permissions in “/, /^Restart “/, /^Switch “.+” to claude-sonnet-5-5$/, /^Archive (“|a thread)/]) {
      check(page.heads.some((h) => re.test(h)), `Agent activity has no row reading ${re}: ${JSON.stringify(page.heads)}`);
    }
    check(!named(page.heads, "Stop turn"), "a session action still reads as its raw name");
    await A.evaluate(() => document.getElementById("settings-page").scrollTo(0, 0));
    const desktop = path.join(SCREENS, "session-actions-desktop.png");
    await A.screenshot(desktop);
    await A.size(390, 844, true);
    await sleep(500);
    const phone = path.join(SCREENS, "session-actions-phone.png");
    await A.screenshot(phone);
    await A.size(1440, 900);
    await sideBySide(desktop, phone, path.join(SCREENS, "session-actions-side-by-side.png"));
    console.log(`4. Agent activity waits with ${JSON.stringify(page.waiting)} and lists ${JSON.stringify(page.heads)}`);
    check(await A.evaluate(() => window.sweepNeverReloaded === true), "A reloaded");

    check(!errors.length, `page errors:\n${errors.slice(0, 5).join("\n")}`);
    console.log("5. no page exceptions");
    console.log("session actions: every check passed");
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
    console.log(`6. closed ${browsers.length} Chrome processes and removed their profiles`);
  }
})().catch((error) => {
  console.error(error.stack || String(error));
  if (errors.length) console.error(`page errors:\n${errors.join("\n")}`);
  process.exit(1);
});
