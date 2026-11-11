import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("./gui-gestures.js", import.meta.url), "utf8");

function el({ inside = [], parent = null, overflowX = "visible", scrollLeft = 0, scrollWidth = 0, clientWidth = 0, dataset = {} } = {}) {
  const classes = new Set();
  const props = new Map();
  const listeners = {};
  return {
    inside, parentElement: parent, overflowX, overflowY: "visible", scrollLeft, scrollTop: 0, scrollWidth, clientWidth, offsetWidth: 360, offsetHeight: 60, offsetTop: 0, offsetParent: {},
    dataset, clicks: 0, events: [],
    click() { this.clicks++; },
    closest(selectors) { return selectors.split(/,(?![^(]*\))/).some((s) => this.inside.includes(s.trim())) ? this : null; },
    querySelector() { return this.title ?? null; },
    querySelectorAll() { return this.rows ?? []; },
    append(child) { child.parentElement = this; },
    remove() { this.parentElement = null; },
    addEventListener(type, fn) { (listeners[type] ??= []).push(fn); },
    dispatchEvent(e) { this.events.push(e.type); },
    listeners,
    classList: { add: (...c) => c.forEach((x) => classes.add(x)), remove: (...c) => c.forEach((x) => classes.delete(x)), toggle: (c, on) => (on ? classes.add(c) : classes.delete(c)), contains: (c) => classes.has(c) },
    style: { setProperty: (k, v) => props.set(k, v), removeProperty: (k) => props.delete(k) },
  };
}

/** The gesture script on a phone showing `screen`, with every timer run at once. */
function phone({ screen = "thread" } = {}) {
  const listeners = {};
  const workspace = el();
  workspace.addEventListener = (type, fn) => (listeners[type] = fn);
  const crumbEl = el({ inside: ["#chat-head"] });
  crumbEl.title = el({ inside: ["button.title"] });
  const headMore = el({ inside: ["#chat-head", "#chat"] });
  const threadList = el();
  const thread = { key: "m:t1", settled: false, waits: false };
  const row = el({ inside: ["#thread-list .thread-row:is(.tall, .line)", "#thread-list :is(.thread-row, .project-head)"], dataset: { tab: thread.key } });
  threadList.rows = [row];
  const did = [];
  const context = {
    workspace, crumbEl, threadList, sideMenu: { hidden: true }, active: {},
    document: { getElementById: () => headMore, body: el(), createElement: () => el() },
    matchMedia: () => ({ matches: false }), getComputedStyle: (e) => e,
    guiOn: () => true, narrow: () => true, icon: (name) => `<i ${name}>`,
    MouseEvent: class { constructor(type) { this.type = type; } },
    phoneScreen: () => screen,
    phoneGo: (to) => did.push(`go ${to}`),
    tabByKey: (key) => (key === thread.key ? thread : null),
    settled: (tab) => tab.settled,
    canSnooze: (tab) => !tab.waits,
    snoozedNow: () => false,
    settle: (tab) => did.push(`settle ${tab.key}`),
    snoozeItems: (list) => list.map((t) => `snooze ${t.key}`),
    openSideMenu: (items, at) => did.push(`menu ${items.join()} at ${at === row ? "row" : "?"}`),
    setTimeout: (fn) => (fn(), 0), clearTimeout: () => {},
  };
  vm.createContext(context);
  vm.runInContext(source, context);
  let now = 0;
  const send = (type, target, points) => {
    const e = { type, target, timeStamp: now, touches: points.map(([x, y]) => ({ clientX: x, clientY: y })), prevented: false, preventDefault() { this.prevented = true; } };
    listeners[type](e);
    return e.prevented;
  };
  /** One finger from `from` to `to` in `steps` moves `stepMs` apart; says which moves the page kept. */
  const swipe = (target, from, to, { steps = 10, stepMs = 40 } = {}) => {
    send("touchstart", target, [from]);
    const prevented = [];
    for (let i = 1; i <= steps; i++) {
      now += stepMs;
      prevented.push(send("touchmove", target, [[from[0] + ((to[0] - from[0]) * i) / steps, from[1] + ((to[1] - from[1]) * i) / steps]]));
    }
    send("touchend", target, []);
    return prevented;
  };
  return { swipe, send, did, thread, row, threadList, title: crumbEl.title, headMore };
}
const chat = el({ inside: ["#chat"] });

