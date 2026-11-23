import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const main = readFileSync(new URL("./main.js", import.meta.url), "utf8");
const settingsSrc = readFileSync(new URL("./settings.js", import.meta.url), "utf8");

// The helper that keeps timers to the visible page, from main.js, so the refetch runs on the real beat.
const helperStart = main.indexOf("const visibleTimers");
const helper = main.slice(helperStart, main.indexOf("const machines = new Map();", helperStart));
// The workflow list and its cards, from TRIGGERS to the picker: the same range workflow-list.test.js renders.
const listSlice = settingsSrc.slice(settingsSrc.indexOf("const ISSUE_STEPS = "), settingsSrc.indexOf("\nfunction triggerChoiceHtml()"));
// The list's redraw, from settings.js.
const redrawSlice = settingsSrc.slice(settingsSrc.indexOf("function redraw() {"), settingsSrc.indexOf("\nfunction edit("));
// The checked line, its refetch and the one registration that drives it, from settings.js.
const checkedSlice = settingsSrc.slice(settingsSrc.indexOf("function checkedText() {"), settingsSrc.indexOf("\nconst trimmed = "));

const minutesAgo = (n) => new Date(Date.now() - n * 60000).toISOString();

const mention = () => ({
  id: "c2",
  name: "Nightly checks",
  enabled: true,
  trigger: { kind: "linear-label", label: "nightly" },
  folder: "/repo",
  machine: "",
  model: "",
  effort: "high",
  permissionMode: "auto",
  worktree: false,
  instructions: "Check the build.",
  fix: "off",
  proof: false,
  columns: { picked: "", coding: "", review: "" },
});

function classes() {
  const names = new Set();
  return {
    toggle: (name, on) => (on ? names.add(name) : names.delete(name)),
    contains: (name) => names.has(name),
  };
}

/**
 * The settings page in a vm: the real visible-timer helper, workflow list, checked line and refetch,
 * with a fake clock and hub. The page element records each write, and a write that carries the checked
 * line updates that line, the way the browser would.
 */
function world({ checked, connectors = [], linear = {}, replies = [], hidden = false, page = "connectors", view = { name: "list" } }) {
  const intervals = new Map();
  const listeners = [];
  const writes = [];
  const calls = [];
  const elements = new Map();
  const line = { textContent: "", classList: classes() };
  elements.set("checked", line);
  const settingsEl = {
    dataset: {},
    set innerHTML(html) {
      writes.push(html);
      const m = /<p class="hint( stale)?" id="checked">([^<]*)<\/p>/.exec(html);
      if (m) {
        line.textContent = m[2];
        line.classList.toggle("stale", !!m[1]);
      }
    },
    get innerHTML() {
      return "";
    },
  };
  elements.set("settings-page", settingsEl);
  const document = {
    hidden,
    body: { dataset: { page: "settings" } },
    activeElement: null,
    addEventListener: (type, fn) => type === "visibilitychange" && listeners.push(fn),
    getElementById: (id) => elements.get(id) ?? null,
  };
  const fresh = () => ({ linear: null, slack: null, connectors: [], checked: { at: minutesAgo(0), error: null } });
  const ctx = {
    console,
    document,
    APP_LOCALE: "en-US",
    // Signed out on this Mac, so a settings call goes to `local_call`; `calls` names the call it carries.
    TAURI: {},
    invoke: (cmd, args) => {
      calls.push(cmd === "local_call" ? args.call.cmd : cmd);
      const reply = replies.length ? replies.shift() : fresh();
      return typeof reply === "function" ? reply() : Promise.resolve(reply);
    },
    agentApproval: () => null,
    hub: null,
    account: null,
    machines: new Map(),
    LOCAL: "local",
    onlineMachines: () => [],
    machineName: () => "?",
    accountName: () => null,
    load: () => null,
    save: () => {},
    setInterval: (fn) => {
      const id = intervals.size + 1;
      intervals.set(id, fn);
      return id;
    },
    clearInterval: (id) => intervals.delete(id),
    setTimeout: (fn) => fn(),
  };
  vm.createContext(ctx);
  vm.runInContext(`${helper}\n${listSlice}\n${redrawSlice}\n${checkedSlice}`, ctx);
  vm.runInContext(
    `settings = ${JSON.stringify({ linear, slack: null, connectors, checked })}; page = ${JSON.stringify(page)}; view = ${JSON.stringify(view)};`,
    ctx,
  );
  return {
    ctx,
    line,
    calls,
    writes,
    elements,
    document,
    // A visibility flip, as the browser fires it.
    flip: (hiddenNow) => {
      document.hidden = hiddenNow;
      listeners.forEach((fn) => fn());
    },
    // One beat of every interval the page still runs.
    tick: () => [...intervals.values()].forEach((fn) => fn()),
    settle: () => new Promise((done) => setImmediate(done)),
  };
}

test("the checked line shows its age when the last check is older than three minutes", () => {
  const at2 = minutesAgo(2);
  const at12 = minutesAgo(12);
  const quiet = world({ checked: { at: at2, error: null } });
  assert.equal(quiet.ctx.checkedText(), `Last checked at ${new Date(at2).toLocaleTimeString("en-US")}. Connectors are checked every minute.`);
  assert.equal(vm.runInContext("checkedStale()", quiet.ctx), false);

  const old = world({ checked: { at: at12, error: null } });
  assert.equal(old.ctx.checkedText(), `Last checked at ${new Date(at12).toLocaleTimeString("en-US")} (12 min ago). Connectors are checked every minute.`);
  assert.equal(vm.runInContext("checkedStale()", old.ctx), true);
});

