import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const fixture = JSON.parse(read("taste.fixture.json"));
const { economical, balanced } = fixture.presets;
const SITE = "github.com/acme/site";
const tastesWith = (over = {}) => ({ orchestrator: balanced, captains: balanced, projects: {}, ...over });
const esc = (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const unesc = (text) => text.replace(/&#(\d+);/g, (_, code) => String.fromCharCode(Number(code)));

const group = (key, label, over = {}) => ({ key, label, runs: 0, done: 0, failed: 0, limited: 0, cancelled: 0, accepted: 0, redone: 0, wall_ms: 0, median_wall_ms: 0, cost_usd: 0, priced: 0, tokens_in: 0, tokens_out: 0, box_usd: 0, box_ms: 0, ...over });
const noStats = (days) => ({ days, since: 0, totals: group("all", "All tasks"), by_model: [], by_provider: [], by_project: [], by_delegator: [], by_outcome: [], slowest: [], priciest: [] });
const openTabs = ["m-mini/tab-7"];

/** The page in a vm: `answer(cmd, args)` plays the backend, and each control is found in the HTML
 * the page last drew, so a click only reaches what a user could see. */
function page({ answer = () => tastesWith(), projects = {}, machineViews = null, skills = {}, chat = async () => "tab-1", stats = () => noStats(7) } = {}) {
  const listeners = {};
  const threads = [];
  const drawn = [];
  const calls = [];
  const opened = [];
  const timers = [];
  const context = {
    document: { body: { dataset: { page: "taste" } }, activeElement: null },
    pageView: { addEventListener: (type, fn) => (listeners[type] = [...(listeners[type] ?? []), fn]), contains: () => false, querySelector: () => null, querySelectorAll: () => [] },
    SHELL_PAGES: {},
    setPage: (html) => drawn.push(html),
    headHtml: (title, rest = "") => `<header><h1>${title}</h1>${rest}</header>`,
    icon: (name) => `<svg data-icon="${name}"></svg>`,
    esc,
    connectorCall: async (cmd, args = {}) => {
      if (cmd === "delegation_stats") return stats(args.days);
      calls.push([cmd, JSON.parse(JSON.stringify(args))]);
      return answer(cmd, args);
    },
    tabByKey: (key) => (openTabs.includes(key) ? { key } : null),
    keyOf: (machine, tab) => `${machine}/${tab}`,
    tell: () => {},
    onlineMachines: () => Object.keys(projects),
    LOCAL: "local",
    machines: new Map(Object.entries(machineViews ?? Object.fromEntries(Object.keys(projects).map((id) => [id, { online: true, name: id }]))).map(([id, view]) => [id, { view }])),
    machineName: (id) => id,
    globalHostMachine: () => Object.keys(projects)[0] ?? "mm",
    globalSettings: { name: "Atlas" },
    callOn: async (machine, call) => {
      if (call.cmd === "skills") return skills[machine] ?? null;
      if (call.cmd === "open_taste_chat") return (opened.push([machine, JSON.parse(JSON.stringify(call))]), chat(machine, call));
      return projects[machine];
    },
    revealTab: async (machine, id) => (threads.push([machine, id]), { machine, id }),
    startTab: (tab) => opened.push(["start", tab]),
    claudeModels: () => [
      { slug: "claude-opus-5-5", name: "Claude Opus 5.5" },
      { slug: "claude-sonnet-5-5", name: "Claude Sonnet 5.5" },
      { slug: "claude-opus-5", name: "Claude Opus 5", legacy: true },
    ],
    OPENCODE_GO_MODELS: [
      { slug: "glm-5.3", name: "GLM-5.3", legacy: true },
      { slug: "deepseek-v4.1-flash", name: "DeepSeek V4.1 Flash", legacy: true },
    ],
    setTimeout: (fn) => timers.push(fn),
    clearTimeout: () => {},
  };
  vm.createContext(context);
  const skillsSrc = read("gui-skills.js").split("\n");
  const line = (start) => skillsSrc.find((l) => l.startsWith(start));
  vm.runInContext(`${line("const baseName = ")}\n${line("const scopeLabel = ")}\n${read("global-orchestrator.js")}\n${read("taste.js")}\n${read("delegation.js")}\n${read("gui-taste.js")}\n${read("gui-delegation.js")}`, context);
  const html = () => drawn.at(-1);
  const control = (pattern) => {
    const tag = [...html().matchAll(/<(?:button|select|input|form)\b[^>]*>/g)].map((m) => m[0]).find((t) => pattern.test(t));
    assert.ok(tag, `the page draws a control matching ${pattern}`);
    const dataset = Object.fromEntries([...tag.matchAll(/\bdata-([\w-]+)(?:="([^"]*)")?/g)].map(([, k, v]) => [k.replace(/-(\w)/g, (_, c) => c.toUpperCase()), unesc(v ?? "")]));
    return { dataset, disabled: /\sdisabled[\s>]/.test(tag) };
  };
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  const fire = (type, event) => listeners[type].forEach((fn) => fn(event));
  return {
    context,
    calls,
    opened,
    threads,
    timers,
    html,
    control,
    settle,
    show: async () => (context.SHELL_PAGES.taste.show(), settle()),
    click: async (pattern) => (fire("click", { target: { closest: () => control(pattern) } }), settle()),
    place: async (list, value) => (fire("change", { target: { value, dataset: { placeAdd: list }, matches: (sel) => sel === "[data-place-add]" } }), settle()),
    pick: (attr, value) => fire("change", { target: { value, matches: (sel) => sel === `[${attr}]` } }),
    pickEffort: async (target) => (fire("change", { target }), settle()),
    type: (value) => fire("input", { target: { value, matches: (sel) => sel === "[data-note-input]" } }),
    submit: () => fire("submit", { target: { matches: (sel) => sel === "[data-note-form]" }, preventDefault() {} }),
    submitSkill: (name) => fire("submit", { target: { matches: (sel) => sel === "[data-skill-form]", querySelector: () => ({ value: name }) }, preventDefault() {} }),
  };
}

const balancedBrief = () => fixture.briefs[1].brief.split("\n");
const lines = (html) => [...html.matchAll(/<div class="taste-line( flash)?">([^<]*)<\/div>/g)].map(([, flash, text]) => `${flash ? "* " : ""}${unesc(text)}`);

test("the page opens on the Captains with their brief, cost and speed, and a tab per project", async () => {
  const p = page({
    projects: {
      mm: [{ path: "/p/site", name: "site", repo: SITE }, { path: "/p/scratch", name: "scratch" }],
      mb: [{ path: "/q/api", name: "api", repo: "github.com/acme/api" }, { path: "/q/site", name: "site", repo: SITE }],
    },
  });
  await p.show();
  const html = p.html();
  assert.deepEqual([...html.matchAll(/data-act="scope"[^>]*aria-pressed="(\w+)">([^<]+)</g)].map(([, on, label]) => `${label}${on === "true" ? " (on)" : ""}`), [
    "Orchestrator",
    "Captains (all) (on)",
    "Captain api",
    "Captain site",
  ]);
  assert.match(html, /<button type="button" class="taste-card active" data-act="profile" data-profile="balanced" aria-pressed="true"><b>Balanced<\/b>/);
  assert.match(html, /<h2>What the Captain receives<\/h2>/);
  assert.deepEqual(lines(html), balancedBrief());
  assert.match(html, /title="A rough estimate from each model's cost tier">\$0\.70–1\.46</);
  assert.match(html, /aria-label="Speed 2 of 3"/);
  assert.doesNotMatch(html, /taste-foot/);
  assert.doesNotMatch(html, /inherited|changed here/);
});

test("Loading shows while the first read runs, and an older hub asks for an update", async () => {
  let fail;
  const p = page({ answer: () => new Promise((_, reject) => (fail = reject)) });
  p.context.SHELL_PAGES.taste.show();
  assert.match(p.html(), /<p class="taste-state">Loading…<\/p>/);
  fail(new Error("this hub runs pinned 0.17.20, which can't answer that: unknown variant `get_tastes`"));
  await p.settle();
  assert.match(p.html(), /<p class="taste-state error">Update pinned on the hub to change the way of working.<\/p>/);
});

test("an edit flashes the lines it changed, makes the profile Custom and shows the footer", async () => {
  const p = page();
  await p.show();
  await p.click(/aria-label="More workers at once"/);
  const html = p.html();
  assert.deepEqual(lines(html), ["* Run up to 6 workers at once.", ...balancedBrief().slice(1)]);
  assert.match(html, /<div class="taste-card custom active"><b>Custom<\/b>/);
  assert.match(html, /<span class="taste-unsaved">Unsaved changes<\/span>/);
  await p.click(/data-act="profile" data-profile="balanced"/);
  assert.deepEqual(lines(p.html()), ["* Run up to 5 workers at once.", ...balancedBrief().slice(1)]);
  assert.doesNotMatch(p.html(), /taste-foot/, "back to the saved tastes, nothing to save");
});

test("a project tab marks only the fields changed there, and Reset inherits again", async () => {
  const p = page({ answer: () => tastesWith({ projects: { [SITE]: { parallel: 6 } } }) });
  await p.show();
  await p.click(/data-act="scope" data-kind="project"/);
  let html = p.html();
  assert.match(html, /<span class="taste-quiet">Inherits from Captains \(all\)\.<\/span>/);
  assert.match(html, /<span>Workers at once<\/span><span class="taste-tag changed">changed here<\/span><button type="button" class="taste-reset" data-act="reset" data-field="parallel"/);
  assert.equal(html.match(/changed here/g).length, 1, "only the project's own field is tagged");
  assert.doesNotMatch(html, /inherited/);
  assert.match(html, /<div class="taste-label"><span>Strategy<\/span><\/div>/);
  assert.equal(lines(html)[0], "Run up to 6 workers at once.");
  await p.click(/data-act="reset" data-field="parallel"/);
  html = p.html();
  assert.match(html, /<div class="taste-label"><span>Workers at once<\/span><\/div>/);
  assert.doesNotMatch(html, /changed here/);
  assert.equal(lines(html)[0], "* Run up to 5 workers at once.");
  assert.match(html, /aria-pressed="true"[^>]*>Captain site</, "an offline project's tab stays open once its patch is gone");
  assert.match(html, /Unsaved changes/);
});

test("the orchestrator's own name heads the page and its preview", async () => {
  const p = page();
  await p.show();
  assert.match(p.html(), /<p class="taste-sub">How the Atlas and the Captains split work, pick models and ask for your input.<\/p>/);
  await p.click(/data-act="scope" data-kind="orchestrator"/);
  assert.match(p.html(), /<h2>What the Atlas receives<\/h2>/);
});

test("Chat to adjust opens a thread on the tab's scope, in the project's folder when it has one", async () => {
  const p = page({ projects: { mm: [{ path: "/p/scratch", name: "scratch" }], mb: [{ path: "/q/site", name: "site", repo: SITE }] } });
  await p.show();
  assert.match(p.html(), /<button type="button" class="outline taste-chat" data-act="chat"><svg data-icon="message-square"><\/svg>Chat to adjust<\/button>/);
  await p.click(/class="outline taste-chat"/);
  await p.click(/data-act="scope" data-kind="project"/);
  await p.click(/class="outline taste-chat"/);
  assert.deepEqual(p.opened, [
    ["mm", { cmd: "open_taste_chat", scope: { kind: "captains" } }],
    ["start", { machine: "mm", id: "tab-1" }],
    ["mb", { cmd: "open_taste_chat", scope: { kind: "project", repo: SITE }, path: "/q/site" }],
    ["start", { machine: "mb", id: "tab-1" }],
  ]);
});

test("Chat to adjust on a machine that predates it asks to update that machine", async () => {
  const p = page({ projects: { mm: [] }, chat: async () => Promise.reject(new Error("unknown variant `open_taste_chat`")) });
  await p.show();
  await p.click(/class="outline taste-chat"/);
  assert.match(p.html(), /<span class="taste-error" role="alert">Update pinned on mm to chat about the way of working.<\/span>/);
  assert.doesNotMatch(p.html(), /Opening…/);
});

test("Skills lists each skill with where it lives, toggles it, and adds one the machines have", async () => {
  const p = page({
    answer: () => tastesWith({ captains: { ...balanced, skills: [{ name: "tdd", enabled: true }] } }),
    projects: { mm: [] },
    skills: { mm: { copies: [
      { key: "tdd", scope: { kind: "personal" }, description: "Write the failing test first." },
      { key: "deploy", scope: { kind: "plugin", plugin: "ops" }, description: "Ship it." },
    ] } },
  });
  await p.show();
  let html = p.html();
  assert.match(html, /<code>tdd<\/code><span class="taste-chip">Personal<\/span><\/span><span class="taste-skill-about">Write the failing test first.<\/span>/);
  assert.match(html, /aria-checked="true" aria-label="Use the tdd skill" data-act="skill" data-skill="0"/);
  assert.match(html, /data-act="adding">Add skill<\/button><button type="button" class="taste-link" data-act="chat">Create by chatting<\/button>/);
  await p.click(/aria-label="Use the tdd skill"/);
  assert.match(p.html(), /aria-checked="false" aria-label="Use the tdd skill"/);
  await p.click(/data-act="adding"/);
  assert.match(p.html(), /<select data-skill-pick><option value="deploy">deploy<\/option><\/select>/, "only skills not on the list yet");
  p.submitSkill("deploy");
  html = p.html();
  assert.doesNotMatch(html, /data-skill-form/);
  assert.match(html, /<code>deploy<\/code><span class="taste-chip">Plugin ops<\/span>/);
  assert.match(html, /aria-checked="true" aria-label="Use the deploy skill"/);
});

test("a session's change reaches the open page without a reload", async () => {
  let stored = tastesWith();
  const p = page({ answer: () => stored });
  await p.show();
  assert.equal(lines(p.html())[0], "Run up to 5 workers at once.");
  stored = tastesWith({ captains: { ...balanced, parallel: 2 } });
  p.context.tastesChanged();
  await p.settle();
  assert.equal(lines(p.html())[0], "Run up to 2 workers at once.");
  assert.doesNotMatch(p.html(), /Unsaved changes/);
});

test("Save sends the changes in order, takes the last answer and shows the toast", async () => {
  const answers = {
    get_tastes: tastesWith({ projects: { [SITE]: { parallel: 6 } } }),
    set_taste: tastesWith({ captains: { ...balanced, second_opinion: true }, projects: { [SITE]: { parallel: 6 } } }),
    reset_taste_field: tastesWith({ orchestrator: { ...balanced, parallel: 1 }, captains: { ...balanced, second_opinion: true } }),
  };
  const p = page({ answer: (cmd) => answers[cmd], projects: { mm: [{ path: "/p/site", name: "site", repo: SITE }] } });
  await p.show();
  await p.click(/aria-label="Second opinion on risky changes"/);
  await p.click(/data-act="scope" data-kind="project"/);
  await p.click(/data-act="reset" data-field="parallel"/);
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls.slice(1), [
    ["set_taste", { scope: { kind: "captains" }, patch: { second_opinion: true } }],
    ["reset_taste_field", { scope: { kind: "project", repo: SITE }, field: "parallel" }],
  ]);
  const html = p.html();
  assert.doesNotMatch(html, /taste-foot/);
  assert.match(html, /<span class="taste-toast" role="status"><svg data-icon="check"><\/svg>Saved. New sessions start with it.<\/span>/);
  assert.match(html, /aria-checked="true" aria-label="Second opinion on risky changes"/);
  assert.equal(lines(html).at(-1), "Have another model review a risky change before it goes ahead.");
  assert.equal(p.timers.length, 1, "the toast goes away by itself");
  await p.click(/data-act="scope" data-kind="orchestrator"/);
  assert.equal(lines(p.html())[0], "Run one worker at a time.", "the backend's answer is the new draft");
});

test("a failed save keeps the draft, says why, and the next save sends only what is left", async () => {
  let refuse = true;
  const p = page({
    answer: (cmd, args) => {
      if (cmd === "get_tastes") return tastesWith();
      if (args.scope.kind === "captains" && refuse) throw new Error("the model ladder needs at least one model");
      return tastesWith({ orchestrator: economical });
    },
  });
  await p.show();
  await p.click(/data-act="scope" data-kind="orchestrator"/);
  await p.click(/data-profile="economical"/);
  await p.click(/data-act="scope" data-kind="captains"/);
  await p.click(/aria-label="Less workers at once"/);
  await p.click(/data-act="save"/);
  let html = p.html();
  assert.match(html, /<span class="taste-unsaved">Unsaved changes<\/span><span class="taste-error" role="alert">the model ladder needs at least one model<\/span>/);
  assert.equal(lines(html)[0], "Run up to 4 workers at once.", "the draft stands");
  refuse = false;
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls.slice(1).map(([cmd, args]) => `${cmd} ${args.scope.kind}`), ["set_taste orchestrator", "set_taste captains", "set_taste captains"]);
});

test("a reload keeps unsaved edits over what another device saved meanwhile, and a save sends only the edits", async () => {
  let stored = tastesWith({ projects: { [SITE]: { parallel: 6 } } });
  const p = page({ answer: (cmd, args) => (cmd === "get_tastes" ? stored : stored), projects: { mm: [{ path: "/p/site", name: "site", repo: SITE }] } });
  await p.show();
  await p.click(/data-act="scope" data-kind="orchestrator"/);
  await p.click(/data-act="set" data-field="autonomy" data-value="ask_first"/);
  stored = tastesWith({ captains: { ...balanced, parallel: 7 }, projects: { [SITE]: { parallel: 6, context: ["Releases go out on Fridays."] } } });
  await p.show();
  await p.click(/data-act="scope" data-kind="captains"/);
  assert.equal(lines(p.html())[0], "Run up to 7 workers at once.", "the other device's Captains change shows");
  assert.match(p.html(), /Unsaved changes/, "the orchestrator's edit is still unsaved");
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls.filter(([cmd]) => cmd !== "get_tastes"), [["set_taste", { scope: { kind: "orchestrator" }, patch: { autonomy: "ask_first" } }]]);
});

test("the ladder adds from the catalog, moves and removes rungs, and keeps the last one", async () => {
  const p = page({ answer: () => tastesWith({ captains: { ...balanced, ladder: [{ model: "glm-5.3" }] } }) });
  await p.show();
  assert.ok(p.control(/aria-label="Remove GLM-5.3 from the ladder"/).disabled);
  assert.match(p.html(), /<option value="glm-5.3" title="glm-5.3" disabled>GLM-5.3<\/option>/);
  p.pick("data-add-model", "claude-opus-5-5");
  await p.click(/aria-label="Move Claude Opus 5.5 up"/);
  const names = () => [...p.html().matchAll(/<span class="name" title="([^"]+)">([^<]+)</g)].map(([, slug, name]) => `${name} (${slug})`);
  assert.deepEqual(names(), ["Claude Opus 5.5 (claude-opus-5-5)", "GLM-5.3 (glm-5.3)"]);
  assert.equal(lines(p.html())[1], "* Start each worker on claude-opus-5-5, and when it fails twice move it one rung up the ladder: glm-5.3.");
  await p.click(/aria-label="Remove Claude Opus 5.5 from the ladder"/);
  assert.deepEqual(names(), ["GLM-5.3 (glm-5.3)"]);
  p.pick("data-hard-model", "grok-4.7\ncursor");
  assert.match(p.html(), /<option value="grok-4.7\ncursor" selected>grok-4.7 through cursor<\/option>/);
});

test("a context note joins the brief and keeps what was typed across other edits", async () => {
  const p = page();
  await p.show();
  p.type("  Releases go out on Fridays. ");
  await p.click(/aria-label="Review each diff before accepting"/);
  assert.match(p.html(), /data-note-input value="  Releases go out on Fridays. "/);
  p.submit();
  const html = p.html();
  assert.match(html, /<li><span>Releases go out on Fridays.<\/span><button type="button" class="taste-icon" data-act="unnote" data-note="0" aria-label="Remove note">/);
  assert.equal(lines(html).at(-1), "* Releases go out on Fridays.");
  assert.match(html, /data-note-input value=""/);
});

test("showing the page again keeps a dirty draft over a fresh read, and a failed read says so in the footer", async () => {
  let read = () => tastesWith();
  const p = page({ answer: () => read() });
  await p.show();
  await p.click(/aria-label="More workers at once"/);
  read = () => tastesWith({ captains: { ...balanced, autonomy: "ask_first" } });
  await p.show();
  assert.equal(lines(p.html())[0], "Run up to 6 workers at once.");
  assert.match(p.html(), /Unsaved changes/);
  await p.click(/data-act="discard"/);
  assert.equal(lines(p.html())[3], "Ask the user before you open a pull request or delete data.", "Discard goes back to the fresh read");
  read = () => Promise.reject(new Error("hub offline"));
  await p.show();
  assert.match(p.html(), /<footer class="taste-foot"><span class="taste-error" role="alert">hub offline<\/span>/);
  assert.equal(lines(p.html())[0], "Run up to 5 workers at once.", "the last read stays on screen");
});

const limits = { tier: "high", default_effort: null, max_effort: "max", budget_cents: 0, time_limit_min: 480, box_budget_cents: 0 };
const limited = (over = {}) => tastesWith({ orchestrator: { ...balanced, ...limits }, captains: { ...balanced, ...limits }, ...over });

test("the Captains' delegation limits show their values, and editing them saves only what changed", async () => {
  const p = page({ answer: (cmd, args) => (cmd === "get_tastes" ? limited() : limited({ captains: { ...balanced, ...limits, ...args.patch } })) });
  await p.show();
  let html = p.html();
  assert.match(html, /<h2>Delegation limits<\/h2><p>pinned holds every worker to these. A call past them is refused, and a worker past its time or budget is stopped.<\/p>/);
  assert.match(html, /data-field="tier" data-value="high" aria-pressed="true"/);
  assert.match(html, /<span>Time limit per worker<\/span>(?:(?!<output).)*<output aria-live="polite">8 h<\/output>/s);
  assert.match(html, /<span>Budget per worker<\/span>(?:(?!<output).)*<output aria-live="polite">No cap<\/output>/s);
  assert.ok(p.control(/aria-label="Less budget per worker"/).disabled, "no cap is the lowest a budget goes");
  assert.match(html, /<select data-effort="default_effort"><option value="" selected>The model's own<\/option>/);

  await p.click(/data-field="tier" data-value="mid"/);
  await p.click(/aria-label="Less time per worker"/);
  await p.click(/aria-label="More budget per worker"/);
  await p.click(/aria-label="More budget per worker"/);
  await p.pickEffort({ value: "medium", dataset: { effort: "default_effort" }, matches: (sel) => sel === "[data-effort]" });
  html = p.html();
  assert.match(html, /<output aria-live="polite">6 h<\/output>/);
  assert.match(html, /<output aria-live="polite">US\$ 1.00<\/output>/);
  assert.match(html, /class="taste-card active" data-act="profile" data-profile="balanced"/, "a limit other than workers at once leaves the profile as it was");
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls.at(-1), ["set_taste", { scope: { kind: "captains" }, patch: { tier: "mid", default_effort: "medium", budget_cents: 100, time_limit_min: 360 } }]);
});

test("lowering the most effort below the default brings the default down", async () => {
  const p = page({ answer: () => limited({ captains: { ...balanced, ...limits, default_effort: "xhigh" } }) });
  await p.show();
  const target = { value: "high", dataset: { effort: "max_effort" }, matches: (sel) => sel === "[data-effort]" };
  await p.pickEffort(target);
  const html = p.html();
  assert.match(html, /<select data-effort="default_effort">(?:(?!<\/select>).)*<option value="high" selected>/s);
  assert.match(html, /<select data-effort="max_effort">(?:(?!<\/select>).)*<option value="high" selected>/s);
});

test("a project tab marks a limit changed here, and the orchestrator tab keeps only workers at once", async () => {
  const p = page({ answer: () => limited({ projects: { [SITE]: { box_budget_cents: 300 } } }) });
  await p.show();
  await p.click(/data-act="scope" data-kind="project"/);
  let html = p.html();
  assert.match(html, /<span>Box budget<\/span><span class="taste-tag changed">changed here<\/span><button type="button" class="taste-reset" data-act="reset" data-field="box_budget_cents"/);
  assert.match(html, /<output aria-live="polite">US\$ 3.00<\/output>/);
  assert.match(html, /<div class="taste-label"><span>Model tier<\/span><\/div>/, "an inherited limit carries no tag");
  await p.click(/data-act="scope" data-kind="orchestrator"/);
  html = p.html();
  assert.match(html, /The orchestrator starts no workers itself\. Each Captain follows the limits under Captains \(all\) or its project\./);
  assert.doesNotMatch(html, /data-field="tier"/);
  assert.match(html, /aria-label="More workers at once"/, "the orchestrator's brief still names its workers at once");
});

test("a hub too old for delegation limits says so instead of drawing empty fields", async () => {
  const p = page();
  await p.show();
  assert.match(p.html(), /Update pinned on the hub to set the other delegation limits\./);
  assert.match(p.html(), /aria-label="More workers at once"/, "workers at once, which every hub has, stays");
});

const statsFor = (days) => ({
  ...noStats(days),
  totals: group("all", "All tasks", { runs: 5, done: 2, failed: 1, limited: 2, accepted: 1, redone: 1, median_wall_ms: 30 * 60000, cost_usd: 9.5, priced: 4, box_usd: 0.5, box_ms: 40 * 60000, tokens_in: 2_400_000, tokens_out: 52_000, limits: { time: 2 } }),
  by_model: [
    group("claude-opus-5-5", "claude-opus-5-5", { runs: 3, done: 1, limited: 2, median_wall_ms: 120 * 60000, cost_usd: 9, priced: 3, limits: { time: 2 } }),
    group("glm-5.3", "glm-5.3", { runs: 2, done: 1, failed: 1, median_wall_ms: 5 * 60000, cost_usd: 0.5, priced: 1, box_usd: 0.5, box_ms: 40 * 60000 }),
  ],
  by_outcome: [group("done", "Done", { runs: 2, done: 2 }), group("limit", "Stopped by a limit", { runs: 2, limited: 2, limits: { time: 2 } })],
  slowest: [
    { id: "task-1", title: "Profile the sync", machine: "m-mini", machine_name: "Mac mini", thread: "tab-7", model: "claude-opus-5-5", delegator: "Captain site", outcome: "limit", limit: "time", wall_ms: 120 * 60000, ended: 1 },
    { id: "task-2", title: "Old thread", machine: "m-book", machine_name: "MacBook", thread: "tab-1", model: "glm-5.3", delegator: "Global orchestrator", outcome: "done", wall_ms: 5 * 60000, ended: 1 },
  ],
});

test("delegation metrics show totals, a hint per breakdown naming the limit it tunes, and links to open threads", async () => {
  const asked = [];
  const p = page({ stats: (days) => (asked.push(days), statsFor(days)) });
  await p.show();
  const html = p.html();
  assert.deepEqual(asked, [7], "the page opens on 7 days");
  assert.match(html, /<h2>Delegation metrics<\/h2>/);
  assert.deepEqual([...html.matchAll(/<dt>([^<]+)<\/dt><dd>([^<]+)<\/dd>/g)].map(([, k, v]) => `${k}: ${unesc(v)}`), [
    "Tasks: 5",
    "Done: 2 · 40%",
    "Median time: 30 min",
    "Spend: $10.00",
    "Tokens in / out: 2.4M / 52k",
    "Accepted / redone: 1 / 1",
  ]);
  const hints = [...html.matchAll(/<li>([^<]+) <span class="delegation-tune">→ ([^<]+)<\/span><\/li>/g)].map(([, text, field]) => `${unesc(text)} → ${unesc(field)}`);
  assert.deepEqual(hints, [
    "2 of 3 claude-opus-5-5 tasks hit the time limit → Time limit",
    "1 of 2 glm-5.3 tasks failed → Tier",
    "$0.50 on boxes over 40 min → Upstash Box budget",
    "2 of 2 tasks hit the time limit → Time limit",
  ]);
  assert.match(html, /data-act="delegation-open" data-machine="m-mini" data-tab="tab-7">Profile the sync<\/button>/);
  assert.match(html, /<span class="delegation-title">Old thread<\/span>/, "a thread no longer open is not a link");
  await p.click(/data-act="delegation-open"/);
  assert.deepEqual(p.threads, [["m-mini", "tab-7"]]);
  await p.click(/data-act="delegation-days" data-days="30"/);
  assert.deepEqual(asked, [7, 30]);
  assert.match(p.html(), /data-days="30" aria-pressed="true">30 d</);
  assert.equal(p.calls.filter(([cmd]) => cmd !== "get_tastes").length, 0, "reading metrics saves nothing");
});

test("delegation metrics say when nothing ended and when the hub is too old", async () => {
  const empty = page();
  await empty.show();
  assert.match(empty.html(), /No delegated task ended in this period\./);
  const old = page({ stats: () => Promise.reject(new Error("unknown variant `delegation_stats`")) });
  await old.show();
  assert.match(old.html(), /Update pinned on the hub to see delegation metrics\./);
});

const MACHINES = { "m-mini": { online: true, name: "mac-mini", state: { workers: 2, workers_at_once: 5 } }, "m-vps": { online: false, name: "lypes-vps" }, "m-book": { online: true, name: "MacBook" } };
const withPlacement = (placement) => ({ answer: () => tastesWith({ placement }), machineViews: MACHINES });
const none = { prefer: [], avoid: [], heavy: "worktree" };
const listOf = (html, label) => {
  const [, body = ""] = new RegExp(`aria-label="${label}">(.*?)</(?:ol|ul)>`, "s").exec(html) ?? [];
  return [...body.matchAll(/<li[^>]*>.*?<span class="name"[^>]*>(?:<s>)?([^<]+)/gs)].map(([, name]) => name);
};

test("Placement lists Prefer in order and Avoid, by machine name, with each machine's state", async () => {
  const p = page(withPlacement({ prefer: ["m-vps", "m-mini"], avoid: ["m-book"], heavy: "worktree" }));
  await p.show();
  const html = p.html();
  assert.deepEqual(listOf(html, "Prefer"), ["lypes-vps", "mac-mini"]);
  assert.deepEqual(listOf(html, "Avoid"), ["MacBook"]);
  assert.match(html, /<span class="pos">1<\/span><span class="name" title="m-vps">lypes-vps<\/span><span class="taste-chip" data-state="offline">offline<\/span>/);
  assert.match(html, /data-state="online">online<\/span><span class="taste-quiet mach-meta">2 of 5 workers<\/span>/);
  assert.match(html, /data-act="pmove" data-id="m-vps" data-step="-1" aria-label="Move lypes-vps up" disabled/);
  assert.match(html, /aria-label="Move lypes-vps down">/);
  assert.match(html, /data-act="pheavy" data-value="worktree" aria-pressed="true">A worker in its own worktree/);
  assert.match(html, /If no machine fits, the worker runs on the machine that asked, and the reason is shown\./);
  assert.match(html, /CPU is over 90% for 1 minute or it has under 2 GB of memory free\./);
  assert.match(html, /A machine with under 10 GB of disk free takes no heavy worker\./);
  assert.doesNotMatch(html, /data-act="[^"]*"[^>]*data-field="when_none"|Fail and say why/, "falling back to the asking machine has no switch");
  assert.doesNotMatch(html, /Nothing is preferred or avoided/);
  await p.click(/data-act="pmove" data-id="m-mini" data-step="-1"/);
  assert.deepEqual(listOf(p.html(), "Prefer"), ["mac-mini", "lypes-vps"]);
});

test("an empty policy says every worker runs where it was asked", async () => {
  const p = page(withPlacement(none));
  await p.show();
  assert.match(p.html(), /Nothing is preferred or avoided, so every worker runs on the machine it was asked on\./);
  assert.deepEqual([listOf(p.html(), "Prefer"), listOf(p.html(), "Avoid")], [[], []]);
  assert.doesNotMatch(p.html(), /taste-foot/);
});

test("a machine the account lost shows struck out with a way to remove it", async () => {
  const p = page(withPlacement({ prefer: ["m-gone", "m-mini"], avoid: [], heavy: "worktree" }));
  await p.show();
  assert.match(p.html(), /<li class="gone"><span class="pos">1<\/span><span class="name" title="m-gone"><s>m-gone<\/s><\/span><span class="taste-chip">removed<\/span>/);
  assert.doesNotMatch(p.html(), /<s>mac-mini/);
  await p.click(/data-act="pdrop" data-list="prefer" data-id="m-gone"/);
  assert.deepEqual(listOf(p.html(), "Prefer"), ["mac-mini"]);
  p.calls.length = 0;
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls[0], ["set_taste", { scope: { kind: "captains" }, patch: { placement: { prefer: ["m-mini"], avoid: [], heavy: "worktree" } } }]);
});

