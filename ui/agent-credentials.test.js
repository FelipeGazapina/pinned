import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
// Values built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => JSON.parse(JSON.stringify(value));

const asked = {
  id: "c-5",
  at: 1794249424,
  machine: "this-mac",
  machine_name: "MacBook",
  tab: "tab-4",
  title: "Wire up the tracker",
  role: "worker",
  about: "",
  request_id: "r-1",
};
const linearAccount = { name: "Ana", email: "ana@acme.dev", organization: "Acme" };
const goWork = { id: "go-work", name: "Go work", provider: "claude", provider_name: "Claude Code", auth: "opencode-go", signed_in: true, builtin: false };

function element(id) {
  return {
    id,
    value: "",
    textContent: "",
    innerHTML: "",
    hidden: false,
    dataset: {},
    focused: false,
    removed: false,
    classList: { contains: () => false, add() {}, remove() {}, toggle() {} },
    addEventListener() {},
    focus() {
      this.focused = true;
    },
    remove() {
      this.removed = true;
    },
    querySelector: () => null,
  };
}

/** The Add an account form as the page builds it, with the fields addAccount reads. */
function addForm() {
  const form = element("add-account");
  Object.assign(form, { provider: element(), name: element(), key: element(), submit: element() });
  form.querySelector = () => form.submit;
  return form;
}

/** settings.js, accounts.js and agent-surface.js as the page runs them, with the hub played by
 * `answer` and every element the code asks for by id kept in `dom`. Settings is open, as on the
 * Agent activity page, so opening a form asks the hub nothing. */
