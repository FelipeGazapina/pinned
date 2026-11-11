import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const connectionSrc = read("connection.js");
const hubSrc = read("hub.js");
const sidebarSrc = read("gui-sidebar.js");
const accountSrc = read("account.js");

const flush = () => new Promise((resolve) => setImmediate(resolve));
// Values built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => JSON.parse(JSON.stringify(value));
const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));
const fn = (src, name, async = false) => {
  const head = `${async ? "async " : ""}function ${name}(`;
  const at = src.indexOf(`\n${head}`) + 1;
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};

const NOW = 1_800_000_000;
const seen = (unix) => `seen ${Math.round((NOW - unix) / 60)} min ago`;
const machine = (name, online, last_seen = NOW - 60) => ({ view: { name, online, last_seen }, tabs: [] });

/** connection.js on its own. */
function connection() {
  const context = { URL };
  vm.createContext(context);
  vm.runInContext(`${connectionSrc}\nObject.assign(globalThis, { offlineMachines, offlineLine, emptyListText, signInProblem, hubUnreachable });`, context);
  return context;
}

test("an offline machine is named with when it was last seen, an online one is not", () => {
  const { offlineMachines, offlineLine } = connection();
  const machines = new Map([
    ["local", machine("This Mac", true, 0)],
    ["a", machine("mini", false)],
    ["b", machine("sweep-node", true)],
    ["c", machine("box", false, 0)],
  ]);
  const away = offlineMachines(machines, { local: "local", threadsSeen: {}, seen, name: (id) => machines.get(id).view.name });
  assert.deepEqual(here(away.map(offlineLine)), ["box offline", "mini offline · seen 1 min ago"]);
});

test("a machine known to have had no threads is not named, one with threads or unknown is", () => {
  const { offlineMachines } = connection();
  const machines = new Map([
    ["empty", machine("empty-box", false)],
    ["busy", machine("busy-box", false)],
    ["new", machine("new-box", false)],
  ]);
  const away = offlineMachines(machines, { local: "local", threadsSeen: { empty: false, busy: true }, seen, name: (id) => machines.get(id).view.name });
  assert.deepEqual(here(away.map((m) => m.name)), ["busy-box", "new-box"]);
});

test("no machine offline, nothing is named, and the empty list says no threads yet", () => {
  const { offlineMachines, emptyListText } = connection();
  const machines = new Map([["local", machine("This Mac", true, 0)], ["a", machine("mini", true)]]);
  assert.deepEqual(here(offlineMachines(machines, { local: "local", threadsSeen: {}, seen, name: () => "mini" })), []);
  assert.equal(emptyListText(false), "No threads yet");
  assert.equal(emptyListText(true), "No threads to show while machines are offline");
});

test("a machine seen online is remembered as having had threads, and closing them on the way out doesn't undo it", () => {
  const stored = [];
  const machines = new Map([["a", machine("mini", true)], ["b", machine("box", true)], ["local", machine("This Mac", true, 0)]]);
  const context = { machines, LOCAL: "local", load: () => null, save: (key, value) => stored.push([key, value]) };
  vm.createContext(context);
  vm.runInContext(`${sidebarSrc.slice(sidebarSrc.indexOf("const threadsSeen"), sidebarSrc.indexOf("/** One line for a machine that is offline"))}\nObject.assign(globalThis, { threadsSeen, noteThreadsSeen });`, context);
  context.noteThreadsSeen("a", 0);
  context.noteThreadsSeen("b", 2);
  context.noteThreadsSeen("local", 3);
  assert.deepEqual(here(context.threadsSeen), { a: false, b: true });
  context.noteThreadsSeen("a", 1);
  context.noteThreadsSeen("b", 0);
  assert.deepEqual(here(context.threadsSeen), { a: true, b: true });
  machines.get("a").view.online = false;
  context.noteThreadsSeen("a", 0);
  assert.deepEqual(here(context.threadsSeen), { a: true, b: true }, "offline changes nothing");
  assert.deepEqual(JSON.parse(stored.at(-1)[1]), { a: true, b: true });
  assert.equal(stored.at(-1)[0], "machine-threads");
});

const element = () => ({
  className: "",
  dataset: {},
  attributes: {},
  innerHTML: "",
  title: "",
  type: "",
  classList: {
    names: new Set(),
    toggle(name, on) {
      this.names[on ? "add" : "delete"](name);
    },
  },
  setAttribute(name, value) {
    this.attributes[name] = String(value);
  },
});