test("a machine can't be added to Prefer and Avoid at once", async () => {
  const p = page(withPlacement({ prefer: ["m-mini"], avoid: ["m-book"], heavy: "worktree" }));
  await p.show();
  const avoidPicker = /<select data-place-add="avoid">(.*?)<\/select>/s.exec(p.html())[1];
  assert.match(avoidPicker, /<option value="m-mini" disabled>mac-mini \(in Prefer\)<\/option>/);
  assert.doesNotMatch(avoidPicker, /<option value="m-book"/, "a machine already in the list is not offered again");
  assert.match(avoidPicker, /<option value="m-vps">lypes-vps<\/option>/);
  await p.place("avoid", "m-mini");
  assert.deepEqual([listOf(p.html(), "Prefer"), listOf(p.html(), "Avoid")], [["mac-mini"], ["MacBook"]]);
  assert.doesNotMatch(p.html(), /taste-foot/, "the refused pick changed nothing");
  await p.place("prefer", "m-book");
  assert.deepEqual([listOf(p.html(), "Prefer"), listOf(p.html(), "Avoid")], [["mac-mini"], ["MacBook"]]);
  await p.place("prefer", "m-vps");
  assert.deepEqual(listOf(p.html(), "Prefer"), ["mac-mini", "lypes-vps"]);
});

