import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");

function harness() {
  const els = {};
  const element = (id) => (els[id] ??= { onclick: null });
  const pages = [];
  const context = { openPage: (name) => pages.push(name), document: { getElementById: (id) => element(id) } };
  vm.createContext(context);
  vm.runInContext(read("gui-orchestration.js"), context);
  return { context, element, pages };
}

test("the section leads to the Way of working page, which holds the delegation limits", async () => {
  const { context } = harness();
  const html = await context.orchestrationSettingsHtml();
  assert.match(html, /<h2 class="looks-title">Captain and workers<\/h2>/);
  assert.match(html, /<label for="orchestration-taste">Way of working<\/label><p class="meta">Profiles, models, pace and delegation limits for the orchestrator and the Captains.<\/p>/);
  assert.doesNotMatch(html, /Max workers|orchestration-max/, "the per-machine cap moved into the account's workers at once");
});

test("Open leaves settings for the Way of working page", () => {
  const { context, element, pages } = harness();
  context.bindOrchestrationSettings();
  element("orchestration-taste").onclick();
  assert.deepEqual(pages, ["taste"]);
});
