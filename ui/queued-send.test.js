import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(name, import.meta.url), "utf8");

function between(src, startAt, endAt) {
  const start = src.indexOf(startAt);
  const end = src.indexOf(endAt, start + startAt.length);
  if (start < 0 || end < 0) throw new Error(`missing slice ${startAt}`);
  return src.slice(start, end);
}

const gui = read("./gui.js");
const composerSrc = read("./gui-composer.js");
const timeline = read("./gui-timeline.js");

/** The real fetchThread and settling, send(), and the pending row's helpers, over a thread whose
 * prompt call and transcript this test scripts. */
function rig() {
  const calls = [];
  const notes = [];
  const paints = [];
  const timers = [];
  const script = { prompt: () => Promise.resolve(null), pages: [] };
  const clock = { now: Date.now() };
  class FakeDate extends Date {
    static now() {
      return clock.now;
    }
  }
  const prompt = { value: "" };
  const context = {
    calls,
    notes,
    paints,
    timers,
    clock,
    Date: FakeDate,
    script,
    prompt,
    active: null,
    performance: { now: () => clock.now },
    THREAD_READ_GAP_MS: 0,
    THREAD_PAGE: 50,
    composer: { dataset: {} },
    document: { body: { dataset: { hub: "online" } } },
    machines: new Map(),
    LOCAL: "local",
    setTimeout: (run, ms) => timers.push({ run, ms }),
    clearTimeout() {},
    threadOf: (tab) => tab.thread,
    expandChips: (_tab, text) => text,
    escapePath: (path) => path,
    chipsOf: () => ({}),
    saveDraft() {},
    autosize() {},
    toggleProjects() {},
    startTab() {},
    compactOffer: () => false,
    closeClaudeDialog: async () => true,
    clearComposer: () => {
      prompt.value = "";
    },
    composerNoteTimer: 0,
    composerNote: {
      text: "",
      error: false,
      set textContent(v) {
        this.text = v;
      },
      toggleAttribute(_name, on) {
        this.error = on;
      },
      set hidden(hidden) {
        notes.push({ text: hidden ? "" : this.text, error: this.error });
      },
    },
    renderThread: () => paints.push(1),
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    markdown: (text) => `<p>${context.esc(text)}</p>`,
    promptMarked: {},
    turnLive: (tab) => tab.view.status === "working",
    screenLines: (tab) => tab.term.lines,
    callOn: (_machine, call) => {
      calls.push(call);
      if (call.cmd === "prompt") return script.prompt(call);
      if (call.cmd === "thread") return Promise.resolve(script.pages.shift() ?? { session: "s", items: [], next: 0, reset: false, agents: [] });
      return Promise.resolve(null);
    },
    takeAgents() {},
    guiOn: () => true,
    terminalOnly: () => false,
    renderTerminalOnly() {},
    renderUnsupported() {},
    markThreadSeen() {},
    renderSidebar() {},
    fetchHistory() {},
    lightenThreads() {},
  };
  vm.createContext(context);
  vm.runInContext(
    `${between(composerSrc, "function showComposerNote", "/** What claude's menu asks")}
${between(gui, "const drawnOf", "/** Threads that keep their whole transcript")}
${between(composerSrc, "async function send()", "/** T3 Code's ⌘⌥Enter")}
${between(timeline, "const QUEUED_HINT", "/** The prompt bubble as every")}`,
    context,
  );
  return context;
}

const tabOf = (id = "t1", view = {}) => ({
  key: `m:${id}`,
  machine: "m",
  view: { id, running: true, status: "idle", ...view },
  thread: { session: "s", next: 0, items: [], loaded: true, open: new Set() },
});

function open(r, tab = tabOf()) {
  r.active = tab;
  return tab;
}

async function typeAndSend(r, text) {
  r.prompt.value = text;
  await r.send();
}

