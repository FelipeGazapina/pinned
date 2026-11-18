// A fake hub with one machine running a busy Orchestrator: the global orchestrator, 8 Captains, 30
// workers and 41 finished threads. It answers the page's calls and sends the frames a real hub
// forwards while agents work, at the per-client rates of the busiest 5-minute census intervals the
// production hub logged on 2026-11-18 (see README.md). Everything comes from a seeded generator, so
// two runs send the same frames in the same order.

const crypto = require("crypto");

const MACHINE = "m-studio";
const PROJECTS = ["web-shop", "billing-api", "mobile-app", "docs-site", "data-pipeline", "auth-service", "search", "infra"];
const VERBS = ["Fix", "Port", "Add", "Remove", "Speed up", "Test", "Refactor", "Document"];
const NOUNS = ["the checkout flow", "invoice export", "push notifications", "the sitemap", "the nightly import", "token refresh", "the ranking job", "the deploy script", "the login form", "rate limits"];
const LOREM =
  "The change keeps the old behaviour for callers that pass a cursor, and the new path only runs when the list is longer than one page. I checked the three call sites and the migration that adds the index. ";

function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const text = (rand, chars) => {
  let out = "";
  while (out.length < chars) out += LOREM.slice(Math.floor(rand() * 40));
  return out.slice(0, chars);
};

