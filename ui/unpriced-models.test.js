import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const usageSrc = read("gui-usage.js");
const between = (src, from, to) => {
  const start = src.indexOf(from);
  assert.ok(start >= 0, `${from} is in the source`);
  return src.slice(start, src.indexOf(to, start));
};
const line = (src, prefix) => src.split("\n").find((l) => l.startsWith(prefix));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const plain = (value) => JSON.parse(JSON.stringify(value));
const text = (html) => html.replace(/<[^>]*>/g, " ").replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/\s+/g, " ").trim();

const NOW = 1793491210;
const DAY = 86400;
const UNPRICED = { model: "glm-5.3", at: NOW - 3600, input: 400, cached: 500, cache_write: 0, output: 100, cost: null };
const PRICED = { model: "claude-opus-5-5", at: NOW - 3600, input: 1000, cached: 2000, cache_write: 0, output: 500, cost: { input: 0.01, cache_read: 0.002, cache_write: 0, output: 0.03 } };
const book = (prices = {}, aliases = {}) => ({ machine: "macbook", label: "MacBook", prices, aliases, unavailable: null });

/** The prices block as the dialog runs it, over one machine's rows and price books. */
function page({ rows, targets, online = true, loaded = true, loading = false }) {
  const context = {
    esc: (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"),
    machineName: (m) => (m === "macbook" ? "MacBook" : m === "macmini" ? "Mac mini" : m),
    envOnline: () => online,
    usageState: { now: NOW, envs: new Map([["macbook", { rows, loaded, loading }]]) },
    priceDialog: { next: 0, selected: null, drafts: [] },
  };
  vm.createContext(context);
  vm.runInContext([
    line(usageSrc, "const rowTokens = "),
    fn(usageSrc, "tokensText"),
    fn(usageSrc, "usageBuckets"),
    line(usageSrc, "const emptyDraft = "),
    between(usageSrc, "const PRICE_FIELDS = [", "const priceForm = "),
    line(usageSrc, "const priceForm = "),
    fn(usageSrc, "parsePriceForm"),
    line(usageSrc, "const modelPrice = "),
    line(usageSrc, "const modelAlias = "),
    fn(usageSrc, "priceChanges"),
    between(usageSrc, "/** The models that ran in the last 30 days", "/** The open prices dialog:"),
    "this.updateDraft = (draft) => { priceDialog.drafts = priceDialog.drafts.filter((d) => d.id !== draft.id).concat(draft); };",
    "this.unpricedModels = unpricedModels; this.unpricedHtml = unpricedHtml; this.addUnpricedRow = addUnpricedRow; this.priceChanges = priceChanges;",
  ].join("\n"), context);
  context.targets = targets;
  return context;
}

test("lists exactly the model with no price, its machine and its tokens", () => {
  const p = page({ rows: [PRICED, UNPRICED, { ...UNPRICED, at: NOW - 40 * DAY }], targets: [book()] });
  assert.deepEqual(plain(p.unpricedModels(p.targets)), [{ model: "glm-5.3", machine: "macbook", tokens: 1000, unpriced: 1 }]);
  const shown = text(p.unpricedHtml(p.targets));
  assert.match(shown, /No price yet · last 30 days/);
  assert.match(shown, /Ran in the last 30 days on MacBook with no price in the book\./);
  assert.match(shown, /glm-5\.3 MacBook · 1K tokens/);
  assert.doesNotMatch(shown, /claude-opus-5-5/);
});

test("a model in the price book is not listed, nor is an alias whose target is priced", () => {
  const rows = [
    UNPRICED,
    { ...UNPRICED, model: "glm-alias" },
    { ...UNPRICED, model: "ghost-alias" },
  ];
  const p = page({ rows, targets: [book({ "glm-5.3": { input: 0.3, output: 1.2 } }, { "glm-alias": "glm-5.3", "ghost-alias": "kimi-k3.1-pro" })] });
  assert.deepEqual(plain(p.unpricedModels(p.targets).map((m) => m.model)), ["ghost-alias"]);
  const none = page({ rows: [UNPRICED, { ...UNPRICED, model: "glm-alias" }], targets: [book({ "glm-5.3": { input: 0.3, output: 1.2 } }, { "glm-alias": "glm-5.3" })] });
  assert.deepEqual(plain(none.unpricedModels(none.targets)), []);
});

test("the button adds the model on its machine, and saving writes its input and output", () => {
  const p = page({ rows: [UNPRICED], targets: [book()] });
  p.addUnpricedRow("macbook", "glm-5.3");
  assert.deepEqual(plain(p.priceDialog.drafts), [{ id: "new:0", model: "glm-5.3", isNew: true, values: {} }]);
  assert.deepEqual([...p.priceDialog.selected], ["macbook"]);
  const typed = [{ ...plain(p.priceDialog.drafts)[0], values: { input: "0.4", output: "1.2" } }];
  const { patch, errors, empty } = p.priceChanges(p.targets[0], typed);
  assert.deepEqual(plain(patch), { prices: { "glm-5.3": { input: 0.4, output: 1.2 } }, aliases: {} });
  assert.equal(errors.size, 0);
  assert.equal(empty, false);
});

test("draws no section when nothing is unpriced", () => {
  const p = page({ rows: [PRICED], targets: [book()] });
  assert.equal(p.unpricedHtml(p.targets), "");
  const priced = page({ rows: [UNPRICED], targets: [book({ "glm-5.3": { input: 0.3, output: 1.2 } })] });
  assert.equal(priced.unpricedHtml(priced.targets), "");
});

test("a partial scan marks the list partial instead of presenting it as complete", () => {
  const partial = page({ rows: [UNPRICED], targets: [book()], loading: true });
  assert.match(text(partial.unpricedHtml(partial.targets)), /Partial: MacBook still scanning\./);
  const complete = page({ rows: [UNPRICED], targets: [book()] });
  assert.doesNotMatch(text(complete.unpricedHtml(complete.targets)), /Partial/);
  const offlineBook = page({ rows: [UNPRICED], targets: [book(), { machine: "macmini", label: "Mac mini", prices: null, aliases: null, unavailable: "Offline" }] });
  assert.match(text(offlineBook.unpricedHtml(offlineBook.targets)), /Partial: no prices from Mac mini\./);
});
