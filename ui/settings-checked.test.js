import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const main = readFileSync(new URL("./main.js", import.meta.url), "utf8");
const settingsSrc = readFileSync(new URL("./settings.js", import.meta.url), "utf8");

// The helper that keeps timers to the visible page, from main.js, so the refetch runs on the real beat.
const helperStart = main.indexOf("const visibleTimers");
const helper = main.slice(helperStart, main.indexOf("const machines = new Map();", helperStart));
// The checked line, its refetch and the one registration that drives it, from settings.js.
const checkedSlice = settingsSrc.slice(settingsSrc.indexOf("function checkedText() {"), settingsSrc.indexOf("\nconst trimmed = "));
const settingsOpen = settingsSrc.split("\n").find((line) => line.startsWith("const settingsOpen = "));

const minutesAgo = (n) => new Date(Date.now() - n * 60000).toISOString();

function classes() {
  const names = new Set();
  return {
    toggle: (name, on) => (on ? names.add(name) : names.delete(name)),
    contains: (name) => names.has(name),
  };
}

/** The settings page in a vm: the real visible-timer helper, checked line and refetch, with a fake clock and hub. */
function world({ checked, replies = [], hidden = false, page = "connectors", view = { name: "list" } }) {
  const intervals = new Map();
  const listeners = [];
  const writes = [];
  const calls = [];
  const elements = new Map();
  const line = { textContent: "", classList: classes() };
  elements.set("checked", line);
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
    page,
    view,
    settingsPage: { dataset: {}, set innerHTML(html) { writes.push(html); }, get innerHTML() { return ""; } },
    settings: { linear: null, slack: null, connectors: [], checked },
    connectorCall: (cmd) => {
      calls.push(cmd);
      return Promise.resolve(replies.length ? replies.shift() : fresh());
    },
    setInterval: (fn) => {
      const id = intervals.size + 1;
      intervals.set(id, fn);
      return id;
    },
    clearInterval: (id) => intervals.delete(id),
    setTimeout: (fn) => fn(),
  };
  vm.createContext(ctx);
  vm.runInContext(`${settingsOpen}\n${helper}\n${checkedSlice}`, ctx);
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
  const fresh = { linear: null, slack: null, connectors: [], checked: { at: minutesAgo(0), error: null } };
  const w = world({ checked: { at: at12, error: null }, replies: [fresh], hidden: true });
  w.ctx.paintChecked();
  assert.match(w.line.textContent, /\(12 min ago\)/);
  assert.equal(w.line.classList.contains("stale"), true);

  w.flip(false);
  await w.settle();
  assert.deepEqual(w.calls, ["get_settings"], "showing the page again fetches settings once");
  assert.equal(w.line.textContent, `Last checked at ${new Date(fresh.checked.at).toLocaleTimeString("en-US")}. Connectors are checked every minute.`);
  assert.equal(w.line.classList.contains("stale"), false);
});

test("a refetch does not rebuild a form being edited", async () => {
  const at12 = minutesAgo(12);
  const w = world({ checked: { at: at12, error: null }, page: "connectors", view: { name: "edit" } });
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
});