const transcript = (...items) => ({ session: "s", items, next: 1, reset: false, agents: [] });
const rows = (r, tab) => [...r.pendingState(tab, tab.thread).rows];
const html = (r, tab) => r.pendingHtml(r.pendingState(tab, tab.thread));
const later = (r, ms) => {
  r.clock.now += ms;
};
const promptCalls = (r) => r.calls.filter((c) => c.cmd === "prompt");

test("a sent message is a pending row at once, whatever the thread is doing", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "rename the helper");
  assert.deepEqual(promptCalls(r).map((c) => c.text), ["rename the helper"]);
  assert.deepEqual(rows(r, tab).map((p) => p.text), ["rename the helper"]);
  assert.equal(r.prompt.value, "", "the composer is cleared");
  assert.match(html(r, tab), /data-state="sending"[\s\S]*rename the helper[\s\S]*Sending…/);
});

test("the row goes when a new user item has its text", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "rename the helper");
  r.script.pages.push(transcript({ kind: "user", id: "u1", at: "t1", text: "rename the helper" }));
  await r.fetchThread(tab);
  assert.deepEqual(rows(r, tab), []);
  assert.equal(html(r, tab), "");
});

test("the row goes when a steer item has its text", async () => {
  const r = rig();
  const tab = open(r, tabOf("t1", { status: "working" }));
  await typeAndSend(r, "also add a test");
  r.script.pages.push(transcript({ kind: "steer", id: "s1", at: "t1", text: "also add a test" }));
  await r.fetchThread(tab);
  assert.deepEqual(rows(r, tab), []);
});

test("the row goes when claude joined the prompts into one, whitespace and all", async () => {
  const r = rig();
  const tab = open(r, tabOf("t1", { status: "working" }));
  await typeAndSend(r, "first\n  thing");
  await typeAndSend(r, "second thing");
  assert.equal(rows(r, tab).length, 2);
  r.script.pages.push(transcript({ kind: "user", id: "u1", at: "t1", text: "first thing\n\nsecond   thing" }));
  await r.fetchThread(tab);
  assert.deepEqual(rows(r, tab), []);
});

test("an older item with the same words, or other words, leaves the row", async () => {
  const r = rig();
  const tab = open(r);
  tab.thread.items.push({ kind: "user", id: "old", at: "t0", text: "rename the helper" });
  await typeAndSend(r, "rename the helper");
  r.script.pages.push(transcript({ kind: "user", id: "u2", at: "t1", text: "something else" }, { kind: "text", id: "x", at: "t2", text: "rename the helper" }));
  await r.fetchThread(tab);
  assert.equal(rows(r, tab).length, 1);
});

test("two identical sends make one prompt call and say it is already queued", async () => {
  const r = rig();
  const tab = open(r, tabOf("t1", { status: "working" }));
  await typeAndSend(r, "run the tests");
  await typeAndSend(r, "run   the tests ");
  assert.equal(promptCalls(r).length, 1);
  assert.equal(r.notes.at(-1).text, "Already queued.");
  assert.notEqual(r.notes.at(-1).error, true, "not an error: it is drawn in the muted colour");
  assert.equal(r.prompt.value, "", "the retyped draft is cleared");
  assert.equal(rows(r, tab).length, 1);
});

test("the Already queued note goes when its row settles, and when the draft changes", async () => {
  const r = rig();
  const tab = open(r, tabOf("t1", { status: "working" }));
  await typeAndSend(r, "run the tests");
  await typeAndSend(r, "run the tests");
  assert.equal(r.notes.at(-1).text, "Already queued.");
  r.script.pages.push(transcript({ kind: "user", id: "u1", at: "t1", text: "run the tests" }));
  await r.fetchThread(tab);
  assert.equal(r.notes.at(-1).text, "", "the row settled, so its note goes");

  await typeAndSend(r, "lint it");
  await typeAndSend(r, "lint it");
  assert.equal(r.notes.at(-1).text, "Already queued.");
  r.clearQueuedNote();
  assert.equal(r.notes.at(-1).text, "", "typing in the composer clears it");

  const before = r.notes.length;
  r.clearQueuedNote();
  assert.equal(r.notes.length, before, "nothing to clear once it is gone");
});

