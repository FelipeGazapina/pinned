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
  };
  vm.createContext(context);
  // Top-level consts don't attach to the sandbox; export the helpers under test.
  const exportHelpers = `;Object.assign(globalThis, { ...(typeof boxAgeText !== "undefined" ? { boxAgeText, boxChipHtml, boxSpent } : {}), ...(typeof parseBoxBudget !== "undefined" ? { parseBoxBudget, boxHoursText, boxRate } : {}) });`;
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
  const context = runSlice(adding, "const BOX_SIZE_CHOICES", 'PALETTE_VIEWS["add-box-budget"]');
  assert.equal(context.parseBoxBudget("2", "small"), 2);
  assert.equal(context.parseBoxBudget("$2.50", "small"), 2.5);
  assert.equal(context.parseBoxBudget("0.05", "small"), null, "below an hour");
  assert.equal(context.parseBoxBudget("much", "small"), null);
  assert.equal(context.parseBoxBudget("0.15", "small"), 0.15);
  assert.equal(context.boxHoursText(2, "small"), "20h");
  assert.equal(context.boxHoursText(0.5, "small"), "5h");
});

test("a budget needs an hour on its size and previews the hours at that size's rate", () => {
  const context = runSlice(adding, "const BOX_SIZE_CHOICES", 'PALETTE_VIEWS["add-box-budget"]');
  assert.deepEqual(["small", "medium", "large"].map(context.boxRate), [0.1, 0.2, 0.4]);
  assert.equal(context.parseBoxBudget("0.15", "medium"), null, "under an hour of Medium");
  assert.equal(context.parseBoxBudget("0.2", "medium"), 0.2);
  assert.equal(context.parseBoxBudget("0.3", "large"), null, "under an hour of Large");
  assert.equal(context.parseBoxBudget("0.4", "large"), 0.4);
  assert.equal(context.boxHoursText(2, "medium"), "10h");
  assert.equal(context.boxHoursText(2, "large"), "5h");
  assert.equal(context.parseBoxBudget("0.2"), 0.2, "no size yet reads as the default Medium");
  assert.equal(context.parseBoxBudget("0.15"), null);
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

test("github and box search the account's repositories and look up any other owner/repo", async () => {
  const start = adding.indexOf("const REPOSITORY_ROWS");
  const end = adding.indexOf("/** A repo picked for a box");
  const repos = [
    { full_name: "felipe/pinned", clone_url: "https://github.com/felipe/pinned.git", private: true, description: "harness" },
    { full_name: "felipe/notes", clone_url: "https://github.com/felipe/notes.git", private: false, description: "" },
    { full_name: "acme/pinned-docs", clone_url: "https://github.com/acme/pinned-docs.git", private: false, description: "docs" },
  ];
  const context = {
    console,
    icon: (name) => `<i data-icon="${name}"></i>`,
    esc: (s) => s,
    PALETTE_VIEWS: {},
    GITHUB_SHORTHAND: /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})\/[A-Za-z0-9._-]+(?:\.git)?$/,
    palette: { hi: -1, shown: [], adding: { machine: "m", source: "box" } },
    connectorCall: async (cmd) => (cmd === "github_repos" ? repos : null),
    paletteOpen: () => true,
    paletteView: () => ({ view: "add-repository", query: "" }),
    renderPalette: () => {},
    filterGroups: (groups, q) => groups.map((g) => ({ ...g, items: g.items.filter((i) => i.terms.join(" ").includes(q.trim())) })).filter((g) => g.items.length),
    tell: () => {},
    confirmed: [],
    lookups: [],
  };
  vm.createContext(context);
  vm.runInContext(`${adding.slice(start, end)}
    confirmBox = (c) => confirmed.push(["box", c]);
    confirmClone = (c) => confirmed.push(["clone", c]);
    lookupGithub = async (typed) => (lookups.push(typed), { nameWithOwner: typed, url: "https://github.com/" + typed + ".git" });
    runPaletteItem = (item) => item.run();
    Object.assign(globalThis, { loadGithubRepos });`, context);
  const view = context.PALETTE_VIEWS["add-repository"];
  const type = (q) => {
    view.onQuery(q);
    return [...(context.palette.shown = view.groups(q).flatMap((g) => g.items))];
  };
  const highlighted = () => context.palette.shown[context.palette.hi]?.fullName ?? null;

  assert.equal(view.empty(""), "Loading your repositories...");
  context.loadGithubRepos(context.palette.adding);
  await new Promise((r) => setTimeout(r));
  assert.deepEqual(type("").map((i) => i.title), ["felipe/pinned", "felipe/notes", "acme/pinned-docs"]);
  assert.deepEqual(type("pinned").map((i) => i.title), ["felipe/pinned", "acme/pinned-docs"]);
  assert.equal(context.palette.shown[0].sub, "Private · harness");
  assert.equal(highlighted(), "felipe/pinned", "Enter's row is the one highlighted");

  await view.submit("pinned");
  assert.equal(context.confirmed.at(-1)[0], "box");
  assert.equal(context.confirmed.at(-1)[1].remoteUrl, "https://github.com/felipe/pinned.git");

  type("acme/pinned-docs");
  assert.equal(highlighted(), "acme/pinned-docs");
  await view.submit("acme/pinned-docs");
  assert.equal(context.confirmed.at(-1)[1].remoteUrl, "https://github.com/acme/pinned-docs.git");
  assert.equal(context.lookups.length, 0);

  type("acme/pinned");
  assert.equal(highlighted(), null, "nothing highlighted, so Enter looks it up");
  await view.submit("acme/pinned");
  assert.deepEqual([...context.lookups], ["acme/pinned"], "an owner/repo outside the list is looked up, not swapped for a near match");

  context.palette.adding.source = "github";
  type("notes");
  await view.submit("notes");
  assert.equal(context.confirmed.at(-1)[0], "clone");
  assert.equal(context.confirmed.at(-1)[1].remoteUrl, "https://github.com/felipe/notes.git");
});