function world(seed = 7) {
  const rand = rng(seed);
  const pick = (list) => list[Math.floor(rand() * list.length)];
  const now = () => Math.floor(Date.now() / 1000);
  const tabs = [];
  let opened = 0;
  const base = (cwd, extra) => ({
    id: `tab-${++opened}`,
    cwd,
    account: "claude",
    program: "agent",
    bypass: true,
    running: true,
    status: "working",
    title: null,
    message: null,
    resume_at: null,
    connector: null,
    opened,
    updated: now(),
    session: `0000${opened}-5e55-4a1d-9c0f-${String(opened).padStart(12, "0")}`,
    alive: true,
    ...extra,
  });
  const goal = base("/Users/dev/.pinned/orchestrator", { orchestrator_global: true, title: "Ship the Q4 roadmap across every project", prompt_title: "Ship the Q4 roadmap", stats: { active_ms: 5400000, cost_usd: 41.2 } });
  tabs.push(goal);
  const captains = PROJECTS.map((project, i) =>
    base(`/Users/dev/work/${project}`, {
      role: "orchestrator",
      title: `${project}: ${pick(VERBS)} ${pick(NOUNS)}`,
      prompt_title: `Plan the ${project} work for Q4`,
      branch: "main",
      orchestrator_turn: i % 2 === 0,
      queued: i % 3,
      stats: { active_ms: 1800000 + i * 60000, cost_usd: 6 + i },
      work: { usd: 20 + i * 3.5, unknown: 0, workers: 4 },
      last_message_at: now(),
      plans: [
        { id: `p${i}`, title: `${pick(VERBS)} ${pick(NOUNS)} in ${project}`, state: "working", done: 2, total: 6, now: `${pick(VERBS)} ${pick(NOUNS)}`, strips: ["done", "done", "working", "todo", "todo", "todo"] },
        { id: `q${i}`, title: `${pick(VERBS)} ${pick(NOUNS)}`, state: "finished", done: 3, total: 3, finished_at: now() - 3600, strips: ["done", "done", "done"] },
      ],
    }),
  );
  tabs.push(...captains);
  for (let i = 0; i < 30; i++) {
    const captain = captains[i % captains.length];
    tabs.push(
      base(`${captain.cwd}/.claude/worktrees/w${i}`, {
        role: "worker",
        spawned_by: captain.id,
        title: `${pick(VERBS)} ${pick(NOUNS)}`,
        prompt_title: `${pick(VERBS)} ${pick(NOUNS)}: ${text(rand, 80)}`,
        message: null,
        branch: `fix/w${i}`,
        stats: { active_ms: 300000 + i * 7000, cost_usd: 0.5 + i / 10 },
        last_message_at: now(),
      }),
    );
  }
  for (let i = 0; i < 41; i++) {
    tabs.push(
      base(`/Users/dev/work/${pick(PROJECTS)}`, {
        status: "done",
        alive: false,
        title: `${pick(VERBS)} ${pick(NOUNS)}`,
        prompt_title: text(rand, 100),
        question: i % 3 ? `${text(rand, 2400)} Should I open the pull request now?` : null,
        branch: `feat/t${i}`,
        pr: i % 3 ? null : { number: 100 + i, url: `https://github.com/acme/repo/pull/${100 + i}` },
        stats: { active_ms: 900000, cost_usd: 2.5 },
        last_message_at: now() - 7200,
      }),
    );
  }

  // Each tab's transcript, as the items a `thread` call returns, with the byte offset after each.
  const logs = new Map();
  const item = (tab, n) => {
    const at = new Date().toISOString();
    const id = `${tab.id}-${n}`;
    const r = rand();
    if (n === 0) return { kind: "user", id, at, text: tab.prompt_title ?? "Start", images: 0, pictures: [] };
    if (r < 0.35) return { kind: "tool", id, at, name: pick(["Read", "Bash", "Edit", "Grep"]), summary: `${pick(NOUNS)}`, detail: text(rand, 160), args: [] };
    if (r < 0.65) return { kind: "result", tool: `${tab.id}-${n - 1}`, at, error: false, text: text(rand, 600 + Math.floor(rand() * 3400)), links: [] };
    if (r < 0.8) return { kind: "thinking", id, at, text: text(rand, 400) };
    return { kind: "text", id, at, text: text(rand, 300 + Math.floor(rand() * 900)) };
  };
  const append = (tab) => {
    const log = logs.get(tab.id) ?? { items: [], ends: [] };
    logs.set(tab.id, log);
    const next = item(tab, log.items.length);
    log.items.push(next);
    log.ends.push((log.ends.at(-1) ?? 0) + JSON.stringify(next).length + 1);
  };
  for (const tab of tabs) for (let n = 0, count = 40 + Math.floor(rand() * 80); n < count; n++) append(tab);

  function page({ tab: id, from = 0, limit, before }) {
    const tab = tabs.find((t) => t.id === id);
    const log = logs.get(id);
    if (!tab || !log) throw "no such tab";
    const head = { session: tab.session, reset: false, agents: [] };
    if (before != null) {
      const end = log.ends.filter((e) => e <= before).length;
      const first = Math.max(0, end - (limit ?? end));
      return { ...head, items: log.items.slice(first, end), start: first ? log.ends[first - 1] : 0, next: log.ends.at(-1) };
    }
    if (!from) {
      const first = Math.max(0, log.items.length - (limit ?? log.items.length));
      return { ...head, items: log.items.slice(first), start: first ? log.ends[first - 1] : 0, next: log.ends.at(-1) };
    }
    const first = log.ends.findIndex((e) => e > from);
    return { ...head, items: first < 0 ? [] : log.items.slice(first), next: log.ends.at(-1) };
  }

  const working = () => tabs.filter((t) => t.status === "working" && t.alive);
  // One change a hook event or a Captain's plan makes, after which the node sends its whole list.
  function mutate() {
    const tab = pick(tabs.slice(0, 40));
    const r = rand();
    tab.updated = now();
    if (r < 0.3) tab.status = tab.status === "working" ? "done" : "working";
    else if (r < 0.4) Object.assign(tab, { status: "needs-input", message: "Claude needs your permission to use Bash", ask_detail: "npm test -- --runInBand" });
    else if (r < 0.6 && tab.plans) {
      const plan = tab.plans[0];
      plan.done = (plan.done + 1) % (plan.total + 1);
      plan.strips = plan.strips.map((_, i) => (i < plan.done ? "done" : i === plan.done ? "working" : "todo"));
      plan.now = `${pick(VERBS)} ${pick(NOUNS)}`;
    } else if (r < 0.8 && tab.stats) tab.stats = { ...tab.stats, cost_usd: Math.round((tab.stats.cost_usd + rand()) * 100) / 100 };
    else tab.queued = Math.floor(rand() * 3);
    if (tab.status !== "needs-input") Object.assign(tab, { message: null, ask_detail: undefined });
  }

  // Claude Code redrawing its spinner and status lines, as the bytes a terminal receives.
  let frame = 0;
  function screen(tab) {
    const spin = "✻✽✶✳✢·"[frame++ % 6];
    const secs = (frame % 600) + 1;
    const rows = [
      `\x1b[?2026h\x1b[2K\x1b[1A\x1b[2K\x1b[1A\x1b[2K\x1b[1A\x1b[2K\x1b[1A\x1b[2K\x1b[G`,
      `\x1b[38;2;215;119;87m${spin}\x1b[39m \x1b[38;2;215;119;87mWrangling…\x1b[39m \x1b[2m(${secs}s · ↓ ${(secs * 37) % 9000} tokens · esc to interrupt)\x1b[22m\r\n`,
      `\x1b[2m  ⎿  ${tab.title ?? ""}\x1b[22m\r\n\r\n`,
      `\x1b[38;2;136;136;136m╭${"─".repeat(78)}╮\x1b[39m\r\n`,
      `\x1b[38;2;136;136;136m│\x1b[39m > ${" ".repeat(75)}\x1b[38;2;136;136;136m│\x1b[39m\r\n`,
      `\x1b[38;2;136;136;136m╰${"─".repeat(78)}╯\x1b[39m\r\n`,
      `  \x1b[2m⏵⏵ bypass permissions on (shift+tab to cycle)\x1b[22m\x1b[?2026l`,
    ];
    return Buffer.from(rows.join("")).toString("base64");
  }

  return { MACHINE, tabs, page, append, mutate, working, screen, rand, pick };
}

