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
  const context = {
    visibleInterval: () => 0,
    console,
    addEventListener() {},
    setTimeout() {},
    TAURI: false,
    LOCAL: "local",
    account: { email: "test@example.test" },
    hubUp: () => true,
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
    accountsOf: async () => [],
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
  return { context, calls, byId, fire: (type, target) => byId("settings-page").listeners[type].forEach((fn) => fn({ target })) };
}

const card = (context) => context.slackCardHtml();
const plain = (html) => html.replace(/\s+/g, " ").replace(/<[^>]+>/g, "|").replace(/\|(\s*\|)+/g, "|");

test("with no bot the Slack card shows the setup steps and a Connect button", () => {
  const { context } = load();
  const html = card(context);
  assert.match(html, /<span class="badge" data-on="false">Not connected<\/span>/);
  assert.match(html, /id="slack-token"/);
  assert.match(html, />Connect<\/button>/);
  assert.doesNotMatch(html, /Add a bot/);
});

test("with one bot the card lists it as the default and offers Add a bot without the token field", () => {
  const { context } = load({ bots: [SUPPORT], connectors: [workflow("Triage")] });
  const html = card(context);
  assert.match(html, /<h3>Acme<\/h3>/);
  assert.match(html, /<b>@pinned-support<\/b> <span class="badge" data-on="true">Default<\/span>/);
  assert.match(html, /xoxb-…0001/);
  assert.match(html, /Used by: Triage/);
  assert.match(html, /data-action="remove-slack-bot" data-bot="UB1">Remove<\/button>/);
  assert.match(html, /data-action="add-slack-bot">Add a bot<\/button>/);
  assert.doesNotMatch(html, /id="slack-token"/);
});

test("two bots in two workspaces are grouped by workspace; only the first is the default", () => {
  const connectors = [workflow("Triage"), workflow("Ops alerts", { slackBot: "UB2" }), workflow("Paused one", { slackBot: "UB2", enabled: false })];
  const { context } = load({ bots: [SUPPORT, OPS], connectors });
  const html = card(context);
  assert.ok(html.indexOf("<h3>Acme</h3>") < html.indexOf("@pinned-support"));
  assert.ok(html.indexOf("@pinned-support") < html.indexOf("<h3>Globex</h3>"));
  assert.ok(html.indexOf("<h3>Globex</h3>") < html.indexOf("@pinned-ops"));
  assert.equal(html.match(/>Default</g).length, 1);
  assert.match(plain(html), /@pinned-support.*Default.*Used by: Triage.*Remove.*Globex.*@pinned-ops.*Used by: Ops alerts, Paused one/);
});

test("two bots in one workspace share one group header", () => {
  const { context } = load({ bots: [SUPPORT, SECOND_IN_ACME] });
  const html = card(context);
  assert.equal(html.match(/<h3>Acme<\/h3>/g).length, 1);
  assert.match(html, /Not used by a workflow/);
});

test("a workflow with no bot of its own counts for the default bot only when it has a Slack trigger", () => {
  const linear = workflow("Linear flow", { trigger: { kind: "linear-label", label: "claude" } });
  const { context } = load({ bots: [SUPPORT, OPS], connectors: [linear] });
  assert.doesNotMatch(card(context), /Used by/);
});

test("connecting a new bot says it was added and lists it", async () => {
  const { context, byId } = load({
    bots: [SUPPORT],
    reply: (call) => (call.cmd === "connect_slack" ? { team: "Globex", name: "pinned-ops", userId: "UB2", bot: OPS, added: true } : { slack_bots: [SUPPORT, OPS], connectors: [] }),
  });
  byId("slack-token").value = "xoxb-test-0002";
  await context.__actions["connect-slack"]();
  const html = byId("settings-page").innerHTML;
  assert.match(html, /<p class="connected" id="slack-note">Added @pinned-ops in Globex<\/p>/);
  assert.match(html, /<h3>Globex<\/h3>/);
});

test("pasting the token of a bot that is already connected says it updated the token", async () => {
  const { context, byId } = load({
    bots: [SUPPORT],
    reply: (call) => (call.cmd === "connect_slack" ? { team: "Acme", name: "pinned-support", userId: "UB1", bot: SUPPORT, added: false } : { slack_bots: [SUPPORT], connectors: [] }),
  });
  byId("slack-token").value = "xoxb-test-0009";
  await context.__actions["connect-slack"]();
  assert.match(byId("settings-page").innerHTML, /<p class="connected" id="slack-note">Updated the token for @pinned-support<\/p>/);
});

