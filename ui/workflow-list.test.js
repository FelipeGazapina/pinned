import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./settings.js", import.meta.url), "utf8");

// The Workflows list renders every saved connector through cardHtml; one connector the app
// doesn't know must never blank the page, or its switch can't be reached to turn it off.
function listWorld(connectors) {
  const start = src.indexOf("const TRIGGERS = {");
  const end = src.indexOf("\nfunction triggerChoiceHtml()");
  const context = {
    console,
    document: { getElementById: () => null },
    account: null,
    machines: new Map(),
    LOCAL: "local",
    onlineMachines: () => [],
    machineName: () => "?",
    accountName: () => null,
    load: () => null,
    save: () => {},
  };
  vm.createContext(context);
  // checkedText lives further down settings.js and is unrelated to the cards under test.
  vm.runInContext(`${src.slice(start, end)};function checkedText() { return ""; };settings = { linear: {}, slack: null, connectors: ${JSON.stringify(connectors)}, checked: null };`, context);
  return context;
}

const slackMessage = () => ({
  id: "c1",
  name: "Triage and routing back to Slack",
  enabled: true,
  trigger: { kind: "slack-message", channel: "ops-product-triage", decide: { model: "", instructions: "", categories: [], fallback: null } },
  folder: "/repo",
  machine: "",
  columns: {},
});

test("a slack-message workflow renders its card with a sort step", () => {
  const context = listWorld([slackMessage()]);
  const html = context.connectorsHtml();
  assert.match(html, /Triage and routing back to Slack/);
  assert.match(html, /Sort message/, "the decide step draws its icon in the chain");
  assert.match(html, /data-toggle="enabled"/, "the switch stays reachable");
});

test("an unknown trigger still renders a card instead of blanking the page", () => {
  const context = listWorld([{ id: "c9", name: "Future thing", enabled: true, trigger: { kind: "cron" }, folder: "/repo", machine: "", columns: {} }]);
  const html = context.connectorsHtml();
  assert.match(html, /Future thing/);
  assert.match(html, /repair its trigger/);
  assert.match(html, /data-toggle="enabled"/);
});