/** Accepts a WebSocket upgrade on `socket` and returns the connection: text frames only, which is
 * all the page sends. A real socket keeps the page's message handling the browser's own. */
function accept(req, socket) {
  const key = crypto.createHash("sha1").update(`${req.headers["sec-websocket-key"]}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest("base64");
  socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${key}\r\n\r\n`);
  socket.setNoDelay(true);
  socket.on("error", () => {});
  let pending = Buffer.alloc(0);
  let handler = () => {};
  socket.on("data", (chunk) => {
    pending = Buffer.concat([pending, chunk]);
    for (;;) {
      if (pending.length < 2) return;
      let length = pending[1] & 0x7f;
      let at = 2;
      if (length === 126) [length, at] = pending.length < 4 ? [Infinity, 4] : [pending.readUInt16BE(2), 4];
      else if (length === 127) [length, at] = pending.length < 10 ? [Infinity, 10] : [Number(pending.readBigUInt64BE(2)), 10];
      const mask = pending[1] & 0x80 ? pending.subarray(at, at + 4) : null;
      if (mask) at += 4;
      if (pending.length < at + length) return;
      const payload = Buffer.from(pending.subarray(at, at + length));
      if (mask) for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i & 3];
      const op = pending[0] & 0x0f;
      pending = pending.subarray(at + length);
      if (op === 0x8) return socket.end();
      if (op === 0x1) handler(payload.toString());
    }
  });
  return {
    send(text) {
      const data = Buffer.from(text);
      const head = data.length < 126 ? Buffer.from([0x81, data.length]) : data.length < 65536 ? Buffer.alloc(4) : Buffer.alloc(10);
      if (data.length >= 126) {
        head[0] = 0x81;
        if (data.length < 65536) {
          head[1] = 126;
          head.writeUInt16BE(data.length, 2);
        } else {
          head[1] = 127;
          head.writeBigUInt64BE(BigInt(data.length), 2);
        }
      }
      if (!socket.destroyed) socket.write(Buffer.concat([head, data]));
    },
    onMessage: (fn) => (handler = fn),
  };
}

/** Serves `w` to the page's socket at the given rates (frames per second). Returns the counters
 * and a stop function. `onCall` sees every call for the report. */