test("Remove asks the hub to disconnect that bot and redraws without it", async () => {
  const { context, calls, byId } = load({
    bots: [SUPPORT, OPS],
    reply: (call) => (call.cmd === "get_settings" ? { slack_bots: [SUPPORT], connectors: [] } : {}),
  });
  await context.__actions["remove-slack-bot"]({ dataset: { bot: "UB2" } });
  assert.deepEqual(JSON.parse(JSON.stringify(calls[0])), { cmd: "disconnect_slack", bot: "UB2" });
  assert.doesNotMatch(byId("settings-page").innerHTML, /pinned-ops/);
});

test("a refused Remove shows the hub's message on that row only", async () => {
  const refusal = "Slack bot @pinned-ops is used by: Ops alerts, Paused one. Move or delete those workflows first.";
  const { context, byId } = load({
    bots: [SUPPORT, OPS],
    reply: (call) => {
      if (call.cmd === "disconnect_slack") throw refusal;
      return {};
    },
  });
  await context.__actions["remove-slack-bot"]({ dataset: { bot: "UB2" } });
  const html = byId("settings-page").innerHTML;
  const ops = html.slice(html.indexOf('data-bot="UB2"'));
  assert.ok(ops.includes(`<p class="error">${refusal}</p>`));
  assert.ok(!html.slice(0, html.indexOf('data-bot="UB2"')).includes("Move or delete"));
  assert.match(html, /@pinned-ops/);
});

for (const step of ["slack-trigger", "message-trigger"]) {
  test(`${step}: no Bot select with one bot`, () => {
    const { context } = load({ bots: [SUPPORT] });
    assert.doesNotMatch(context.__steps[step].fields(workflow("A")), /Bot <select/);
  });

  test(`${step}: with two bots the Bot select lists them, the default first, and binds to the connector`, () => {
    const { context } = load({ bots: [SUPPORT, OPS] });
    const html = context.__steps[step].fields(workflow("A"));
    assert.match(
      html,
      /<label>Bot <select data-bind="slackBot"><option value="" selected>@pinned-support in Acme \(default\)<\/option><option value="UB2">@pinned-ops in Globex<\/option><\/select><\/label>/,
    );
    assert.match(context.__steps[step].fields(workflow("A", { slackBot: "UB2" })), /<option value="UB2" selected>/);
  });
}

test("the Slack trigger counts as connected with bots, even without the single legacy account", () => {
  const { context } = load({ bots: [SUPPORT, OPS] });
  context.__setSettings({ linear: null, slack: null, slack_bots: [SUPPORT, OPS], connectors: [], checked: null });
  assert.deepEqual(JSON.parse(JSON.stringify(context.__steps["slack-trigger"].issues(workflow("A")))), []);
  context.__setSettings({ linear: null, slack: null, slack_bots: [], connectors: [], checked: null });
  assert.deepEqual(JSON.parse(JSON.stringify(context.__steps["slack-trigger"].issues(workflow("A")))), ["Connect Slack under Connectors."]);
});

test("picking the second bot saves slackBot on the connector, not on the trigger, and loads that bot's channels", async () => {
  const { context, calls, fire } = load({ bots: [SUPPORT, OPS], reply: (call) => (call.cmd === "slack_suggestions" ? ["ops-alerts"] : {}) });
  const draft = workflow("A", { repo: "", model: "", account: "", team: "", label: "", instructions: "" });
  context.__setView({ name: "edit", index: 0, draft, original: "", open: null, zoom: null, pan: null, menu: false, graphPick: null, selectedEdge: null, connectFrom: null, drag: null });
  const target = { dataset: { bind: "slackBot" }, value: "UB2", type: "select-one" };
  fire("input", target);
  assert.equal(draft.slackBot, "UB2");
  assert.equal("slackBot" in draft.trigger, false);
  fire("change", target);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), { cmd: "slack_suggestions", bot: "UB2" });
  fire("input", { dataset: { bind: "slackBot" }, value: "", type: "select-one" });
  assert.equal(draft.slackBot, "");
});

