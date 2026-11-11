import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./account.js", import.meta.url), "utf8");
const fn = (name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in account.js`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const machineActions = src.indexOf("Object.assign(actions, {", src.indexOf("function machineBackgroundHtml("));
const actionsSrc = src.slice(machineActions, src.indexOf("\n});\n", machineActions) + 4);

/** Settings › Machines and the General page, with the hub and this app's machine faked. */
function page({ tauri = false, account = { machine_id: "m-book" }, fail = null } = {}) {
  const calls = [];
  const drawn = [];
  const context = {
    esc: (text) => String(text).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]),
    LOCAL: "local",
    TAURI: tauri ? {} : null,
    COMPUTER: "Mac",
    account,
    page: "machines",
    actions: {},
    machinesUi: { renaming: null, removing: null, background: null },
    machines: new Map(),
    updateStanding: () => ({ standing: "current", text: "", button: { label: "Update", disabled: false } }),
    updateButtonHtml: () => "",
    machineStateHtml: () => "",
    machineWork: () => ({ running: 0, waiting: 0 }),
    lastSeen: () => "seen 3 h ago",
    rowMachine: (el) => el.machine,
    callOn: async (machine, call) => {
      calls.push([machine, call]);
      if (fail) throw new Error(fail);
      return call.on;
    },
    showMachinesPage: () => drawn.push("machines"),
    showGeneral: () => drawn.push("general"),
  };
  vm.createContext(context);
  vm.runInContext([fn("machineEntryHtml"), fn("machineBackgroundHtml"), fn("setMachineBackground"), actionsSrc].join("\n"), context);
  return { ui: context, calls, drawn };
}

const studio = { id: "m-studio", name: "studio", os: "macos", online: true, last_seen: 1, background: false };
const node = { id: "m-vps", name: "vps", os: "linux", online: true, last_seen: 1 };
const switchOf = (html) => html.match(/<label class="switch" data-action="machine-background" data-on="(?:true|false)"><input type="checkbox" aria-label="Run in the background"( checked)?( disabled)?><span><\/span><\/label>/);

test("a machine running the app gets a Run in the background switch in its row, set as it reported", () => {
  const { ui } = page();
  const off = switchOf(ui.machineEntryHtml("m-studio", studio));
  assert.deepEqual([off?.[1], off?.[2]], [undefined, undefined], "shown, off and usable");
  const on = switchOf(ui.machineEntryHtml("m-studio", { ...studio, background: true }));
  assert.deepEqual([on?.[1], on?.[2]], [" checked", undefined]);
  assert.match(ui.machineEntryHtml("m-studio", studio), /<div class="who"><b>studio<\/b>.*<div class="side-setting" data-setting="background" data-machine="m-studio">.*<\/div><\/div>/s);
});

test("a headless node reports no such setting and its row has no switch", () => {
  const { ui } = page();
  const row = ui.machineEntryHtml("m-vps", node);
  assert.match(row, /<b>vps<\/b>/);
  assert.equal(switchOf(row), null);
  assert.ok(!row.includes("Run in the background"));
});

test("signed out, this app's switch is off and disabled, and says why", () => {
  const { ui } = page({ tauri: true, account: null });
  const html = ui.machineBackgroundHtml("local", { background: true });
  const box = switchOf(html);
  assert.deepEqual([box?.[1], box?.[2]], [undefined, " disabled"]);
  assert.match(html, /<p class="meta" data-why>Sign in under Account first\. With no window, the web app is how you reach this Mac\.<\/p>/);
  const signedIn = page({ tauri: true }).ui.machineBackgroundHtml("local", { background: false });
  assert.deepEqual([switchOf(signedIn)?.[2], /data-why/.test(signedIn)], [undefined, false], "signed in, it is usable");
});

test("switching it off from the web asks that machine and redraws the row from what it answered", async () => {
  const { ui, calls, drawn } = page();
  const view = { ...studio, background: true };
  ui.machines.set("m-studio", { view });
  await ui.actions["machine-background"]({ machine: "m-studio", dataset: { on: "true" }, querySelector: () => ({ checked: false, disabled: false }) });
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [["m-studio", { cmd: "set_background", on: false }]]);
  assert.equal(view.background, false);
  assert.deepEqual(drawn, ["machines"]);
});

test("a refused switch keeps the old value and shows the machine's reason under it", async () => {
  const { ui } = page({ fail: "this machine runs pinned without the app, so it has no window" });
  const view = { ...studio };
  ui.machines.set("m-studio", { view });
  await ui.actions["machine-background"]({ machine: "m-studio", dataset: { on: "false" }, querySelector: () => ({ checked: true, disabled: false }) });
  assert.equal(view.background, false);
  assert.match(ui.machineEntryHtml("m-studio", view), /<p class="error">Error: this machine runs pinned without the app, so it has no window<\/p>/);
});

test("a disabled switch sends nothing, and Close window turns the setting back on", async () => {
  const { ui, calls } = page({ tauri: true });
  ui.page = "general";
  await ui.actions["machine-background"]({ machine: "local", dataset: { on: "false" }, querySelector: () => ({ checked: false, disabled: true }) });
  assert.equal(calls.length, 0);
  const html = ui.machineBackgroundHtml("local", { background: true });
  assert.match(html, /<button class="secondary" data-action="machine-background-close">Close window<\/button>/);
  await ui.actions["machine-background-close"]({ machine: "local" });
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [["local", { cmd: "set_background", on: true }]]);
  assert.ok(!ui.machineBackgroundHtml("local", { background: false }).includes("Close window"), "no Close window while it is off");
});
