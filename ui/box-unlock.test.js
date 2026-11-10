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
function app(answers = {}, world = {}) {
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
    ...world,
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(hubSrc, context);
  vm.runInContext(`${settingsSrc}
    var hub = new Hub("https://hub.example", "login", { locked: askOwnerPassword });
    globalThis.__card = upstashCardHtml;
    globalThis.__settings = settings;
    globalThis.__actions = actions;
    globalThis.__loadBoxLogins = loadBoxLogins;`, context);
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

/** This Mac and a Mac mini, online, with the accounts each machine's `list_accounts` answers. */
function twoMachines() {
  const listed = {
    local: [
      { id: "claude", provider: "claude", auth: "login", signed_in: true, email: "felipe@home.dev", plan: "max" },
      { id: "codex", provider: "codex", auth: "login", signed_in: true },
      { id: "go", provider: "claude", auth: "opencode-go", signed_in: true, plan: "OpenCode Go" },
    ],
    "mini-id": [
      { id: "claude", provider: "claude", auth: "login", signed_in: true, email: "felipe@home.dev", plan: "max" },
      { id: "bloom", provider: "claude", auth: "login", signed_in: true, email: "felipe@bloom.dev", plan: "team", org: "Bloom" },
      { id: "spare", provider: "claude", auth: "login", signed_in: false },
    ],
  };
  const machines = new Map([
    ["local", { view: { name: "MacBook", online: true } }],
    ["mini-id", { view: { name: "Mac mini", online: true } }],
    ["box-id", { view: { name: "Box · site", online: true, upstash_box: "b1" } }],
  ]);
  return {
    LOCAL: "local",
    machines,
    account: { email: "ana@acme.dev", machine_id: "mac-id" },
    onlineMachines: () => [...machines.entries()].filter(([, m]) => m.view.online).map(([id]) => id),
    machineName: (id) => (id === "local" ? "This Mac" : machines.get(id).view.name),
    accountsOf: async (id) => listed[id] ?? [],
  };
}

test("the Upstash card lists each Claude account signed in on a machine with its email and plan", async () => {
  const { context } = app({}, twoMachines());
  context.__settings.upstash = { key_hint: "…a1b2", claude_hint: "…9f3c" };
  context.__settings.box_claude_tokens = { bloom: "…oken" };
  await context.__loadBoxLogins();
  const rows = [...context.__card().matchAll(/<div class="row box-login"><span>([\s\S]*?)<\/span>/g)].map(([, row]) => row.replace(/<\/?b>/g, ""));
  assert.deepEqual(rows, ["claude felipe@home.dev · max · on This Mac", "bloom felipe@bloom.dev · team · Bloom · on Mac mini"]);
  const card = context.__card();
  assert.match(card, /data-action="use-for-boxes" data-account="claude" data-machine="mac-id">Use for boxes</);
  assert.match(card, /<span class="hint">Used for boxes<\/span>\s*<button class="secondary" data-action="use-for-boxes" data-account="bloom" data-machine="mini-id">Renew</);
});

test("Use for boxes needs the password on a locked app, then keeps the token the machine mints", async () => {
  const { context, calls, prompts } = app({ mint_box_claude_token: { bloom: "…oken" } }, twoMachines());
  context.__settings.upstash = { key_hint: "…a1b2", claude_hint: "…9f3c" };
  await context.__loadBoxLogins();
  await context.__actions["use-for-boxes"]({ dataset: { account: "bloom", machine: "mini-id" } });
  assert.deepEqual(prompts, [""], "one password prompt");
  assert.deepEqual(here(calls), [
    { cmd: "mint_box_claude_token", machine: "mini-id", account: "bloom" },
    { cmd: "mint_box_claude_token", machine: "mini-id", account: "bloom" },
  ]);
  assert.deepEqual(here(context.__settings.box_claude_tokens), { bloom: "…oken" }, "the app gets the hint, never the token");
  assert.match(context.__card(), /data-account="bloom" data-machine="mini-id">Renew</);
});
