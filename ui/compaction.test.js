import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const here = (value) => JSON.parse(JSON.stringify(value));

// The file's consts stay in its script scope, so a bridge hands its API out.
const pure = () => {
  const context = {};
  vm.createContext(context);
  vm.runInContext(`${read("compaction.js")}\nthis.api = { COMPACT_DEFAULT_TOKENS, COMPACT_CHOICES, compactionLabel, compactionOptions, compactionChoice, mergeCompaction, compactionWrites, compactOfferTokens };`, context);
  return context.api;
};

const NOW = Date.parse("2026-11-24T12:00:00Z");
const cold = (tokens) => ({ tokens, at: new Date(NOW - 71 * 60_000).toISOString() });

test("the page offers never and 50k to 500k tokens, 200k by default", () => {
  const { COMPACT_DEFAULT_TOKENS, COMPACT_CHOICES, compactionOptions } = pure();
  assert.equal(COMPACT_DEFAULT_TOKENS, 200_000);
  assert.deepEqual(here(COMPACT_CHOICES).map(([tokens]) => tokens), [null, 50_000, 100_000, 150_000, 200_000, 300_000, 500_000]);
  assert.equal(
    compactionOptions(200_000),
    '<option value="never">Never</option><option value="50000">From 50k tokens</option><option value="100000">From 100k tokens</option><option value="150000">From 150k tokens</option><option value="200000" selected>From 200k tokens</option><option value="300000">From 300k tokens</option><option value="500000">From 500k tokens</option>',
  );
  assert.match(compactionOptions(null), /<option value="never" selected>Never<\/option>/);
});

test("a value a machine holds that the page does not offer is listed as it is, and selected", () => {
  const { compactionOptions, compactionLabel } = pure();
  assert.equal(compactionLabel(75_000), "From 75k tokens");
  assert.match(compactionOptions(75_000), /<option value="75000" selected>From 75k tokens<\/option>$/);
  assert.equal(compactionLabel(12_345), "From 12345 tokens");
});

test("a select's never is null and the rest are tokens", () => {
  const { compactionChoice } = pure();
  assert.equal(compactionChoice("never"), null);
  assert.equal(compactionChoice("150000"), 150_000);
});

test("the account's view is the first machine that answered, and the default when none did", () => {
  const { mergeCompaction } = pure();
  assert.deepEqual(here(mergeCompaction([])), { offer_tokens: 200_000 });
  assert.deepEqual(here(mergeCompaction([{ id: "mb", online: false, settings: null }])), { offer_tokens: 200_000 }, "no machine answered");
  const view = mergeCompaction([
    { id: "mb", online: false, settings: null },
    { id: "mm", online: true, settings: { offer_tokens: 100_000 } },
    { id: "vps", online: true, settings: { offer_tokens: 500_000 } },
  ]);
  assert.deepEqual(here(view), { offer_tokens: 100_000 });
  assert.deepEqual(here(mergeCompaction([{ id: "mm", online: true, settings: { offer_tokens: null } }])), { offer_tokens: null }, "never is an answer, not a gap");
});

test("a change is written to every online machine that can take it", () => {
  const { compactionWrites } = pure();
  const machines = [
    { id: "mb", online: true, update: false },
    { id: "mm", online: true, update: true },
    { id: "vps", online: false, update: false },
    { id: "mini", online: true, update: false },
  ];
  assert.deepEqual(here(compactionWrites({ offer_tokens: null }, machines)), [
    { machine: "mb", settings: { offer_tokens: null } },
    { machine: "mini", settings: { offer_tokens: null } },
  ]);
});

test("a cold context is offered from the threshold, never at null, and from 200k when none is given", () => {
  const { compactOfferTokens } = pure();
  assert.equal(compactOfferTokens(cold(59_999), 50_000, NOW), 59_999);
  assert.equal(compactOfferTokens(cold(49_999), 50_000, NOW), null);
  assert.equal(compactOfferTokens(cold(900_000), null, NOW), null, "never");
  assert.equal(compactOfferTokens(cold(199_999), undefined, NOW), null);
  assert.equal(compactOfferTokens(cold(200_000), undefined, NOW), 200_000, "the default");
});

