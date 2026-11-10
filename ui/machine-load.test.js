import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./account.js", import.meta.url), "utf8");
const updateAllSrc = readFileSync(new URL("./update-all.js", import.meta.url), "utf8");
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
    tabs: [],
    phaseOf: (tab) => tab.phase,
    waitingKindOf: (view) => view.waiting ?? null,
    CustomEvent: class {},
    document: { dispatchEvent() {} },
    lastSeen: () => "seen 3 h ago",
  };
  vm.createContext(context);
  vm.runInContext(updateAllSrc, context);
  vm.runInContext([lines("const gigabytes"), fn("machineStateHtml"), fn("machineWork"), fn("updateButtonHtml"), fn("machineEntryHtml")].join("\n"), context);
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

const mile = (patch) => ({ ...mini, id: "m-mile", name: "mile", version: "0.17.23", ...patch });
const button = (html) => html.match(/<button class="(primary|secondary)" data-action="update-machine"( disabled)?(?: title="([^"]*)")?>([^<]*)<\/button>/)?.slice(1);
const standingLine = (html) => html.match(/<span class="hint" data-standing="(\w+)">([^<]*)<\/span>/)?.slice(1);

test("a machine behind the newest version says so and offers Update to that version", () => {
  const ui = page();
  ui.setNewestVersion("0.17.24");
  const html = ui.machineEntryHtml("m-mile", mile({}));
  assert.deepEqual(standingLine(html), ["behind", "v0.17.23 · update to v0.17.24"]);
  assert.deepEqual(button(html), ["primary", undefined, undefined, "Update to v0.17.24"]);
});

test("a machine on the newest version reads up to date and has no Update to press", () => {
  const ui = page();
  ui.setNewestVersion("0.17.24");
  const html = ui.machineEntryHtml("m-mile", mile({ version: "0.17.24" }));
  assert.deepEqual(standingLine(html), ["current", "up to date · v0.17.24"]);
  assert.deepEqual(button(html), ["secondary", " disabled", "Already on v0.17.24", "Up to date"]);
});

test("an offline machine's Update is disabled and says why", () => {
  const ui = page();
  ui.setNewestVersion("0.17.24");
  const html = ui.machineEntryHtml("m-mile", mile({ online: false }));
  assert.deepEqual(standingLine(html), ["offline", "v0.17.23"]);
  assert.deepEqual(button(html), ["secondary", " disabled", "This machine is offline, so it can&#39;t update".replace("&#39;", "'"), "Update"]);
});

test("before the newest version is known no row claims to be up to date", () => {
  const ui = page();
  const html = ui.machineEntryHtml("m-mile", mile({}));
  assert.deepEqual(standingLine(html), ["unknown", "v0.17.23"]);
  assert.deepEqual(button(html), ["secondary", undefined, undefined, "Update"]);
});

test("an update under way disables the button, and a failed one offers Retry with the reason", () => {
  const ui = page();
  ui.setNewestVersion("0.17.24");
  const going = ui.machineEntryHtml("m-mile", mile({ update: { state: "updating", to: "0.17.24" } }));
  assert.deepEqual(standingLine(going), ["updating", "updating to v0.17.24"]);
  assert.deepEqual(button(going), ["secondary", " disabled", undefined, "Updating…"]);
  const failed = ui.machineEntryHtml("m-mile", mile({ update: { state: "failed", to: "0.17.24", reason: "checksum mismatch" } }));
  assert.deepEqual(standingLine(failed), ["failed", "update failed: checksum mismatch"]);
  assert.deepEqual(button(failed), ["secondary", undefined, undefined, "Retry update"]);
});

test("versions compare by number, not as text", () => {
  const ui = page();
  assert.equal(ui.versionOrder("0.17.9", "0.17.10"), -1);
  assert.equal(ui.versionOrder("0.17.24", "v0.17.24"), 0);
  assert.equal(ui.versionOrder("1.0.0", "0.99.99"), 1);
  assert.equal(ui.versionOrder("", "0.1.0"), -1);
});

test("a machine row counts what runs and waits on its threads, and only its own", () => {
  const ui = page();
  const tab = (machine, phase, waiting) => ({ machine, phase, view: { waiting } });
  ui.tabs.push(tab("m-mile", "working", null), tab("m-mile", "delegating", null), tab("m-mile", "asking", "needs_input"), tab("m-mile", "done", null), tab("other", "working", null));
  const html = ui.machineEntryHtml("m-mile", mile({}));
  assert.match(html, /<span class="stat">Threads <b>2<\/b> running · <b>1<\/b> waiting<\/span>/);
  const quiet = ui.machineEntryHtml("m-mile", mile({ online: false }));
  assert.ok(!quiet.includes("Threads"), "an offline machine shows no work");
});

test("machines behind are the online ones on an older version, boxes left out", () => {
  const ui = page();
  ui.machines.set("a", { view: { id: "a", name: "a", online: true, version: "0.17.23" } });
  ui.machines.set("b", { view: { id: "b", name: "b", online: true, version: "0.17.24" } });
  ui.machines.set("c", { view: { id: "c", name: "c", online: false, version: "0.17.1" } });
  ui.machines.set("d", { view: { id: "d", name: "d", online: true, version: "0.17.1", upstash_box: "box" } });
  assert.deepEqual([...ui.machinesBehind("0.17.24").map((v) => v.name)], ["a"]);
  assert.deepEqual([...ui.machinesBehind("").map((v) => v.name)], []);
});