test("saving the policy sends it with the Captains, and the hub's answer becomes the saved policy", async () => {
  const saved = { prefer: ["m-mini"], avoid: ["m-book"], heavy: "all_workers" };
  const p = page({ answer: (cmd) => (cmd === "set_taste" ? tastesWith({ placement: saved }) : tastesWith({ placement: none })), machineViews: MACHINES });
  await p.show();
  await p.place("prefer", "m-mini");
  await p.place("avoid", "m-book");
  await p.click(/data-act="pheavy" data-value="all_workers"/);
  assert.match(p.html(), /Unsaved changes/);
  p.calls.length = 0;
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls, [["set_taste", { scope: { kind: "captains" }, patch: { placement: saved } }]]);
  assert.doesNotMatch(p.html(), /Unsaved changes/);
  assert.match(p.html(), /data-act="pheavy" data-value="all_workers" aria-pressed="true"/);
});

test("the orchestrator tab shows the same policy, and a project tab points to the account's", async () => {
  const p = page({ ...withPlacement({ prefer: ["m-mini"], avoid: [], heavy: "worktree" }), projects: { "m-mini": [{ path: "/p/site", name: "site", repo: SITE }] } });
  await p.show();
  await p.click(/data-kind="orchestrator"/);
  assert.deepEqual(listOf(p.html(), "Prefer"), ["mac-mini"]);
  await p.click(/data-kind="project" data-repo="github.com\/acme\/site"/);
  assert.doesNotMatch(p.html(), /data-act="pmove"|data-place-add/);
  assert.match(p.html(), /This is set for the whole account, under <button type="button" class="taste-link" data-act="scope" data-kind="captains">Captains \(all\)<\/button>/);
});

