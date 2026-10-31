const REFUSED = [1008, 4001, 4003];
const RETRY_MS = [1000, 30000];

class Hub {
  constructor(base, token, on) {
    this.url = base.replace(/\/$/, "").replace(/^http/, "ws") + "/ws/client";
    this.token = token;
    this.on = on;
    this.online = false;
    this.nextId = 1;
    this.pending = new Map();
    this.attached = new Map();
    this.retry = RETRY_MS[0];
    this.closed = false;
    this.connect();
  }

  connect() {
    const ws = (this.ws = new WebSocket(this.url));
    ws.onopen = () => ws.send(JSON.stringify({ type: "auth", token: this.token }));
    ws.onmessage = (e) => this.receive(JSON.parse(e.data));
    ws.onclose = (e) => {
      const wasOnline = this.online;
      this.online = false;
      for (const { reject } of this.pending.values()) reject("lost the connection to the hub");
      this.pending.clear();
      if (wasOnline) this.on.online?.(false);
      if (this.closed) return;
      if (!wasOnline && REFUSED.includes(e.code)) return this.on.refused?.();
      setTimeout(() => this.closed || this.connect(), this.retry);
      this.retry = Math.min(this.retry * 2, RETRY_MS[1]);
    };
  }

  receive(frame) {
    switch (frame.type) {
      case "hello":
        this.online = true;
        this.retry = RETRY_MS[0];
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
    }
  }

  send(frame) {
    if (this.ws.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(frame));
  }

  request(frame) {
    if (!this.online) return Promise.reject("not connected to the hub");
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ ...frame, id });
    });
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
