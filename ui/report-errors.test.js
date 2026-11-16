import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./report-errors.js", import.meta.url), "utf8");

/** report-errors.js in a window of its own: `fire` plays the page's events, `logged` is what reached the real console.error. */
function page() {
  const handlers = {};
  const logged = [];
  const context = {
    window: { addEventListener: (name, fn) => (handlers[name] = fn) },
    console: { error: (...a) => logged.push(a) },
    Date,
    Error,
    JSON,
    String,
  };
  vm.createContext(context);
  vm.runInContext(src, context);
  return {
    errors: vm.runInContext("reportErrors", context),
    fire: (name, event) => handlers[name](event),
    log: (...a) => context.console.error(...a),
    logged,
  };
}
const bare = (lines) => Array.from(lines, (l) => l.replace(/^\d\d:\d\d:\d\d /, ""));

test("console.error, window errors and unhandled rejections each leave one line, and console.error still logs", () => {
  const p = page();
  p.log("diff: view dropped", new Error("empty result"));
  p.fire("error", { message: "x is not a function", filename: "http://127.0.0.1/ui/gui-pr.js", lineno: 41 });
  p.fire("unhandledrejection", { reason: "lost the connection to the hub" });
  assert.deepEqual(bare(p.errors.last(10)), ["diff: view dropped empty result", "x is not a function (gui-pr.js:41)", "Unhandled rejection: lost the connection to the hub"]);
  assert.equal(p.logged.length, 1);
});

test("the ring keeps the last 50 lines, each cut to 300 characters on one line", () => {
  const p = page();
  for (let i = 1; i <= 60; i++) p.log(`line ${i}`);
  p.log("a\n".repeat(400));
  assert.equal(p.errors.count(), 50);
  assert.equal(bare(p.errors.last(50))[0], "line 12");
  const long = bare(p.errors.last(1))[0];
  assert.ok(long.length <= 300 && !long.includes("\n"));
  assert.deepEqual(bare(p.errors.last(2)).slice(0, 1), ["line 60"]);
});
