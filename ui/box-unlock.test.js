import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const hubSrc = readFileSync(new URL("./hub.js", import.meta.url), "utf8");
const settingsSrc = readFileSync(new URL("./settings.js", import.meta.url), "utf8");
const LOCKED = "unlock with your password in the app to do this";
/** The hub calls only the owner makes on a locked login (`surface::owner_only`): the manifest's
 * confirm actions, read from the manifest itself. */
const manifest = readFileSync(new URL("../crates/core/src/surface.rs", import.meta.url), "utf8");
const CONFIRM = new Set([...manifest.matchAll(/\bh\("([a-z_]+)", Confirm\b/g)].map(([, cmd]) => cmd));
const here = (value) => JSON.parse(JSON.stringify(value));

function element() {
  return { addEventListener() {}, dataset: {}, hidden: false, innerHTML: "", textContent: "", classList: { contains() { return false }, add() {}, remove() {}, toggle() {} } };
}

/** The app signed in on a locked login, with a hub that answers LOCKED to a confirm call until the
 * user types the password, and `answers[cmd]` to the rest. */
function app(answers = {}) {
  const calls = [];
  const prompts = [];
  let unlocked = false;
  class WebSocket {
    static OPEN = 1;
    constructor() {
      this.readyState = 1;
      WebSocket.last = this;
    }
    send(text) {
      const frame = JSON.parse(text);
      if (frame.type !== "hub") return;
      const { cmd } = frame.call;
      const reply = (body) => this.onmessage({ data: JSON.stringify({ type: "reply", id: frame.id, ...body }) });
      if (cmd === "unlock") {
        unlocked = true;
        return reply({ ok: { token: "u-1", until: 1794300000 } });
      }
      calls.push(frame.call);
      if (CONFIRM.has(cmd) && !unlocked) return reply({ err: LOCKED });
      reply({ ok: answers[cmd] ?? null });
    }
    close() {}
  }
  const context = {
    console,
    WebSocket,
    setTimeout: (run) => run(),
    addEventListener() {},
    account: { email: "ana@acme.dev" },
    load() { return null; },
    save() {},
    listen() {},
    agentApproval: () => null,
    agentAsking: () => null,
    agentAskingHtml: () => "",
    askOwnerPassword: async (error) => (prompts.push(error), "correct horse"),
    document: { body: { dataset: {} }, getElementById() { return element(); }, querySelector() { return element(); }, querySelectorAll() { return []; } },
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(hubSrc, context);
  vm.runInContext(`${settingsSrc}
    var hub = new Hub("https://hub.example", "login", { locked: askOwnerPassword });
    globalThis.__card = upstashCardHtml;
    globalThis.__settings = settings;
    globalThis.__actions = actions;`, context);
  const ws = WebSocket.last;
  ws.onopen();
  ws.onmessage({ data: JSON.stringify({ type: "hello", tabs: {} }) });
  return { context, calls, prompts };
}

test("the manifest this test reads lists the confirm actions", () => {
  assert.ok(CONFIRM.has("set_box_access") && CONFIRM.has("set_box_claude_token") && CONFIRM.size > 20, [...CONFIRM].join(" "));
});

test("yes on the Upstash card's box question is one call, with no password prompt", async () => {
  const { context, calls, prompts } = app();
  context.__settings.upstash = { key_hint: "…a1b2", claude_hint: "…9f3c" };
  context.__settings.box_asks = [{ id: "ask1", connector: "session", subject: { issue: "sessions of mac tab-1" } }];
  assert.match(context.__card(), /Allow a box for sessions of mac tab-1\?[\s\S]*data-action="answer-box" data-id="ask1" data-allow="yes"/);
  await context.__actions["answer-box"]({ dataset: { id: "ask1", allow: "yes" } });
  assert.deepEqual(here(calls), [{ cmd: "answer_box", id: "ask1", allow: true }]);
  assert.deepEqual(prompts, [], "no password prompt");
  assert.deepEqual(here(context.__settings.box_asks), [], "the answered question leaves the card");
});

test("changing who may use a box still asks for the password on the same app", async () => {
  const { context, calls, prompts } = app();
  await context.hub.hub({ cmd: "set_box_access", access: { worker: { mode: "fresh" } } });
  assert.deepEqual(prompts, [""], "one password prompt");
  assert.deepEqual(here(calls).map((c) => c.cmd), ["set_box_access", "set_box_access"], "sent again once unlocked");
});
