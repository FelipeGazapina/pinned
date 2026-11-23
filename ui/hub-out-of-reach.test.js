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
  const offline = [
    { id: "m-local", name: "owner-mbp", os: "macos", online: false, version: "0.18.7" },
    { id: "m-mini", name: "mac-mini", os: "macos", online: false, version: "0.18.7" },
  ];
  const hubSays = (context) => {
    vm.runInContext(fn(read("main.js"), "showMachines"), context);
    vm.runInContext("showMachines(machinesFromHub, {})", Object.assign(context, { machinesFromHub: offline }));
    return here(context.machineCounts());
  };
  const stubs = { hub: { transport: () => ({}) }, setTabs() {}, refreshTabs: async () => {}, retryUploadImages() {}, refreshSyncStopped() {}, render() {}, settingsOpen: () => false };
  assert.deepEqual(hubSays(signedIn(stubs).context), { machinesOnline: 1, machinesTotal: 2 });
  const web = signedIn({ ...stubs, TAURI: null, account: { machine_id: null, email: "owner@example.test" }, machines: new Map() }).context;
  assert.deepEqual(hubSays(web), { machinesOnline: 0, machinesTotal: 2 }, "the web page has no machine of its own to count");
});

test("Settings, Machines shows this Mac's row from what the app knows while the hub never sent its view", () => {
  const { context, page } = signedIn();
  context.showMachinesPage();
  const row = /<div class="machine-row" data-machine="local">[\s\S]*?<\/div>\s*<\/div>/.exec(page.innerHTML)?.[0] ?? "";
  assert.match(row, /<span class="dot" data-online="true"><\/span>/);
  assert.match(row, /<b>This Mac<\/b><span class="hint">macos · this Mac<\/span><span class="hint" data-standing="unknown">v0\.18\.7<\/span>/);
  assert.match(page.innerHTML, /<button class="secondary" data-action="update-machine">Update<\/button>/);
});

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

function cloning({ signed = true, up, fail = null }) {
  const asked = [];
  const context = {
    LOCAL: "local",
    hubIdOf: (machine) => (machine === "local" ? "m-local" : machine),
    invoke: (cmd, args) => (asked.push(["here", cmd, here(args)]), fail ? Promise.reject(fail) : Promise.resolve("/Users/owner/code/site")),
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
  assert.deepEqual(down.asked, [["here", "local_call", { call: { cmd: "clone", machine: "local", repo: "acme/site", dest: "/Users/owner/code" } }], ["hub", { cmd: "clone", machine: "m-mini", repo: "acme/site" }]]);

  const up = cloning({ up: true });
  await up.clone("local", "acme/site");
  assert.deepEqual(up.asked, [["hub", { cmd: "clone", machine: "m-local", repo: "acme/site" }]]);

  const out = cloning({ signed: false });
  await out.clone("local", "acme/site");
  assert.deepEqual(out.asked, [["here", "local_call", { call: { cmd: "clone", machine: "local", repo: "acme/site" } }]]);
});