test("a hub that predates placement says so instead of drawing empty lists", async () => {
  const p = page({ machineViews: MACHINES });
  await p.show();
  assert.match(p.html(), /Update pinned on the hub to choose where heavy work runs\./);
  assert.doesNotMatch(p.html(), /data-place-add/);
});

const sized = (captains = null, projects = {}) => limited({ captains: { ...balanced, ...limits, box_size: captains }, projects });
const boxSize = (html) => html.match(/<output aria-live="polite">((?:Small|Medium|Large) \([^)]*\))<\/output>/)?.[1];

test("the default box size shows the size and where it comes from", async () => {
  let tastes = sized();
  const p = page({ answer: () => tastes });
  await p.show();
  let html = p.html();
  assert.match(html, /<span>Default box size<\/span><\/div>/, "the account's own row has no Reset while unset");
  assert.equal(boxSize(html), "Small (built-in)");
  assert.match(html, /data-field="box_size" data-value="small" aria-pressed="true"/);

  tastes = sized("medium");
  await p.show();
  html = p.html();
  assert.equal(boxSize(html), "Medium (account default)");
  assert.match(html, /data-field="box_size" data-value="medium" aria-pressed="true"/);
  assert.match(html, /<span>Default box size<\/span><button type="button" class="taste-reset" data-act="reset" data-field="box_size"/);
});

