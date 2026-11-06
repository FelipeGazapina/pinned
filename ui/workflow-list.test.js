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