test("a signed-in clone on this Mac names the hub's GitHub token when git asks for one, and keeps any other failure", async () => {
  const auth = cloning({ up: false, fail: "could not read Username for 'https://github.com': terminal prompts disabled" });
  await assert.rejects(auth.clone("local", "acme/private", "/Users/owner/code"), /A private repository needs the account's GitHub token on the hub\./);
  const denied = cloning({ up: false, fail: "Authentication failed for 'https://github.com/acme/private/'" });
  await assert.rejects(denied.clone("local", "acme/private"), /GitHub token on the hub/);
  const missing = cloning({ up: false, fail: "The requested URL returned error: 404" });
  await assert.rejects(missing.clone("local", "acme/missing"), /404/);
  const signedOut = cloning({ signed: false, fail: "Authentication failed" });
  await assert.rejects(signedOut.clone("local", "acme/private"), /^Authentication failed$/);
});

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

test("the external agents card says the hub can't be reached, and does not ask it", async () => {
  const asked = [];
  const card = { outerHTML: "" };
  const context = {
    account: { email: "owner@example.test" },
    hub: { online: false, hub: (call) => (asked.push(call.cmd), Promise.reject("not connected to the hub")) },
    actions: {},
    settingsGen: 0,
    agentApproval: () => null,
    invoke: () => Promise.reject("invoke should not run"),
    settingsOpen: () => true,
    page: "integrations",
    document: { body: { dataset: { page: "settings" } }, getElementById: (id) => (id === "external-agents" ? card : null) },
    esc: (text) => String(text ?? ""),
  };
  vm.createContext(context);
  vm.runInContext(`${read("connection.js")}\n${line(read("main.js"), "const hubUp = ")}\n${fn(settingsSrc, "plainHubNeed")}\n${fn(settingsSrc, "connectorCall")}\n${read("agents-mcp.js")}\nglobalThis.__load = loadExternalAgents; globalThis.__ui = agentsUi;`, context);
  await context.__load();
  assert.deepEqual(asked, []);
  assert.equal(context.__ui.error, "Can't reach the hub, and this needs it.");
  assert.match(card.outerHTML, /Can't reach the hub, and this needs it\./);
  assert.doesNotMatch(card.outerHTML, /not connected to the hub/);
});

test("the GitHub picker asks to connect GitHub only when GitHub is the one not connected", async () => {
  assert.equal(await githubPicker("GitHub is not connected"), "Connect GitHub under Settings → Connectors first.");
  assert.equal(await githubPicker("not connected to the hub"), "not connected to the hub");
});

const LAST_SENT = { linear: { name: "Owner", email: "owner@example.test", organization: "Acme" }, slack: null, slack_bots: [], connectors: [], checked: null };

const HUB_ANSWERS = { get_settings: LAST_SENT, linear_suggestions: { labels: [], columns: [], teams: [] }, vercel_models: [] };

function settingsApp({ up, sent, copy = null, tauri = {} }) {
  const elements = new Map();
  const byId = (id) => {
    if (!elements.has(id)) elements.set(id, { id, innerHTML: "", textContent: "", value: "", dataset: {}, listeners: {}, addEventListener(type, f) { (this.listeners[type] ??= []).push(f); }, querySelector: () => null, querySelectorAll: () => [], focus() {} });
    return elements.get(id);
  };
  const asked = [];
  const context = {
    URL,
    console: { error() {}, warn() {}, log() {} },
    addEventListener() {},
    setTimeout() {},
    TAURI: tauri,
    LOCAL: "local",
    account: { email: "owner@example.test", machine_id: "m-local" },
    hub: { online: up, hub: (call) => (asked.push(["hub", call.cmd, here(call)]), up ? Promise.resolve(HUB_ANSWERS[call.cmd] ?? (call.cmd === "release_workflows" ? {} : null)) : Promise.reject("not connected to the hub")) },
    invoke: (cmd, args) => {
      asked.push(["local", cmd, args && here(args)]);
      if (cmd === "account_copy") return Promise.resolve(copy);
      if (cmd === "local_call") {
        const call = args.call;
        if (call.cmd === "get_settings") return Promise.resolve(sent ?? { linear: null, slack: null, connectors: [], checked: null });
        if (call.cmd === "connect_linear") return Promise.resolve({ name: "Owner", email: "owner@example.test", organization: "Acme" });
        if (call.cmd === "linear_suggestions") return Promise.resolve({ labels: [], columns: [], teams: [] });
        if (call.cmd === "github_repos" || call.cmd === "vercel_models" || call.cmd === "slack_suggestions") return Promise.resolve([]);
        return Promise.reject("needs the hub, which is off");
      }
      return Promise.reject(`${cmd} should not run here while signed in`);
    },
    machines: new Map([["local", { view: { online: true }, projects: [] }]]),
    load: () => null,
    save() {},
    listen() {},
    tilde: (path) => path,
    agentApproval: () => null,
    agentAsking: () => null,
    agentAskingHtml: () => "",
    agentConsentHtml: () => "",
    externalAgentsCardHtml: () => "",
    convexCardHtml: () => "",
    loadConvexProjects() {},
    loadExternalAgents() {},
    icon: (name) => `<svg data-icon="${name}"></svg>`,
    visibleInterval: () => {},
    document: { body: { dataset: {} }, getElementById: byId, querySelector: () => byId("nav"), querySelectorAll: () => [] },
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(`${read("connection.js")}\n${line(read("main.js"), "const hubUp = ")}\n${settingsSrc}\nglobalThis.__actions = actions; globalThis.__onHubSettings = onHubSettings; globalThis.__showPage = showPage; globalThis.__pullSettings = pullSettings;`, context);
  if (sent) {
    context.hub.online = true;
    context.__onHubSettings(sent);
    context.hub.online = up;
  }
  const show = async (page) => {
    context.__showPage(page);
    await new Promise((resolve) => setImmediate(resolve));
    return byId("settings-page").innerHTML;
  };
  return { context, byId, show, asked };
}

test("on the web, Connectors and Workflows stay read-only while the hub has not answered", async () => {
  const before = settingsApp({ up: false, sent: null, tauri: null });
  assert.equal(await before.show("integrations"), `<h2>Connectors</h2><div class="card"><p>Can't reach the hub, and this needs it.</p></div>`);
  assert.equal(await before.show("connectors"), `<h2>Workflows</h2><div class="card"><p>Can't reach the hub, and this needs it.</p></div>`);
  const after = settingsApp({ up: false, sent: LAST_SENT, tauri: null });
  const connectors = await after.show("integrations");
  assert.ok(connectors.startsWith(`<h2>Connectors</h2><div class="card"><p>Can't reach the hub, and this needs it. These are the last settings it sent.</p></div><fieldset class="hub-out" disabled>`), connectors.slice(0, 200));
  assert.match(connectors, /Connected as <b>Owner<\/b> \(owner@example\.test\) in Acme\./);
});

test("signed in on this Mac before the hub ever answered, Connectors and Workflows stay editable", async () => {
  const { show } = settingsApp({ up: false, sent: null });
  assert.ok((await show("integrations")).startsWith(`<h2>Connectors</h2>\n    <p class="hint">Services the harness can read from and act on.</p>`));
  assert.match(await show("connectors"), /Connect Linear, Slack or PostHog first/);
  assert.doesNotMatch(await show("connectors"), /hub-out/);
});

test("with the hub gone, this Mac keeps the settings it has and says which sentence fits the copy", async () => {
  const unreachable = { copied: true, runner: "m-local", machine_name: "MacBook", here: false, gone_secs: null, holds_changes: false };
  const gone = settingsApp({ up: false, sent: LAST_SENT, copy: unreachable });
  const connectors = await gone.show("integrations");
  assert.doesNotMatch(connectors, /hub-out|These are the last settings it sent/);
  assert.match(connectors, /Connected as <b>Owner<\/b> \(owner@example\.test\) in Acme\./);
  assert.match(await gone.show("connectors"), /This Mac can't reach the hub\. Workflows keep running on the hub for your other machines\. This Mac's sessions and settings work from its copy\./);

  const runner = settingsApp({ up: false, sent: LAST_SENT, copy: { copied: true, runner: "m-local", machine_name: "MacBook", here: true, gone_secs: 360, holds_changes: true } });
  assert.match(await runner.show("connectors"), /The hub is off\. This Mac checks Linear and Slack and runs the workflows itself\. It hands everything back when the hub returns\./);

  const other = settingsApp({ up: false, sent: LAST_SENT, copy: { copied: true, runner: "m-book", machine_name: "This Mac", here: false, gone_secs: 180, holds_changes: false } });
  other.context.machines.set("m-book", { view: { id: "m-book", name: "MacBook", online: false } });
  assert.match(await other.show("connectors"), /While the hub is off, workflows run on MacBook\. This Mac's sessions and settings work from its copy\./);
});

test("settings read while the hub answers stay editable after it drops", async () => {
  const { context, show, asked } = settingsApp({ up: true, sent: null });
  assert.deepEqual(here(await context.__pullSettings()), LAST_SENT);
  assert.ok(asked.some((row) => row[0] === "hub" && row[1] === "get_settings"));
  context.hub.online = false;
  const connectors = await show("integrations");
  assert.doesNotMatch(connectors, /These are the last settings it sent|hub-out/);
  assert.match(connectors, /Connected as <b>Owner<\/b> \(owner@example\.test\) in Acme\./);
});

test("redrawing the workflow editor before this sign-in's settings arrive draws the editor", () => {
  const { context, byId } = settingsApp({ up: false, sent: null });
  const page = byId("settings-page");
  page.querySelector = () => null;
  vm.runInContext(`view = { name: "edit", open: null, draft: { id: "", name: "Answer", enabled: true, trigger: { kind: "called" } } }; redraw();`, context);
  assert.match(page.innerHTML, /class="canvas"/);
  assert.match(page.innerHTML, /value="Answer"/);
  assert.doesNotMatch(page.innerHTML, /Can't reach the hub|hub-out/);
});

test("with the hub answering, the pages are drawn as before", async () => {
  const { show } = settingsApp({ up: true, sent: LAST_SENT });
  assert.ok((await show("integrations")).startsWith(`<h2>Connectors</h2>\n    <p class="hint">Services the harness can read from and act on.</p>`));
  assert.doesNotMatch(await show("connectors"), /hub-out|Can't reach/);
});

test("while the hub is back and holding, Workflows names who has them and offers the release", async () => {
  const { context, show, asked } = settingsApp({ up: true, sent: { ...LAST_SENT, workflows_held_for: "m-book", offline_runner: "m-local" } });
  context.machines.set("m-book", { view: { id: "m-book", name: "MacBook", online: true } });
  const workflows = await show("connectors");
  assert.match(workflows, /Workflows wait for MacBook, which ran them while the hub was off, to hand them back/);
  assert.match(workflows, /Run them on the hub now/);
  assert.match(workflows, /Anything MacBook started while the hub was off may start again\./);
  assert.match(workflows, /Offline runner/);
  await context.__actions["release-workflows"]();
  assert.ok(asked.some((row) => row[0] === "hub" && row[1] === "release_workflows"));
});

test("a connector change while the hub doesn't answer goes to this Mac's copy", async () => {
  const { context, byId, asked } = settingsApp({ up: false, sent: null });
  byId("linear-key").value = "lin_api_123";
  await context.__actions["connect-linear"]();
  assert.equal(byId("linear-error").textContent, "");
  assert.deepEqual(asked.filter((row) => row[0] === "local" && row[1] === "local_call"), [["local", "local_call", { call: { cmd: "connect_linear", key: "lin_api_123" } }]]);
  assert.deepEqual(asked.filter((row) => row[0] === "hub"), []);
});

test("a call the copy will not answer keeps the plain sentence", async () => {
  const { context, byId, asked } = settingsApp({ up: false, sent: LAST_SENT });
  byId("upstash-key").value = "box-key";
  byId("claude-token").value = "setup-token";
  await context.__actions["connect-upstash"]();
  assert.equal(byId("upstash-error").textContent, "Can't reach the hub, and this needs it.");
  assert.deepEqual(asked.filter((row) => row[0] === "hub"), []);
  assert.deepEqual(asked.filter((row) => row[1] === "local_call").map((row) => row[2].call.cmd), ["connect_upstash"]);
});

test("an unset offline runner is a placeholder, so the only machine can still be chosen", () => {
  const context = {
    LOCAL: "local",
    account: { machine_id: "m-local" },
    machines: new Map([["local", { view: { name: "This Mac" }, hubView: { id: "m-local", name: "This Mac" } }]]),
    settings: {},
    esc: (value) => value,
    runnerLabel: (id) => id,
  };
  vm.createContext(context);
  vm.runInContext(`${fn(settingsSrc, "runnerChoices")}\n${fn(settingsSrc, "runnerPicker")}`, context);
  const unset = context.runnerPicker();
  assert.match(unset, /<option value="" disabled selected>Choose a machine<\/option>/);
  assert.match(unset, /value="m-local">This Mac/);
  context.settings = { offline_runner: "m-local" };
  const set = context.runnerPicker();
  assert.doesNotMatch(set, /Choose a machine/);
  assert.match(set, /value="m-local" selected/);
});

test("signing out asks before it drops a copy that still holds changes", async () => {
  const asked = [];
  const context = {
    TAURI: {},
    account: { machine_id: "m-local" },
    hub: { hub: (call) => (asked.push(call.cmd), Promise.resolve()) },
    invoke: (cmd, args) => {
      asked.push([cmd, args && here(args)]);
      if (cmd === "account_copy") return Promise.resolve({ holds_changes: true });
      return Promise.resolve({ connectors: [] });
    },
    confirm: () => false,
    disconnectHub() { asked.push("disconnect"); },
    showAccount() { asked.push("account"); },
    connectorCall: (cmd, args) => context.invoke("local_call", { call: { cmd, ...args } }),
    console: { warn() {} },
  };
  vm.createContext(context);
  vm.runInContext(`${fn(accountSrc, "signOutApp")}\nglobalThis.signOutApp = signOutApp;`, context);
  await context.signOutApp();
  assert.deepEqual(asked, [["account_copy", undefined]], "a refused confirm leaves the machine on the account");
  context.confirm = (text) => {
    assert.match(text, /This Mac still has changes to hand back\. Signing out drops them\./);
    return true;
  };
  asked.length = 0;
  await context.signOutApp();
  assert.deepEqual(asked, [
    ["account_copy", undefined],
    "remove_machine",
    ["sign_out", { anyway: true }],
    "disconnect",
    ["local_call", { call: { cmd: "get_settings" } }],
    "account",
  ]);
});

test("signing out leaves the machine when the copy cannot be read", async () => {
  const asked = [];
  const context = {
    TAURI: {},
    account: { machine_id: "m-local" },
    hub: { hub: (call) => (asked.push(call.cmd), Promise.resolve()) },
    invoke: (cmd) => {
      asked.push(cmd);
      return Promise.reject(new Error("copy unreadable"));
    },
    confirm: () => true,
    disconnectHub() { asked.push("disconnect"); },
    showAccount() { asked.push("account"); },
    connectorCall: () => Promise.resolve({ connectors: [] }),
    console: { warn() {} },
  };
  vm.createContext(context);
  vm.runInContext(`${fn(accountSrc, "signOutApp")}\nglobalThis.signOutApp = signOutApp;`, context);
  await context.signOutApp();
  assert.deepEqual(asked, ["account_copy"]);
});

test("signed out, a settings call goes to local_call", async () => {
  const asked = [];
  const context = {
    hub: null,
    TAURI: {},
    settingsGen: 0,
    agentApproval: () => null,
    invoke: (cmd, args) => (asked.push([cmd, here(args)]), Promise.resolve({ connectors: [] })),
  };
  vm.createContext(context);
  vm.runInContext(`${read("connection.js")}\n${fn(settingsSrc, "plainHubNeed")}\n${fn(settingsSrc, "connectorCall")}\nglobalThis.connectorCall = connectorCall;`, context);
  assert.deepEqual(here(await context.connectorCall("get_settings")), { connectors: [] });
  assert.deepEqual(asked, [["local_call", { call: { cmd: "get_settings" } }]]);
  context.invoke = () => Promise.reject("needs the hub");
  await assert.rejects(context.connectorCall("list_schedules"), (error) => error === "Can't reach the hub, and this needs it.");
});

test("while the hub doesn't answer, this Mac's own update phases land on its row and finish Update every machine", async () => {
  const drawn = [];
  const { context } = signedIn({
    hubUp: () => false,
    hub: { call: () => Promise.reject("not connected to the hub") },
    invoke: (cmd) => Promise.resolve(cmd === "install_update" ? { state: "updating", to: "" } : null),
    setTimeout: () => ({}),
    settingsOpen: () => true,
    page: "machines",
  });
  vm.runInContext(`${fn(read("main.js"), "onMachineEvent")}\nglobalThis.onMachineEvent = onMachineEvent; showMachinesPage = () => drawn.push("machines page");`, Object.assign(context, { drawn }));
  let finished = false;
  const updating = vm.runInContext("updateEveryMachine()", context).then(() => (finished = true));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(finished, false, "an updating phase is not the end");
  context.onMachineEvent("local", { kind: "update", status: { state: "updating", to: "0.18.8" } });
  assert.deepEqual(here(context.accountMachines()[0].update), { state: "updating", to: "0.18.8" });
  context.onMachineEvent("local", { kind: "update", status: { state: "up_to_date", version: "0.18.8" } });
  await updating;
  assert.equal(finished, true);
  assert.deepEqual(here(context.accountMachines()[0].update), { state: "up_to_date", version: "0.18.8" });
  assert.deepEqual(drawn, ["machines page", "machines page"]);
});
