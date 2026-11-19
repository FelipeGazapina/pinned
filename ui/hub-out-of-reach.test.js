import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const updateAllSrc = read("update-all.js");
const modelSrc = read("redesign-model.js");
const accountSrc = read("account.js");
const settingsSrc = read("settings.js");

const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const here = (value) => JSON.parse(JSON.stringify(value));

/** The desktop app (or the web page with `TAURI: null`) signed in as machine m-local, with this Mac's
 * own entry and no view of it from the hub yet. */
function signedIn(extra = {}) {
  const page = { innerHTML: "", querySelector: () => null };
  const context = {
    TAURI: {},
    LOCAL: "local",
    COMPUTER: "Mac",
    account: { machine_id: "m-local", email: "owner@example.test" },
    machines: new Map([["local", { view: { id: "local", name: "This Mac", os: "macos", online: true, last_seen: 0 } }]]),
    viewListeners: [],
    document: { body: { dataset: { version: "v0.18.7" } }, dispatchEvent() {} },
    CustomEvent: class {},
    settingsPage: page,
    machinesUi: { renaming: null, removing: null, error: "", pair: null },
    machineWork: () => ({ running: 0, waiting: 0 }),
    machineStateHtml: () => "",
    machineBackgroundHtml: () => "",
    lastSeen: () => "seen 2 h ago",
    clockTime: () => "",
    ...extra,
  };
  vm.createContext(context);
  vm.runInContext(
    [updateAllSrc, fn(modelSrc, "machineCounts"), line(settingsSrc, "const esc = "), fn(accountSrc, "machineEntryHtml"), fn(accountSrc, "updateButtonHtml"), fn(accountSrc, "showMachinesPage"),
      "Object.assign(globalThis, { accountMachines, machineCounts, showMachinesPage });"].join("\n"),
    context,
  );
  return { context, page };
}

test("in the app this Mac is an online machine of the account before the hub sends its view", () => {
  const { context } = signedIn();
  assert.deepEqual(here(context.accountMachines()), [{ id: "m-local", name: "This Mac", os: "macos", last_seen: 0, online: true, version: "0.18.7" }]);
  assert.deepEqual(here(context.machineCounts()), { machinesOnline: 1, machinesTotal: 1 });
});

test("the footer counts this Mac online in the app even when the hub last saw every machine go", () => {
  const { context } = signedIn();
  context.machines.get("local").hubView = { id: "m-local", name: "owner-mbp", os: "macos", online: false, version: "0.18.7" };
  context.machines.set("m-mini", { view: { id: "m-mini", name: "mac-mini", online: false } });
  assert.deepEqual(here(context.machineCounts()), { machinesOnline: 1, machinesTotal: 2 });

  const web = signedIn({ TAURI: null, account: { machine_id: null, email: "owner@example.test" }, machines: new Map([["m-mini", { view: { id: "m-mini", name: "mac-mini", online: false } }]]) }).context;
  assert.deepEqual(here(web.machineCounts()), { machinesOnline: 0, machinesTotal: 1 }, "the web page has no machine of its own to count");
});

test("Settings, Machines shows this Mac's row from what the app knows while the hub never sent its view", () => {
  const { context, page } = signedIn();
  context.showMachinesPage();
  const row = /<div class="machine-row" data-machine="local">[\s\S]*?<\/div>\s*<\/div>/.exec(page.innerHTML)?.[0] ?? "";
  assert.match(row, /<span class="dot" data-online="true"><\/span>/);
  assert.match(row, /<b>This Mac<\/b><span class="hint">macos · this Mac<\/span><span class="hint" data-standing="unknown">v0\.18\.7<\/span>/);
  assert.match(page.innerHTML, /<button class="secondary" data-action="update-machine">Update<\/button>/);
});

/** This computer's `update-every-machine` with the hub answering or not: what each machine was asked
 * through the hub and what this computer ran itself. */