/** The sidebar nav's Orchestrator row on a page whose machines are `entries`, with the rest stubbed. */
function orchestratorNav(entries) {
  const machines = new Map(entries);
  const part = () => ({
    ...element(),
    textContent: "",
    hidden: false,
    style: {},
    parts: {},
    removeAttribute(name) {
      delete this.attributes[name];
    },
    querySelector(selector) {
      return (this.parts[selector] ??= part());
    },
  });
  const rows = {};
  const context = {
    machines,
    LOCAL: "local",
    active: null,
    globalSettings: { name: "Orchestrator" },
    askGlobalSettings() {},
    navCounts: () => ({ needsYou: 0, orchestrator: 0, reviews: 0, usage: null, machinesOnline: 0, machinesTotal: machines.size }),
    shellPage: () => null,
    navRow: (name) => (rows[name] ??= part()),
    isGlobalOrchestratorTab: () => false,
    machineName: (id) => machines.get(id)?.view.name ?? "machine",
    globalHostMachine: () => [...machines.entries()].find(([, m]) => m.view.online)?.[0] ?? null,
    document: { getElementById: () => part() },
    accountButton: part(),
    bindingLabel: () => null,
  };
  vm.createContext(context);
  vm.runInContext(`${line(sidebarSrc, "const setText")}\n${fn(sidebarSrc, "drawCount")}\n${fn(sidebarSrc, "drawNav")}\ndrawNav();`, context);
  return rows.orchestrator;
}

test("with every machine offline the Orchestrator row stays, disabled, with the machine's name and why", () => {
  const row = orchestratorNav([["a", machine("sweep-node", false)], ["b", machine("sweep-node-2", false)]]);
  assert.equal(row.hidden, false);
  assert.equal(row.attributes["aria-disabled"], "true");
  assert.ok(row.classList.names.has("disabled"));
  assert.equal(row.title, "sweep-node is offline, so the orchestrator can't open");
  assert.equal(row.parts[".host"].textContent, "sweep-node · offline");
  assert.equal(row.parts[".name"].textContent, "Orchestrator");
});

test("with a machine online the Orchestrator row is on it and not disabled", () => {
  const row = orchestratorNav([["a", machine("sweep-node", false)], ["b", machine("sweep-node-2", true)]]);
  assert.ok(!row.classList.names.has("disabled"));
  assert.equal(row.attributes["aria-disabled"], undefined);
  assert.equal(row.title, "");
  assert.equal(row.parts[".host"].textContent, "sweep-node-2");
});

test("a machine that dropped is named in the sidebar by a note with its name and last seen", () => {
  const context = { document: { createElement: element }, esc: (text) => String(text) };
  vm.createContext(context);
  vm.runInContext(`${fn(sidebarSrc, "machineNote")}\nthis.note = machineNote;`, context);
  const note = context.note({ id: "m-1", name: "mini", seen: "seen 1 min ago" });
  assert.equal(note.className, "machine-note");
  assert.equal(note.attributes.role, "status");
  assert.equal(note.innerHTML, `<span class="dot"></span><span class="text"><b>mini</b> offline · seen 1 min ago</span>`);
});

test("the sidebar's timers stop where the hub link dropped and run again when it is back", () => {
  const rows = [{ dataset: { since: "1000" }, textContent: "" }];
  let now = 1600;
  const context = {
    nowS: () => now,
    durationLabel: (s) => `${s}s`,
    threadList: { querySelectorAll: () => rows },
    document: { querySelectorAll: () => [] },
  };
  vm.createContext(context);
  vm.runInContext(`let linkDownAt = null;\n${fn(sidebarSrc, "noteHubLink")}\n${line(sidebarSrc, "const clockMs")}\n${fn(sidebarSrc, "tickDurations")}\nObject.assign(globalThis, { noteHubLink, tickDurations, clockMs });`, context);
  context.tickDurations();
  assert.equal(rows[0].textContent, "600s");
  context.noteHubLink(false);
  now = 1900;
  context.tickDurations();
  assert.equal(rows[0].textContent, "600s", "held at the moment the link dropped");
  context.noteHubLink(false);
  now = 2000;
  context.tickDurations();
  assert.equal(rows[0].textContent, "600s", "a second drop report doesn't move it");
  context.noteHubLink(true);
  context.tickDurations();
  assert.equal(rows[0].textContent, "1000s");
});

