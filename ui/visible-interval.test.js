import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const main = readFileSync(new URL("./main.js", import.meta.url), "utf8");
const start = main.indexOf("const visibleTimers");
const slice = main.slice(start, main.indexOf("const machines = new Map();", start));

/** Runs the helper against a fake clock and a fake document whose visibility the test flips. */
function page({ hidden = false } = {}) {
  const clock = { now: 0, timers: new Map(), next: 1 };
  const listeners = [];
  const document = { hidden, addEventListener: (type, fn) => type === "visibilitychange" && listeners.push(fn) };
  const api = new Function("document", "setInterval", "clearInterval", "setTimeout", `${slice}; return { visibleInterval };`)(
    document,
    (fn, ms) => {
      const id = clock.next++;
      clock.timers.set(id, { fn, ms, at: clock.now + ms });
      return id;
    },
    (id) => clock.timers.delete(id),
    (fn) => {
      const id = clock.next++;
      clock.timers.set(id, { fn, ms: 0, at: clock.now, once: true });
      return id;
    },
  );
  const advance = (ms) => {
    const end = clock.now + ms;
    for (;;) {
      const due = [...clock.timers].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      const [id, timer] = due;
      clock.now = timer.at;
      if (timer.once) clock.timers.delete(id);
      else timer.at += timer.ms;
      timer.fn();
    }
    clock.now = end;
  };
  const show = (value) => {
    document.hidden = value;
    listeners.forEach((fn) => fn());
  };
  return { ...api, advance, show, live: () => clock.timers.size };
}

test("an interval beats while the page is visible, sleeps while it is hidden, and runs once on return", () => {
  const p = page();
  let beats = 0;
  p.visibleInterval(() => beats++, 1000);
  p.advance(3000);
  assert.equal(beats, 3);
  p.show(true);
  assert.equal(p.live(), 0, "no timer is left running while hidden");
  p.advance(60_000);
  assert.equal(beats, 3, "nothing fires while hidden");
  p.show(false);
  p.advance(0);
  assert.equal(beats, 4, "one catch-up beat as the page shows");
  p.advance(2000);
  assert.equal(beats, 6, "and the normal beat after it");
});

test("an interval made while the page is hidden waits for it to show", () => {
  const p = page({ hidden: true });
  let beats = 0;
  p.visibleInterval(() => beats++, 500);
  p.advance(5000);
  assert.equal(beats, 0);
  p.show(false);
  p.advance(1000);
  assert.equal(beats, 3, "a catch-up beat, then two of 500 ms");
});
