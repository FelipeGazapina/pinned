import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const memorySrc = readFileSync(new URL("./gui-memory.js", import.meta.url), "utf8");

const MB = 1024 * 1024;
const REPORT = {
  version: "0.18.9",
  os: "macos",
  measure: "physical footprint",
  uptime_secs: 3 * 3600 + 25 * 60,
  bytes: 2600 * MB,
  processes: 14,
  unreadable: 2,
  groups: [
    { name: "App", bytes: 310 * MB, processes: 1, unreadable: 0, rows: [] },
    { name: "Webview helpers", bytes: 1200 * MB, processes: 3, unreadable: 0, rows: [] },
    {
      name: "Threads and terminals",
      bytes: 1090 * MB,
      processes: 8,
      unreadable: 2,
      rows: [
        { label: "Terminal 1", kind: "terminal", agent: null, state: "idle", bytes: 5 * MB, processes: 1, unreadable: 0 },
        { label: "Fix the build", kind: "thread", agent: "claude", state: "running", bytes: 1085 * MB, processes: 5, unreadable: 1 },
        { label: "Delegated task", kind: "delegated", agent: null, state: "running", bytes: 0, processes: 1, unreadable: 1 },
      ],
    },
    { name: "Other", bytes: 0, processes: 2, unreadable: 0, rows: [] },
  ],
};

function load({ tauri = {}, invoke = async () => REPORT } = {}) {
  const sections = [];
  const memory = { onclick: null };
  const context = {
    TAURI: tauri,
    invoke,
    page: "memory",
    settingsOpen: () => true,
    settingsSection: (section) => sections.push(section),
    settingsPage: { innerHTML: "" },
    document: { getElementById: (id) => (id === "memory" ? memory : null) },
    icon: (name) => `<svg data-icon="${name}"></svg>`,
    esc: (text) => String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;"),
    navigator: {},
    console,
    Date,
  };
  vm.createContext(context);
  vm.runInContext(memorySrc, context);
  return { context, sections };
}

test("the copied report lists the groups, each thread and terminal largest first, and no path", () => {
  const { context } = load();
  const text = context.memoryReportText(REPORT);
  assert.equal(
    text,
    [
      "pinned 0.18.9 on macos, physical footprint",
      "Uptime: 3 h 25 min",
      "Total: 2.5 GB, 14 processes, 2 could not be read",
      "App: 310 MB, 1 process",
      "Webview helpers: 1.2 GB, 3 processes",
      "Threads and terminals: 1.1 GB, 8 processes, 2 could not be read",
      "  Fix the build (claude, running): 1.1 GB, 5 processes, 1 could not be read",
      "  Terminal 1 (idle): 5 MB, 1 process",
      "  Delegated task (running): could not read, 1 process, 1 could not be read",
      "Other: 0 MB, 2 processes",
      "",
    ].join("\n"),
  );
  assert.ok(!text.includes("/"), "no path");
  assert.ok(!/[<>]/.test(text), "no HTML");
});

test("the Memory section is in the desktop app's settings only", () => {
  const app = load();
  assert.deepEqual(app.sections.map((s) => [s.page, s.label, s.icon]), [["memory", "Memory", "chart-column"]]);
  assert.equal(app.sections[0].search[0].title, "Memory");
  assert.deepEqual(load({ tauri: null }).sections, []);
});

test("the page reads the report when it opens, and a failure shows its text and no Copy button", async () => {
  let calls = 0;
  const shown = load({ invoke: async (cmd) => (calls++, assert.equal(cmd, "memory_report"), REPORT) });
  await shown.sections[0].show();
  assert.equal(calls, 1);
  const html = shown.context.settingsPage.innerHTML;
  for (const text of ["Up 3 h 25 min", "Total", "2.5 GB", "App", "Webview helpers", "Threads and terminals", "Other", "Fix the build", "claude, running", "could not read", "data-memory=\"copy\""]) {
    assert.ok(html.includes(text), `the page shows ${text}`);
  }
  assert.ok(html.indexOf("Fix the build") < html.indexOf("Terminal 1"), "the largest row comes first");

  const error = "The memory report is not available on Windows yet.";
  const failed = load({ invoke: () => Promise.reject(error) });
  await failed.sections[0].show();
  const page = failed.context.settingsPage.innerHTML;
  assert.ok(page.includes(error));
  assert.ok(!page.includes("data-memory=\"copy\""), "no Copy button");
  assert.ok(!page.includes("Copy"));
});