function serve(ws, w, rates, onCall) {
  const sent = { frames: 0, bytes: 0, kinds: {} };
  const watched = new Set();
  const attached = new Set();
  const send = (frame, kind) => {
    const data = JSON.stringify(frame);
    sent.frames++;
    sent.bytes += data.length;
    sent.kinds[kind] = (sent.kinds[kind] ?? 0) + 1;
    ws.send(data);
  };
  const event = (event) => send({ type: "event", machine: w.MACHINE, event }, event.kind);
  const tabsCopy = () => JSON.parse(JSON.stringify(w.tabs));
  let timer = null;
  let carry = {};
  const start = () => {
    const TICK = 50;
    timer = setInterval(() => {
      const due = (kind) => {
        carry[kind] = (carry[kind] ?? 0) + (rates[kind] ?? 0) * (TICK / 1000);
        const n = Math.floor(carry[kind]);
        carry[kind] -= n;
        return n;
      };
      const live = w.working();
      for (let n = due("output"); n > 0; n--) {
        const tab = w.pick(live.filter((t) => attached.has(t.id)));
        if (tab) event({ kind: "output", tab: tab.id, data: w.screen(tab) });
      }
      for (let n = due("transcript"); n > 0; n--) {
        const tab = w.pick(live);
        if (!tab) continue;
        w.append(tab);
        if (watched.has(tab.id)) event({ kind: "thread", tab: tab.id });
      }
      for (let n = due("tabs"); n > 0; n--) {
        w.mutate();
        event({ kind: "tabs", tabs: tabsCopy() });
      }
      for (let n = due("usage"); n > 0; n--) {
        const pct = 40 + w.rand() * 20;
        event({ kind: "usage", account: "claude", usage: { five_hour: { used_percentage: pct, resets_at: Math.floor(Date.now() / 1000) + 3600 }, seven_day: { used_percentage: 30, resets_at: Math.floor(Date.now() / 1000) + 86400 } } });
      }
      for (let n = due("agent"); n > 0; n--) {
        const tab = w.pick(live);
        if (tab) event({ kind: "agent", event: { tab: tab.id, status: "working", session: tab.session, cwd: tab.cwd, message: null } });
      }
    }, TICK);
  };

  const answer = (call) => {
    switch (call.cmd) {
      case "tabs":
        return tabsCopy();
      case "thread":
        return w.page(call);
      case "watch_threads":
        watched.clear();
        for (const id of call.tabs) watched.add(id);
        return null;
      case "workspace": {
        const tab = w.tabs.find((t) => t.id === call.tab);
        return { project: tab?.cwd.replace(/\/\.claude\/worktrees\/.*/, "") ?? "/", repo: true, branch: tab?.branch ?? "main", worktree: !!tab?.cwd.includes("/worktrees/"), scratch: false };
      }
      case "project_history":
        return [];
      case "list_accounts":
        return [{ id: "claude", provider: "claude", name: "Claude", provider_name: "Claude", auth: "login" }];
      case "list_projects":
        return [...new Set(w.tabs.filter((t) => t.role === "orchestrator").map((t) => t.cwd))].map((path) => ({ path, name: path.split("/").pop(), account: "claude" }));
      case "pull_requests":
        return { rows: [] };
      case "skills":
      case "editors":
        return [];
      default:
        return null;
    }
  };
  const hubAnswer = (call) => {
    if (call.cmd === "session_marks") return [];
    if (call.cmd === "get_settings") return { connectors: [], box_asks: [], convex: [], boxes: {} };
    return null;
  };

  ws.onMessage((raw) => {
    const frame = JSON.parse(raw);
    if (frame.type === "auth") {
      send({ type: "hello", account: { id: "acct-1", email: "dev@example.test" }, machines: [{ id: w.MACHINE, name: "studio", os: "macos", online: true, last_seen: Math.floor(Date.now() / 1000) }], tabs: { [w.MACHINE]: tabsCopy() } }, "hello");
      return;
    }
    if (frame.type === "attach") {
      attached.add(frame.tab);
      return;
    }
    if (frame.type === "detach") {
      attached.delete(frame.tab);
      return;
    }
    if (frame.type !== "call" && frame.type !== "hub") return;
    const call = frame.call;
    onCall?.(frame.type === "hub" ? `hub:${call.cmd}` : call.cmd);
    try {
      const ok = frame.type === "hub" ? hubAnswer(call) : answer(call);
      send({ type: "reply", id: frame.id, ok }, "reply");
    } catch (err) {
      send({ type: "reply", id: frame.id, err: String(err) }, "reply");
    }
  });
  return { sent, watched, attached, start, stop: () => clearInterval(timer) };
}

module.exports = { world, serve, accept };