function updating(up) {
  const viaHub = [];
  const ran = [];
  const { context } = signedIn({
    hubUp: () => up,
    hub: { call: (id, call) => (viaHub.push([id, call.cmd]), up ? Promise.resolve({ state: "up_to_date", version: "0.18.7" }) : Promise.reject("not connected to the hub")) },
    invoke: (cmd) => (ran.push(cmd), Promise.resolve({ state: "up_to_date", version: "0.18.7" })),
    setTimeout: () => ({}),
  });
  context.machines.get("local").hubView = { id: "m-local", name: "owner-mbp", online: true, version: "0.18.6" };
  context.machines.set("m-mini", { view: { id: "m-mini", name: "mac-mini", online: true, version: "0.18.6" } });
  return { context, viaHub, ran };
}

test("Update every machine updates this Mac through its own installer while the hub doesn't answer", async () => {
  const down = updating(false);
  await vm.runInContext("updateEveryMachine()", down.context);
  assert.deepEqual(down.viaHub, [["m-mini", "update_self"]]);
  assert.deepEqual(down.ran, ["install_update"]);
  assert.deepEqual(here(down.context.accountMachines().map((view) => [view.id, view.update ?? null])), [["m-local", null], ["m-mini", { state: "failed", to: "", reason: "not connected to the hub" }]], "the other machine says why it didn't update");

  const up = updating(true);
  await vm.runInContext("updateEveryMachine()", up.context);
  assert.deepEqual(up.viaHub, [["m-mini", "update_self"], ["m-local", "update_self"]], "with the hub up, every machine goes through it as before");
  assert.deepEqual(up.ran, []);
});

/** main.js's clone routing, signed in (`signed`) with the hub answering or not (`up`). */
function cloning({ signed = true, up }) {
  const asked = [];
  const context = {
    LOCAL: "local",
    hubIdOf: (machine) => (machine === "local" ? "m-local" : machine),
    invoke: (cmd, args) => (asked.push(["here", cmd, here(args)]), Promise.resolve("/Users/owner/code/site")),
    hub: signed ? { online: up, hub: (call) => (asked.push(["hub", here(call)]), up ? Promise.resolve("/Users/owner/code/site") : Promise.reject("not connected to the hub")) } : null,
  };
  vm.createContext(context);
  vm.runInContext(`${line(read("main.js"), "const hubUp = ")}\n${fn(read("main.js"), "cloneOn")}\nglobalThis.cloneOn = cloneOn;`, context);
  return { clone: context.cloneOn, asked };
}

test("a clone on this Mac runs here while the hub doesn't answer, and through the hub while it does", async () => {
  const down = cloning({ up: false });
  assert.equal(await down.clone("local", "acme/site", "/Users/owner/code"), "/Users/owner/code/site");
  await assert.rejects(down.clone("m-mini", "acme/site"), /not connected to the hub/, "another machine is only reached through the hub");
  assert.deepEqual(down.asked, [["here", "clone_repo", { repo: "acme/site", dest: "/Users/owner/code" }], ["hub", { cmd: "clone", machine: "m-mini", repo: "acme/site" }]]);

  const up = cloning({ up: true });
  await up.clone("local", "acme/site");
  assert.deepEqual(up.asked, [["hub", { cmd: "clone", machine: "m-local", repo: "acme/site" }]]);

  const out = cloning({ signed: false });
  await out.clone("local", "acme/site");
  assert.deepEqual(out.asked, [["here", "clone_repo", { repo: "acme/site" }]]);
});

/** The project picker's Clone from GitHub, whose repository list fails with `error`. */
async function githubPicker(error) {
  const el = () => ({ innerHTML: "", textContent: "", focus() {} });
  const parts = { ".choices": el(), ".browse input": el(), ".folders": el(), ".browse": el() };
  const shown = el();
  const context = {
    LOCAL: "local",
    picker: { querySelector: (selector) => parts[selector] },
    picking: { machine: "local" },
    connectorCall: () => Promise.reject(error),
    document: { getElementById: (id) => (id === "picker-error" ? shown : null) },
  };
  vm.createContext(context);
  vm.runInContext(`${fn(read("main.js"), "cloneFromGithub")}\nglobalThis.done = cloneFromGithub();`, context);
  await context.done;
  return shown.textContent;
}

test("the GitHub picker asks to connect GitHub only when GitHub is the one not connected", async () => {
  assert.equal(await githubPicker("GitHub is not connected"), "Connect GitHub under Settings → Connectors first.");
  assert.equal(await githubPicker("not connected to the hub"), "not connected to the hub");
});
