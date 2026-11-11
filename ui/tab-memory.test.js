import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = (file) => readFileSync(new URL(`./${file}`, import.meta.url), "utf8");
const slice = (src, from, to) => src.slice(src.indexOf(from), src.indexOf(to, src.indexOf(from)));

/** A stand-in for xterm's Terminal: what was written, its options, and whether it has elements. */
function fakeTerm(modes = {}) {
  return {
    written: [],
    resets: 0,
    element: null,
    cols: 80,
    rows: 24,
    options: { scrollback: 0 },
    buffer: { active: { type: modes.alternate ? "alternate" : "normal" } },
    modes: { applicationCursorKeysMode: false, applicationKeypadMode: false, bracketedPasteMode: false, sendFocusMode: false, mouseTrackingMode: "none", ...modes },
    write(data) {
      this.written.push(typeof data === "string" ? data : [...data]);
    },
    reset() {
      this.resets++;
      this.written.push("reset");
    },
    loadAddon() {},
    open(el) {
      this.element = { parent: el, remove() {} };
    },
    resize(cols, rows) {
      Object.assign(this, { cols, rows });
    },
    paste(text) {
      this.written.push(`pasted:${text}`);
    },
  };
}

/** main.js's terminal plumbing over fake terminals and a transport that records its calls. */
function terminals({ gui = true, drawer = "closed" } = {}) {
  const main = source("main.js");
  const calls = [];
  const typed = [];
  const state = { tabs: [], active: null, gui, drawer };
  const transport = {
    sinks: new Map(),
    attach(tab, sink) {
      calls.push(`attach ${tab}`);
      this.sinks.set(tab, sink);
    },
    async detach(tab) {
      calls.push(`detach ${tab}`);
    },
  };
  const sandbox = {
    calls,
    typed,
    console,
    machines: new Map([["m", { transport }]]),
    guiOn: () => state.gui,
    callOn: (machine, call) => (calls.push(`${call.cmd} ${call.cols}x${call.rows}`), Promise.resolve()),
    Terminal: function () {
      return fakeTerm();
    },
    FitAddon: { FitAddon: function () {
      return { proposeDimensions: () => ({ cols: 120, rows: 40 }), fit() {} };
    } },
  };
  sandbox.Terminal.prototype = {};
  Object.defineProperty(sandbox, "document", { get: () => ({ body: { dataset: { view: "expanded", drawer: state.drawer } } }) });
  Object.defineProperty(sandbox, "tabs", { get: () => state.tabs });
  Object.defineProperty(sandbox, "active", { get: () => state.active });
  vm.createContext(sandbox);
  vm.runInContext(
    `${slice(main, "function newTerminal()", "function addTab(")}
     ${slice(main, "const TERMINAL_PIECE", "function typeInto(")}
     function typeInto(tab, data) { typed.push(data); }
     ${slice(main, "let ruler = null;", "function render()")}
     ${slice(main, "function pasteInto(", "// Claude Code attaches")}`,
    sandbox,
  );
  const tab = (id) => {
    const made = { machine: "m", view: { id, running: true }, term: fakeTerm(), fit: { proposeDimensions: () => ({ cols: 100, rows: 30 }), fit() {} }, el: { hidden: false, append() {} }, sink: null, attached: Promise.resolve() };
    state.tabs.push(made);
    sandbox.attachTerm(made);
    return made;
  };
  return { sandbox, state, calls, typed, transport, tab };
}

test("a terminal is fed its session's output a few kilobytes at a time", () => {
  const { sandbox } = terminals();
  const term = fakeTerm();
  sandbox.feed(term, Uint8Array.from({ length: 10000 }, (_, i) => i % 251));
  assert.deepEqual(term.written.map((piece) => piece.length), [4096, 4096, 1808]);
  assert.deepEqual(term.written.flat(), Array.from({ length: 10000 }, (_, i) => i % 251));
});

test("a tab's terminal off screen has no elements and no history, and its session still gets the size the terminal view would give it", async () => {
  const { sandbox, state, calls, tab } = terminals({ gui: true, drawer: "closed" });
  const a = tab("a");
  state.active = a;
  await a.attached;
  sandbox.refit(a);
  assert.equal(a.term.element, null);
  assert.equal(a.term.options.scrollback, 0);
  assert.deepEqual([a.term.cols, a.term.rows], [120, 40]);
  assert.deepEqual(calls, ["attach a", "resize_pty 120x40"]);
});

