import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

// The Captain and workers section renders from the machine's settings and saves through it.
function harness({ stored = null, failing = false, app = true } = {}) {
  const els = {};
  const element = (id) => (els[id] ??= { value: "", onchange: null, onclick: null, textContent: "" });
  const invokeCalls = [];
  const pages = [];
  const context = {
    TAURI: app ? {} : null,
    COMPUTER: "Mac",
    invoke: async (cmd, args) => {
      invokeCalls.push([cmd, args]);
      if (cmd === "get_orchestration") {
        if (failing) throw new Error("no machine");
        return stored ?? { max_workers: 100 };
      }
      return null;
    },
    openPage: (name) => pages.push(name),
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    console: { warn: () => {} },
    document: { getElementById: (id) => element(id) },
  };
  vm.createContext(context);
  vm.runInContext(read("gui-orchestration.js"), context);
  return { context, invokeCalls, element, pages };
}

test("the section shows the worker cap and the way to the Way of working page", async () => {
  const { context } = harness({ stored: { max_workers: 40 } });
  const html = await context.orchestrationSettingsHtml();
  assert.match(html, /<h2 class="looks-title">Captain and workers<\/h2>/);
  assert.match(html, /<label for="orchestration-taste">Way of working<\/label><p class="meta">Profiles, models and pace for the orchestrator and the Captains.<\/p>/);
  assert.match(html, /id="orchestration-max" min="1" max="10000" value="40"/);
  assert.doesNotMatch(html, /Cost style/);
});

// Tauri hands a command its arguments by their camelCase names, so `max_workers` arrives as `maxWorkers`.
test("saving clamps the cap and clears the error", async () => {
  const { context, invokeCalls, element } = harness();
  context.bindOrchestrationSettings();
  element("orchestration-error").textContent = "earlier failure";
  element("orchestration-max").value = "99999999";
  await element("orchestration-max").onchange();
  assert.deepEqual(JSON.parse(JSON.stringify(invokeCalls.at(-1))), ["set_orchestration", { maxWorkers: 10000 }]);
  assert.equal(element("orchestration-max").value, 10000);
  assert.equal(element("orchestration-error").textContent, "");
});

test("Open leaves settings for the Way of working page", () => {
  const { context, element, pages } = harness();
  context.bindOrchestrationSettings();
  element("orchestration-taste").onclick();
  assert.deepEqual(pages, ["taste"]);
});

test("when the machine cannot answer it still renders the default cap", async () => {
  const { context } = harness({ failing: true });
  const html = await context.orchestrationSettingsHtml();
  assert.match(html, /id="orchestration-max" min="1" max="10000" value="100"/);
});

test("in the browser the section keeps only the Way of working row", async () => {
  const { context, invokeCalls } = harness({ app: false });
  const html = await context.orchestrationSettingsHtml();
  assert.match(html, /id="orchestration-taste">Open<\/button>/);
  assert.doesNotMatch(html, /Max workers/);
  assert.deepEqual(invokeCalls, []);
});