test("the thread's own timer reads the same held clock", () => {
  const context = { nowS: () => 1900, Date: { now: () => 1_900_000 } };
  vm.createContext(context);
  vm.runInContext(`let linkDownAt = null;\n${fn(sidebarSrc, "noteHubLink")}\n${line(sidebarSrc, "const clockMs")}\nObject.assign(globalThis, { noteHubLink, clockMs });`, context);
  assert.equal(context.clockMs(), 1_900_000);
  context.nowS = () => 1700;
  context.noteHubLink(false);
  assert.equal(context.clockMs(), 1_700_000, "held at the drop, in ms");
  context.noteHubLink(true);
  assert.equal(context.clockMs(), 1_900_000);
  assert.match(read("gui-timeline.js"), /ticking\(clockMs\(\) - Number\(ticker\.dataset\.since\)\)/);
});

/** hub.js on sockets the test closes itself. `draws` are the jitter draws in order; waits are what the
 * socket asked setTimeout for. */
function app({ draws = [], start = 0 } = {}) {
  const sockets = [];
  const waits = [];
  const reports = [];
  const time = { now: start };
  class WebSocket {
    static OPEN = 1;
    constructor() {
      this.readyState = 1;
      sockets.push(this);
    }
    send() {}
    close() {}
  }
  const context = {
    WebSocket,
    setTimeout: (run, ms) => (waits.push(ms), (pending.run = run)),
    draw: () => (draws.length ? draws.shift() : 0.5),
    now: () => time.now,
    reports,
  };
  const pending = {};
  vm.createContext(context);
  vm.runInContext(`${hubSrc}\nthis.hub = new Hub("https://hub.example", "login", { online: (on) => reports.push(on) }, { draw, now });`, context);
  const open = () => sockets.at(-1).onmessage({ data: JSON.stringify({ type: "hello", tabs: {} }) });
  const drop = (code) => {
    sockets.at(-1).onclose({ code });
    pending.run?.();
  };
  return { hub: context.hub, sockets, waits, reports, time, open, drop };
}

test("a hub that stays away is retried after 1, 2, 4 … 30 s, and no wait passes 30 s", () => {
  const { drop, waits } = app();
  for (let i = 0; i < 8; i++) drop(1006);
  assert.deepEqual(waits, [1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000]);
});

test("each wait varies by at most 25% either way, from an injected draw, and still never passes 30 s", () => {
  const { drop, waits } = app({ draws: [0, 0.999999, 0, 1, 0, 0.999999, 0.999999, 0.999999] });
  for (let i = 0; i < 8; i++) drop(1006);
  assert.deepEqual(waits, [750, 2500, 3000, 10000, 12000, 30000, 30000, 30000]);
  for (const [i, wait] of waits.entries()) {
    const base = Math.min(1000 * 2 ** i, 30000);
    assert.ok(wait >= base * 0.75 - 1 && wait <= Math.min(base * 1.25 + 1, 30000), `wait ${i} is ${wait}`);
  }
});

test("a connection that stayed up 30 s starts the waits over, one that dropped at once does not", () => {
  const { drop, open, time, waits } = app();
  drop(1006);
  drop(1006);
  assert.deepEqual(waits, [1000, 2000]);
  open();
  time.now += 29_999;
  drop(1006);
  assert.equal(waits.at(-1), 4000, "29.999 s is not stable, the wait keeps growing");
  open();
  time.now += 30_000;
  drop(1006);
  assert.equal(waits.at(-1), 1000, "30 s is stable, back to 1 s");
  drop(1006);
  assert.equal(waits.at(-1), 2000, "and doubling again from there");
});

test("the hub closing with 1001, going away, is retried in about a second even after many misses", () => {
  const { drop, open, waits } = app({ draws: [0.5, 0.5, 0.5, 0.5, 0, 0.999999] });
  for (let i = 0; i < 4; i++) drop(1006);
  assert.deepEqual(waits, [1000, 2000, 4000, 8000]);
  open();
  drop(1001);
  assert.equal(waits.at(-1), 750, "the lowest quick reconnect is 0.75 s");
  open();
  drop(1001);
  assert.equal(waits.at(-1), 1250, "the highest is 1.25 s");
  drop(1006);
  assert.equal(waits.at(-1), 2000, "a plain drop after one starts the climb again");
});

