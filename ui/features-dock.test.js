import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

/** gui-features.js decides the window level and the Dock presence together. */
function loadFeatures({ stored = null, failWindows = false } = {}) {
  const calls = [];
  const saved = {};
  const box = { checked: true, onchange: null };
  const context = {
    COMPUTER: "Mac",
    TAURI: {
      window: {
        getCurrentWindow: () => ({
          setAlwaysOnTop: async (on) => {
            calls.push(["always", on]);
            if (failWindows) throw new Error("no window");
          },
        }),
      },
    },
    load: () => stored,
    save: (key, value) => {
      saved[key] = value;
    },
    invoke: async (cmd, args) => {
      calls.push([cmd, args]);
    },
    console: { warn: () => {} },
    document: {
      getElementById: (id) => (id === "feature-alwaysOnTop" ? box : null),
    },
  };
  vm.createContext(context);
  vm.runInContext(readFileSync(new URL("./gui-features.js", import.meta.url), "utf8"), context);
  return { context, calls, saved, box };
}

const settle = () => new Promise((resolve) => setImmediate(resolve));
/** Calls cross the vm boundary, so plain objects come back with another prototype. */
const plain = (calls) => JSON.parse(JSON.stringify(calls));

test("boot without always on top shows the app in the Dock", async () => {
  const { calls } = loadFeatures({ stored: JSON.stringify({ alwaysOnTop: false }) });
  await settle();
  await settle();
  assert.deepEqual(plain(calls), [
    ["always", false],
    ["set_dock_shown", { shown: true }],
  ]);
});

test("boot with always on top hides the Dock presence", async () => {
  const { calls } = loadFeatures({ stored: JSON.stringify({ alwaysOnTop: true }) });
  await settle();
  await settle();
  assert.deepEqual(plain(calls), [
    ["always", true],
    ["set_dock_shown", { shown: false }],
  ]);
});

test("toggling always on top flips the Dock with it and saves", async () => {
  const { context, calls, saved, box } = loadFeatures({ stored: JSON.stringify({ alwaysOnTop: true }) });
  await settle();
  await settle();
  calls.length = 0;
  context.bindFeaturesSettings();
  box.checked = false;
  await box.onchange();
  assert.deepEqual(plain(calls), [
    ["always", false],
    ["set_dock_shown", { shown: true }],
  ]);
  assert.equal(JSON.parse(saved["gui-features"]).alwaysOnTop, false);
});

test("a window failure reverts the toggle and saves nothing", async () => {
  const { context, calls, saved, box } = loadFeatures({ failWindows: true });
  await settle();
  await settle();
  calls.length = 0;
  context.bindFeaturesSettings();
  box.checked = false;
  await box.onchange();
  assert.equal(box.checked, true);
  assert.deepEqual(saved, {});
});