test("on a thread, a right swipe from the left edge past 30% of the width goes back to Threads", () => {
  const far = phone();
  far.swipe(chat, [10, 400], [140, 400]);
  assert.deepEqual(far.did, ["go threads"]);
  const short = phone();
  short.swipe(chat, [10, 400], [80, 400]);
  assert.deepEqual(short.did, []);
});

test("a right swipe that starts away from the edge, or off a thread, goes nowhere", () => {
  const middle = phone();
  middle.swipe(chat, [120, 400], [330, 400]);
  assert.deepEqual(middle.did, []);
  const orchestrator = phone({ screen: "orchestrator" });
  orchestrator.swipe(chat, [10, 400], [300, 400]);
  assert.deepEqual(orchestrator.did, []);
});

test("a fast flick from the edge goes back short of the reach", () => {
  const p = phone();
  p.swipe(chat, [10, 400], [70, 400], { steps: 3, stepMs: 16 });
  assert.deepEqual(p.did, ["go threads"]);
});

test("on Threads a left swipe past the reach settles the row and a short one leaves it", () => {
  const far = phone({ screen: "threads" });
  far.swipe(far.row, [300, 200], [120, 200]);
  assert.deepEqual(far.did, ["settle m:t1"]);
  const short = phone({ screen: "threads" });
  short.swipe(short.row, [300, 200], [220, 200]);
  assert.deepEqual(short.did, []);
});

test("on Threads a right swipe opens the row's snooze times, and a thread that asks can't be snoozed", () => {
  const p = phone({ screen: "threads" });
  p.swipe(p.row, [60, 200], [260, 200]);
  assert.deepEqual(p.did, ["menu snooze m:t1 at row"]);
  const asking = phone({ screen: "threads" });
  asking.thread.waits = true;
  assert.deepEqual(asking.swipe(asking.row, [60, 200], [260, 200]), Array(10).fill(false), "the touch stays the browser's");
  assert.deepEqual(asking.did, []);
});

test("a vertical drag on a row stays the list's scroll", () => {
  const p = phone({ screen: "threads" });
  assert.deepEqual(p.swipe(p.row, [200, 200], [200, 20]), Array(10).fill(false));
  assert.deepEqual(p.did, []);
});

test("a long press on a row opens its menu and keeps the tap that would close it", () => {
  const p = phone({ screen: "threads" });
  p.send("touchstart", p.row, [[200, 200]]);
  assert.deepEqual(p.row.events, ["contextmenu"]);
  assert.equal(p.send("touchend", p.row, []), true, "the release is no tap");
  p.send("touchstart", p.row, [[200, 200]]);
  const native = { isTrusted: true, prevented: false, stopped: false, preventDefault() { this.prevented = true; }, stopImmediatePropagation() { this.stopped = true; } };
  p.threadList.listeners.contextmenu[0](native);
  assert.deepEqual([native.prevented, native.stopped], [true, true], "the phone's own contextmenu for the same press is dropped");
});

test("the browser keeps a touch until a gesture claims its axis", () => {
  const p = phone();
  assert.deepEqual(p.swipe(chat, [10, 400], [100, 400]), [false, true, true, true, true, true, true, true, true, true]);
  assert.deepEqual(p.swipe(el({ inside: ["#composer"] }), [200, 400], [200, 600]), Array(10).fill(false));
});

test("a touch in a field or in a code block that can still scroll stays theirs", () => {
  const p = phone();
  assert.deepEqual(p.swipe(el({ inside: ["#chat", "textarea"] }), [10, 400], [300, 400]), Array(10).fill(false));
  const scrolled = el({ inside: ["#chat"], overflowX: "auto", scrollLeft: 40, scrollWidth: 900, clientWidth: 300 });
  assert.deepEqual(p.swipe(el({ inside: ["#chat"], parent: scrolled }), [10, 400], [300, 400]), Array(10).fill(false));
  assert.deepEqual(p.did, []);
});

test("a pull from the header opens the title menu and a left swipe over the chat opens the options", () => {
  const p = phone();
  p.swipe(el({ inside: ["#chat-head", "#chat"] }), [150, 30], [150, 130]);
  p.swipe(chat, [340, 400], [140, 400]);
  assert.deepEqual([p.title.clicks, p.headMore.clicks, p.did.length], [1, 1, 0]);
});