const APPS_LABEL = "Also read posts from apps and integrations";
const DECIDE = { model: "typesafe-ai/jev", instructions: "Sort", categories: [], fallback: null };
/** A saved slack-message workflow as the hub keeps it: the legacy trigger and the graph's trigger node. */
const alerts = (apps) => {
  const trigger = { kind: "slack-message", channel: "alerts", decide: DECIDE, ...(apps === undefined ? {} : { apps }) };
  const watch = { kind: "slack-message", channel: "alerts", ...(apps === undefined ? {} : { apps }) };
  const graph = { nodes: [{ id: "trigger-1", name: "Trigger", position: [0, 0], type: "trigger", watch }, { id: "session-2", name: "Session", position: [1, 0], type: "session" }], edges: [{ from: "trigger-1", port: "out", to: "session-2" }] };
  return workflow("Alerts", { id: "w1", trigger, graph, repo: "", model: "", account: "", team: "", label: "", instructions: "", columns: { picked: "", coding: "", review: "" } });
};
const checkbox = (on) => `<label class="toggle"><input type="checkbox" data-bind="trigger.apps"${on ? " checked" : ""}> ${APPS_LABEL}</label>`;

test("the message trigger offers the apps option, off unless the workflow has it on", () => {
  const { context } = load();
  const fields = (c) => context.__steps["message-trigger"].fields(c);
  assert.ok(fields(alerts()).includes(checkbox(false)), "an old workflow, stored without apps, shows it off");
  assert.ok(fields(alerts(false)).includes(checkbox(false)));
  assert.ok(fields(alerts(true)).includes(checkbox(true)));
  assert.ok(!context.__steps["slack-trigger"].fields(workflow("A")).includes(APPS_LABEL), "a mention trigger has no such option");
});

test("a workflow saved with the apps option on comes back with it on", async () => {
  const saved = [];
  const { context, calls, byId, fire } = load({
    bots: [SUPPORT],
    reply: (call) => {
      if (call.cmd !== "save_connector") return {};
      saved.push(JSON.parse(JSON.stringify(call.connector)));
      return call.connector;
    },
  });
  const draft = alerts();
  context.__setSettings({ linear: null, slack: SUPPORT, slack_bots: [SUPPORT], connectors: [draft], checked: null });
  context.__actions["edit-connector"]({ closest: () => ({ dataset: { index: "0" } }) });
  assert.ok(byId("settings-page").innerHTML.includes(checkbox(false)), "the editor opens with it off");
  fire("input", { dataset: { bind: "trigger.apps" }, type: "checkbox", checked: true });
  await context.__actions["save-connector"]();
  const sent = saved.at(-1);
  assert.equal(calls.at(-1).cmd, "save_connector");
  assert.equal(sent.trigger.apps, true, "the saved trigger carries it");
  assert.equal(sent.graph.nodes[0].watch.apps, true, "and so does the graph's trigger node");
  context.__setSettings({ linear: null, slack: SUPPORT, slack_bots: [SUPPORT], connectors: [sent], checked: null });
  context.__actions["edit-connector"]({ closest: () => ({ dataset: { index: "0" } }) });
  assert.ok(byId("settings-page").innerHTML.includes(checkbox(true)), "reopened, it shows on");
});

test("a workflow saved with the apps option off stores no apps field, in its trigger or its graph", async () => {
  const saved = [];
  const { context, fire } = load({
    bots: [SUPPORT],
    reply: (call) => {
      if (call.cmd !== "save_connector") return {};
      saved.push(JSON.parse(JSON.stringify(call.connector)));
      return call.connector;
    },
  });
  context.__setSettings({ linear: null, slack: SUPPORT, slack_bots: [SUPPORT], connectors: [alerts()], checked: null });
  context.__actions["edit-connector"]({ closest: () => ({ dataset: { index: "0" } }) });
  fire("input", { dataset: { bind: "trigger.apps" }, type: "checkbox", checked: true });
  fire("input", { dataset: { bind: "trigger.apps" }, type: "checkbox", checked: false });
  await context.__actions["save-connector"]();
  const sent = saved.at(-1);
  assert.ok(!("apps" in sent.trigger), JSON.stringify(sent.trigger));
  assert.ok(!("apps" in sent.graph.nodes[0].watch), JSON.stringify(sent.graph.nodes[0].watch));
});
