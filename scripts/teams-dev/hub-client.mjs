// A small client for the dev hub, shared by seed.mjs and smoke.mjs (Node 22: global fetch and WebSocket).
// Nothing here prints a password, a token or a key.
import fs from "node:fs";
import path from "node:path";

export const HUB = process.env.TEAMS_DEV_HUB || "http://127.0.0.1:8811";
export const DIR = process.env.TEAMS_DEV_DIR;
export const ACCOUNTS = DIR && path.join(DIR, "accounts.json");

/** accounts.json: { "ana@example.test": { password, role }, ... }, mode 600. */
export function readAccounts() {
  try {
    return JSON.parse(fs.readFileSync(ACCOUNTS, "utf8"));
  } catch {
    return {};
  }
}

export function writeAccounts(accounts) {
  fs.writeFileSync(ACCOUNTS, JSON.stringify(accounts, null, 2), { mode: 0o600 });
  fs.chmodSync(ACCOUNTS, 0o600);
}

/** POST `body` as JSON to the hub's `route`; the parsed answer, or an Error carrying the status. */
export async function post(route, body, token) {
  const res = await fetch(HUB + route, { method: "POST", headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) }, body: JSON.stringify(body) });
  const text = await res.text();
  if (!res.ok) throw Object.assign(new Error(`${route} answered ${res.status}: ${text.slice(0, 200)}`), { status: res.status });
  return text ? JSON.parse(text) : {};
}

/** The session token of `email`, who must be in accounts.json. */
export async function signIn(email) {
  const account = readAccounts()[email];
  if (!account) throw new Error(`${email} is not in ${ACCOUNTS}`);
  return (await post("/api/login", { email, password: account.password })).token;
}

/** One authenticated /ws/client connection: `call({cmd, ...})` sends a HubCall and resolves its `ok`. */
export async function connect(token) {
  const ws = new WebSocket(HUB.replace(/^http/, "ws") + "/ws/client");
  const waiting = new Map();
  const events = [];
  let next = 0;
  const hello = new Promise((resolve, reject) => {
    ws.addEventListener("error", () => reject(new Error("the hub's /ws/client refused the connection")));
    ws.addEventListener("close", () => reject(new Error("the hub closed the connection")));
    ws.addEventListener("open", () => ws.send(JSON.stringify({ type: "auth", token })));
    ws.addEventListener("message", (m) => {
      const frame = JSON.parse(m.data);
      if (frame.type === "hello") resolve(frame);
      else if (frame.type === "reply") {
        const slot = waiting.get(frame.id);
        waiting.delete(frame.id);
        if (slot) "err" in frame ? slot.reject(new Error(frame.err)) : slot.resolve(frame.ok);
      } else events.push(frame);
    });
  });
  await hello;
  return {
    events,
    call: (call, wait = 120_000) =>
      new Promise((resolve, reject) => {
        const id = ++next;
        const timer = setTimeout(() => reject(new Error(`${call.cmd} got no answer in ${wait / 1000}s`)), wait);
        waiting.set(id, { resolve: (v) => (clearTimeout(timer), resolve(v)), reject: (e) => (clearTimeout(timer), reject(e)) });
        ws.send(JSON.stringify({ type: "hub", id, call }));
      }),
    close: () => ws.close(),
  };
}
