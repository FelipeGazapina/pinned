import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

// The Capitão and workers section renders from the machine's settings and saves through it.
function harness(stored = null, failing = false) {
  const els = {};
  const element = (id) => (els[id] ??= { value: "", onchange: null, textContent: "" });
  const invokeCalls = [];
  const context = {
    TAURI: {},
    COMPUTER: "Mac",
    invoke: async (cmd, args) => {
      invokeCalls.push([cmd, args]);
      if (cmd === "get_orchestration") {
        if (failing) throw new Error("no machine");
        return stored ?? { style: "balanced", max_workers: 100 };
      }
      return null;
    },
    load: () => null,
    save: () => {},
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    console: { warn: () => {} },
    document: { getElementById: (id) => element(id) },
  };
  vm.createContext(context);
  vm.runInContext(read("gui-orchestration.js"), context);
  return { context, invokeCalls, element };
}

test("the section shows the machine style and worker cap", async () => {
  const { context } = harness({ style: "quality", max_workers: 40 });
  const html = await context.orchestrationSettingsHtml();
  assert.match(html, /<h2 class="looks-title">Capitão and workers<\/h2>/);
  assert.match(html, /value="quality" selected/);
  assert.match(html, /value="40"/);
});

test("saving clamps the cap and clears the error", async () => {
  const { context, invokeCalls, element } = harness();
  context.bindOrchestrationSettings();
  element("orchestration-style").value = "economical";
  element("orchestration-max").value = "99999999";
  await element("orchestration-style").onchange();
  const [cmd, args] = invokeCalls.at(-1);
  assert.equal(cmd, "set_orchestration");
  assert.equal(args.style, "economical");
  assert.equal(args.max_workers, 10000);
  assert.equal(element("orchestration-max").value, 10000);
  assert.equal(element("orchestration-error").textContent, "");
});

test("when the machine cannot answer it still renders defaults", async () => {
  const { context } = harness(null, true);
  const html = await context.orchestrationSettingsHtml();
  assert.match(html, /Max workers/);
  assert.match(html, /value="balanced" selected/);
});
