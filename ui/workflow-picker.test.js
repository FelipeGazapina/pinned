import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import * as PinnedGraph from "./graph.js";

const settingsSrc = readFileSync(new URL("./settings.js", import.meta.url), "utf8");

const element = (id) => ({
  id,
  listeners: {},
  addEventListener(type, fn) {
    (this.listeners[type] ??= []).push(fn);
  },
  dataset: {},
  hidden: false,
  innerHTML: "",
  textContent: "",
  value: "",
  classList: { contains: () => false, add() {}, remove() {}, toggle() {} },
  focus() {},
  scrollIntoView() {},
  querySelector: () => element("any"),
  querySelectorAll: () => [],
});

const LINEAR = { name: "Ada Example", email: "ada@example.test", organization: "Example Co" };
const BOT = { id: "UB1", teamId: "T1", team: "Acme", userId: "UB1", name: "pinned-support", appId: "A1", keyHint: "xoxb-…0001" };

/** settings.js alone, on the Workflows page, with the hub's answers scripted per command. */
const LISTS = new Set(["vercel_models", "github_repos", "slack_suggestions", "linear_suggestions"]);

function load({ linear = null, bots = [], reply = () => ({}) } = {}) {
  const elements = new Map();
  const byId = (id) => {
    if (!elements.has(id)) elements.set(id, element(id));
    return elements.get(id);
  };
  const calls = [];
  const context = {
    console,
    addEventListener() {},
    setTimeout() {},
    TAURI: false,
    active: null,
    accountsOf: async () => [],
    LOCAL: "local",
    account: { email: "test@example.test" },
    hub: {
      async hub(call) {
        calls.push(call);
        if (call.cmd === "linear_suggestions") return { labels: [], columns: [], teams: [] };
        return LISTS.has(call.cmd) ? [] : reply(call);
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
    document: { body: { dataset: { page: "settings" } }, getElementById: byId, querySelector: () => element(), querySelectorAll: () => [] },
  };
  context.globalThis = context;
  vm.runInNewContext(
    `${settingsSrc}\nglobalThis.__actions = actions; globalThis.__state = () => ({ page, view }); globalThis.__setSettings = (next) => { settings = next; }; globalThis.__settings = () => settings;`,
    context,
  );
  context.__setSettings({ linear, slack: bots[0] ?? null, slack_bots: bots, connectors: [], checked: null });
  const html = () => byId("settings-page").innerHTML;
  /** The picker's button for `kind`, as `{ action, hint }`. */
  const option = (kind) => {
    const at = html().indexOf(`data-kind="${kind}"`);
    assert.ok(at >= 0, `the picker lists ${kind}`);
    const open = html().lastIndexOf("<button", at);
    const close = html().indexOf("</button>", at);
    const button = html().slice(open, close);
    return { action: button.match(/data-action="([^"]+)"/)[1], text: button.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim() };
  };
  /** Clicks the picker's option for `kind`, as the page's click handler would. */
  const click = (kind) => context.__actions[option(kind).action]({ dataset: { kind }, closest: () => null });
  return { context, calls, html, option, click, state: () => context.__state() };
}

/** Opens Settings › Workflows › New workflow. */
async function openPicker(env) {
  await env.context.__actions["new-connector"]();
  assert.equal(env.state().view.name, "pick");
}

test("with Linear connected, clicking the Linear option opens the new workflow, not Connectors", async () => {
  const env = load({ linear: LINEAR });
  await openPicker(env);
  assert.equal(env.option("linear-label").action, "pick-trigger");
  assert.doesNotMatch(env.option("linear-label").text, /Connect .* first/);
  await env.click("linear-label");
  assert.notEqual(env.state().page, "integrations");
  assert.equal(env.state().view.name, "edit");
  assert.equal(env.state().view.draft.trigger.kind, "linear-label");
});

test("with Slack connected, every Slack option opens the new workflow with its own trigger", async () => {
  for (const kind of ["slack-mention", "slack-message", "called"]) {
    const env = load({ bots: [BOT] });
    await openPicker(env);
    assert.equal(env.option(kind).action, "pick-trigger", kind);
    await env.click(kind);
    assert.equal(env.state().view.name, "edit", kind);
    assert.equal(env.state().view.draft.trigger.kind, kind);
  }
});

test("a bot in slack_bots alone, without the legacy slack field, counts as Slack connected", async () => {
  const env = load({ bots: [BOT] });
  env.context.__setSettings({ ...env.context.__settings(), slack: null });
  await openPicker(env);
  assert.equal(env.option("slack-mention").action, "pick-trigger");
});

test("an option names the connector it is missing: Slack options say Slack, Linear says Linear", async () => {
  const env = load({ linear: LINEAR });
  await openPicker(env);
  assert.match(env.option("slack-mention").text, /Connect Slack/);
  assert.doesNotMatch(env.option("slack-mention").text, /Linear under/);
});

test("without Linear, the Linear option says why, and Connectors explains the detour and returns to the new workflow once connected", async () => {
  const env = load({ reply: (call) => (call.cmd === "connect_linear" ? LINEAR : {}) });
  await openPicker(env);
  assert.match(env.option("linear-label").text, /Connect Linear under Connectors first/);
  await env.click("linear-label");
  assert.equal(env.state().page, "integrations");
  assert.match(env.html(), /creating the workflow/i);
  assert.match(env.html(), /A Linear issue gets a label/);
  assert.match(env.html(), /Connect Linear below/);
  env.context.document.getElementById("linear-key").value = "lin_api_fake";
  await env.context.__actions["connect-linear"]();
  assert.equal(env.state().page, "connectors");
  assert.equal(env.state().view.name, "edit");
  assert.equal(env.state().view.draft.trigger.kind, "linear-label");
});

test("without Slack, a Slack option returns to the new workflow once a bot is connected", async () => {
  let bots = [];
  const env = load({ reply: (call) => (call.cmd === "connect_slack" ? ((bots = [BOT]), { name: "pinned-support", team: "Acme", added: true }) : call.cmd === "get_settings" ? { linear: null, slack: bots[0] ?? null, slack_bots: bots, connectors: [], checked: null } : {}) });
  await openPicker(env);
  await env.click("slack-mention");
  assert.equal(env.state().page, "integrations");
  assert.match(env.html(), /Connect Slack below/);
  env.context.document.getElementById("slack-token").value = "xoxb-fake";
  await env.context.__actions["connect-slack"]();
  assert.equal(env.state().page, "connectors");
  assert.equal(env.state().view.draft.trigger.kind, "slack-mention");
});

test("an unfinished draft is kept: going to Connectors and back resumes it as it was", async () => {
  const env = load({ bots: [BOT], reply: (call) => (call.cmd === "connect_linear" ? LINEAR : {}) });
  await openPicker(env);
  await env.click("slack-mention");
  const draft = env.state().view.draft;
  draft.name = "My half-done workflow";
  await env.context.__actions["connect-for-trigger"]({ dataset: { kind: "linear-label" }, closest: () => null });
  assert.equal(env.state().page, "integrations");
  await env.context.__actions["resume-workflow"]();
  assert.equal(env.state().page, "connectors");
  assert.equal(env.state().view.draft, draft);
  assert.equal(env.state().view.draft.name, "My half-done workflow");
});

test("leaving the detour drops the note: Connectors opened on its own shows no workflow banner", async () => {
  const env = load({});
  await openPicker(env);
  await env.click("linear-label");
  await env.context.__actions["forget-pending-workflow"]();
  assert.doesNotMatch(env.html(), /creating the workflow/i);
});