test("an open settings page refetches get_settings when the window becomes visible", async () => {
  const at12 = minutesAgo(12);
  const fresh = { linear: {}, slack: null, connectors: [], checked: { at: minutesAgo(0), error: null } };
  const w = world({ checked: { at: at12, error: null }, linear: {}, replies: [fresh], hidden: true });
  w.ctx.paintChecked();
  assert.match(w.line.textContent, /\(12 min ago\)/);
  assert.equal(w.line.classList.contains("stale"), true);

  w.flip(false);
  await w.settle();
  assert.deepEqual(w.calls, ["get_settings"], "showing the page again fetches settings once");
  assert.equal(w.line.textContent, `Last checked at ${new Date(fresh.checked.at).toLocaleTimeString("en-US")}. Connectors are checked every minute.`);
  assert.equal(w.line.classList.contains("stale"), false);
});

test("a minute refetch redraws the workflow list with the connectors it just fetched", async () => {
  const renamed = { ...mention(), name: "Nightly build checks" };
  const w = world({
    checked: { at: minutesAgo(0), error: null },
    connectors: [mention()],
    replies: [{ linear: {}, slack: null, connectors: [renamed], checked: { at: minutesAgo(0), error: null } }],
  });
  w.ctx.redraw();
  assert.match(w.writes.at(-1), /Nightly checks/, "the list as drawn when the page opened");

  w.tick();
  await w.settle();
  assert.deepEqual(w.calls, ["get_settings"]);
  assert.match(w.writes.at(-1), /Nightly build checks/, "the list on the page shows the connector the refetch returned");
  assert.doesNotMatch(w.writes.at(-1), /Nightly checks/, "the old name is gone from the list");
});

test("a refetch does not rebuild a form being edited, and keeps the connectors it fetched", async () => {
  const at12 = minutesAgo(12);
  const renamed = { ...mention(), name: "Nightly build checks" };
  const w = world({
    checked: { at: at12, error: null },
    connectors: [mention()],
    replies: [{ linear: {}, slack: null, connectors: [renamed], checked: { at: minutesAgo(0), error: null } }],
    page: "connectors",
    view: { name: "edit" },
  });
  const input = { id: "connector-name", value: "half typed" };
  w.elements.set("connector-name", input);
  w.document.activeElement = input;
  w.ctx.paintChecked();

  w.tick();
  await w.settle();
  assert.deepEqual(w.calls, ["get_settings"]);
  assert.equal(w.document.getElementById("connector-name"), input, "the input is the same element");
  assert.equal(input.value, "half typed", "the unsaved text is kept");
  assert.equal(w.writes.length, 0, "the page is not redrawn while a form is open");
  assert.match(w.line.textContent, /Last checked at/, "only the checked line changes");
  assert.equal(vm.runInContext("settings.connectors[0].name", w.ctx), "Nightly build checks", "the fetched connectors replace the old ones");
});

const SWITCH_ON = '<input type="checkbox" data-toggle="enabled" checked>';
const SWITCH_OFF = '<input type="checkbox" data-toggle="enabled">';

test("a refetch in flight when the hub pushes newer settings does not put the older list back", async () => {
  const older = { linear: {}, slack: null, connectors: [mention()], checked: { at: minutesAgo(0), error: null } };
  const newer = { ...older, connectors: [{ ...mention(), enabled: false }] };
  let answer;
  const w = world({ checked: older.checked, connectors: [mention()], replies: [() => new Promise((done) => (answer = done))] });
  w.ctx.redraw();
  assert.ok(w.writes.at(-1).includes(SWITCH_ON));

  w.tick();
  await w.settle();
  assert.deepEqual(w.calls, ["get_settings"], "the refetch is sent");
  w.ctx.onHubSettings(newer);
  assert.ok(w.writes.at(-1).includes(SWITCH_OFF), "the push draws the switch off");
  answer(older);
  await w.settle();

  assert.equal(vm.runInContext("settings.connectors[0].enabled", w.ctx), false);
  assert.ok(w.writes.at(-1).includes(SWITCH_OFF), "the switch stays off after the older reply arrives");
});

test("a refetch in flight across a save does not put the older list back", async () => {
  const older = { linear: {}, slack: null, connectors: [mention()], checked: { at: minutesAgo(0), error: null } };
  let answer;
  const w = world({
    checked: older.checked,
    connectors: [mention()],
    replies: [() => new Promise((done) => (answer = done)), { ...mention(), enabled: false }],
  });
  w.tick();
  await w.settle();
  await vm.runInContext("connectorCall('save_connector', { connector: { ...settings.connectors[0], enabled: false } }).then((saved) => (settings.connectors[0] = saved))", w.ctx);
  await w.settle();
  assert.deepEqual(w.calls, ["get_settings", "save_connector"]);
  assert.equal(vm.runInContext("settings.connectors[0].enabled", w.ctx), false, "the save's answer is kept");
  answer(older);
  await w.settle();

  assert.equal(vm.runInContext("settings.connectors[0].enabled", w.ctx), false, "the older reply is dropped");
});

test("a refetch with nothing changed while it was in flight still applies", async () => {
  const renamed = { linear: {}, slack: null, connectors: [{ ...mention(), name: "Nightly build checks" }], checked: { at: minutesAgo(0), error: null } };
  let answer;
  const w = world({ checked: renamed.checked, connectors: [mention()], replies: [() => new Promise((done) => (answer = done))] });
  w.tick();
  await w.settle();
  answer(renamed);
  await w.settle();
  assert.equal(vm.runInContext("settings.connectors[0].name", w.ctx), "Nightly build checks");
});
