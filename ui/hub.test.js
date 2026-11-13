import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const hubSrc = readFileSync(new URL("./hub.js", import.meta.url), "utf8");
const LOCKED = "unlock with your password in the app to do this";
const flush = () => new Promise((resolve) => setImmediate(resolve));
// Values built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => JSON.parse(JSON.stringify(value));

/** hub.js signed in with login "login", on sockets the test plays the hub on. `passwords` are what
 * the user types at each prompt, null for Cancel. */
function app(passwords) {
  const sockets = [];
  const asked = [];
  class WebSocket {
    static OPEN = 1;
    constructor() {
      this.readyState = 1;
      this.sent = [];
      sockets.push(this);
    }
    send(text) {
      this.sent.push(JSON.parse(text));
    }
    close() {}
  }
  const locked = passwords && (async (error) => (asked.push(error), passwords.shift()));
  const context = { WebSocket, setTimeout: (run) => run(), locked };
  vm.createContext(context);
  vm.runInContext(`${hubSrc}\nthis.hub = new Hub("https://hub.example", "login", locked ? { locked } : {});`, context);
  const open = () => {
    const ws = sockets.at(-1);
    ws.onopen();
    ws.onmessage({ data: JSON.stringify({ type: "hello", tabs: {} }) });
    return ws;
  };
  const answer = (ws, reply) => ws.onmessage({ data: JSON.stringify({ type: "reply", ...reply }) });
  return { hub: context.hub, sockets, asked, open, answer };
}

test("the app locks its login, unlocks with the password on a locked call, and sends the call again", async () => {
  const { hub, asked, open, answer } = app(["correct horse"]);
  const ws = open();
  assert.deepEqual(ws.sent, [{ type: "auth", token: "login", lock: true }]);
  const removed = hub.hub({ cmd: "remove_machine", id: "imac" });
  answer(ws, { id: 1, err: LOCKED });
  await flush();
  assert.deepEqual(here(asked), [""]);
  assert.deepEqual(ws.sent.at(-1), { type: "hub", id: 2, call: { cmd: "unlock", password: "correct horse" } });
  answer(ws, { id: 2, ok: { token: "u-1", until: 1794300000 } });
  await flush();
  assert.deepEqual(ws.sent.at(-1), { type: "hub", id: 3, call: { cmd: "remove_machine", id: "imac" } });
  answer(ws, { id: 3, ok: null });
  assert.equal(await removed, null);
});

test("after a reconnect the app sends the unlock it holds with its login", async () => {
  const { hub, sockets, open, answer } = app(["correct horse"]);
  const ws = open();
  const decided = hub.hub({ cmd: "decide_confirmation", id: "c-1", approve: true });
  answer(ws, { id: 1, err: LOCKED });
  await flush();
  answer(ws, { id: 2, ok: { token: "u-1", until: 1794300000 } });
  await flush();
  answer(ws, { id: 3, ok: { decision: "approved", outcome: "done" } });
  await decided;
  ws.onclose({ code: 1006 });
  assert.equal(sockets.length, 2);
  assert.deepEqual(open().sent, [{ type: "auth", token: "login", lock: true, unlock: "u-1" }]);
});

test("a wrong password asks again with the hub's words, and Cancel fails the call", async () => {
  const { hub, asked, open, answer } = app(["nope", null]);
  const ws = open();
  const removed = assert.rejects(hub.hub({ cmd: "remove_machine", id: "imac" }), (e) => e === "this needs your password");
  answer(ws, { id: 1, err: LOCKED });
  await flush();
  answer(ws, { id: 2, err: "that is not this account's password" });
  await flush();
  assert.deepEqual(here(asked), ["", "that is not this account's password"]);
  await removed;
  assert.equal(ws.sent.length, 3, "nothing is sent again");
});

test("calls held up together ask for the password once", async () => {
  const { hub, asked, open, answer } = app(["correct horse"]);
  const ws = open();
  const updates = [hub.call("mini", { cmd: "update_self" }), hub.call("imac", { cmd: "update_self" })];
  answer(ws, { id: 1, err: LOCKED });
  answer(ws, { id: 2, err: LOCKED });
  await flush();
  assert.equal(asked.length, 1);
  answer(ws, { id: 3, ok: { token: "u-1", until: 1794300000 } });
  await flush();
  assert.deepEqual(here(ws.sent.slice(-2).map((f) => [f.machine, f.call.cmd])), [["mini", "update_self"], ["imac", "update_self"]]);
  answer(ws, { id: 4, ok: null });
  answer(ws, { id: 5, ok: null });
  assert.deepEqual(here(await Promise.all(updates)), [null, null]);
});

test("an app with no password prompt passes the refusal on", async () => {
  const { hub, open, answer } = app(null);
  const ws = open();
  const removed = hub.hub({ cmd: "remove_machine", id: "imac" });
  answer(ws, { id: 1, err: LOCKED });
  await assert.rejects(removed, (e) => e === LOCKED);
});

/** Plays `frames` to the hub.js in `source`, signed in on a socket the test plays the hub on. */
function appOf(source, on = {}) {
  const sockets = [];
  class WebSocket {
    static OPEN = 1;
    constructor() {
      this.readyState = 1;
      this.sent = [];
      this.closed = false;
      sockets.push(this);
    }
    send(text) {
      this.sent.push(JSON.parse(text));
    }
    close() {
      this.closed = true;
    }
  }
  const context = { WebSocket, setTimeout: () => 0, on };
  vm.createContext(context);
  vm.runInContext(`${source}\nthis.hub = new Hub("https://hub.example", "login", on);`, context);
  const ws = sockets.at(-1);
  ws.onopen();
  const play = (frame) => ws.onmessage({ data: JSON.stringify(frame) });
  play({ type: "hello", tabs: {} });
  return { hub: context.hub, ws, play, sockets };
}

test("an ideas_changed frame tells the app once, and carries nothing", () => {
  const told = [];
  const { play } = appOf(hubSrc, { ideasChanged: (...args) => told.push(args) });
  play({ type: "ideas_changed" });
  play({ type: "ideas_changed" });
  assert.deepEqual(told, [[], []]);
});

test("an app of v0.18.1 or v0.17.x drops an ideas_changed frame and keeps its connection", async () => {
  const old = readFileSync(new URL("./fixtures/hub-v0.18.1.js", import.meta.url), "utf8");
  assert.doesNotMatch(old, /ideas_changed/, "that release has never heard of it");
  const heard = [];
  const { hub, ws, play, sockets } = appOf(old, { settings: (s) => heard.push(s), machines: () => heard.push("machines") });
  play({ type: "ideas_changed" });
  play({ type: "settings", settings: { connectors: [] } });
  assert.deepEqual(here(heard), [{ connectors: [] }], "it goes on to the frames it knows");
  assert.equal(ws.closed, false, "the socket stays open");
  assert.equal(sockets.length, 1, "and it does not reconnect");
  const call = hub.hub({ cmd: "list_ideas" });
  play({ type: "reply", id: 1, ok: [] });
  assert.deepEqual(here(await call), [], "a call still gets its answer after the frame");
});
