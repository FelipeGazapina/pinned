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