test("the app tells its UI the link is down once per outage, and up when the hub answers", () => {
  const { drop, open, reports } = app();
  drop(1006);
  drop(1006);
  drop(1006);
  assert.deepEqual(reports, [false], "down once, however many tries fail");
  open();
  assert.deepEqual(reports, [false, true]);
  drop(1001);
  assert.deepEqual(reports, [false, true, false]);
});

test("closing the hub on purpose reports no outage and schedules no retry", () => {
  const { hub, sockets, reports, waits } = app();
  hub.close();
  sockets.at(-1).onclose({ code: 1005 });
  assert.deepEqual(reports, []);
  assert.deepEqual(waits, []);
});

test("a refused sign-in is not retried or reported as an outage", () => {
  const { sockets, reports, waits } = app();
  sockets.at(-1).onclose({ code: 1008 });
  assert.deepEqual(reports, []);
  assert.deepEqual(waits, []);
});

test("sign-in says plainly when the hub can't be reached, in each browser's words and the app's", () => {
  const { signInProblem, hubUnreachable } = connection();
  const plain = "Can't reach the hub at hub.example.test. Check your connection and try again.";
  for (const error of [new TypeError("Failed to fetch"), "TypeError: Load failed", "NetworkError when attempting to fetch resource.", "error sending request for url (https://hub.example.test/api/login): error trying to connect: tcp connect error: Connection refused (os error 61)", "operation timed out"]) {
    assert.equal(signInProblem(error, "https://hub.example.test/"), plain, String(error));
    assert.equal(hubUnreachable(error), true);
  }
  assert.equal(signInProblem("TypeError: Failed to fetch", "http://127.0.0.1:1"), "Can't reach the hub at 127.0.0.1:1. Check your connection and try again.");
});

test("sign-in passes the hub's own answers through, such as a wrong password", () => {
  const { signInProblem, hubUnreachable } = connection();
  assert.equal(signInProblem("wrong email or password", "https://hub.example.test"), "wrong email or password");
  assert.equal(hubUnreachable("wrong email or password"), false);
});

/** The web sign-in form, with fetch failing the way `fetch` does when nothing answers. */
function loginPage(fetchFails) {
  const error = { textContent: "" };
  const retry = { hidden: true, onclick: null };
  const fields = { email: { value: "sweep@example.test" }, password: { value: "pw" }, invite: { value: "" } };
  const loginForm = { ...fields, querySelector: (sel) => (sel === ".error" ? error : retry) };
  let calls = 0;
  const context = {
    loginForm,
    URL,
    location: { origin: "https://hub.example.test" },
    fetch: async () => {
      calls++;
      if (fetchFails) throw new TypeError("Failed to fetch");
      return { ok: false, text: async () => JSON.stringify({ error: "wrong email or password" }), statusText: "" };
    },
    save() {},
    connectHub() {},
    document: { body: { dataset: {} } },
    console,
  };
  vm.createContext(context);
  vm.runInContext(`${connectionSrc}\n${fn(accountSrc, "showSignInError")}\n${fn(accountSrc, "loginWeb", true)}\nthis.loginWeb = loginWeb;`, context);
  return { context, error, retry, calls: () => calls };
}

test("web sign-in with the hub down shows the plain sentence and a Retry that tries again", async () => {
  const { context, error, retry, calls } = loginPage(true);
  await context.loginWeb(false);
  assert.equal(error.textContent, "Can't reach the hub at hub.example.test. Check your connection and try again.");
  assert.equal(retry.hidden, false);
  await retry.onclick();
  await flush();
  assert.equal(calls(), 2, "Retry sends the sign-in again");
});

test("web sign-in with a wrong password shows the hub's words and no Retry", async () => {
  const { context, error, retry } = loginPage(false);
  await context.loginWeb(false);
  assert.equal(error.textContent, "wrong email or password");
  assert.equal(retry.hidden, true);
});

test("the reconnect bar and the Retry button are in the page, hidden until their state", () => {
  const html = read("index.html");
  assert.match(html, /<div id="reconnect-bar" role="status">[^<]*<span class="spin"><\/span>Reconnecting… showing the last state<\/div>/);
  assert.match(html, /<button type="button" class="retry" name="retry" hidden>Retry<\/button>/);
  const css = read("gui-sidebar.css");
  assert.match(css, /body\[data-hub="offline"\] #reconnect-bar \{ display: flex; \}/);
});
