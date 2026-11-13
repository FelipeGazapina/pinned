const REFUSED = [1008, 4001, 4003];
const RECONNECT_MIN_MS = 1000;
const RECONNECT_MAX_MS = 30000;
/** A connection the hub accepted that stayed up this long was healthy, so the next wait starts over. */
const RECONNECT_STABLE_MS = 30000;
/** Every wait varies by this share either way, so apps the hub dropped together don't return together. */
const RECONNECT_JITTER = 0.25;
/** The hub closes every socket with this code when it is restarting. */
const CLOSE_GOING_AWAY = 1001;

/** The wait before each try to get the hub back: 1 s, then twice the last, up to 30 s, each varied by
 * ±25% (never past 30 s). The same rule as the node's link in `link.rs`. `draw` gives a number in
 * [0, 1), so a test can pin the jitter. */
class Backoff {
  constructor(draw = Math.random) {
    this.draw = draw;
    this.next = RECONNECT_MIN_MS;
  }

  wait() {
    const base = this.next;
    this.next = Math.min(base * 2, RECONNECT_MAX_MS);
    return Math.min(Math.round(base * (1 + (this.draw() * 2 - 1) * RECONNECT_JITTER)), RECONNECT_MAX_MS);
  }

  reset() {
    this.next = RECONNECT_MIN_MS;
  }

  /** A socket closed with `code`. It had been up `upMs` after the hub accepted it, or never was
   * accepted. Returns how long to wait before the next try. A hub that says it is restarting, or a
   * link that stayed up, starts the waits over; one that never got in, or dropped at once, backs off. */
  closed({ accepted, upMs, code }) {
    if (code === CLOSE_GOING_AWAY || (accepted && upMs >= RECONNECT_STABLE_MS)) this.reset();
    return this.wait();
  }
}
/** The hub's answer to a call only the account's owner makes, until this app unlocks (`surface::LOCKED`). */
const LOCKED = "unlock with your password in the app to do this";

class Hub {
  /** `clock` swaps the random draw and the time, so a test can pin the reconnect waits. */
  constructor(base, token, on, clock = {}) {
    this.url = base.replace(/\/$/, "").replace(/^http/, "ws") + "/ws/client";
    this.token = token;
    this.on = on;
    this.online = false;
    this.nextId = 1;
    this.pending = new Map();
    this.attached = new Map();
    this.backoff = new Backoff(clock.draw);
    this.now = clock.now ?? Date.now;
    /** When the hub last accepted this app, and whether the app has told its UI the link is down. */
    this.acceptedAt = 0;
    this.down = false;
    this.closed = false;
    /** What the hub answered the password. It stays in memory, so a login read off this machine's
     * disk unlocks nothing. */
    this.unlock = null;
    this.unlocking = null;
    this.connect();
  }

  connect() {
    const ws = (this.ws = new WebSocket(this.url));
    // The lock makes the login alone, which the app keeps on disk, run no call only the owner makes.
    ws.onopen = () => ws.send(JSON.stringify({ type: "auth", token: this.token, lock: true, ...(this.unlock ? { unlock: this.unlock } : {}) }));
    ws.onmessage = (e) => this.receive(JSON.parse(e.data));
    ws.onclose = (e) => {
      const wasOnline = this.online;
      this.online = false;
      for (const { reject } of this.pending.values()) reject("lost the connection to the hub");
      this.pending.clear();
      if (this.closed) return;
      if (!wasOnline && REFUSED.includes(e.code)) return this.on.refused?.();
      // Once per outage, however many tries it takes: the UI shows it is reconnecting until the hub answers.
      if (!this.down) {
        this.down = true;
        this.on.online?.(false);
      }
      const wait = this.backoff.closed({ accepted: wasOnline, upMs: this.now() - this.acceptedAt, code: e.code });
      setTimeout(() => this.closed || this.connect(), wait);
    };
  }

  receive(frame) {
    switch (frame.type) {
      case "hello":
        this.online = true;
        this.down = false;
        this.acceptedAt = this.now();
        // The hub forgot our subscriptions with the old socket; each replays its screen, so the terminal starts over.
        for (const [key, { machine, tab, sinks }] of this.attached) {
          if (!frame.tabs[machine]?.some((t) => t.id === tab)) continue;
          for (const sink of sinks) sink.reset();
          this.send({ type: "attach", machine, tab });
        }
        this.on.hello?.(frame);
        this.on.online?.(true);
        break;
      case "reply": {
        const waiting = this.pending.get(frame.id);
        if (!waiting) break;
        this.pending.delete(frame.id);
        if ("err" in frame) waiting.reject(frame.err);
        else waiting.resolve(frame.ok);
        break;
      }
      case "machines":
        this.on.machines?.(frame.machines, frame.tabs);
        break;
      case "event":
        if (frame.event.kind === "output") {
          const bytes = Uint8Array.from(atob(frame.event.data), (c) => c.charCodeAt(0));
          for (const sink of this.attached.get(`${frame.machine}:${frame.event.tab}`)?.sinks ?? []) sink.write(bytes);
        } else {
          this.on.event?.(frame.machine, frame.event);
        }
        break;
      case "settings":
        this.on.settings?.(frame.settings);
        break;
      case "session_marks":
        this.on.marks?.(frame.marks);
        break;
      case "confirmations":
        this.on.confirmations?.(frame.items);
        break;
      // An idea was saved or deleted by one of the account's apps or agents. It carries nothing: read them again.
      case "ideas_changed":
        this.on.ideasChanged?.();
        break;
    }
  }

  send(frame) {
    if (this.ws.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(frame));
  }

  request(frame) {
    return this.exchange(frame).catch((e) => (e === LOCKED && this.on.locked ? this.unlockThen(frame) : Promise.reject(e)));
  }

  exchange(frame) {
    if (!this.online) return Promise.reject("not connected to the hub");
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ ...frame, id });
    });
  }

  /** Asks for the password once for every call it holds up, then sends `frame` again. */
  async unlockThen(frame) {
    this.unlocking ??= this.askUnlock().finally(() => (this.unlocking = null));
    await this.unlocking;
    return this.exchange(frame);
  }

  async askUnlock() {
    let error = "";
    for (;;) {
      const password = await this.on.locked(error);
      if (password == null) throw "this needs your password";
      try {
        this.unlock = (await this.exchange({ type: "hub", call: { cmd: "unlock", password } })).token;
        return;
      } catch (e) {
        error = String(e);
        if (error.startsWith("too many")) throw e;
      }
    }
  }

  call(machine, call) {
    return this.request({ type: "call", machine, call });
  }

  hub(call) {
    return this.request({ type: "hub", call });
  }

  attach(machine, tab, sink) {
    const key = `${machine}:${tab}`;
    if (!this.attached.has(key)) this.attached.set(key, { machine, tab, sinks: new Set() });
    const { sinks } = this.attached.get(key);
    sinks.add(sink);
    if (sinks.size === 1 && this.online) this.send({ type: "attach", machine, tab });
  }

  detach(machine, tab, sink) {
    const key = `${machine}:${tab}`;
    const entry = this.attached.get(key);
    if (!entry) return;
    entry.sinks.delete(sink);
    if (entry.sinks.size) return;
    this.attached.delete(key);
    this.send({ type: "detach", machine, tab });
  }

  transport(machine) {
    return {
      call: (call) => this.call(machine, call),
      attach: (tab, sink) => this.attach(machine, tab, sink),
      detach: (tab, sink) => this.detach(machine, tab, sink),
    };
  }

  close() {
    this.closed = true;
    this.ws.close();
  }
}
