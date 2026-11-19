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