test("a terminal that comes on screen gets its elements and its history replayed, with the modes its session had set", async () => {
  const { sandbox, state, calls, transport, tab } = terminals({ gui: false });
  const a = tab("a");
  await a.attached;
  Object.assign(a.term.modes, { bracketedPasteMode: true, applicationCursorKeysMode: true, mouseTrackingMode: "vt200" });
  state.active = a;
  sandbox.refit(a);
  await a.attached;
  assert.equal(a.term.element.parent, a.el);
  assert.equal(a.term.options.scrollback, 1000);
  assert.deepEqual(calls, ["attach a", "resize_pty 80x24", "detach a", "attach a"]);
  assert.deepEqual(a.term.written, ["reset", "\x1b[?1h\x1b[?66l\x1b[?2004h\x1b[?1004l\x1b[?1000h"]);
  transport.sinks.get("a").write(Uint8Array.of(104, 105));
  assert.deepEqual(a.term.written.at(-1), [104, 105]);
});

test("output an earlier attach still delivers does not land in the terminal twice", async () => {
  const { sandbox, state, transport, tab } = terminals({ gui: false });
  const a = tab("a");
  await a.attached;
  const first = transport.sinks.get("a");
  state.active = a;
  sandbox.refit(a);
  await a.attached;
  first.write(Uint8Array.of(1, 2, 3));
  assert.deepEqual(a.term.written, ["reset", "\x1b[?1l\x1b[?66l\x1b[?2004l\x1b[?1004l"]);
});

test("the line that says how to restart an exited session goes back under the replay", async () => {
  const { sandbox, state, transport, tab } = terminals({ gui: false });
  const a = tab("a");
  await a.attached;
  a.exitNote = "[claude exited, press Enter to restart]";
  state.active = a;
  sandbox.refit(a);
  await a.attached;
  const sink = transport.sinks.get("a");
  sink.write(Uint8Array.of(7));
  sink.write(Uint8Array.of(8));
  assert.deepEqual(a.term.written.slice(2), [[7], "[claude exited, press Enter to restart]", [8]]);
});

test("the three terminals shown last keep their history, and one shown before them gives it up", async () => {
  const { sandbox, state, calls, tab } = terminals({ gui: false });
  const shown = ["a", "b", "c", "d"].map(tab);
  for (const each of shown) {
    for (const other of shown) other.el.hidden = other !== each;
    state.active = each;
    sandbox.refit(each);
    await each.attached;
  }
  assert.deepEqual(shown.map((t) => t.term.options.scrollback), [0, 1000, 1000, 1000]);
  const replays = () => calls.filter((call) => call.startsWith("detach")).length;
  const before = replays();
  for (const other of shown) other.el.hidden = other !== shown[2];
  state.active = shown[2];
  sandbox.refit(shown[2]);
  await shown[2].attached;
  assert.equal(replays(), before, "a terminal that still has its history is not replayed again");
  for (const other of shown) other.el.hidden = other !== shown[0];
  state.active = shown[0];
  sandbox.refit(shown[0]);
  assert.deepEqual(shown.map((t) => t.term.options.scrollback), [1000, 0, 1000, 1000]);
});

test("a paste into a terminal without elements is sent as xterm sends it", () => {
  const { sandbox, typed } = terminals();
  const plain = { term: fakeTerm() };
  sandbox.pasteInto(plain, "one\r\ntwo\nthree");
  const bracketed = { term: fakeTerm({ bracketedPasteMode: true }) };
  sandbox.pasteInto(bracketed, "a b ");
  assert.deepEqual(typed, ["one\rtwo\rthree", "\x1b[200~a b \x1b[201~"]);
  const opened = { term: Object.assign(fakeTerm(), { element: {} }) };
  sandbox.pasteInto(opened, "x");
  assert.deepEqual(opened.term.written, ["pasted:x"]);
});

/** gui.js's handling of the threads off screen, over threads a test fills in. */
function threads() {
  const gui = source("gui.js");
  const state = { tabs: [], active: null, live: new Set() };
  const store = new Map();
  const sandbox = {
    threadOf: (tab) => store.get(tab) ?? store.set(tab, { items: [], loaded: false }).get(tab),
    followed: (tab) => tab === state.active || state.live.has(tab),
  };
  Object.defineProperty(sandbox, "tabs", { get: () => state.tabs });
  Object.defineProperty(sandbox, "active", { get: () => state.active });
  vm.createContext(sandbox);
  vm.runInContext(`${slice(gui, "const WHOLE_THREADS", "function onThreadEvent(")}\nglobalThis.show = (tab) => { threadOf(tab).shownAt = ++shownClock; };`, sandbox);
  return { sandbox, state };
}

