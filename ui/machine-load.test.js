import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./account.js", import.meta.url), "utf8");
const lines = (from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
const fn = (name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in account.js`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};

function page() {
  const context = {
    esc: (text) => String(text).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]),
    LOCAL: "local",
    TAURI: false,
    COMPUTER: "computer",
    account: { machine_id: "m-book" },
    machinesUi: { renaming: null, removing: null },
    machines: new Map(),
    rowUpdate: () => "",
    lastSeen: () => "seen 3 h ago",
  };
  vm.createContext(context);
  vm.runInContext([lines("const gigabytes"), fn("machineStateHtml"), fn("machineEntryHtml")].join("\n"), context);
  return context;
}

const mini = {
  id: "m-mini",
  name: "mac-mini",
  os: "macos",
  online: true,
  last_seen: 1,
  state: {
    load: { cpus: 8, cpu_percent: 92, mem_available_mb: 1434, disk_free_mb: 8806 },
    accounts: [
      { id: "claude", driver_kind: "claude", can_run_child_task: true, limited: false },
      { id: "cursor", driver_kind: "cursor", can_run_child_task: true, limited: true },
      { id: "codex", driver_kind: "codex", can_run_child_task: false, limited: false },
    ],
    sessions: 6,
    workers: 3,
    workers_at_once: 3,
  },
};

test("a machine's row shows its workers, CPU, free memory and disk, and each account", () => {
  const html = page().machineStateHtml(mini);
  const stats = [...html.matchAll(/<span class="stat"( data-level="(\w+)")?>(.*?)<\/span>/g)].map((m) => [m[3].replace(/<\/?b>/g, ""), m[2] ?? null]);
  assert.deepEqual(stats, [
    ["Workers 3 of 3 · full", "warn"],
    ["CPU 92%", "hot"],
    ["Memory 1.4 GB free", "warn"],
    ["Disk 8.6 GB free", "warn"],
  ]);
  const accounts = [...html.matchAll(/<span class="acct" data-state="(\w+)" title="driver: (\w+)">(.*?)<\/span>/g)].map((m) => m.slice(1));
  assert.deepEqual(accounts, [
    ["ready", "claude", "claude"],
    ["limited", "cursor", "cursor · limited"],
    ["off", "codex", "codex · not ready"],
  ]);
});

test("a machine with room shows no warning, and a stat the machine can't read is left out", () => {
  const roomy = { ...mini, state: { ...mini.state, load: { cpu_percent: 12, disk_free_mb: 120 * 1024 }, workers: 1, workers_at_once: 5 } };
  const html = page().machineStateHtml(roomy);
  assert.ok(!html.includes("data-level"), html);
  assert.ok(html.includes("Workers <b>1</b> of 5") && html.includes("Disk <b>120 GB</b> free"), html);
  assert.ok(!html.includes("Memory"), html);
});

test("an offline machine shows no load, and an online one that reports nothing says why", () => {
  const ui = page();
  assert.equal(ui.machineStateHtml({ ...mini, online: false }), "");
  assert.match(ui.machineStateHtml({ ...mini, state: undefined }), /Load and accounts show once this machine runs a pinned that reports them/);
});

test("the machine row carries the state inside its text column", () => {
  const ui = page();
  ui.machines.set("m-mini", { view: mini });
  const row = ui.machineEntryHtml("m-mini", mini);
  assert.match(row, /<div class="who"><b>mac-mini<\/b>.*<div class="machine-load">.*<div class="machine-accounts">.*<\/div><\/div>\s*<button/s);
  assert.ok(!/token|@/.test(row), "the row never shows a credential or an email");
});