test("a context under 70 minutes old is not offered, its cache still warm", () => {
  const { compactOfferTokens } = pure();
  const warm = { tokens: 300_000, at: new Date(NOW - 69 * 60_000).toISOString() };
  assert.equal(compactOfferTokens(warm, 50_000, NOW), null);
  assert.equal(compactOfferTokens({ tokens: 300_000, at: new Date(NOW - 70 * 60_000).toISOString() }, 50_000, NOW), 300_000, "exactly 70 minutes");
});

/** gui-composer.js's own compactOffer and the threshold helper it reads, over machines whose
 * compaction answers this test holds, and an ask the test answers or fails. */
function composer(entries) {
  const src = read("gui-composer.js");
  const start = src.indexOf("function compactThreshold");
  const end = src.indexOf('const compactPill = document.getElementById("compact-pill");');
  assert.ok(start >= 0 && end > start, "compactOffer is in gui-composer.js");
  const asks = [];
  const renders = [];
  const context = {
    machines: new Map(Object.entries(entries)),
    active: null,
    contextOf: (tab) => tab.context,
    phaseOf: (tab) => tab.phase ?? "idle",
    renderComposer: (tab) => renders.push(tab.machine),
    callOn: (machine, call) => new Promise((resolve, reject) => asks.push({ machine, call, resolve, reject })),
  };
  vm.createContext(context);
  vm.runInContext(`${read("compaction.js")}\n${src.slice(start, end)}\nthis.compactOffer = compactOffer;`, context);
  return { context, asks, renders };
}
const recent = (tokens) => ({ tokens, at: new Date(Date.now() - 71 * 60_000).toISOString() });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("the composer offers from the threshold of the tab's machine", () => {
  const { context, asks } = composer({ fifty: { compaction: { offer_tokens: 50_000 } }, never: { compaction: { offer_tokens: null } } });
  const tab = (machine, tokens, more = {}) => ({ machine, context: recent(tokens), ...more });
  assert.equal(context.compactOffer(tab("fifty", 60_000)), 60_000);
  assert.equal(context.compactOffer(tab("fifty", 49_999)), null);
  assert.equal(context.compactOffer(tab("never", 900_000)), null, "never offered");
  assert.equal(context.compactOffer(tab("fifty", 60_000, { menu: {} })), null, "not while claude shows a menu");
  assert.equal(context.compactOffer(tab("fifty", 60_000, { phase: "working" })), null, "not while it works");
  assert.equal(context.compactOffer({ machine: "fifty", context: null }), null, "no context yet");
  assert.deepEqual(asks, [], "machines that answered are not asked again");
});

test("a machine that has not answered counts as 200k, is asked once, and its answer redraws the composer", async () => {
  const { context, asks, renders } = composer({ mini: {} });
  const tab = (tokens) => ({ machine: "mini", context: recent(tokens) });
  context.active = tab(0);
  assert.equal(context.compactOffer(tab(60_000)), null);
  assert.equal(context.compactOffer(tab(199_999)), null);
  assert.equal(context.compactOffer(tab(200_000)), 200_000, "the default meanwhile");
  assert.deepEqual(here(asks.map(({ machine, call }) => ({ machine, call }))), [{ machine: "mini", call: { cmd: "compaction_settings" } }], "one ask");
  asks[0].resolve({ offer_tokens: 50_000 });
  await settle();
  assert.deepEqual(renders, ["mini"], "the active tab's composer is drawn again");
  assert.equal(context.compactOffer(tab(60_000)), 60_000);
});

test("a machine whose ask failed counts as 200k and is not asked again until it is back online", async () => {
  const { context, asks } = composer({ old: {} });
  const tab = (tokens) => ({ machine: "old", context: recent(tokens) });
  context.compactOffer(tab(60_000));
  asks[0].reject(new Error("unknown variant `compaction_settings`"));
  await settle();
  assert.equal(context.compactOffer(tab(60_000)), null);
  assert.equal(context.compactOffer(tab(200_000)), 200_000);
  assert.equal(asks.length, 1);
  delete context.machines.get("old").compaction;
  context.compactOffer(tab(60_000));
  assert.equal(asks.length, 2, "showMachines dropped its answer, so it is asked again");
});