test("a project shows its own box size, or what it inherits", async () => {
  const other = "github.com/acme/other";
  const p = page({ answer: () => sized("medium", { [SITE]: { box_size: "large" }, [other]: { parallel: 2 } }) });
  await p.show();
  const tabs = p.html().match(/data-act="scope" data-kind="project"[^>]*>/g);
  assert.equal(tabs.length, 2);
  await p.click(new RegExp(`data-act="scope" data-kind="project" data-repo="${SITE}"`));
  let html = p.html();
  assert.equal(boxSize(html), "Large (this project)");
  assert.match(html, /<span>Default box size<\/span><span class="taste-tag changed">changed here<\/span><button type="button" class="taste-reset" data-act="reset" data-field="box_size"/);
  assert.doesNotMatch(html, /Inherits (?:Small|Medium|Large) from/);

  await p.click(/data-act="reset" data-field="box_size"/);
  html = p.html();
  assert.equal(boxSize(html), "Medium (account default)");
  assert.match(html, /Inherits Medium from Captains \(all\)\./);
  assert.match(html, /<span>Default box size<\/span><\/div>/, "an inherited size carries no tag");
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls.at(-1), ["reset_taste_field", { scope: { kind: "project", repo: SITE }, field: "box_size" }]);
});

test("a project with no account default inherits the built-in small", async () => {
  const p = page({ answer: () => sized(null, { [SITE]: { parallel: 2 } }) });
  await p.show();
  await p.click(/data-act="scope" data-kind="project"/);
  const html = p.html();
  assert.equal(boxSize(html), "Small (built-in)");
  assert.match(html, /Inherits Small from the built-in default\./);
});

