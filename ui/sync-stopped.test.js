import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const mainSrc = read("main.js");
const accountSrc = read("account.js");
const sidebarSrc = read("gui-sidebar.js");
const settingsSrc = read("settings.js");

const line = (src, start) => {
  const at = src.indexOf(start);
  return at < 0 ? "" : src.slice(at, src.indexOf("\n", at));
};
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  return at < 0 ? "" : src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const run = (code, context) => {
  vm.createContext(context);
  vm.runInContext(code, context);
  return context;
};
const escFn = line(settingsSrc, "const esc = ");
const why = "the sync state /home/me/.config/pinned/sync.json is corrupt (EOF while parsing a value at line 1 column 0)";

function sidebar(machines, account = { machine_id: "this-mac" }) {
  const syncAlert = { innerHTML: "", hidden: true };
  run(`${escFn}\n${line(mainSrc, "const machineName = ")}\n${fn(sidebarSrc, "renderSyncAlert")}`, {
    LOCAL: "local",
    COMPUTER: "Mac",
    account,
    machines: new Map(Object.entries(machines)),
    syncAlert,
    icon: (name) => `<i data-icon="${name}"></i>`,
  }).renderSyncAlert();
  return syncAlert;
}

test("the sidebar names every machine whose sync stopped", () => {
  const alert = sidebar({
    local: { syncStopped: why },
    box: { view: { name: `<b>"box"</b>` }, syncStopped: why },
    studio: { view: { name: "studio" }, syncStopped: null },
  });
  assert.equal(
    alert.innerHTML,
    `<i data-icon="circle-alert"></i><span>Sync stopped on This Mac, &#60;b&#62;&#34;box&#34;&#60;/b&#62;</span><button type="button">Review</button>`,
  );
  assert.equal(alert.hidden, false);
});

test("the sidebar line hides once every machine syncs again", () => {
  const alert = sidebar({ local: { syncStopped: null }, box: { view: { name: "box" } } });
  assert.equal(alert.innerHTML, "");
  assert.equal(alert.hidden, true);
});

test("the sidebar line hides after sign out", () => {
  const alert = sidebar({ local: { syncStopped: why } }, null);
  assert.equal(alert.innerHTML, "");
  assert.equal(alert.hidden, true);
});

function refresher(answer, { machines = { box: { view: { name: "box" } } }, page = "machines" } = {}) {
  const drawn = [];
  const context = run(fn(mainSrc, "refreshSyncStopped"), {
    machines: new Map(Object.entries(machines)),
    callOn: (machine, call) => (call.cmd === "sync_stopped" ? answer(context, machine) : Promise.reject(new Error(call.cmd))),
    render: () => drawn.push("render"),
    settingsOpen: () => true,
    page,
    machinesUi: { renaming: false },
    showMachinesPage: () => drawn.push("machines page"),
  });
  return { context, drawn };
}

test("a machine's answer becomes its notice and redraws the open Machines page", async () => {
  const { context, drawn } = refresher(async () => why);
  await context.refreshSyncStopped("box");
  assert.equal(context.machines.get("box").syncStopped, why);
  assert.deepEqual(drawn, ["render", "machines page"]);
});

test("a null answer after a reset clears the notice", async () => {
  const { context } = refresher(async () => null, { machines: { box: { view: { name: "box" }, syncStopped: why } } });
  await context.refreshSyncStopped("box");
  assert.equal(context.machines.get("box").syncStopped, null);
});

test("a node too old to answer keeps the last notice, or none", async () => {
  const old = async () => {
    throw new Error("this machine runs pinned 0.16.0, which can't answer that: unknown variant `sync_stopped`");
  };
  const stopped = refresher(old, { machines: { box: { view: { name: "box" }, syncStopped: why } } });
  await stopped.context.refreshSyncStopped("box");
  assert.equal(stopped.context.machines.get("box").syncStopped, why);
  const fresh = refresher(old);
  await fresh.context.refreshSyncStopped("box");
  assert.equal(fresh.context.machines.get("box").syncStopped, null);
});

test("an answer for a machine replaced while asking is dropped", async () => {
  const { context, drawn } = refresher(async (ctx, machine) => {
    ctx.machines.set(machine, { view: { name: "box again" } });
    return why;
  });
  await context.refreshSyncStopped("box");
  assert.equal(context.machines.get("box").syncStopped, undefined);
  assert.deepEqual(drawn, []);
});

test("opening the Machines page asks every online machine again", () => {
  const asked = [];
  const drawn = [];
  const sections = [];
  run(`${fn(accountSrc, "openMachinesPage")}\n${line(settingsSrc, 'settingsSection({ page: "machines"')}`, {
    LOCAL: "local",
    machines: new Map([
      ["local", { hubView: { id: "this-mac", online: true } }],
      ["box", { view: { id: "box", online: true } }],
      ["old", { view: { id: "old", online: false } }],
    ]),
    settingsSection: (section) => sections.push(section),
    showMachinesPage: () => drawn.push("machines page"),
    refreshSyncStopped: (machine) => asked.push(machine),
  });
  assert.equal(sections.length, 1);
  sections[0].show();
  assert.deepEqual(drawn, ["machines page"]);
  assert.deepEqual(asked, ["local", "box"]);
});

function entry(syncStopped) {
  return run(`${escFn}\n${fn(accountSrc, "machineEntryHtml")}`, {
    LOCAL: "local",
    TAURI: false,
    COMPUTER: "Mac",
    account: { machine_id: "this-mac" },
    machinesUi: { renaming: null, removing: null },
    machines: new Map([["box", { view: { id: "box", name: "box" }, syncStopped }]]),
    rowUpdate: () => "",
    lastSeen: () => "",
  }).machineEntryHtml("box", { id: "box", name: "box", os: "linux", online: true });
}

test("a stopped machine's row carries the reason and a Start sync over button", () => {
  const html = entry(`${why} <script>`);
  assert.match(html, /<div class="sync-stopped" data-machine="box"><b>Sync stopped\.<\/b> box can't read its record of the last sync/);
  assert.match(html, /<span class="hint">the sync state \/home\/me\/\.config\/pinned\/sync\.json is corrupt \(EOF while parsing a value at line 1 column 0\) &#60;script&#62;<\/span>/);
  assert.match(html, /<button class="secondary" data-action="reset-sync">Start sync over<\/button>/);
});

test("a syncing machine's row has no notice", () => {
  assert.doesNotMatch(entry(null), /sync-stopped|reset-sync/);
});
