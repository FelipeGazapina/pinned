import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// The cost helpers sit between statsLabel and statsHtml; the vm gets the few globals they read.
function load() {
  const src = readFileSync(new URL("./gui-sidebar.js", import.meta.url), "utf8");
  const durationLabel = src.slice(src.indexOf("const durationLabel"), src.indexOf("/** The thread's active time and cost"));
  const helpers = src.slice(src.indexOf("function statsLabel"), src.indexOf("function statsHtml"));
  const context = { statusOf: () => "idle", nowS: () => 0 };
  vm.createContext(context);
  vm.runInContext(`${durationLabel}\n${helpers}\nthis.api = { statsLabel, costOf, projectCostLabel, costLabel };`, context);
  return context.api;
}
const tab = (view) => ({ key: "m:tab", view });

test("a Captain's card shows the cost of its session and its workers, with how many", () => {
  const { statsLabel } = load();
  const captain = tab({ stats: { active_ms: 72 * 60000, cost_usd: 0.84 }, work: { usd: 1.26, unknown: 0, workers: 2 } });
  assert.equal(statsLabel(captain), "1h 12m · $1.26 · 2 workers");
  assert.equal(statsLabel(tab({ stats: { active_ms: 60000, cost_usd: 0.84 }, work: { usd: 0.9, unknown: 0, workers: 1 } })), "1m · $0.90 · 1 worker");
});

test("a total with a worker of unknown cost is a floor, marked with a plus", () => {
  const { statsLabel } = load();
  assert.equal(statsLabel(tab({ stats: { active_ms: 60000, cost_usd: 0.2 }, work: { usd: 0.5, unknown: 1, workers: 3 } })), "1m · $0.50+ · 3 workers");
});

test("a thread that delegated nothing still shows its own cost", () => {
  const { statsLabel } = load();
  assert.equal(statsLabel(tab({ stats: { active_ms: 5000, cost_usd: 0.42 } })), "5s · $0.42");
  assert.equal(statsLabel(tab({})), "");
});

test("a project adds its top-level threads once: a Captain's total already holds its workers", () => {
  const { costOf, projectCostLabel } = load();
  const captain = tab({ stats: { cost_usd: 0.84 }, work: { usd: 1.26, unknown: 0, workers: 2 } });
  const loose = tab({ stats: { cost_usd: 0.1 } });
  const idle = tab({});
  assert.deepEqual({ ...costOf([captain, loose, idle]) }, { usd: 1.36, unknown: 0, workers: 2 });
  assert.equal(projectCostLabel([captain, loose, idle]), "$1.36 · 2 workers");
  assert.equal(projectCostLabel([idle]), "");
  assert.equal(projectCostLabel([tab({ stats: { cost_usd: 0.003 } })]), "<$0.01");
});

test("a phone's header gets the cost alone, with the plus for a floor, and nothing when none is known", () => {
  const { statsLabel } = load();
  assert.equal(statsLabel(tab({ stats: { active_ms: 72 * 60000, cost_usd: 0.84 }, work: { usd: 1.26, unknown: 0, workers: 2 } }), true), "$1.26");
  assert.equal(statsLabel(tab({ stats: { active_ms: 60000, cost_usd: 0.2 }, work: { usd: 0.5, unknown: 1, workers: 3 } }), true), "$0.50+");
  assert.equal(statsLabel(tab({ stats: { active_ms: 5000 } }), true), "");
});
