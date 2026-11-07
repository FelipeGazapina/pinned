import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("./gui-gestures.js", import.meta.url), "utf8");

function el({ inside = [], parent = null, overflowX = "visible", scrollLeft = 0, scrollWidth = 0, clientWidth = 0 } = {}) {
  const classes = new Set();
  const props = new Map();
  return {
    inside, parentElement: parent, overflowX, overflowY: "visible", scrollLeft, scrollTop: 0, scrollWidth, clientWidth, offsetWidth: 360, offsetParent: {},
    clicks: 0,
    click() { this.clicks++; },
    closest(selectors) { return selectors.split(",").some((s) => this.inside.includes(s.trim())) ? this : null; },
    querySelector() { return this.title ?? null; },
    classList: { add: (...c) => c.forEach((x) => classes.add(x)), remove: (...c) => c.forEach((x) => classes.delete(x)), toggle: (c, on) => (on ? classes.add(c) : classes.delete(c)), contains: (c) => classes.has(c) },
    style: { setProperty: (k, v) => props.set(k, v), removeProperty: (k) => props.delete(k) },
  };
}

/** The gesture script on a phone with the sidebar shut, a thread open, and every timer run at once. */
function phone({ sidebarOpen = false } = {}) {
  const listeners = {};
  const workspace = el();
  workspace.addEventListener = (type, fn) => (listeners[type] = fn);
  const crumbEl = el({ inside: ["#chat-head"] });
  crumbEl.title = el({ inside: ["button.title"] });
  const headMore = el({ inside: ["#chat-head", "#chat"] });
  const toggles = [];
  const context = {
    workspace, crumbEl, sidebarEl: el({ inside: ["#sidebar"] }), sideMenu: { hidden: true }, active: {}, ui: { sidebarOpen },
    document: { getElementById: () => headMore, body: el() },
    matchMedia: () => ({ matches: false }), getComputedStyle: (e) => e,
    guiOn: () => true, narrow: () => true,
    toggleSidebar: (open) => (toggles.push(open), (context.ui.sidebarOpen = open)),
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
  return { swipe, toggles, title: crumbEl.title, headMore };
}
const chat = el({ inside: ["#chat"] });

test("a right swipe past 40% of the sidebar's width opens it, a shorter slow one leaves it shut", () => {
  const far = phone();
  far.swipe(chat, [20, 400], [170, 400]);
  assert.deepEqual(far.toggles, [true]);
  const short = phone();
  short.swipe(chat, [20, 400], [120, 400]);
  assert.deepEqual(short.toggles, []);
});

test("a fast flick opens the sidebar short of the reach", () => {
  const p = phone();
  p.swipe(chat, [20, 400], [80, 400], { steps: 3, stepMs: 16 });
  assert.deepEqual(p.toggles, [true]);
});

test("a left swipe on the open sidebar or its scrim closes it", () => {
  const onSidebar = phone({ sidebarOpen: true });
  onSidebar.swipe(el({ inside: ["#sidebar"] }), [300, 400], [100, 400]);
  assert.deepEqual(onSidebar.toggles, [false]);
});

test("the browser keeps a touch until a gesture claims its axis", () => {
  const p = phone();
  assert.deepEqual(p.swipe(chat, [20, 400], [100, 400]), [false, true, true, true, true, true, true, true, true, true]);
  assert.deepEqual(p.swipe(el({ inside: ["#composer"] }), [200, 400], [200, 600]), Array(10).fill(false));
});

test("a touch in a field or in a code block that can still scroll stays theirs", () => {
  const p = phone();
  assert.deepEqual(p.swipe(el({ inside: ["#chat", "textarea"] }), [20, 400], [300, 400]), Array(10).fill(false));
  const scrolled = el({ inside: ["#chat"], overflowX: "auto", scrollLeft: 40, scrollWidth: 900, clientWidth: 300 });
  assert.deepEqual(p.swipe(el({ inside: ["#chat"], parent: scrolled }), [20, 400], [300, 400]), Array(10).fill(false));
  assert.deepEqual(p.toggles, []);
});

test("a pull from the header opens the title menu and a left swipe over the chat opens the options", () => {
  const p = phone();
  p.swipe(el({ inside: ["#chat-head", "#chat"] }), [150, 30], [150, 130]);
  p.swipe(chat, [340, 400], [140, 400]);
  assert.deepEqual([p.title.clicks, p.headMore.clicks, p.toggles.length], [1, 1, 0]);
});
