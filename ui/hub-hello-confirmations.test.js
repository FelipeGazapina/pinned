import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const mainSrc = read("main.js");
const surfaceSrc = read("agent-surface.js");
const needsYouSrc = read("gui-needs-you.js");
const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));
const fn = (src, name) => {
  const at = src.indexOf(`\nfunction ${name}(`);
  assert.ok(at >= 0, `missing ${name}`);
  return src.slice(at + 1, src.indexOf("\n}\n", at) + 2);
};
/** A declaration that runs over several lines, such as a const object, through its closing brace. */
const decl = (src, start) => {
  const from = src.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  const close = src.slice(from).search(/\n\}+;?\n/);
  return src.slice(from, from + close + src.slice(from + close).indexOf("\n", 1) + 1);
};
const flush = () => new Promise((resolve) => setImmediate(resolve));

const PENDING = {
  id: "d51d09f3-02e0-4aa1-b9ee-739582947506",
  at: 1_794_429_783,
  machine: "this-mac",
  machine_name: "MacBook",
  tab: "tab-1",
  title: "Remember how I like the release notes.",
  role: "worker",
  action: "set_taste",
  about: "Remembers a taste of yours.",
  args: { text: "Short release notes." },
};
/** The read answers with the ones that wait first, then the last decided ones. */
const DECIDED = { ...PENDING, id: "decided-1", decision: "approved", decided_at: 1_794_429_790 };

/** main.js's connectHub as the page runs it, with the app around it stubbed. The fake Hub keeps
 * every call the app makes and the listeners the hub would call back. */
function app() {
  const calls = [];
  const pushed = [];
  const answers = { get_settings: () => ({ connectors: [] }), confirmations: () => [PENDING, DECIDED] };
  const context = {
    console,
    TAURI: false,
    document: { dispatchEvent() {}, body: { dataset: {} } },
    account: null,
    hub: null,
    page: "sidebar",
    settingsOpen: () => false,
    forgetThreadWatch() {},
    showMachines() {},
    onHubSettings() {},
    showAgentConsent() {},
    marksHub: { hello: () => Promise.resolve() },
    // The real onAgentConfirmations, which writes what the hub lists, is all that a push would do.
    agentSurfaceChanged() {},
    agentReadTastes() {},
    Hub: class {
      constructor(url, token, listeners) {
        this.listeners = listeners;
        context.hubListeners = listeners;
      }
      hub(call) {
        calls.push(call);
        return Promise.resolve(answers[call.cmd]?.(call) ?? null);
      }
    },
  };
  vm.createContext(context);
  vm.runInContext(
    [
      line(surfaceSrc, "let agentConfirmations"),
      line(surfaceSrc, "let agentApproving"),
      line(surfaceSrc, "let agentShown"),
      fn(surfaceSrc, "onAgentConfirmations"),
      decl(needsYouSrc, "const NEEDS_YOU_KINDS"),
      line(needsYouSrc, "const needsYouDismissed"),
      fn(needsYouSrc, "needsYouCards"),
      fn(mainSrc, "connectHub"),
      "globalThis.__connectHub = connectHub;",
      "globalThis.__cards = needsYouCards;",
    ].join("\n"),
    context,
  );
  // The Needs you page reads the waiting confirmations, with nothing else waiting.
  Object.assign(context, {
    needsYouItems: () => [],
    tabs: [],
    nowS: () => PENDING.at + 60,
    agentActionTitle: (action) => ({ set_taste: "Save a taste" })[action] ?? action,
    agentMachineName: (id) => id,
    agentWords: (role) => role,
    agentQuoted: (c) => `“${c.title}”`,
  });
  return { context, calls, pushed, run: (code) => vm.runInContext(code, context) };
}

test("a hello frame asks for the waiting confirmations, and the answer draws their card with no push", async () => {
  const { context, calls, pushed, run } = app();
  run("__connectHub")({ hub: "wss://hub.test", user_token: "t", machine_id: "this-mac", account_id: "a-1" });
  const pushedTo = context.hubListeners.confirmations;
  context.hubListeners.confirmations = (items) => (pushed.push(items), pushedTo(items));
  context.hubListeners.hello({ account: { id: "a-1", email: "ada@example.test" }, machines: [], tabs: {} });
  assert.deepEqual(calls.map((c) => c.cmd), ["get_settings", "confirmations"], "every hello rebuilds what a lost push would leave out");
  await flush();
  assert.deepEqual(JSON.parse(JSON.stringify(run("agentConfirmations"))).map((c) => c.id), [PENDING.id], "only the ones that still wait go on the page");
  const cards = JSON.parse(JSON.stringify(run("__cards()")));
  assert.deepEqual(cards.map((c) => [c.kind, c.key, c.title]), [["agent", `agent:${PENDING.id}`, "Save a taste"]]);
  assert.equal(run("__cards()[0].confirmation.action"), "set_taste");
  assert.deepEqual(pushed, [], "no push reached this client");
});