test("settling another row leaves the Already queued note of a row still waiting", async () => {
  const r = rig();
  const tab = open(r, tabOf("t1", { status: "working" }));
  await typeAndSend(r, "first");
  await typeAndSend(r, "second");
  await typeAndSend(r, "second");
  assert.equal(r.notes.at(-1).text, "Already queued.");
  const before = r.notes.length;
  r.script.pages.push(transcript({ kind: "user", id: "u1", at: "t1", text: "first" }));
  await r.fetchThread(tab);
  assert.equal(rows(r, tab).length, 1);
  assert.equal(r.notes.length, before, "its own row still waits");
});

test("once the row settles the same words go out again", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "run the tests");
  r.script.pages.push(transcript({ kind: "user", id: "u1", at: "t1", text: "run the tests" }));
  await r.fetchThread(tab);
  await typeAndSend(r, "run the tests");
  assert.equal(promptCalls(r).length, 2);
  assert.equal(rows(r, tab).length, 1);
});

test("a row older than the guard no longer blocks the same words", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "run the tests");
  rows(r, tab)[0].at -= 121_000;
  await typeAndSend(r, "run the tests");
  assert.equal(promptCalls(r).length, 2);
});

test("different words send even while another row waits", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "one");
  await typeAndSend(r, "two");
  assert.deepEqual(promptCalls(r).map((c) => c.text), ["one", "two"]);
  assert.equal(rows(r, tab).length, 2);
});