const transcript = () => [
  { kind: "model", name: "Opus 5.5" },
  { kind: "user", id: "u1", text: "fix the parser", pictures: ["data:image/png;base64,AAAA"], images: 1 },
  { kind: "thinking", id: "t1", text: "the cursor is off by one" },
  { kind: "tool", id: "c1", name: "Bash", detail: "cargo test" },
  { kind: "result", id: "r1", tool: "c1", text: "ok" },
  { kind: "text", id: "a1", text: "Fixed the cursor." },
  { kind: "pr", number: 7, url: "https://github.com/acme/parser/pull/7", repo: "acme/parser", at: "2026-11-10T12:00:00Z" },
];

test("a thread shown before the last six keeps what the sidebar reads and drops tool calls, results, reasoning and pictures", () => {
  const { sandbox, state } = threads();
  const tabs = Array.from({ length: 8 }, (_, i) => ({ key: `t${i}` }));
  state.tabs.push(...tabs);
  for (const tab of tabs) {
    Object.assign(sandbox.threadOf(tab), { items: transcript(), loaded: true, history: [{ patch: "diff" }] });
    sandbox.show(tab);
  }
  state.active = tabs[7];
  sandbox.lightenThreads();
  const kinds = (tab) => sandbox.threadOf(tab).items.map((i) => i.kind);
  assert.deepEqual(kinds(tabs[0]), ["model", "user", "text", "pr"]);
  assert.deepEqual(kinds(tabs[1]), ["model", "user", "text", "pr"]);
  assert.equal(JSON.stringify(sandbox.threadOf(tabs[0]).items[1]), '{"kind":"user","id":"u1","text":"fix the parser","pictures":[],"images":1}');
  assert.equal(sandbox.threadOf(tabs[0]).history, null);
  for (const tab of tabs.slice(2)) assert.equal(kinds(tab).length, 7, `${tab.key} is one of the six shown last`);
});

test("a thread that dropped its tool calls holds them nowhere else, and still builds its turns from what it kept", () => {
  const timeline = source("gui-timeline.js");
  const { turnsFor } = new Function(`${slice(timeline, "const PLAN_TOOL", "function duration(ms)")}; return { turnsFor };`)();
  const { sandbox, state } = threads();
  const tabs = Array.from({ length: 8 }, (_, i) => ({ key: `t${i}` }));
  state.tabs.push(...tabs);
  for (const tab of tabs) {
    Object.assign(sandbox.threadOf(tab), { items: transcript(), loaded: true });
    // What the thread view does when the thread is on screen.
    turnsFor(sandbox.threadOf(tab));
    sandbox.show(tab);
  }
  state.active = tabs[7];
  sandbox.lightenThreads();
  const reachable = (root) => {
    const seen = new Set();
    for (const stack = [root]; stack.length; ) {
      const value = stack.pop();
      if (!value || typeof value !== "object" || seen.has(value)) continue;
      seen.add(value);
      stack.push(...(value instanceof Map ? [...value.keys(), ...value.values()] : Object.values(value)));
    }
    return [...seen];
  };
  const tools = (tab) => reachable(sandbox.threadOf(tab)).filter((value) => value.kind === "tool" || value.kind === "result").length;
  assert.deepEqual([tools(tabs[0]), tools(tabs[1])], [0, 0]);
  assert.ok(tools(tabs[2]) > 0, "one of the six shown last keeps its turns");
  assert.deepEqual(turnsFor(sandbox.threadOf(tabs[0])).map((turn) => [turn.user.text, turn.answer.text]), [["fix the parser", "Fixed the cursor."]]);
});

test("a thread never shown keeps its first read, and one with a turn running keeps its transcript", () => {
  const { sandbox, state } = threads();
  const tabs = Array.from({ length: 9 }, (_, i) => ({ key: `t${i}` }));
  state.tabs.push(...tabs);
  for (const tab of tabs) Object.assign(sandbox.threadOf(tab), { items: transcript(), loaded: true });
  for (const tab of tabs.slice(1)) sandbox.show(tab);
  state.live.add(tabs[1]);
  state.active = tabs[8];
  sandbox.lightenThreads();
  assert.deepEqual(tabs.map((tab) => sandbox.threadOf(tab).items.length), [7, 7, 4, 7, 7, 7, 7, 7, 7]);
});
