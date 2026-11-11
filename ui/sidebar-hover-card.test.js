import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function load() {
  const src = readFileSync(new URL("./gui-sidebar.js", import.meta.url), "utf8");
  const start = src.indexOf("const cardFitsBeside");
  const end = src.indexOf("function showCard");
  const context = {};
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}\nglobalThis.cardFitsBeside = cardFitsBeside;`, context);
  return context.cardFitsBeside;
}

test("a row card opens beside a row only when it fits in the window", () => {
  const fits = load();
  assert.equal(fits(700, 320, 1200), true);
  assert.equal(fits(480, 320, 520), false);
  assert.equal(fits(868, 320, 1200), true);
  assert.equal(fits(869, 320, 1200), false);
});

test("Thread details names the machine, the box's age and cost, the account and its provider, which the hover card shows only to a mouse", () => {
  const sidebar = readFileSync(new URL("./gui-sidebar.js", import.meta.url), "utf8");
  const composer = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
  const between = (src, start, end) => src.slice(src.indexOf(start), src.indexOf(end, src.indexOf(start)));
  const context = {
    LOCAL: "local",
    DEFAULT_ACCOUNT: "claude",
    PROVIDERS: { claude: "Claude Code", codex: "Codex", "opencode-go": "OpenCode Go" },
    machines: new Map([["box-1", { view: { name: "Box · release" } }], ["mini", { view: { name: "mac-mini" } }]]),
    machineName: (id) => (id === "local" ? "This Mac" : context.machines.get(id).view.name),
    accountName: (machine, id) => ({ claude: "Claude Code", go: "Go plan" })[id],
    accountOf: (machine, id) => ({ go: { provider: "claude", auth: "opencode-go" } })[id] ?? null,
    icon: (name) => `<i ${name}></i>`,
    esc: (text) => String(text),
  };
  vm.createContext(context);
  vm.runInContext(`${between(sidebar, "const boxMachineOf", "const boxUnavailableHtml")}\nconst boxInfo = new Map([["box-1", { age_secs: 7500, rate_usd_per_hour: 0.1, budget_usd: 5 }]]);\n${between(composer, "function detailsRunsOnHtml", "function renderDetails")}`, context);
  const lines = (tab) => [...context.detailsRunsOnHtml(tab).matchAll(/data-detail="(\w+)"><i (\w+)><\/i><span class="label">([^<]*)<\/span>(?:<span class="dim">([^<]*)<\/span>)?/g)].map((m) => m.slice(1).filter(Boolean).join(" | "));
  assert.deepEqual(lines({ machine: "local", view: {} }), ["machine | monitor | This Mac", "account | claude | Claude Code | Claude Code"]);
  assert.deepEqual(lines({ machine: "box-1", view: { account: "go" } }), ["machine | monitor | Box · release", "box | server | 2h05 online · $0.21 of $5.00", "account | opencode | Go plan | OpenCode Go"]);
  vm.runInContext(`boxDown.add("box-1"); boxInfo.delete("box-1");`, context);
  assert.deepEqual(lines({ machine: "box-1", view: {} }).slice(1, 2), ["box | server | Box cost unavailable"]);
});
