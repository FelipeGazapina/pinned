import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui-teams.js", import.meta.url), "utf8");
const flush = () => new Promise((resolve) => setImmediate(resolve));
const here = (value) => JSON.parse(JSON.stringify(value));
const text = (html) => html.replace(/<[^>]+>/g, " ").replace(/&#(\d+);/g, (_, c) => String.fromCharCode(c)).replace(/\s+/g, " ").trim();

const ACME = { id: "t1", name: "North Studio", owner: "a1", role: "owner", credential: { upstash: false, claude: false, github: false, slack: false } };
const SOLO = { id: "t2", name: "Side Gig", owner: "a9", role: "member", credential: { upstash: true, claude: true, github: true, slack: false } };

/** gui-teams.js in a page whose hub answers each team op from `answers` (a value, or a function of the call),
 * with `projects` as the person's own projects (name, repo) and `pages` as the registered shell pages. */
function app({ answers = {}, projects = [], pages = [], stored = null, raw = null, body = {}, catalogNotReady = false } = {}) {
  const frames = [];
  const listeners = {};
  const events = [];
  const store = new Map(stored ? [["teams-current", JSON.stringify(stored)]] : raw ? [["teams-current", raw]] : []);
  const shown = [];
  // Fake timers: each pending one by its id, with its wait in ms.
  const timers = new Map();
  let timerIds = 0;
  const context = {
    console,
    catalogNotReady,
    setTimeout: (fn, ms) => (timers.set(++timerIds, { fn, ms }), timerIds),
    clearTimeout: (id) => timers.delete(id),
    CustomEvent: class {
      constructor(type, init) {
        this.type = type;
        this.detail = init?.detail;
      }
    },
    document: {
      body: { dataset: { page: "", ...body } },
      getElementById: () => null,
      dispatchEvent: (e) => events.push(e),
      activeElement: null,
      visibilityState: "visible",
    },
    SHELL_PAGES: Object.fromEntries(pages.map((p) => [p, {}])),
    pageView: { addEventListener: (type, fn) => (listeners[type] = fn), querySelector: () => null },
    setPage: (html) => shown.push(html),
    headHtml: (title) => `<header class="page-head"><h1>${title}</h1></header>`,
    esc: (v) => String(v ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    load: (key) => store.get(key),
    save: (key, value) => (value === null ? store.delete(key) : store.set(key, value)),
    globalProjectsCatalog: () => {
      if (context.catalogNotReady) throw new ReferenceError("PinnedProjects is not defined");
      return { projects: projects.map(([name, repo]) => ({ slug: name, name, repo, machines: [] })), machines: [] };
    },
    account: { email: "ana.prado@acme.dev" },
    navigator: { clipboard: { writeText: (t) => ((context.copied = t), Promise.resolve()) } },
    opened: [],
    closed: 0,
    openPage: (name) => context.opened.push(name),
    closePage: () => context.closed++,
    // The bots UI's opener of the Create a bot page, a cross-file global.
    openTeamBotNew: () => context.opened.push("team-bot-new"),
    hub: {
      hub: async (call) => {
        frames.push(here(call));
        if (call.cmd !== "team") throw new Error(`unexpected ${call.cmd}`);
        const answer = answers[call.op.op];
        const out = typeof answer === "function" ? answer(call) : answer;
        if (out instanceof Error) throw out.message;
        return here(out ?? null);
      },
    },
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(src, context);
  const run = (code) => vm.runInContext(code, context);
  const page = () => shown[shown.length - 1] ?? "";
  /** A click on the element that carries `data-tm` and these attributes. */
  const click = (tm, data = {}, page = "team-create") => {
    context.document.body.dataset.page = page;
    const el = { dataset: { tm, ...data }, disabled: false };
    listeners.click({ target: { closest: (sel) => (sel === "[data-tm]" ? el : null) } });
  };
  return { context, run, frames, events, store, shown, page, listeners, click, timers, show: (name) => (context.document.body.dataset.page = name, context.SHELL_PAGES[name].show()) };
}

const created = (over = {}) => ({ ...ACME, ...over });

test("a team call carries the team for every op but List, Create and Join", async () => {
  const a = app({ answers: { list: [ACME], create: ACME, join: SOLO, members: [] }, stored: { id: "t1", name: "North Studio", role: "owner", credential: {} } });
  await a.context.teamsList();
  await a.context.teamCall({ op: "create", name: "North Studio" });
  await a.context.teamCall({ op: "join", code: "ABCD-EFGH" });
  await a.context.teamCall({ op: "members" });
  assert.deepEqual(a.frames, [
    { cmd: "team", op: { op: "list" } },
    { cmd: "team", op: { op: "create", name: "North Studio" } },
    { cmd: "team", op: { op: "join", code: "ABCD-EFGH" } },
    { cmd: "team", team: "t1", op: { op: "members" } },
  ]);
});

test("a team call with no team chosen is refused before it reaches the hub", async () => {
  const a = app();
  await assert.rejects(a.context.teamCall({ op: "members" }), /no team/);
  assert.deepEqual(a.frames, []);
});

test("the current team is kept per viewer, read back, and fires teams:changed only when it changes", async () => {
  const a = app({ answers: { list: [ACME, SOLO] } });
  assert.equal(a.context.teamsCurrent(), null);
  await a.context.teamsList();
  a.run("teamsSelect(teamsKnown[1])");
  assert.deepEqual(here(a.context.teamsCurrent()), { id: "t2", name: "Side Gig", role: "member", credential: SOLO.credential, me: null });
  assert.equal(JSON.parse(a.store.get("teams-current")).id, "t2");
  a.run("teamsSelect(teamsKnown[1])");
  a.run("teamsSelect(null)");
  assert.equal(a.context.teamsCurrent(), null);
  assert.equal(a.store.has("teams-current"), false);
  assert.deepEqual(a.events.map((e) => [e.type, e.detail?.id ?? null]), [["teams:changed", "t2"], ["teams:changed", null]]);
  assert.equal(globalThis.teamsCurrent, undefined, "the vm's globals stay in the vm");
});

test("a saved team is current at start, and refreshed from the list; one the person left falls back to Just me", async () => {
  const saved = { id: "t1", name: "Old name", role: "member", credential: {} };
  const a = app({ answers: { list: [created({ name: "New name" })] }, stored: saved });
  assert.equal(a.context.teamsCurrent().name, "Old name");
  await a.context.teamsList();
  assert.deepEqual(here(a.context.teamsCurrent()), { id: "t1", name: "New name", role: "owner", credential: ACME.credential, me: "a1" });
  assert.equal(a.events.length, 0, "a refresh is not a change of team");
  const b = app({ answers: { list: [SOLO] }, stored: saved });
  await b.context.teamsList();
  assert.equal(b.context.teamsCurrent(), null);
  assert.equal(b.events.length, 1);
});

test("teamsCurrent carries me: the owner's own account id from the wire's owner, null for a member", async () => {
  const a = await (async () => {
    const x = app({ answers: { list: [ACME, SOLO] } });
    await x.context.teamsList();
    return x;
  })();
  a.run("teamsSelect(teamsKnown[0])");
  assert.equal(a.context.teamsCurrent().me, "a1");
  assert.equal(JSON.parse(a.store.get("teams-current")).me, "a1", "kept with the team");
  a.run("teamsSelect(teamsKnown[1])");
  assert.equal(a.context.teamsCurrent().me, null, "the wire gives a member no id of their own");
  const b = app({ stored: { id: "t1", name: "North Studio", role: "owner", credential: {}, me: "a1" } });
  assert.equal(b.context.teamsCurrent().me, "a1", "read back at start");
});

test("teamsCurrent, teamsList and teamCall are real properties of globalThis", () => {
  const a = app();
  for (const name of ["teamsCurrent", "teamsList", "teamCall"]) assert.equal(a.run(`Object.prototype.hasOwnProperty.call(globalThis, "${name}") && typeof globalThis.${name}`), "function", name);
});

test("a saved value that is not a team leaves no team chosen and breaks nothing", () => {
  for (const raw of ["{not json", "42", '{"id":7}', "null"]) assert.equal(app({ raw }).context.teamsCurrent(), null, raw);
});

test("the menu block lists Just me, each team with its role, Create a team and Join a team", async () => {
  const a = app({ answers: { list: [ACME, SOLO] }, stored: { id: "t2", name: "Side Gig", role: "member", credential: {} } });
  await a.context.teamsList();
  const html = a.run("teamsWhereHtml()");
  assert.match(text(html), /^Where you work AP Just me NS North Studio Owner SG Side Gig you are here \+ Create a team → Join a team/);
  assert.match(html, /data-team-pick="t2"[^>]*aria-current="true"/);
  assert.match(html, /class="tm-create"[^>]*data-team-page="team-create"/);
  assert.match(html, /class="tm-join"[^>]*data-team-page="team-join"/);
  const solo = app({ answers: { list: [ACME] } });
  await solo.context.teamsList();
  assert.match(text(solo.run("teamsWhereHtml()")), /Just me you are here NS North Studio Owner/);
});

test("a team's name typed by a person never lands in the markup raw", async () => {
  const evil = { ...ACME, name: `<img src=x onerror=alert(1)>` };
  const a = app({ answers: { list: [evil] } });
  await a.context.teamsList();
  assert.ok(!a.run("teamsWhereHtml()").includes("<img"));
  a.run("teamsSelect(teamsKnown[0])");
  a.run("resetWizard()");
  a.context.args = "x";
  a.run("teamWizard.name = args; teamWizard.team = teamsKnown[0]; teamWizard.step = 5");
  a.run("renderCreate()");
  assert.ok(!a.page().includes("<img"));
});

test("step 1 asks the name, and Continue sends create once with the exact name", async () => {
  const a = app({ answers: { create: ACME, list: [ACME] } });
  a.show("team-create");
  assert.match(a.page(), /What is the team called\?/);
  assert.match(text(a.page()), /Step 1 of 4/);
  assert.match(text(a.page()), /You are the owner\. You can rename it later\./);
  assert.match(text(a.page()), /Bots that do the repeated work/);
  // An empty name never reaches the hub.
  a.click("next");
  await flush();
  assert.match(a.page(), /Give the team a name\./);
  assert.deepEqual(a.frames, []);
  a.listeners.input({ target: { id: "tm-team-name", value: "North Studio" } });
  a.click("next");
  await flush();
  assert.deepEqual(a.frames[0], { cmd: "team", op: { op: "create", name: "North Studio" } });
  assert.match(text(a.page()), /Step 2 of 4/);
});

test("going back and forward never repeats the create; a changed name renames", async () => {
  const a = app({ answers: { create: ACME, list: [ACME], rename: null } });
  a.show("team-create");
  a.listeners.input({ target: { id: "tm-team-name", value: "North Studio" } });
  a.click("next");
  await flush();
  a.click("prev");
  assert.match(text(a.page()), /Step 1 of 4/);
  a.click("next");
  await flush();
  assert.equal(a.frames.filter((f) => f.op.op === "create").length, 1);
  a.click("prev");
  a.listeners.input({ target: { id: "tm-team-name", value: "North Studio Two" } });
  a.click("next");
  await flush();
  assert.equal(a.frames.filter((f) => f.op.op === "create").length, 1);
  assert.deepEqual(a.frames.find((f) => f.op.op === "rename"), { cmd: "team", team: "t1", op: { op: "rename", name: "North Studio Two" } });
  assert.match(a.page(), /Which of your projects go to North Studio Two\?/);
});

test("a create the hub refuses shows its words inline and stays on step 1", async () => {
  const a = app({ answers: { create: new Error("you already own a team with that name") } });
  a.show("team-create");
  a.listeners.input({ target: { id: "tm-team-name", value: "Dup" } });
  a.click("next");
  await flush();
  assert.match(a.page(), /<p class="tm-error" role="alert">you already own a team with that name<\/p>/);
  assert.match(text(a.page()), /Step 1 of 4/);
});

async function atStep2(extra = {}) {
  const a = app({
    answers: { create: ACME, list: [ACME], attach_project: null, detach_project: null, ...extra.answers },
    projects: [["storefront", "git@github.com:acme/storefront.git"], ["orders-api", "https://github.com/Acme/orders-api"], ["notes", ""], ["dup", "https://github.com/acme/orders-api.git"]],
    pages: extra.pages ?? [],
  });
  a.show("team-create");
  a.listeners.input({ target: { id: "tm-team-name", value: "North Studio" } });
  a.click("next");
  await flush();
  return a;
}

test("step 2 lists the person's own projects, one per repository, a project with no remote shut", async () => {
  const a = await atStep2();
  const t = text(a.page());
  assert.match(t, /Which of your projects go to North Studio\?/);
  assert.match(t, /In the projects that go in, the team's bots can work\. The others stay yours\./);
  assert.ok(!/summary/i.test(t), "the team is promised no summary of what you finish");
  assert.equal((a.page().match(/data-tm="toggle"/g) ?? []).length, 3);
  assert.match(a.page(), /data-key="github.com\/acme\/storefront"/);
  assert.match(a.page(), /data-key="github.com\/acme\/orders-api"/);
  assert.match(a.page(), /data-key="notes" disabled/);
  assert.match(t, /notes no remote/);
});

test("Continue on step 2 sends attach_project for each project turned on, and for no other", async () => {
  const a = await atStep2();
  a.click("toggle", { key: "github.com/acme/orders-api" });
  a.click("toggle", { key: "github.com/acme/storefront" });
  assert.match(a.page(), /aria-checked="true"[^>]*data-key="github.com\/acme\/storefront"/);
  a.click("next");
  await flush();
  assert.deepEqual(a.frames.filter((f) => f.op.op === "attach_project"), [
    { cmd: "team", team: "t1", op: { op: "attach_project", key: "github.com/acme/storefront" } },
    { cmd: "team", team: "t1", op: { op: "attach_project", key: "github.com/acme/orders-api" } },
  ]);
  assert.match(text(a.page()), /Step 3 of 4/);
});

test("an attach that fails stops on step 2 with the hub's words, and a retry skips what went in", async () => {
  let n = 0;
  const a = await atStep2({ answers: { attach_project: () => (++n === 2 ? new Error("the team has one with that key") : null) } });
  a.click("toggle", { key: "github.com/acme/storefront" });
  a.click("toggle", { key: "github.com/acme/orders-api" });
  a.click("next");
  await flush();
  assert.match(text(a.page()), /Could not add orders-api: the team has one with that key/);
  assert.match(text(a.page()), /Step 2 of 4/);
  a.click("toggle", { key: "github.com/acme/orders-api" });
  a.click("next");
  await flush();
  const keys = a.frames.filter((f) => f.op.op === "attach_project").map((f) => f.op.key);
  assert.deepEqual(keys, ["github.com/acme/storefront", "github.com/acme/orders-api"]);
  assert.match(text(a.page()), /Step 3 of 4/);
});

test("a project turned off after it went in is detached, not left behind", async () => {
  const a = await atStep2();
  a.click("toggle", { key: "github.com/acme/storefront" });
  a.click("next");
  await flush();
  a.click("prev");
  a.click("toggle", { key: "github.com/acme/storefront" });
  a.click("next");
  await flush();
  assert.deepEqual(a.frames.filter((f) => f.op.op === "detach_project"), [{ cmd: "team", team: "t1", op: { op: "detach_project", key: "github.com/acme/storefront" } }]);
});

async function atStep3(team = ACME, extra = {}) {
  const a = await atStep2({ answers: { create: team, list: [team], ...extra.answers }, pages: extra.pages });
  a.click("next");
  await flush();
  return a;
}

test("step 3 is the team's AI account: three rows with the state of Team.credential, and no plan choice", async () => {
  const a = await atStep3(created({ credential: { upstash: true, claude: false, github: false, slack: false } }));
  const t = text(a.page());
  assert.match(t, /The team has its own AI account/);
  assert.match(t, /Upstash key Opens the boxes the bots work in Connected/);
  assert.match(t, /Claude token for boxes Signs the bots in to Claude inside the box Connect now/);
  assert.match(t, /GitHub token Clones the team's repositories Connect now/);
  assert.match(t, /No rush: you can connect later\. Until then you can set up bots and groups, but no bot runs\./);
  assert.ok(!/API key|team plan/i.test(t));
  assert.match(a.page(), /data-tm="connect" data-cred="claude"/);
  assert.ok(!/data-tm="connect" data-cred="upstash"/.test(a.page()));
  assert.match(a.page(), />Connect later</);
});

test("a member sees the states and no Connect now", async () => {
  const a = await atStep3(created({ role: "member" }));
  assert.ok(!/Connect now/.test(a.page()));
  assert.match(text(a.page()), /Only the owner connects these\./);
});

test("Connect now opens a password form per credential and sends the personal call through the team", async () => {
  const done = { id: "t1" };
  const a = await atStep3(ACME, { answers: { hub: null, list: () => [done.cred ? { ...ACME, credential: { ...ACME.credential, ...done.cred } } : ACME] } });
  const submit = async (cred, fields) => {
    a.click("connect", { cred });
    assert.match(a.page(), new RegExp(`<form class="tm-form" data-tm="form" data-cred="${cred}"`));
    const form = { dataset: { tm: "form", cred }, querySelectorAll: () => Object.entries(fields).map(([k, v]) => ({ dataset: { field: k }, value: v })) };
    const inputs = form.querySelectorAll();
    form.querySelectorAll = () => inputs;
    let prevented = false;
    a.listeners.submit({ preventDefault: () => (prevented = true), target: { closest: () => form } });
    assert.ok(prevented);
    assert.deepEqual(inputs.map((i) => i.value), inputs.map(() => ""), "the field is emptied once sent");
    await flush();
  };
  const forms = a.page();
  assert.equal((forms.match(/type="password"/g) ?? []).length, 0, "no form is open before Connect now");
  a.click("connect", { cred: "upstash" });
  assert.equal((a.page().match(/type="password"/g) ?? []).length, 2);
  assert.ok(!/value="[^"]*(box_|sk-ant)/.test(a.page()));

  done.cred = { upstash: true };
  await submit("upstash", { key: "box_SECRET1", claude_token: "sk-ant-oat01-SECRET2" });
  done.cred = { upstash: true, claude: true };
  await submit("claude", { name: "team", token: "sk-ant-oat01-SECRET3" });
  done.cred = { upstash: true, claude: true, github: true };
  await submit("github", { token: "ghp_SECRET4" });

  const hubs = a.frames.filter((f) => f.op.op === "hub");
  assert.deepEqual(hubs, [
    { cmd: "team", team: "t1", op: { op: "hub", call: { cmd: "connect_upstash", key: "box_SECRET1", claude_token: "sk-ant-oat01-SECRET2" } } },
    { cmd: "team", team: "t1", op: { op: "hub", call: { cmd: "set_box_claude_token", name: "team", token: "sk-ant-oat01-SECRET3" } } },
    { cmd: "team", team: "t1", op: { op: "hub", call: { cmd: "connect_github", token: "ghp_SECRET4" } } },
  ]);
  assert.ok(!/SECRET/.test(a.page()), "a key is never drawn again");
  assert.ok(!a.shown.some((h) => /SECRET/.test(h)), "nor was it ever drawn");
  const t = text(a.page());
  assert.match(t, /Upstash key[^|]*Connected/);
  assert.match(t, /GitHub token[^|]*Connected/);
  assert.match(a.page(), />Continue</, "all three connected: no more Connect later");
});

const KEY_ADVICE = "The bot can do whatever the team's keys allow, so connect keys with the least access the bot needs.";

test("next to each key field the form says the bot can do whatever the team's keys allow", async () => {
  const a = await atStep3(ACME, { answers: { hub: null } });
  for (const [cred, keys] of [["upstash", 2], ["claude", 1], ["github", 1]]) {
    a.click("connect", { cred });
    const page = a.page();
    const fields = page.match(/<label class="tm-field">.*?<\/label>/g) ?? [];
    const withKey = fields.filter((f) => f.includes('type="password"'));
    assert.equal(withKey.length, keys, cred);
    for (const f of withKey) assert.equal(text(f).split(KEY_ADVICE).length - 1, 1, `${cred}: the sentence sits in the key field's own label`);
    assert.equal((text(page).match(/whatever the team's keys allow/g) ?? []).length, keys, `${cred}: one sentence per key field, none on the name field`);
  }
});

test("a key the hub refuses shows its words inline and keeps the form open without the key", async () => {
  const a = await atStep3(ACME, { answers: { hub: new Error("only the team's owner runs connect_github on the team") } });
  a.click("connect", { cred: "github" });
  const input = { dataset: { field: "token" }, value: "ghp_WRONG" };
  const form = { dataset: { tm: "form", cred: "github" }, querySelectorAll: () => [input] };
  a.listeners.submit({ preventDefault() {}, target: { closest: () => form } });
  await flush();
  assert.match(text(a.page()), /only the team's owner runs connect_github on the team/);
  assert.match(a.page(), /data-cred="github"/);
  assert.ok(!a.page().includes("ghp_WRONG"));
  // A blank field is caught before the hub sees it.
  const before = a.frames.length;
  const blank = { dataset: { field: "token" }, value: "  " };
  a.listeners.submit({ preventDefault() {}, target: { closest: () => ({ dataset: { tm: "form", cred: "github" }, querySelectorAll: () => [blank] }) } });
  await flush();
  assert.equal(a.frames.length, before);
  assert.match(text(a.page()), /Fill in every field\./);
});

test("step 4 makes invite codes one by one, shows each with Copy, and Skip is just Finish", async () => {
  let n = 0;
  const a = await atStep3(ACME, { answers: { invite: () => ({ code: `CODE-000${++n}`, hint: `…000${n}` }) } });
  a.click("next");
  await flush();
  const t = text(a.page());
  assert.match(t, /Who works with you\?/);
  assert.ok(!/e-?mail/i.test(t), "no e-mail field");
  assert.ok(!/<input/.test(a.page()));
  a.click("invite");
  await flush();
  a.click("invite");
  await flush();
  assert.deepEqual(a.frames.filter((f) => f.op.op === "invite"), [{ cmd: "team", team: "t1", op: { op: "invite" } }, { cmd: "team", team: "t1", op: { op: "invite" } }]);
  assert.match(a.page(), /<code>CODE-0001<\/code>/);
  assert.match(a.page(), /<code>CODE-0002<\/code>/);
  assert.match(text(a.page()), /Send this code to the person\. It works once\. Shown only now\./);
  a.click("copy", { i: "1" });
  await flush();
  assert.equal(a.context.copied, "CODE-0002");
  assert.match(a.page(), /Copied/);
});

test("Finish leads to the Done screen with a truthful summary, and the team is current once the person opens it", async () => {
  let n = 0;
  const a = await atStep3(ACME, { answers: { invite: () => ({ code: `C${++n}`, hint: "" }) } });
  a.click("toggle", { key: "github.com/acme/storefront" });
  a.click("prev");
  a.click("next");
  await flush();
  a.click("next");
  await flush();
  a.click("next");
  await flush();
  a.click("invite");
  await flush();
  a.click("next");
  await flush();
  const t = text(a.page());
  assert.match(t, /North Studio is ready/);
  assert.match(t, /‹ Back/, "the prototype's Back stays on the last screen");
  assert.match(t, /1 project added\./);
  assert.match(t, /The team's AI account is not connected yet, so no bot runs until it is\./);
  assert.match(t, /1 invite code made\. You pass it on yourself\./);
  assert.ok(!/e-mail.*invit|receive the invit/i.test(t));
  assert.match(t, /Create the first bot/);
  assert.equal(a.context.teamsCurrent(), null, "the team is not current until the person opens it");
  a.click("open-team");
  assert.equal(a.context.teamsCurrent().id, "t1");
  assert.deepEqual(a.context.opened, ["team-calls"], "Open the team is the team's Calls page");
});

test("Create the first bot opens the Create a bot page, and Open the team opens the Calls page", async () => {
  const a = await atStep3(ACME);
  a.run("teamWizard.step = 5");
  a.run("renderCreate()");
  assert.match(a.page(), />Create the first bot</);
  a.click("first-bot");
  assert.deepEqual(a.context.opened, ["team-bot-new"]);
  a.click("open-team");
  assert.deepEqual(a.context.opened, ["team-bot-new", "team-calls"]);
  assert.ok(!/team-home|team-runs|team-bots"/.test(src), "no guessed page id is left");
});

test("the step list on the left is the prototype's five steps; steps ahead stay shut, steps behind open", async () => {
  const a = await atStep3();
  const html = a.page();
  const t = text(html);
  assert.match(t, /Create a team Four steps, about 2 minutes\. You can go back and change any of them\./);
  for (const name of ["Name", "Projects", "The team's AI account", "People", "Done"]) assert.ok(t.includes(name), name);
  assert.match(html, /class="tm-step current" data-tm="go" data-step="3" aria-current="step"/);
  assert.match(html, /class="tm-step todo" data-tm="go" data-step="4"[^>]*disabled/);
  assert.match(html, /class="tm-step done" data-tm="go" data-step="1"/);
  a.click("go", { step: "1" });
  assert.match(text(a.page()), /Step 1 of 4/);
  const frames = a.frames.length;
  a.click("go", { step: "3" });
  assert.equal(a.frames.length, frames, "visiting a step sends nothing");
});

test("Back to my account and Cancel leave the page", async () => {
  const a = app();
  a.show("team-create");
  assert.match(text(a.page()), /‹ Back to my account/);
  a.click("leave");
  assert.equal(a.context.closed, 1);
});

// ---- Join ----------------------------------------------------------------------------------------

const THEIRS = [
  { key: "github.com/acme/storefront", name: "storefront", repo: "https://github.com/acme/storefront", member: "a1" },
  { key: "github.com/acme/mobile", name: "mobile", repo: "https://github.com/acme/mobile", member: "a2" },
];
function joinApp(extra = {}) {
  return app({
    answers: { join: SOLO, projects: THEIRS, list: [SOLO], attach_project: null, ...extra.answers },
    projects: [["storefront", "git@github.com:acme/storefront.git"], ["orders-api", "https://github.com/acme/orders-api"], ["notes", ""]],
    pages: extra.pages ?? [],
  });
}

test("Join asks the code first and sends join with exactly what was pasted", async () => {
  const a = joinApp();
  a.show("team-join");
  assert.match(text(a.page()), /Join a team Someone on the team makes you an invite code\. It works once\./);
  assert.match(text(a.page()), /Paste the invite code/);
  a.listeners.input({ target: { id: "tm-join-code", value: " ABCD-EFGH " } });
  const form = { dataset: { tm: "join-form" } };
  a.context.document.body.dataset.page = "team-join";
  a.listeners.submit({ preventDefault() {}, target: { closest: () => form } });
  await flush();
  assert.deepEqual(a.frames[0], { cmd: "team", op: { op: "join", code: "ABCD-EFGH" } });
  assert.deepEqual(a.frames[1], { cmd: "team", team: "t2", op: { op: "projects" } });
});

test("a bad code shows the hub's words inline and keeps the code field", async () => {
  const a = joinApp({ answers: { join: new Error("that code is not valid or was used") } });
  a.show("team-join");
  a.listeners.input({ target: { id: "tm-join-code", value: "NOPE" } });
  a.context.document.body.dataset.page = "team-join";
  a.listeners.submit({ preventDefault() {}, target: { closest: () => ({ dataset: { tm: "join-form" } }) } });
  await flush();
  assert.match(a.page(), /<p class="tm-error" role="alert">that code is not valid or was used<\/p>/);
  assert.match(a.page(), /value="NOPE"/);
  assert.equal(a.frames.length, 1);
});

async function joined(extra) {
  const a = joinApp(extra);
  a.show("team-join");
  a.listeners.input({ target: { id: "tm-join-code", value: "ABCD-EFGH" } });
  a.listeners.submit({ preventDefault() {}, target: { closest: () => ({ dataset: { tm: "join-form" } }) } });
  await flush();
  return a;
}

test("after joining: the team's name, the person's projects marked by what the team already has, and what the team sees", async () => {
  const a = await joined();
  const t = text(a.page());
  assert.match(t, /You joined Side Gig/);
  assert.match(t, /Which of your projects belong to the team\?/);
  assert.match(t, /1 of yours matches a project the team already has\./);
  assert.match(t, /storefront The team already has it The team sees/);
  assert.match(t, /orders-api https:\/\/github.com\/acme\/orders-api Only you/);
  assert.match(t, /notes no remote[^|]* Only you/);
  assert.match(t, /What the team will see The names and repositories of the projects you add, and the runs of the team's bots in them\./);
  assert.match(t, /Never your conversations, your other projects or your passwords\. You can take a project out any time\./);
  assert.ok(!/summary|what you finish|tells the team/i.test(t), "no promise of summaries");
  assert.match(a.page(), /data-key="github.com\/acme\/storefront"[^>]*>|aria-checked="true"[^>]*data-key="github.com\/acme\/storefront"/);
  assert.match(a.page(), />Join with 1 project</);
  assert.match(a.page(), />Continue without any</);
  assert.equal(a.context.teamsCurrent(), null, "the team becomes current once the person continues");
});

test("Join with N projects sends attach_project for each turned on, then opens the team", async () => {
  const a = await joined();
  a.click("toggle", { key: "github.com/acme/orders-api" }, "team-join");
  assert.match(a.page(), />Join with 2 projects</);
  a.click("join-with", {}, "team-join");
  await flush();
  assert.deepEqual(a.frames.filter((f) => f.op.op === "attach_project"), [
    { cmd: "team", team: "t2", op: { op: "attach_project", key: "github.com/acme/storefront" } },
    { cmd: "team", team: "t2", op: { op: "attach_project", key: "github.com/acme/orders-api" } },
  ]);
  assert.equal(a.context.teamsCurrent().id, "t2");
  assert.deepEqual(a.context.opened, ["team-calls"]);
  assert.equal(a.events.length, 1);
});

test("Continue without any attaches nothing and still joins", async () => {
  const a = await joined();
  a.click("join-without", {}, "team-join");
  await flush();
  assert.equal(a.frames.filter((f) => f.op.op === "attach_project").length, 0);
  assert.equal(a.context.teamsCurrent().name, "Side Gig");
  assert.deepEqual(a.context.opened, ["team-calls"]);
});

test("an attach that fails stays on the page with the hub's words and sends no repeat", async () => {
  let n = 0;
  const a = await joined({ answers: { attach_project: () => (++n === 1 ? new Error("you already shared that project") : null) } });
  a.click("join-with", {}, "team-join");
  await flush();
  assert.match(text(a.page()), /Could not add storefront: you already shared that project/);
  assert.equal(a.context.teamsCurrent(), null);
  a.click("join-with", {}, "team-join");
  await flush();
  assert.equal(a.frames.filter((f) => f.op.op === "attach_project").length, 2);
  assert.equal(a.context.teamsCurrent().id, "t2");
});

test("a team whose projects can't be read still lets the person pick their own", async () => {
  const a = await joined({ answers: { projects: new Error("not built yet") } });
  const t = text(a.page());
  assert.match(t, /Could not read the team's projects: not built yet/);
  assert.match(t, />?storefront/);
  assert.match(a.page(), />Join the team</);
});

test("a team with nothing in common says so", async () => {
  const a = await joined({ answers: { projects: [] } });
  assert.match(text(a.page()), /The team has no project that matches yours yet\. You can still add some\./);
});

test("repository keys are the hub's spelling whichever way a machine wrote the remote", () => {
  const a = app();
  const key = (r) => a.run(`teamsRepoKey(${JSON.stringify(r)})`);
  assert.equal(key("git@github.com:Acme/Site.git"), "github.com/acme/site");
  assert.equal(key("https://github.com/acme/site/"), "github.com/acme/site");
  assert.equal(key("https://user:tok@github.com/acme/site.git"), "github.com/acme/site");
  assert.equal(key(""), "");
  assert.equal(key(null), "");
});

test("the script loads before the project catalog is ready, and the pages read the projects when they open", () => {
  const a = app({ catalogNotReady: true, projects: [["storefront", "https://github.com/acme/storefront"]], pages: [] });
  assert.ok("team-create" in a.context.SHELL_PAGES && "team-join" in a.context.SHELL_PAGES);
  a.context.catalogNotReady = false;
  a.show("team-create");
  a.listeners.input({ target: { id: "tm-team-name", value: "North Studio" } });
  assert.match(a.page(), /What is the team called\?/);
  a.run("teamWizard.step = 2");
  a.run("renderCreate()");
  assert.match(a.page(), /data-key="github.com\/acme\/storefront"/);
});

// ---- Invites (owner only) -----------------------------------------------------------------------

const OWNER_STORED = { id: "t1", name: "North Studio", role: "owner", credential: {} };
const MEMBER_STORED = { id: "t2", name: "Side Gig", role: "member", credential: {} };
const submit = (a, tm) => a.listeners.submit({ target: { closest: () => ({ dataset: { tm } }) }, preventDefault() {} });

const NOW = 1790000000;
const when = (unix) => new Date(unix * 1000).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
const OPEN = [
  { hint: "WXYZ", created: NOW, made_by: "ana" },
  { hint: "ABCD", created: NOW - 86400, made_by: "0b7e5d1c" },
];

test("the invites page says a code works once and never that it expires or what the hub does not do", async () => {
  const a = app({ stored: OWNER_STORED, pages: ["team-invites"], answers: { invites: [] } });
  a.show("team-invites");
  await flush();
  const t = text(a.page());
  assert.ok(t.includes("A code works once. It stays open until the person uses it or you revoke it."), t);
  assert.ok(!/expir|valid for|days|hours/i.test(t), "there is no expiry");
  assert.ok(!/does not|sends no|no copy|on this device|first characters/i.test(t), t);
  assert.ok(t.includes("No open invites."), t);
});

test("the owner's page lists the open invites with {op:\"invites\"}: hint, when made, who made it, Revoke", async () => {
  const a = app({ stored: OWNER_STORED, pages: ["team-invites"], answers: { invites: OPEN } });
  a.show("team-invites");
  await flush();
  assert.deepEqual(a.frames, [{ cmd: "team", team: "t1", op: { op: "invites" } }]);
  assert.equal(JSON.stringify(a.frames[0].op), '{"op":"invites"}');
  const t = text(a.page());
  assert.ok(t.includes(`WXYZ… Made ${when(NOW)} by ana`), t);
  assert.ok(t.includes(`ABCD… Made ${when(NOW - 86400)} by 0b7e5d1c`), t);
  assert.ok(t.indexOf("WXYZ…") < t.indexOf("ABCD…"), "in the order the hub sends: newest first");
  assert.equal((a.page().match(/data-tm="invite-revoke"/g) ?? []).length, 2);
  assert.ok(!/<input/.test(a.page()), "no revoke-by-characters field");
  assert.equal(a.store.size, 1, "nothing about invites is kept on the device");
});

test("an invite is one invite op; its code is shown once, then the list is read again", async () => {
  let open = [];
  const a = app({ stored: OWNER_STORED, pages: ["team-invites"], answers: { invites: () => open, invite: () => ((open = [{ hint: "ABCD", created: NOW, made_by: "ana" }, ...open]), { code: "ABCD-EFGH-JKMN-PQRS", hint: "ABCD" }) } });
  a.show("team-invites");
  await flush();
  a.frames.length = 0;
  a.click("invite-make", {}, "team-invites");
  await flush();
  assert.deepEqual(a.frames.map((f) => f.op), [{ op: "invite" }, { op: "invites" }]);
  const t = text(a.page());
  assert.ok(t.includes("ABCD-EFGH-JKMN-PQRS"), "the code is shown now");
  assert.ok(t.includes("Shown only now."), t);
  assert.ok(t.includes("ABCD… Made"), "and the open list names it by its first characters");
  assert.equal(a.page().match(/EFGH/g).length, 1, "a code is shown once");
});

test("Revoke sends revoke_invite with the hint, then reads the list again", async () => {
  let open = OPEN.slice();
  const a = app({ stored: OWNER_STORED, pages: ["team-invites"], answers: { invites: () => open, revoke_invite: (c) => ((open = open.filter((x) => x.hint !== c.op.hint)), null) } });
  a.show("team-invites");
  await flush();
  a.frames.length = 0;
  a.click("invite-revoke", { hint: "WXYZ" }, "team-invites");
  await flush();
  assert.deepEqual(a.frames.map((f) => f.op), [{ op: "revoke_invite", hint: "WXYZ" }, { op: "invites" }]);
  const t = text(a.page());
  assert.ok(t.includes("Revoked WXYZ…") && t.includes("ABCD…") && !t.includes("WXYZ… Made"), t);
});

test("a revoke the hub refuses shows the hub's words and the list as the hub has it", async () => {
  const a = app({ stored: OWNER_STORED, pages: ["team-invites"], answers: { invites: [], revoke_invite: new Error("there is no unused invite with that hint") } });
  a.show("team-invites");
  await flush();
  a.click("invite-revoke", { hint: "ZZZZ" }, "team-invites");
  await flush();
  assert.ok(text(a.page()).includes("there is no unused invite with that hint"), "the hub's words, as sent");
});

test("text a person typed (the maker's name) reaches the page escaped", async () => {
  const a = app({ stored: OWNER_STORED, pages: ["team-invites"], answers: { invites: [{ hint: "ABCD", created: NOW, made_by: "<img src=x onerror=1>" }] } });
  a.show("team-invites");
  await flush();
  assert.ok(!a.page().includes("<img"), a.page());
  assert.ok(a.page().includes("&#60;img"));
});

test("a list the hub refuses shows the hub's words", async () => {
  const a = app({ stored: OWNER_STORED, pages: ["team-invites"], answers: { invites: new Error("only the owner lists invites") } });
  a.show("team-invites");
  await flush();
  assert.match(a.page(), /<p class="tm-error" role="alert">only the owner lists invites<\/p>/);
});

test("at 20 open invites the hub's refusal is shown as it was sent", async () => {
  const refusal = "this team has 20 open invite codes; revoke some first";
  const a = app({ stored: OWNER_STORED, pages: ["team-invites"], answers: { invites: [], invite: new Error(refusal) } });
  a.show("team-invites");
  await flush();
  a.click("invite-make", {}, "team-invites");
  await flush();
  assert.ok(text(a.page()).includes(refusal), text(a.page()));
  assert.match(a.page(), /<p class="tm-error" role="alert">this team has 20 open invite codes; revoke some first<\/p>/);
});

test("a member is told that only the owner makes invites, and sees no list and no controls", async () => {
  const a = app({ stored: MEMBER_STORED, pages: ["team-invites"], answers: { invites: OPEN } });
  a.show("team-invites");
  await flush();
  assert.ok(text(a.page()).includes("Only the team's owner makes invite codes."));
  assert.ok(!a.page().includes('data-tm="invite-make"') && !a.page().includes("WXYZ") && !a.page().includes("Open invites"));
  a.click("invite-make", {}, "team-invites");
  a.click("invite-revoke", { hint: "WXYZ" }, "team-invites");
  await flush();
  assert.deepEqual(a.frames, [], "no invite op leaves a member's page");
});

test("an invite made in the create wizard keeps nothing on the device", async () => {
  const a = app({ answers: { create: ACME, invite: { code: "ABCD-EFGH-JKMN-PQRS", hint: "ABCD" } } });
  a.show("team-create");
  a.run('teamWizard.team = { id: "t1", name: "North Studio" }; teamWizard.step = 4; renderCreate()');
  a.click("invite");
  await flush();
  assert.equal(a.store.has("team-invites:t1"), false);
});

test("the create wizard no longer promises a team chat", () => {
  const a = app();
  a.show("team-create");
  const t = text(a.page());
  assert.ok(t.includes("Bots that do the repeated work on their own"), t);
  assert.ok(t.includes("Groups of people the bots call when they need a yes or an answer."), t);
  assert.ok(!/chat/i.test(t), "chat is not in 0.18.6");
  assert.ok(!/chat/i.test(a.run("JSON.stringify(TEAMS_TEXT.nameHas)")));
});

test("the team list keeps the line the hub holds for the team's owner, and only the owner's", async () => {
  const held = { since: 1795359900, why: "settings_unread" };
  const a = app({ answers: { list: [{ ...ACME, line_held: held }, { ...SOLO, line_held: held }] } });
  await a.context.teamsList();
  a.run("teamsSelect(teamsKnown[0])");
  assert.deepEqual(here(a.context.teamsCurrent()), { id: "t1", name: "North Studio", role: "owner", credential: ACME.credential, me: "a1", line_held: held });
  a.run("teamsSelect(teamsKnown[1])");
  assert.deepEqual(here(a.context.teamsCurrent()), { id: "t2", name: "Side Gig", role: "member", credential: SOLO.credential, me: null }, "a member's team keeps none");
  const b = app({ answers: { list: [ACME] } });
  await b.context.teamsList();
  b.run("teamsSelect(teamsKnown[0])");
  assert.equal("line_held" in b.context.teamsCurrent(), false, "a team the hub holds nothing for has no line");
});

/** The waits of the pending fake timers, in ms. */
const waits = (a) => [...a.timers.values()].map((t) => t.ms);
/** Runs the one pending fake timer and lets what it started settle. */
async function fire(a) {
  assert.equal(a.timers.size, 1, "one timer pending");
  const [[id, timer]] = [...a.timers];
  a.timers.delete(id);
  timer.fn();
  for (let i = 0; i < 5; i++) await flush();
}

test("an owner's team list is read again on one timer, and the held line goes once the hub lets it go", async () => {
  const held = { since: 1795359900, why: "settings_unread" };
  let list = [{ ...ACME, line_held: held }, SOLO];
  const a = app({ answers: { list: () => list }, body: { hub: "online" } });
  const drawn = [];
  a.context.tcNavDraw = () => drawn.push(here(a.context.teamsCurrent()));
  await a.context.teamsList();
  a.run("teamsSelect(teamsKnown[0])");
  assert.deepEqual(here(a.context.teamsCurrent().line_held), held);
  assert.deepEqual(waits(a), [30000], "every 30 s while an owned team's line is held");

  list = [ACME, SOLO];
  drawn.length = 0;
  await fire(a);
  assert.equal(a.frames.filter((f) => f.op.op === "list").length, 2, "the timer read the list again");
  assert.equal("line_held" in a.context.teamsCurrent(), false, "the held line is gone");
  assert.ok(drawn.length > 0 && drawn.every((t) => !("line_held" in t)), "the nav is drawn again, with no held line");
  assert.deepEqual(waits(a), [120000], "every 2 min once nothing is held");

  await a.context.teamsList();
  await a.context.teamsList();
  assert.deepEqual(waits(a), [120000], "reads by other ways never stack a second timer");

  a.context.document.visibilityState = "hidden";
  a.run("teamsPollArm()");
  assert.deepEqual(waits(a), [], "no timer while the page is hidden");
  a.context.document.visibilityState = "visible";
  a.run("teamsPollArm()");
  assert.deepEqual(waits(a), [120000]);
  a.context.document.body.dataset.hub = "offline";
  a.run("teamsPollArm()");
  assert.deepEqual(waits(a), [], "no timer while the hub is not online");
});

test("a person who owns no team, or is offline, gets no timer", async () => {
  const member = app({ answers: { list: [{ ...SOLO, line_held: { since: 1795359900, why: "settings_unread" } }] }, body: { hub: "online" } });
  await member.context.teamsList();
  assert.deepEqual(waits(member), [], "a member's list is not read on a timer");
  const offline = app({ answers: { list: [ACME] } });
  await offline.context.teamsList();
  assert.deepEqual(waits(offline), [], "nothing while the hub is not online");
});