test("changing the box size saves it on the scope it was changed on", async () => {
  let tastes = sized();
  const p = page({ answer: (cmd, args) => (cmd === "get_tastes" ? tastes : (tastes = sized(args.patch?.box_size ?? null, tastes.projects))) });
  await p.show();
  await p.click(/data-field="box_size" data-value="medium"/);
  assert.equal(boxSize(p.html()), "Medium (account default)");
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls.at(-1), ["set_taste", { scope: { kind: "captains" }, patch: { box_size: "medium" } }]);

  tastes = sized("medium", { [SITE]: { parallel: 2 } });
  await p.show();
  await p.click(/data-act="scope" data-kind="project"/);
  await p.click(/data-field="box_size" data-value="large"/);
  assert.equal(boxSize(p.html()), "Large (this project)");
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls.at(-1), ["set_taste", { scope: { kind: "project", repo: SITE }, patch: { box_size: "large" } }]);

  tastes = sized("medium");
  await p.show();
  await p.click(/data-act="scope" data-kind="captains"/);
  await p.click(/data-act="reset" data-field="box_size"/);
  assert.equal(boxSize(p.html()), "Small (built-in)");
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls.at(-1), ["reset_taste_field", { scope: { kind: "captains" }, field: "box_size" }]);
});

test("the orchestrator tab and a hub without box sizes show no box size control", async () => {
  const p = page({ answer: () => sized() });
  await p.show();
  await p.click(/data-act="scope" data-kind="orchestrator"/);
  assert.doesNotMatch(p.html(), /box_size|Default box size/);
  const old = page({ answer: () => limited() });
  await old.show();
  assert.doesNotMatch(old.html(), /Default box size/);
});
