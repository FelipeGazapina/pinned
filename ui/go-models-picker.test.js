import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
const line = (start) => {
  const from = src.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  return src.slice(from, src.indexOf("\n", from));
};
const decl = (start) => {
  const from = src.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  const close = src.slice(from).search(/\n\}\n/);
  assert.ok(close > 0, `missing the end of ${start}`);
  return src.slice(from, from + close + 2);
};

/** The picker's HTML for `models`, carried by the account as `list_accounts` sends it. */
function pickerHtml(models) {
  const list = { innerHTML: "", querySelectorAll: () => [] };
  const context = {
    machines: new Map([["book", { accounts: [{ id: "go", provider: "claude", auth: "opencode-go", name: "personal", signed_in: true, models }] }]]),
    DEFAULT_ACCOUNT: "claude",
    modelPicker: { provider: "go", search: "", legacy: false, highlight: 0 },
    modelFavorites: [],
    modelOf: () => null,
    esc: (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/"/g, "&quot;"),
    icon: () => "",
    markModelJumps: () => {},
    composerMenu: { hidden: false, dataset: { kind: "models" }, querySelectorAll: () => [], querySelector: (sel) => (sel === ".list" ? list : null) },
  };
  vm.createContext(context);
  vm.runInContext(
    `${line("const modelsForAccount")}\n${decl("function modelAccounts")}\n${line("const providerLabel")}\n${decl("function renderModels")}\nglobalThis.renderModels = renderModels;`,
    context,
  );
  context.renderModels({ machine: "book", view: { account: "go" } });
  return list.innerHTML;
}

/** What the gateway refresh writes to go_models_file: ClaudeModel as serde lays it out. */
const FILE = JSON.stringify([
  { slug: "fresh-model", name: "Fresh Model", efforts: ["low", "high", "xhigh"], context: 1000000, cost: "high", legacy: false, new: false, fast: false, free: false },
  { slug: "beam-free", name: "Beam Free", efforts: ["low", "medium", "high", "xhigh", "max"], context: 1048576, cost: "low", legacy: false, new: false, fast: false, free: true },
]);

test("a refreshed gateway list reaches the model rows, including a model the page's own table lacks", () => {
  const html = pickerHtml(JSON.parse(FILE));
  const rows = [...html.matchAll(/data-row="([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual(rows, ["fresh-model", "beam-free"]);
  assert.match(html, /<span class="free" aria-label="Free model">Free<\/span>/);
  assert.equal((html.match(/aria-label="Free model"/g) ?? []).length, 1, "only the free model carries the badge");
});

test("a cached go_models_file lists the same models with the free badge when the gateway never answers", () => {
  const html = pickerHtml(JSON.parse(FILE));
  assert.deepEqual([...html.matchAll(/data-row="([^"]+)"/g)].map((m) => m[1]), ["fresh-model", "beam-free"]);
  assert.match(html, /data-row="beam-free"[^>]*>.*?<span class="free" aria-label="Free model">Free<\/span>/);
  assert.match(html, /data-row="fresh-model"[^>]*>.*?<span>Fresh Model<\/span>/);
});
