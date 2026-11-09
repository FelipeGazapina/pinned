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

/** The page in a vm: `answer(cmd, args)` plays the backend, and each control is found in the HTML
 * the page last drew, so a click only reaches what a user could see. */
function page({ answer = () => tastesWith(), projects = {} } = {}) {
  const listeners = {};
  const drawn = [];
  const calls = [];
  const timers = [];
  const context = {
    document: { body: { dataset: { page: "taste" } }, activeElement: null },
    pageView: { addEventListener: (type, fn) => (listeners[type] = fn), contains: () => false, querySelector: () => null, querySelectorAll: () => [] },
    SHELL_PAGES: {},
    setPage: (html) => drawn.push(html),
    headHtml: (title, rest = "") => `<header><h1>${title}</h1>${rest}</header>`,
    icon: (name) => `<svg data-icon="${name}"></svg>`,
    esc,
    connectorCall: async (cmd, args = {}) => {
      calls.push([cmd, JSON.parse(JSON.stringify(args))]);
      return answer(cmd, args);
    },
    onlineMachines: () => Object.keys(projects),
    callOn: async (machine) => projects[machine],
    CLAUDE_MODELS: [
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
  vm.runInContext(`${read("global-orchestrator.js")}\n${read("taste.js")}\n${read("gui-taste.js")}`, context);
  const html = () => drawn.at(-1);
  const control = (pattern) => {
    const tag = [...html().matchAll(/<(?:button|select|input|form)\b[^>]*>/g)].map((m) => m[0]).find((t) => pattern.test(t));
    assert.ok(tag, `the page draws a control matching ${pattern}`);
    const dataset = Object.fromEntries([...tag.matchAll(/\bdata-([\w-]+)(?:="([^"]*)")?/g)].map(([, k, v]) => [k.replace(/-(\w)/g, (_, c) => c.toUpperCase()), unesc(v ?? "")]));
    return { dataset, disabled: /\sdisabled[\s>]/.test(tag) };
  };
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  return {
    context,
    calls,
    timers,
    html,
    control,
    settle,
    show: async () => (context.SHELL_PAGES.taste.show(), settle()),
    click: async (pattern) => (listeners.click({ target: { closest: () => control(pattern) } }), settle()),
    pick: (attr, value) => listeners.change({ target: { value, matches: (sel) => sel === `[${attr}]` } }),
    type: (value) => listeners.input({ target: { value, matches: (sel) => sel === "[data-note-input]" } }),
    submit: () => listeners.submit({ target: { matches: (sel) => sel === "[data-note-form]" }, preventDefault() {} }),
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
  assert.match(html, /title="A rough estimate from each model's cost tier">\$0\.67–1\.39</);
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
  await p.click(/aria-label="More parallel workers"/);
  const html = p.html();
  assert.deepEqual(lines(html), ["* Run up to 4 workers at once.", ...balancedBrief().slice(1)]);
  assert.match(html, /<div class="taste-card custom active"><b>Custom<\/b>/);
  assert.match(html, /<span class="taste-unsaved">Unsaved changes<\/span>/);
  await p.click(/data-act="profile" data-profile="balanced"/);
  assert.deepEqual(lines(p.html()), ["* Run up to 3 workers at once.", ...balancedBrief().slice(1)]);
  assert.doesNotMatch(p.html(), /taste-foot/, "back to the saved tastes, nothing to save");
});

test("a project tab marks each field inherited or changed here, and Reset inherits again", async () => {
  const p = page({ answer: () => tastesWith({ projects: { [SITE]: { parallel: 5 } } }) });
  await p.show();
  await p.click(/data-act="scope" data-kind="project"/);
  let html = p.html();
  assert.match(html, /<span class="taste-quiet">Inherits from Captains \(all\)\.<\/span>/);
  assert.match(html, /<span>Parallel workers<\/span><span class="taste-tag changed">changed here<\/span><button type="button" class="taste-reset" data-act="reset" data-field="parallel"/);
  assert.match(html, /<span>Strategy<\/span><span class="taste-tag">inherited<\/span>/);
  assert.equal(lines(html)[0], "Run up to 5 workers at once.");
  await p.click(/data-act="reset" data-field="parallel"/);
  html = p.html();
  assert.match(html, /<span>Parallel workers<\/span><span class="taste-tag">inherited<\/span>/);
  assert.equal(lines(html)[0], "* Run up to 3 workers at once.");
  assert.match(html, /aria-pressed="true"[^>]*>Captain site</, "an offline project's tab stays open once its patch is gone");
  assert.match(html, /Unsaved changes/);
});

test("Save sends the changes in order, takes the last answer and shows the toast", async () => {
  const answers = {
    get_tastes: tastesWith({ projects: { [SITE]: { parallel: 5 } } }),
    set_taste: tastesWith({ captains: { ...balanced, second_opinion: true }, projects: { [SITE]: { parallel: 5 } } }),
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
  await p.click(/aria-label="Fewer parallel workers"/);
  await p.click(/data-act="save"/);
  let html = p.html();
  assert.match(html, /<span class="taste-unsaved">Unsaved changes<\/span><span class="taste-error" role="alert">the model ladder needs at least one model<\/span>/);
  assert.equal(lines(html)[0], "Run up to 2 workers at once.", "the draft stands");
  refuse = false;
  await p.click(/data-act="save"/);
  assert.deepEqual(p.calls.slice(1).map(([cmd, args]) => `${cmd} ${args.scope.kind}`), ["set_taste orchestrator", "set_taste captains", "set_taste captains"]);
});

test("a reload keeps unsaved edits over what another device saved meanwhile, and a save sends only the edits", async () => {
  let stored = tastesWith({ projects: { [SITE]: { parallel: 5 } } });
  const p = page({ answer: (cmd, args) => (cmd === "get_tastes" ? stored : stored), projects: { mm: [{ path: "/p/site", name: "site", repo: SITE }] } });
  await p.show();
  await p.click(/data-act="scope" data-kind="orchestrator"/);
  await p.click(/data-act="set" data-field="autonomy" data-value="ask_first"/);
  stored = tastesWith({ captains: { ...balanced, parallel: 5 }, projects: { [SITE]: { parallel: 5, context: ["Releases go out on Fridays."] } } });
  await p.show();
  await p.click(/data-act="scope" data-kind="captains"/);
  assert.equal(lines(p.html())[0], "Run up to 5 workers at once.", "the other device's Captains change shows");
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
  await p.click(/aria-label="More parallel workers"/);
  read = () => tastesWith({ captains: { ...balanced, autonomy: "ask_first" } });
  await p.show();
  assert.equal(lines(p.html())[0], "Run up to 4 workers at once.");
  assert.match(p.html(), /Unsaved changes/);
  await p.click(/data-act="discard"/);
  assert.equal(lines(p.html())[3], "Ask the user before you open a pull request or delete data.", "Discard goes back to the fresh read");
  read = () => Promise.reject(new Error("hub offline"));
  await p.show();
  assert.match(p.html(), /<footer class="taste-foot"><span class="taste-error" role="alert">hub offline<\/span>/);
  assert.equal(lines(p.html())[0], "Run up to 3 workers at once.", "the last read stays on screen");
});
