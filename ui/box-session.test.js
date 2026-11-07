import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const sidebar = readFileSync(new URL("./gui-sidebar.js", import.meta.url), "utf8");
const adding = readFileSync(new URL("./gui-add-project.js", import.meta.url), "utf8");

function runSlice(source, startAt, endAt, extra = "") {
  const start = source.indexOf(startAt);
  const end = source.indexOf(endAt, start + startAt.length);
  const context = {
    console,
    icon: (name) => `<i data-icon="${name}"></i>`,
    BOX_USD_PER_HOUR: 0.1,
  };
  vm.createContext(context);
  // Top-level consts don't attach to the sandbox; export the helpers under test.
  const exportHelpers = `;Object.assign(globalThis, { ...(typeof boxAgeText !== "undefined" ? { boxAgeText, boxChipHtml, boxSpent } : {}), ...(typeof parseBoxBudget !== "undefined" ? { parseBoxBudget, boxHoursText } : {}) });`;
  vm.runInContext(`${source.slice(start, end)}\n${exportHelpers}\n${extra}`, context);
  return context;
}

test("box ages read as minutes, hours and days", () => {
  const context = runSlice(sidebar, "const boxAgeText", "const boxInfo");
  assert.equal(context.boxAgeText(0), "0m");
  assert.equal(context.boxAgeText(45 * 60), "45m");
  assert.equal(context.boxAgeText(83 * 60), "1h23");
  assert.equal(context.boxAgeText(50 * 3600), "2d2h");
});

test("the sidebar chip shows age and spend against the budget", () => {
  const context = runSlice(sidebar, "const boxAgeText", "const boxInfo");
  const chip = context.boxChipHtml({ age_secs: 83 * 60, budget_usd: 2, rate_usd_per_hour: 0.1 });
  assert.match(chip, /1h23 · \$0\.14/);
  assert.match(chip, /of \$2\.00/);
  const free = context.boxChipHtml({ age_secs: 60, budget_usd: null, rate_usd_per_hour: 0.1 });
  assert.match(free, /1m · \$0\.00/);
  assert.doesNotMatch(free, /of \$/);
});

test("budgets parse to cents and preview hours", () => {
  const context = runSlice(adding, "const BOX_SIZES", 'PALETTE_VIEWS["add-box-budget"]');
  assert.equal(context.parseBoxBudget("2"), 2);
  assert.equal(context.parseBoxBudget("$2.50"), 2.5);
  assert.equal(context.parseBoxBudget("0.05"), null, "below an hour");
  assert.equal(context.parseBoxBudget("much"), null);
  assert.equal(context.boxHoursText(2), "20h");
  assert.equal(context.boxHoursText(0.5), "5h");
});

test("the sources list a box once upstash is connected", () => {
  const start = adding.indexOf('PALETTE_VIEWS["add-sources"]');
  const end = adding.indexOf("function pickSource");
  // PALETTE_VIEWS needs a home in the sandbox.
  const withViews = (upstash) => {
    const context = {
      console,
      icon: (name) => `<i data-icon="${name}"></i>`,
      PALETTE_VIEWS: {},
      palette: { adding: { machine: "m", github: {}, upstash } },
      filterGroups: (groups) => groups,
      pickSource: () => {},
      newProjectOn: () => {},
      browseForProject: () => {},
    };
    vm.createContext(context);
    vm.runInContext(adding.slice(start, end), context);
    return context.PALETTE_VIEWS["add-sources"].groups("").flatMap((g) => g.items);
  };
  const on = withViews({ key_hint: "…a1b2" });
  const box = on.find((i) => i.title === "Box");
  assert.ok(box && !box.disabled, "box is offered");
  const off = withViews(null);
  const gated = off.find((i) => i.title === "Box");
  assert.ok(gated?.disabled, "box asks for setup");
  assert.match(gated.trail, /Setup Required/);
});