test("a busy thread says it is queued; an idle one says it is sending", async () => {
  const r = rig();
  const busy = open(r, tabOf("t1", { status: "working" }));
  await typeAndSend(r, "later");
  assert.match(html(r, busy), /data-state="queued"[\s\S]*Queued · claude is busy; it goes in when it&#39;s free/);
  const idle = open(r, tabOf("t2"));
  await typeAndSend(r, "now");
  assert.match(html(r, idle), /data-state="sending"[\s\S]*Sending…/);
  assert.doesNotMatch(html(r, idle), /Queued/);
});

test("claude's own queued-messages hint on its screen makes an idle thread queued", async () => {
  const r = rig();
  const tab = open(r, { ...tabOf(), term: { lines: ["", "  ⏵⏵ accept edits on", "  ↑ to edit queued messages"] } });
  await typeAndSend(r, "after the compaction");
  assert.match(html(r, tab), /Queued · claude is busy/);
  tab.term.lines = ["", "  ? for shortcuts"];
  assert.match(html(r, tab), /Sending…/);
});

test("a screen that cannot be read leaves the label at what the status says", async () => {
  const r = rig();
  const tab = open(r, { ...tabOf(), term: { get lines() { throw new Error("no screen"); } } });
  await typeAndSend(r, "hello");
  assert.match(html(r, tab), /Sending…/);
});

test("a plain error takes the row back and puts the draft back", async () => {
  const r = rig();
  const tab = open(r);
  r.script.prompt = () => Promise.reject("claude isn't running");
  await typeAndSend(r, "rename the helper");
  assert.deepEqual(rows(r, tab), []);
  assert.equal(r.prompt.value, "rename the helper");
  assert.deepEqual(r.notes.at(-1), { text: "claude isn't running", error: true });
});

test("a dropped hub socket keeps the row as unconfirmed and does not restore the draft", async () => {
  const r = rig();
  const tab = open(r);
  r.script.prompt = () => Promise.reject("lost the connection to the hub");
  await typeAndSend(r, "rename the helper");
  assert.equal(r.prompt.value, "", "no second copy of the draft to send");
  assert.equal(rows(r, tab)[0].state, "unconfirmed");
  const drawn = html(r, tab);
  assert.match(drawn, /data-state="unconfirmed"/);
  assert.match(drawn, /Could not confirm it was delivered\. Check the session before sending again\./);
  assert.match(drawn, /data-send-again="pending:\d+">Send again<\/button>/);
  assert.equal(r.notes.length, 0);
});

test("an unconfirmed row still settles when the transcript shows it", async () => {
  const r = rig();
  const tab = open(r);
  r.script.prompt = () => Promise.reject("lost the connection to the hub");
  await typeAndSend(r, "rename the helper");
  r.script.pages.push(transcript({ kind: "user", id: "u1", at: "t1", text: "rename the helper" }));
  await r.fetchThread(tab);
  assert.deepEqual(rows(r, tab), []);
});

test("Send again on an unconfirmed row sends it once, whatever the guard says", async () => {
  const r = rig();
  const tab = open(r);
  r.script.prompt = () => Promise.reject("lost the connection to the hub");
  await typeAndSend(r, "rename the helper");
  r.script.prompt = () => Promise.resolve(null);
  const [row] = rows(r, tab);
  await r.sendAgain(tab, row.id);
  assert.deepEqual(promptCalls(r).map((c) => c.text), ["rename the helper", "rename the helper"]);
  assert.equal(rows(r, tab).length, 1, "still one row");
  assert.equal(rows(r, tab)[0].state, "sending");
  assert.doesNotMatch(html(r, tab), /Could not confirm/);
});

test("Send again that fails again goes back to unconfirmed", async () => {
  const r = rig();
  const tab = open(r);
  r.script.prompt = () => Promise.reject("lost the connection to the hub");
  await typeAndSend(r, "rename the helper");
  await r.sendAgain(tab, rows(r, tab)[0].id);
  assert.equal(rows(r, tab)[0].state, "unconfirmed");
});

test("Send again is only on an unconfirmed or a stale row, never on a queued or sending one", async () => {
  const r = rig();
  const busy = open(r, tabOf("t1", { status: "working" }));
  await typeAndSend(r, "later");
  assert.match(html(r, busy), /data-state="queued"/);
  assert.doesNotMatch(html(r, busy), /data-send-again/, "queued: delivered and claude is busy");
  const idle = open(r, tabOf("t2"));
  r.script.prompt = () => new Promise(() => {});
  r.prompt.value = "now";
  r.send();
  await new Promise((done) => setImmediate(done));
  assert.match(html(r, idle), /data-state="sending"/);
  assert.doesNotMatch(html(r, idle), /data-send-again/, "sending: the call is still out");
  delete r.composer.dataset.sending;
  const lost = open(r, tabOf("t3"));
  r.script.prompt = () => Promise.reject("lost the connection to the hub");
  await typeAndSend(r, "dropped");
  assert.match(html(r, lost), /data-state="unconfirmed"[\s\S]*data-send-again/);
  const slow = open(r, tabOf("t4"));
  r.script.prompt = () => Promise.resolve(null);
  await typeAndSend(r, "delivered");
  later(r, 31_000);
  assert.match(html(r, slow), /data-state="stale"[\s\S]*data-send-again/);
});

test("a delivered row on an idle tab turns stale after 30 s and redraws on its own", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "rename the helper");
  assert.match(html(r, tab), /data-state="sending"[\s\S]*Sending…/);
  later(r, 29_000);
  assert.doesNotMatch(html(r, tab), /not in the session yet/);
  const timer = r.timers.find((t) => t.ms > 30_000);
  assert.ok(timer, "a timer is set for the 30 s mark");
  const painted = r.paints.length;
  later(r, 2_000);
  timer.run();
  assert.equal(r.paints.length, painted + 1, "the thread redraws when the 30 s pass");
  const drawn = html(r, tab);
  assert.match(drawn, /data-state="stale"[\s\S]*Sent · not in the session yet[\s\S]*data-send-again="pending:\d+">Send again<\/button>/);
  assert.doesNotMatch(drawn, /Sending…|Queued/);
});

