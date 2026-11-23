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
    visibleInterval: () => 0,
    console,
    addEventListener() {},
    setTimeout() {},
    TAURI: false,
    active: null,
    accountsOf: async () => [],
    LOCAL: "local",
    account: { email: "test@example.test" },
    hubUp: () => true,
    hub: {
      async hub(call) {
        calls.push(call);
        if (call.cmd === "linear_suggestions") return { labels: [], columns: [], teams: [] };
        if (LISTS.has(call.cmd)) return [];
        const got = reply(call);
        return call.cmd === "get_settings" && !got?.connectors ? context.__settings() : got;
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
    `${settingsSrc}\nglobalThis.__actions = actions; globalThis.__steps = STEPS; globalThis.__tidy = tidy; globalThis.__state = () => ({ page, view }); globalThis.__setSettings = (next) => { settings = next; }; globalThis.__settings = () => settings;`,
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

test("with Linear connected, every Linear option opens a workflow on that same connector", async () => {
  for (const kind of ["linear-label", "linear-status", "linear-assigned"]) {
    const env = load({ linear: LINEAR });
    await openPicker(env);
    assert.equal(env.option(kind).action, "pick-trigger", kind);
    assert.doesNotMatch(env.option(kind).text, /Connect .* first/);
    await env.click(kind);
    assert.equal(env.state().view.name, "edit", kind);
    assert.equal(env.state().view.draft.trigger.kind, kind);
    assert.equal(env.state().view.draft.graph.nodes[0].watch.kind, kind);
  }
});

test("a linear status draft keeps its columns on the graph the hub stores", async () => {
  const env = load({ linear: LINEAR });
  await openPicker(env);
  await env.click("linear-status");
  const draft = env.state().view.draft;
  draft.trigger.from = "Todo";
  draft.trigger.to = " Ready to Dev ";
  draft.trigger.scope = { team: " ENG ", project: "" };
  draft.trigger.by = "not-me";
  const saved = env.context.__tidy(draft);
  // tidy runs in the settings vm, so compare the JSON, not the vm's objects.
  const plain = (value) => JSON.parse(JSON.stringify(value));
  assert.deepEqual(plain(saved.trigger), {
    kind: "linear-status",
    from: "Todo",
    to: "Ready to Dev",
    scope: { team: "ENG", project: "" },
    by: "not-me",
  });
  assert.deepEqual(plain(saved.graph.nodes[0].watch), plain(saved.trigger));
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

test("Linear connected elsewhere while the note is up: the note says so and Back opens the new workflow", async () => {
  const env = load({});
  await openPicker(env);
  await env.click("linear-label");
  assert.match(env.context.pendingWorkflowHtml(), /Connect Linear below/);
  env.context.onHubSettings({ ...env.context.__settings(), linear: LINEAR });
  assert.match(env.context.pendingWorkflowHtml(), /Linear is connected now/);
  await env.context.__actions["resume-workflow"]();
  assert.equal(env.state().page, "connectors");
  assert.equal(env.state().view.draft.trigger.kind, "linear-label");
});

test("Back to the workflow reads settings again, so Linear connected in another window counts", async () => {
  let linear = null;
  const env = load({ reply: (call) => (call.cmd === "get_settings" ? { linear, slack: null, slack_bots: [], connectors: [], checked: null } : {}) });
  await openPicker(env);
  await env.click("linear-label");
  linear = LINEAR;
  await env.context.__actions["resume-workflow"]();
  assert.equal(env.state().page, "connectors");
  assert.equal(env.state().view.name, "edit");
  assert.equal(env.state().view.draft.trigger.kind, "linear-label");
});

test("Back to the workflow while Linear is still missing returns to the picker, which still says why", async () => {
  const env = load({});
  await openPicker(env);
  await env.click("linear-label");
  await env.context.__actions["resume-workflow"]();
  assert.equal(env.state().page, "connectors");
  assert.equal(env.state().view.name, "pick");
  assert.match(env.option("linear-label").text, /Connect Linear under Connectors first/);
});

const POSTHOG = { host: "https://us.posthog.com", project: "12345", project_name: "Web app", key_hint: "…k3y9" };

test("the picker lists the PostHog errors workflow, and without PostHog its detour names PostHog and comes back", async () => {
  let posthog = null;
  const env = load({
    linear: LINEAR,
    reply: (call) => {
      if (call.cmd === "connect_posthog") return (posthog = POSTHOG);
      if (call.cmd === "get_settings") return { linear: LINEAR, slack: null, slack_bots: [], posthog, connectors: [], checked: null };
      return {};
    },
  });
  await openPicker(env);
  assert.match(env.option("observability").text, /An error shows up in PostHog/);
  assert.equal(env.option("observability").action, "connect-for-trigger");
  assert.match(env.option("observability").text, /Connect PostHog under Connectors first/);
  await env.click("observability");
  assert.equal(env.state().page, "integrations");
  assert.match(env.html(), /An error shows up in PostHog/);
  assert.match(env.html(), /Connect PostHog below/);
  const byId = env.context.document.getElementById;
  byId("posthog-key").value = "phx_fake";
  byId("posthog-project").value = "12345";
  await env.context.__actions["connect-posthog"]();
  const sent = env.calls.find((call) => call.cmd === "connect_posthog");
  assert.deepEqual({ ...sent }, { cmd: "connect_posthog", key: "phx_fake", project: "12345", host: "us" });
  assert.equal(env.state().page, "connectors");
  assert.equal(env.state().view.name, "edit");
  assert.equal(env.state().view.draft.trigger.kind, "observability");
});

test("with PostHog connected, the new workflow watches PostHog errors, and its panels show the once-per-error hint and the ticket's team and label", async () => {
  const env = load({ linear: LINEAR });
  env.context.__setSettings({ ...env.context.__settings(), posthog: POSTHOG });
  await openPicker(env);
  assert.equal(env.option("observability").action, "pick-trigger");
  await env.click("observability");
  const draft = env.state().view.draft;
  const plain = (value) => JSON.parse(JSON.stringify(value));
  assert.deepEqual(plain(draft.trigger), { kind: "observability", source: { kind: "posthog" } });
  assert.deepEqual(plain(draft.graph.nodes[0].watch), { kind: "observability", source: { kind: "posthog" } });
  assert.match(draft.instructions, /pinned linear create/);
  draft.team = " ENG ";
  const saved = env.context.__tidy(draft);
  assert.deepEqual(plain(saved.trigger), { kind: "observability", source: { kind: "posthog" } }, "the source is saved");
  assert.equal(saved.team, "ENG");
  const trigger = env.context.__steps["observability-trigger"].fields(draft);
  assert.match(trigger, /every minute, only for errors first seen after the workflow is saved or turned on, and starts each error once/);
  assert.match(trigger, /Web app/);
  const ticket = env.context.__steps.ticket.fields(draft);
  assert.match(ticket, /Team <input data-bind="team"/);
  assert.match(ticket, /Label <input data-bind="label"/);
  assert.match(ticket, /at most one ticket per error with <code>pinned linear create<\/code>/);
  Object.assign(env.context, { machineName: () => "this Mac", accountOf: () => null, onlineMachines: () => [] });
  const session = env.context.sessionFields(draft);
  assert.match(session, /File a Linear ticket/);
  assert.doesNotMatch(session, /If it reports a bug|Issue columns/);
});

test("the ticket step says to connect Linear when Linear is missing", () => {
  const env = load({});
  const draft = env.context.blank("observability");
  assert.match(env.context.__steps.ticket.fields(draft), /Connect Linear under Connectors\./);
});

test("under an observability trigger, steps that need a thread or an issue say so", () => {
  const env = load({});
  const draft = env.context.blank("observability");
  draft.graph = env.context.PinnedGraph.startingGraph(draft);
  for (const type of ["reply", "page", "file-issue", "take-issue", "move-column", "stage-picked"]) {
    const issues = env.context.graphNodeIssues({ id: "n", type }, draft);
    assert.ok(issues.includes("This step needs a Slack thread or a Linear issue, which an observability trigger does not give it."), `${type}: ${issues}`);
  }
});

test("the PostHog card: not connected it asks for host, project and key; connected it shows the project and key hint, never the key", () => {
  const env = load({});
  const empty = env.context.posthogCardHtml();
  for (const want of [/US cloud/, /EU cloud/, /Self-hosted/, /id="posthog-host"/, /id="posthog-project"/, /id="posthog-key" type="password"/, /data-action="connect-posthog"/, /id="posthog-error"/]) {
    assert.match(empty, want);
  }
  assert.match(empty, /Not connected/);
  env.context.__setSettings({ ...env.context.__settings(), posthog: POSTHOG });
  const connected = env.context.posthogCardHtml();
  assert.match(connected, /Web app/);
  assert.match(connected, /12345/);
  assert.match(connected, /US cloud/);
  assert.match(connected, /…k3y9/);
  assert.match(connected, /data-action="disconnect-posthog"/);
  assert.doesNotMatch(connected, /posthog-key|phx_/, "no key field and no key");
});

test("Connect on the PostHog card names the first empty field and sends nothing", async () => {
  const env = load({});
  const byId = env.context.document.getElementById;
  const focused = [];
  for (const id of ["posthog-host", "posthog-project", "posthog-key"]) byId(id).focus = () => focused.push(id);
  await env.context.__actions["connect-posthog"]();
  assert.equal(byId("posthog-error").textContent, "Enter the project id.");
  byId("posthog-project").value = "12345";
  await env.context.__actions["connect-posthog"]();
  assert.equal(byId("posthog-error").textContent, "Paste the key.");
  assert.deepEqual(focused, ["posthog-project", "posthog-key"]);
  assert.ok(!env.calls.some((call) => call.cmd === "connect_posthog"), "nothing reaches the hub");
  const card = env.context.posthogCardHtml();
  for (const label of ["PostHog host address", "PostHog project id", "PostHog personal API key"]) assert.match(card, new RegExp(`aria-label="${label}"`));
});