test("generation errors tell what to change before retrying", () => {
  const start = src.indexOf("/** Backend generation errors");
  const end = src.indexOf("const actions = {");
  const context = { console };
  vm.createContext(context);
  vm.runInContext(src.slice(start, end), context);
  assert.match(context.generateErrorHint('step "Message" has no usable trigger'), /when mentioned in #support/);
  assert.match(context.generateErrorHint("the model did not answer with a workflow map"), /Simplify/);
  assert.match(context.generateErrorHint("claude did not answer in time"), /fewer steps/);
  assert.match(context.generateErrorHint("this machine runs pinned 0.18.1, which can't answer that: unknown variant `draft_workflow`"), /Update pinned there/);
  assert.match(context.generateErrorHint("claude: Not logged in · Please run /login"), /Sign in that Claude account/);
  assert.equal(context.generateErrorHint("boom"), "boom");
});

function generateWorld({ machines = [], local = "local" } = {}) {
  const start = src.indexOf("/** Backend generation errors");
  const end = src.indexOf("const actions = {");
  const known = new Map(machines.map((m) => [m.id, { view: { online: m.online ?? true, upstash_box: m.box ?? false, name: m.name ?? m.id } }]));
  const context = {
    console,
    LOCAL: local,
    machines: known,
    onlineMachines: () => [...known.entries()].filter(([, m]) => m.view.online).map(([id]) => id),
    machineName: (id) => (id === local ? "This computer" : known.get(id)?.view.name ?? "machine"),
  };
  vm.createContext(context);
  vm.runInContext(`const esc = (s) => String(s);${src.slice(start, end)}`, context);
  return context;
}

const work = { id: "work", provider: "claude", name: "Work", auth: "login", signed_in: true };

test("the generate dialog offers signed-in Claude Code accounts first and the gateway's models after", () => {
  const context = generateWorld({ machines: [{ id: "local" }] });
  const accounts = [
    work,
    { id: "out", provider: "claude", name: "Signed out", auth: "login", signed_in: false },
    { id: "go", provider: "claude", name: "Go", auth: "opencode-go", signed_in: true },
    { id: "codex", provider: "codex", name: "Codex", signed_in: true },
  ];
  const options = context.generateModelOptions(context.draftMachine(), accounts, [{ id: "openai/gpt-5" }]);
  assert.deepEqual(JSON.parse(JSON.stringify(options)), [
    { value: "claude:work", label: "Claude · Work" },
    { value: "claude:go", label: "Claude · Go" },
    { value: "vercel:openai/gpt-5", label: "openai/gpt-5" },
  ]);
});

test("a Claude draft names the remote machine it runs on, never a box", () => {
  const context = generateWorld({ machines: [{ id: "local", online: false }, { id: "box", box: true }, { id: "mb", name: "Studio" }] });
  assert.equal(context.draftMachine(), "mb");
  assert.equal(context.generateModelOptions("mb", [work], [])[0].label, "Claude · Work on Studio");
});

test("with no Claude login and no gateway the dialog says what to connect", () => {
  const context = generateWorld();
  assert.equal(context.draftMachine(), undefined);
  const options = JSON.parse(JSON.stringify(context.generateModelOptions(undefined, [work], [])));
  assert.deepEqual(options, [{ value: "", label: "Sign in a Claude account on a machine or connect the Vercel AI Gateway", disabled: true }]);
});

test("a call step may run a workflow whose trigger answers threads", () => {
  const context = generateWorld();
  const graphed = (kind) => ({ graph: { nodes: [{ type: "trigger", watch: { kind } }] }, trigger: { kind: "linear-label" } });
  assert.equal(context.answersThreads(graphed("called")), true);
  assert.equal(context.answersThreads(graphed("slack-mention")), true);
  assert.equal(context.answersThreads(graphed("slack-message")), false);
  assert.equal(context.answersThreads({ trigger: { kind: "called" } }), true, "a workflow saved before maps reads its trigger");
});

test("a connector without a trigger still renders a card", () => {
  const context = listWorld([{ id: "c0", name: "Old thing", enabled: false, folder: "/repo", machine: "", columns: {} }]);
  const html = context.connectorsHtml();
  assert.match(html, /Old thing/);
  assert.match(html, /data-toggle="enabled"/);
});

const mention = () => ({
  id: "c2",
  name: "Nightly checks",
  enabled: true,
  trigger: { kind: "linear-label", label: "nightly" },
  folder: "/repo",
  machine: "",
  model: "",
  effort: "high",
  permissionMode: "auto",
  worktree: false,
  instructions: "Check the build.",
  fix: "off",
  proof: false,
  columns: { picked: "", coding: "", review: "" },
});

// The card of `connector` after the workflow's last Run now went as `last`, if any.
function runNowCard(connector, last) {
  const context = listWorld([connector]);
  if (last) vm.runInContext(`runNow.set(${JSON.stringify(connector.id)}, ${JSON.stringify(last)})`, context);
  return context.connectorsHtml();
}

test("a workflow's card offers Run now, with nothing under it before a run", () => {
  const html = runNowCard(mention());
  assert.match(html, /<button class="secondary run-now" data-action="run-now"[^>]*>Run now<\/button>/);
  assert.doesNotMatch(html, /run-now-outcome/);
});

test("Run now turns into a disabled Starting… while the run starts, so it can't start twice", () => {
  const html = runNowCard(mention(), { state: "running" });
  assert.match(html, /<button class="secondary run-now" disabled aria-busy="true">Starting…<\/button>/);
  assert.doesNotMatch(html, /data-action="run-now"/);
});

test("a started run names its machine and opens its session", () => {
  const html = runNowCard(mention(), { state: "started", workflow: "c2", machine: "m-1", machine_name: "Mac mini", tab: "tab-9" });
  assert.match(html, /Started on Mac mini\. <button class="link" data-action="run-now-open" data-machine="m-1" data-tab="tab-9">Open session<\/button>/);
  assert.match(html, /data-action="run-now"/, "it can run again");
});

test("a run the limits refused says why in plain words", () => {
  const error = "1 worker already runs in this project, the most it allows at once. Wait for one to finish, or raise Workers at once in Settings › Way of working.";
  const html = runNowCard(mention(), { state: "refused", error });
  assert.match(html, /<p class="run-now-outcome refused" role="alert">Didn't start: 1 worker already runs in this project, the most it allows at once\./);
  assert.ok(runNowCard(mention(), { state: "refused", error: "<b>x</b>" }).includes("Didn't start: &#60;b&#62;x&#60;/b&#62;"), "the error is text, not markup");
});

test("a workflow with no session of its own shows Run now disabled with the reason", () => {
  const html = runNowCard(slackMessage());
  assert.match(html, /<button class="secondary run-now" disabled title="It sorts Slack messages into other workflows and has no session of its own\.">Run now<\/button>/);
  const called = listWorld([]).runNowHtml({ ...mention(), trigger: { kind: "called" } });
  assert.equal(called.button, '<button class="secondary run-now" disabled title="It answers only the Slack threads another workflow hands it.">Run now</button>');
});

async function generateClick({ value, fail }) {
  const helpers = src.slice(src.indexOf("/** Backend generation errors"), src.indexOf("const actions = {"));
  const action = src.slice(src.indexOf('  "generate-workflow"() {'), src.indexOf('  "connect-for-trigger"(button) {'));
  const els = {};
  const el = (id) => (els[id] ??= { value: "", textContent: "", dataset: {}, isConnected: true, innerHTML: "" });
  const button = { disabled: false, textContent: "Generate", closest: () => button };
  const calls = [];
  const context = {
    console,
    LOCAL: "local",
    machines: new Map([["local", { view: { online: true }, accounts: [work] }]]),
    onlineMachines: () => ["local"],
    machineName: () => "This computer",
    esc: String,
    suggestions: { models: [{ id: "openai/gpt-5" }] },
    settings: { connectors: [{ id: "fixer", name: "Fix", trigger: { kind: "called" } }] },
    showDialog: (html) => (el("generate-model").html = html),
    shellDialog: { dataset: {}, querySelector: () => button },
    document: { getElementById: el },
    accountsOf: async () => [],
    callOn: async (machine, call) => {
      calls.push([machine, call]);
      if (fail) throw new Error(fail);
      return { nodes: [{ id: "t", type: "trigger", watch: { kind: "called" } }] };
    },
    connectorCall: async (cmd, args) => (calls.push([cmd, args]), { nodes: [] }),
    blank: (kind) => ({ kind }),
    closeDialog: () => calls.push("closed"),
    edit: () => {},
    redraw: () => {},
  };
  vm.createContext(context);
  vm.runInContext(`${helpers};var actions = {${action}};`, context);
  context.actions["generate-workflow"]();
  el("generate-model").value = value;
  el("generate-text").value = "Triage";
  await button.onclick({ target: button });
  return { html: el("generate-model").html, calls: JSON.parse(JSON.stringify(calls)), button, error: el("generate-error").textContent };
}

test("generating with a Claude login drafts on the machine and the gateway path is unchanged", async () => {
  const claude = await generateClick({ value: "claude:work" });
  assert.match(claude.html, /<option value="claude:work">Claude · Work<\/option><option value="vercel:openai\/gpt-5">/);
  assert.deepEqual(claude.calls, [["local", { cmd: "draft_workflow", account: "work", description: "Triage", workflows: [{ id: "fixer", name: "Fix", answers_threads: true }] }], "closed"]);
  const vercel = await generateClick({ value: "vercel:openai/gpt-5" });
  assert.deepEqual(vercel.calls[0], ["generate_workflow", { model: "openai/gpt-5", description: "Triage" }]);
});

test("a failed generation gives the button back with a hint", async () => {
  const failed = await generateClick({ value: "claude:work", fail: "claude did not answer in time" });
  assert.equal(failed.button.disabled, false);
  assert.equal(failed.button.textContent, "Generate");
  assert.match(failed.error, /fewer steps/);
});