test("the stale state is part of the thread's render key, so the change is drawn", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "rename the helper");
  const key = () => JSON.stringify(r.pendingState(tab, tab.thread).stale);
  const before = key();
  later(r, 31_000);
  assert.notEqual(key(), before);
  assert.match(timeline, /waiting\.busy, waiting\.stale\]/);
});

test("a delivered row on a busy tab stays queued, with no button, however long it waits", async () => {
  const r = rig();
  const tab = open(r, tabOf("t1", { status: "working" }));
  await typeAndSend(r, "later");
  later(r, 10 * 60_000);
  const drawn = html(r, tab);
  assert.match(drawn, /data-state="queued"[\s\S]*Queued · claude is busy/);
  assert.doesNotMatch(drawn, /data-send-again|not in the session yet/);
});

test("a stale row goes back to queued when claude turns busy, and settles when the transcript has it", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "rename the helper");
  later(r, 31_000);
  assert.match(html(r, tab), /data-state="stale"/);
  tab.view.status = "working";
  assert.match(html(r, tab), /data-state="queued"/);
  tab.view.status = "idle";
  r.script.pages.push(transcript({ kind: "user", id: "u1", at: "t1", text: "rename the helper" }));
  await r.fetchThread(tab);
  assert.equal(html(r, tab), "");
});

test("a row the transcript brings in before the 30 s never turns stale", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "rename the helper");
  r.script.pages.push(transcript({ kind: "user", id: "u1", at: "t1", text: "rename the helper" }));
  await r.fetchThread(tab);
  later(r, 31_000);
  const painted = r.paints.length;
  r.timers.filter((t) => t.ms > 30_000).forEach((t) => t.run());
  assert.equal(r.paints.length, painted, "nothing to redraw for a settled row");
  assert.equal(html(r, tab), "");
});

test("Send again on a stale row types it once more and the 30 s start over", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "rename the helper");
  later(r, 31_000);
  await r.sendAgain(tab, rows(r, tab)[0].id);
  assert.deepEqual(promptCalls(r).map((c) => c.text), ["rename the helper", "rename the helper"]);
  assert.match(html(r, tab), /data-state="sending"/);
  later(r, 31_000);
  assert.match(html(r, tab), /data-state="stale"/);
});

test("a slash command is not a prompt to wait for", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, "/model");
  assert.equal(promptCalls(r).length, 1);
  assert.deepEqual(rows(r, tab), []);
});

test("the compact that goes first is not shown, only the message", async () => {
  const r = rig();
  const tab = open(r);
  r.compactOffer = () => true;
  await typeAndSend(r, "continue the work");
  assert.deepEqual(promptCalls(r).map((c) => c.text), ["/compact", "continue the work"]);
  assert.deepEqual(rows(r, tab).map((p) => p.text), ["continue the work"]);
});

test("the pending text is escaped, never raw", async () => {
  const r = rig();
  const tab = open(r);
  await typeAndSend(r, `<img src=x onerror="boom()">`);
  assert.doesNotMatch(html(r, tab), /<img/);
  assert.match(html(r, tab), /&#60;img/);
});

test("the Orchestrator, a Captain and a plain thread all draw the row, each in its own thread", async () => {
  const r = rig();
  const kinds = [
    tabOf("global", { role: "orchestrator", orchestrator_global: true }),
    tabOf("captain", { role: "orchestrator" }),
    tabOf("plain", { status: "working" }),
  ];
  for (const tab of kinds) {
    open(r, tab);
    await typeAndSend(r, `message to ${tab.view.id}`);
    assert.match(html(r, tab), new RegExp(`message to ${tab.view.id}`), tab.view.id);
    assert.equal(rows(r, tab).length, 1, tab.view.id);
  }
  assert.equal(kinds.filter((tab) => /message to global/.test(html(r, tab))).length, 1, "one thread's row stays out of the others");
  assert.equal(promptCalls(r).length, 3);
});
