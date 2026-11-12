import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import * as PinnedGraph from "./graph.js";

const settingsSrc = readFileSync(new URL("./settings.js", import.meta.url), "utf8");

const element = (id) => {
  const listeners = {};
  return {
    id,
    listeners,
    addEventListener(type, fn) {
      (listeners[type] ??= []).push(fn);
    },
    dataset: {},
    hidden: false,
    innerHTML: "",
    textContent: "",
    value: "",
    classList: { contains: () => false, add() {}, remove() {}, toggle() {} },
    focus() {},
    querySelector: () => element("any"),
    querySelectorAll: () => [],
  };
};

const SUPPORT = { id: "UB1", teamId: "T1", team: "Acme", userId: "UB1", name: "pinned-support", appId: "A1", keyHint: "xoxb-…0001" };
const OPS = { id: "UB2", teamId: "T2", team: "Globex", userId: "UB2", name: "pinned-ops", appId: "A2", keyHint: "xoxb-…0002" };
const SECOND_IN_ACME = { id: "UB3", teamId: "T1", team: "Acme", userId: "UB3", name: "pinned-docs", appId: "A3", keyHint: "xoxb-…0003" };
const workflow = (name, extra = {}) => ({ id: name, name, enabled: true, trigger: { kind: "slack-mention", channel: "support" }, folder: "/repo", machine: "", columns: {}, ...extra });

/** settings.js alone, with the hub's answers scripted per command. */
function load({ bots = [], connectors = [], reply = () => ({}) } = {}) {
  const elements = new Map();
  const byId = (id) => {
    if (!elements.has(id)) elements.set(id, element(id));
    return elements.get(id);
  };
  const calls = [];
  const shown = [];
  const context = {
    console: { error() {}, warn() {}, log() {} },
    flash: (text, actions, ms) => shown.push({ text, ms }),
    addEventListener() {},
    setTimeout() {},
    TAURI: false,
    LOCAL: "local",
    account: { email: "test@example.test" },
    hub: {
      async hub(call) {
        calls.push(call);
        return reply(call);
      },
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
    PinnedGraph,
    projectPathOf: (tab) => tab.view.cwd,
    async openSettings() {},
    document: { body: { dataset: {} }, getElementById: byId, querySelector: () => element(), querySelectorAll: () => [] },
  };
  context.globalThis = context;
  vm.runInNewContext(
    `${settingsSrc}\nglobalThis.__settings = settings; globalThis.__actions = actions; globalThis.__steps = STEPS; globalThis.__setView = (next) => { view = next; }; globalThis.__setSettings = (next) => { settings = next; };`,
    context,
  );
  context.__setSettings({ linear: null, slack: bots[0] ?? null, slack_bots: bots, connectors, checked: null });
  return { context, calls, shown, byId, fire: (type, target, event = {}) => byId("settings-page").listeners[type].forEach((fn) => fn({ target, preventDefault() {}, ...event })) };
}

/** A click on the element carrying data-action, through the page's own dispatcher. */
const click = (fire, action, data = {}) => {
  const button = { dataset: { action, ...data } };
  return fire("click", { closest: (selector) => (selector === "[data-action]" ? button : null) });
};
const settle = () => new Promise((resolve) => setImmediate(resolve));

test("a rejected Disconnect on a card with no catch of its own shows the hub's text in the note", async () => {
  const refusal = "Vercel refused: the key is in use.";
  const { context, fire, shown } = load({
    reply: (call) => {
      if (call.cmd === "disconnect_vercel") throw refusal;
      return {};
    },
  });
  context.__setSettings({ linear: null, slack: null, slack_bots: [], connectors: [], vercel: { team: "Acme" }, checked: null });
  click(fire, "disconnect-vercel");
  await settle();
  assert.deepEqual(shown, [{ text: "Vercel refused: the key is in use.", ms: 6000 }]);
});

test("an Error's message shows without the Error: prefix", async () => {
  const { context, fire, shown } = load({
    reply: (call) => {
      if (call.cmd === "delete_connector") throw new Error("connector is running");
      return {};
    },
  });
  context.__setView({ name: "edit", draft: { id: "c1" } });
  click(fire, "delete-connector", { armed: "" });
  await settle();
  assert.deepEqual(shown.map((n) => n.text), ["connector is running"]);
});

test("the Enter key on a token field shows a failure from the hub too", async () => {
  const { byId, fire, shown, context } = load({
    reply: (call) => {
      if (call.cmd === "connect_vercel") throw "the hub is unreachable";
      return {};
    },
  });
  // connect-vercel writes its own error into #vercel-error, so an escape from it needs another route:
  context.__actions["connect-vercel"] = async () => {
    throw "the hub is unreachable";
  };
  byId("vercel-key").value = "vck_test";
  fire("keydown", { id: "vercel-key" }, { key: "Enter" });
  await settle();
  assert.deepEqual(shown.map((n) => n.text), ["the hub is unreachable"]);
});

test("the Slack bot row's refusal stays on the row and does not also flash", async () => {
  const refusal = "Slack bot @pinned-ops is used by: Ops alerts. Move or delete those workflows first.";
  const { fire, byId, shown } = load({
    bots: [SUPPORT, OPS],
    reply: (call) => {
      if (call.cmd === "disconnect_slack") throw refusal;
      return {};
    },
  });
  click(fire, "remove-slack-bot", { bot: "UB2" });
  await settle();
  assert.ok(byId("settings-page").innerHTML.includes(`<p class="error">${refusal}</p>`));
  assert.deepEqual(shown, []);
});

test("a cancelled password prompt shows nothing", async () => {
  const { fire, shown } = load({
    reply: (call) => {
      if (call.cmd === "disconnect_vercel") throw "this needs your password";
      return {};
    },
  });
  click(fire, "disconnect-vercel");
  await settle();
  assert.deepEqual(shown, []);
});