function app({ answer, machines = [] } = {}) {
  const calls = [];
  const told = [];
  const dom = {};
  const form = addForm();
  const byId = (id) => (dom[id] ??= element(id));
  const settingsPage = byId("settings-page");
  settingsPage.querySelector = (selector) => (selector === ".add-account" ? form : null);
  const context = {
    console,
    addEventListener() {},
    load: () => null,
    save() {},
    listen() {},
    LOCAL: "local",
    account: { machine_id: "this-mac", email: "ana@acme.dev" },
    machines: new Map([["local", { view: { online: true }, hubView: { name: "MacBook" } }], ...machines]),
    hub: { hub: (call) => (calls.push(call), answer(call)) },
    picker: {},
    active: null,
    icon: () => "",
    guiOn: () => false,
    renderSidebar() {},
    render() {},
    tell: (text) => told.push(text),
    ago: () => "5m",
    inviteDate: () => "Nov 16",
    clockTime: () => "14:30",
    machineName: (id) => ({ local: "MacBook", "imac-id": "iMac" })[id],
    accountsOf: async () => {},
    agentConsentHtml: () => "",
    externalAgentsCardHtml: () => "",
    loadExternalAgents() {},
    convexCardHtml: () => "",
    loadConvexProjects() {},
    document: { body: { dataset: { page: "settings" } }, getElementById: byId, querySelector: () => element(), createElement: () => element() },
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(
    [
      read("settings.js"),
      `const onlineMachines = () => [...machines.entries()].filter(([, m]) => m.view.online).map(([id]) => id);`,
      fn(read("main.js"), "setAccounts"),
      fn(read("gui-shell.js"), "openSettings"),
      read("agent-surface.js"),
      read("accounts.js"),
    ].join("\n"),
    context,
  );
  const state = (name) => vm.runInContext(name, context);
  return { context, calls, told, dom, form, state, actions: state("actions") };
}

function click(context, agent, confirmation) {
  const row = { dataset: { confirmation } };
  const button = { dataset: { agent }, disabled: false, closest: (selector) => (selector === "[data-agent]" ? button : row) };
  context.agentSurfaceClick({ target: button });
  return flush();
}

const waitingIds = (context) => [...(context.agentWaitingRows()[0]?.innerHTML ?? "").matchAll(/data-confirmation="([^"]+)"/g)].map(([, id]) => id);

test("Approve on connect_linear opens the Connectors page's Linear form, says who asked, and sends nothing", async () => {
  const { context, calls, dom, state } = app();
  context.onAgentConfirmations([{ ...asked, action: "connect_linear", args: {} }]);
  await click(context, "approve", "c-5");
  assert.deepEqual(calls, []);
  assert.equal(state("page"), "integrations");
  assert.match(
    dom["settings-page"].innerHTML,
    /<p class="agent-asking" id="agent-asking" role="status">Asked by “Wire up the tracker” on MacBook\. Entering the key here approves it\.\s*<button type="button" class="secondary" data-action="cancel-agent-approval">Cancel<\/button><\/p><ol class="steps">/,
  );
  assert.match(dom["settings-page"].innerHTML, /<input id="linear-key" type="password" placeholder="lin_api_…" autocomplete="off">/);
  assert.equal(dom["linear-key"].focused, true);
  assert.deepEqual(waitingIds(context), ["c-5"]);
});

test("the Linear key typed in that form approves the request with it, and the app takes Linear's answer as Connect's", async () => {
  const { context, calls, told, dom, state, actions } = app({ answer: async () => ({ decision: "approved", outcome: "done", result: linearAccount }) });
  context.onAgentConfirmations([{ ...asked, action: "connect_linear", args: {} }]);
  await click(context, "approve", "c-5");
  dom["linear-key"].value = "lin_api_typed";
  await actions["connect-linear"]();
  assert.deepEqual(here(calls), [{ cmd: "decide_confirmation", id: "c-5", approve: true, entered: { key: "lin_api_typed" } }]);
  assert.deepEqual(here(state("settings.linear")), linearAccount);
  assert.match(dom["settings-page"].innerHTML, /<p class="connected">Connected as <b>Ana<\/b> \(ana@acme\.dev\) in Acme\.<\/p>/);
  assert.doesNotMatch(dom["settings-page"].innerHTML, /agent-asking/);
  assert.deepEqual(told, ["Approved connect_linear: done"]);
  assert.deepEqual(waitingIds(context), []);
});

test("a key the service refuses shows in the form's error line, the request keeps waiting, and another key tries again", async () => {
  const answers = [Promise.reject("Linear refused the key"), Promise.resolve({ decision: "approved", outcome: "done", result: linearAccount })];
  answers[0].catch(() => {});
  const { context, calls, told, dom, actions } = app({ answer: () => answers.shift() });
  context.onAgentConfirmations([{ ...asked, action: "connect_linear", args: {} }]);
  await click(context, "approve", "c-5");
  dom["linear-key"].value = "lin_api_wrong";
  await actions["connect-linear"]();
  assert.equal(dom["linear-error"].textContent, "Linear refused the key");
  assert.deepEqual(told, []);
  assert.deepEqual(waitingIds(context), ["c-5"]);
  assert.match(context.agentWaitingRows()[0].innerHTML, /data-agent="approve">Approve<\/button>/);
  dom["linear-key"].value = "lin_api_right";
  await actions["connect-linear"]();
  assert.deepEqual(here(calls), [
    { cmd: "decide_confirmation", id: "c-5", approve: true, entered: { key: "lin_api_wrong" } },
    { cmd: "decide_confirmation", id: "c-5", approve: true, entered: { key: "lin_api_right" } },
  ]);
  assert.deepEqual(waitingIds(context), []);
});

test("Cancel leaves the request waiting, and the form connects Linear as itself again", async () => {
  const { context, calls, dom, actions } = app({ answer: async () => linearAccount });
  context.onAgentConfirmations([{ ...asked, action: "connect_linear", args: {} }]);
  await click(context, "approve", "c-5");
  actions["cancel-agent-approval"]();
  assert.equal(dom["agent-asking"].removed, true);
  dom["linear-key"].value = "lin_api_mine";
  await actions["connect-linear"]();
  assert.deepEqual(here(calls), [{ cmd: "connect_linear", key: "lin_api_mine" }]);
  assert.deepEqual(waitingIds(context), ["c-5"]);
});

test("a request the hub stops listing, as when it is denied elsewhere, no longer rides on the form", async () => {
  const { context, calls, dom, actions } = app({ answer: async () => linearAccount });
  context.onAgentConfirmations([{ ...asked, action: "connect_linear", args: {} }]);
  await click(context, "approve", "c-5");
  context.onAgentConfirmations([]);
  assert.equal(dom["agent-asking"].removed, true);
  dom["linear-key"].value = "lin_api_mine";
  await actions["connect-linear"]();
  assert.deepEqual(here(calls), [{ cmd: "connect_linear", key: "lin_api_mine" }]);
});

test("Approve on set_account_key opens that machine's Accounts page with the replace box of the account it names", async () => {
  const saved = { ...goWork, signed_in: true, email: "ana@acme.dev" };
  const imac = ["imac-id", { view: { online: true, name: "iMac" }, accounts: [goWork] }];
  const { context, calls, dom, state, actions } = app({ machines: [imac], answer: async () => ({ decision: "approved", outcome: "done", result: saved }) });
  context.onAgentConfirmations([{ ...asked, action: "set_account_key", args: { account: "go-work" }, target: "imac-id" }]);
  await click(context, "approve", "c-5");
  assert.equal(state("page"), "accounts");
  assert.match(dom["settings-page"].innerHTML, /<button class="chip active" data-action="accounts-machine" data-machine="imac-id">iMac<\/button>/);
  assert.match(
    dom["account-list"].innerHTML,
    /<div class="card" data-account="go-work">[\s\S]*<div class="replace-key"><p class="agent-asking" id="agent-asking" role="status">Asked by “Wire up the tracker” on MacBook\. Entering the key here approves it\.[\s\S]*<input name="key" type="password" placeholder="oc_sk_\.\.\."/,
  );
  const input = { value: "oc_sk_new" };
  const card = { dataset: { account: "go-work" }, querySelector: (selector) => (selector === "[name=key]" ? input : element()) };
  await actions["save-account-key"]({ closest: () => card, disabled: false });
  assert.deepEqual(here(calls), [{ cmd: "decide_confirmation", id: "c-5", approve: true, entered: { key: "oc_sk_new" } }]);
  assert.deepEqual(here(context.machines.get("imac-id").accounts), [saved]);
  assert.equal(state("accountsUi.replacing"), null);
});

test("Approve on add_account fills the Add an account form with the name and provider asked for, and adding sends only the key", async () => {
  const added = { ...goWork, id: "side-go", name: "Side Go" };
  const { context, calls, dom, form } = app({ answer: async () => ({ decision: "approved", outcome: "done", result: added }) });
  context.onAgentConfirmations([{ ...asked, action: "add_account", args: { provider: "claude", name: "Side Go", auth: "opencode-go" } }]);
  await click(context, "approve", "c-5");
  const html = dom["settings-page"].innerHTML;
  assert.match(html, /<p class="agent-asking" id="agent-asking" role="status">Asked by “Wire up the tracker” on MacBook\. Entering the key here approves it\./);
  assert.match(html, /<select name="provider" disabled><option value="claude">Claude Code<\/option><option value="codex">Codex<\/option><option value="cursor">Cursor<\/option><option value="opencode-go" selected>OpenCode Go<\/option><\/select>/);
  assert.match(html, /<input name="name" placeholder="Work" autocomplete="off" spellcheck="false" value="Side Go" readonly>/);
  Object.assign(form.provider, { value: "opencode-go" });
  Object.assign(form.name, { value: "Side Go" });
  form.provider.onchange();
  assert.equal(form.key.hidden, false);
  form.key.value = "oc_sk_1";
  await context.addAccount(form);
  assert.deepEqual(here(calls), [{ cmd: "decide_confirmation", id: "c-5", approve: true, entered: { key: "oc_sk_1" } }]);
  assert.deepEqual(here(context.machines.get("local").accounts), [added]);
});

test("an add_account without a key says adding approves it, and sends entered empty", async () => {
  const added = { ...goWork, id: "side", name: "Side", provider: "codex", auth: undefined };
  const { context, calls, dom, form } = app({ answer: async () => ({ decision: "approved", outcome: "done", result: added }) });
  context.onAgentConfirmations([{ ...asked, action: "add_account", args: { provider: "codex", name: "Side" } }]);
  await click(context, "approve", "c-5");
  assert.match(dom["settings-page"].innerHTML, /Asked by “Wire up the tracker” on MacBook\. Adding the account here approves it\./);
  assert.match(dom["settings-page"].innerHTML, /<option value="codex" selected>Codex<\/option>/);
  Object.assign(form.provider, { value: "codex" });
  Object.assign(form.name, { value: "Side" });
  await context.addAccount(form);
  assert.deepEqual(here(calls), [{ cmd: "decide_confirmation", id: "c-5", approve: true, entered: {} }]);
});
