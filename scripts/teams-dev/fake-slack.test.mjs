// Runs the fake Slack on a free port and checks what a dev stack relies on: a message posted into a
// thread reads back from conversations.replies, the sink records each call without its token, and
// a token that is not a dev token is refused.
//
//   node scripts/teams-dev/fake-slack.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const fake = path.join(path.dirname(fileURLToPath(import.meta.url)), "fake-slack");
const DEV = "xoxb-dev-test-1";
const DEPLOYS = "C0DEVDEPLO";

// Starts the fake on port 0 and resolves with the base URL it printed.
async function start(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "fake-slack-test-"));
  const log = path.join(dir, "calls.jsonl");
  const child = spawn(process.execPath, [fake, "--port", "0", "--log", log], { stdio: ["ignore", "pipe", "pipe"] });
  t.after(() => {
    child.kill();
    fs.rmSync(dir, { recursive: true, force: true });
  });
  let out = "";
  child.stdout.setEncoding("utf8");
  const base = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`no listening line, got: ${out}`)), 10_000);
    child.once("exit", (code) => reject(new Error(`fake-slack exited ${code}`)));
    child.stdout.on("data", (chunk) => {
      out += chunk;
      const match = out.match(/listening on (\S+)/);
      if (match) {
        clearTimeout(timer);
        resolve(match[1]);
      }
    });
  });
  return { base, log };
}

const call = (base, method, { token = DEV, query, json, form } = {}) => {
  const headers = { authorization: `Bearer ${token}` };
  const url = new URL(method, base);
  if (query) url.search = new URLSearchParams(query).toString();
  if (json) return fetch(url, { method: "POST", headers: { ...headers, "content-type": "application/json" }, body: JSON.stringify(json) }).then((r) => r.json());
  if (form) return fetch(url, { method: "POST", headers, body: new URLSearchParams(form) }).then((r) => r.json());
  return fetch(url, { headers }).then((r) => r.json());
};

const sink = (base) => fetch(new URL("/sink", base)).then((r) => r.json());

test("a message posted into a thread reads back from its replies and shows in the sink", async (t) => {
  const { base } = await start(t);
  const history = await call(base, "conversations.history", { query: { channel: DEPLOYS, limit: "50" } });
  assert.equal(history.ok, true);
  const parent = history.messages.find((m) => m.text.includes("export button"));
  assert.ok(parent, "the seeded thread is in the channel history");

  const posted = await call(base, "chat.postMessage", { json: { channel: DEPLOYS, thread_ts: parent.ts, text: "on it" } });
  assert.equal(posted.ok, true);
  assert.equal(posted.message.thread_ts, parent.ts);

  const replies = await call(base, "conversations.replies", { query: { channel: DEPLOYS, ts: parent.ts } });
  assert.equal(replies.ok, true);
  const last = replies.messages.at(-1);
  assert.equal(last.text, "on it");
  assert.equal(last.ts, posted.ts);
  assert.equal(last.user, "UDEVBOT0001");

  const calls = await sink(base);
  const entry = calls.find((c) => c.method === "chat.postMessage");
  assert.deepEqual(
    { ok: entry.ok, channel: entry.channel, text: entry.text, thread_ts: entry.thread_ts },
    { ok: true, channel: DEPLOYS, text: "on it", thread_ts: parent.ts },
  );
  assert.ok(!JSON.stringify(calls).includes("xoxb"), "the sink never holds a token");
});

test("form-encoded bodies are read as well as JSON", async (t) => {
  const { base } = await start(t);
  const posted = await call(base, "chat.postMessage", { form: { channel: "C0DEVGEN001", text: "form hello" } });
  assert.equal(posted.ok, true);
  const history = await call(base, "conversations.history", { query: { channel: "C0DEVGEN001" } });
  assert.equal(history.messages[0].text, "form hello");
});

test("a token that is not a dev token is refused and the refusal is recorded", async (t) => {
  const { base } = await start(t);
  const real = await call(base, "auth.test", { token: "xoxb-1234-real" });
  assert.deepEqual(real, { ok: false, error: "invalid_auth" });
  const none = await call(base, "conversations.list", { token: "" });
  assert.deepEqual(none, { ok: false, error: "invalid_auth" });
  const calls = await sink(base);
  assert.deepEqual(
    calls.map((c) => [c.method, c.ok, c.error]),
    [
      ["auth.test", false, "invalid_auth"],
      ["conversations.list", false, "invalid_auth"],
    ],
  );
  assert.ok(!JSON.stringify(calls).includes("xoxb"), "the sink never holds a token");
});

test("auth.test names a bot, which the client needs to identify it", async (t) => {
  const { base } = await start(t);
  const me = await call(base, "auth.test");
  assert.equal(me.ok, true);
  assert.match(me.bot_id, /^B/);
  assert.equal(me.user_id, "UDEVBOT0001");
});

test("a second reaction from the bot is already_reacted, as Slack says", async (t) => {
  const { base } = await start(t);
  const history = await call(base, "conversations.history", { query: { channel: DEPLOYS } });
  const ts = history.messages[0].ts;
  const react = () => call(base, "reactions.add", { json: { channel: DEPLOYS, timestamp: ts, name: "robot_face" } });
  assert.deepEqual(await react(), { ok: true });
  assert.deepEqual(await react(), { ok: false, error: "already_reacted" });
});

test("POST /sink/reset clears the sink and the log file", async (t) => {
  const { base, log } = await start(t);
  await call(base, "auth.test");
  assert.equal((await sink(base)).length, 1);
  assert.ok(fs.readFileSync(log, "utf8").trim().length > 0, "the log file holds the call");
  const reset = await fetch(new URL("/sink/reset", base), { method: "POST" }).then((r) => r.json());
  assert.deepEqual(reset, { ok: true });
  assert.deepEqual(await sink(base), []);
  assert.equal(fs.readFileSync(log, "utf8"), "");
});
