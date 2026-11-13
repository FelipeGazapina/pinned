import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const ideasSrc = read("gui-ideas.js");
const sidebarSrc = read("gui-sidebar.js");
const phoneSrc = read("gui-phone.js");
const html = read("index.html");

const flush = () => new Promise((resolve) => setImmediate(resolve));
const here = (value) => JSON.parse(JSON.stringify(value));
const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));
const fn = (src, name) => {
  const at = src.indexOf(`\nfunction ${name}(`) + 1;
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};

const NOW = 1_800_000_000;
const DAY = 86400;
const user = { kind: "user" };
const orchestrator = { kind: "agent", role: "global_orchestrator", machine: "mini", tab: "t1", session_title: "Orchestrator" };
const SITE = "github.com/acme/site";
const API = "github.com/acme/api";
let n = 0;
const idea = (title, extra = {}) => ({ id: `i${++n}`, title, text: "", project: null, status: "open", task: null, created_by: user, created_at: NOW - 2 * DAY, updated_at: NOW - 2 * DAY, ...extra });
const SEED = () => (n = 0, [
  idea("iPad layout for the sidebar", { text: "Split view with threads left.\nSecond line.", project: SITE, updated_at: NOW - 100 }),
  idea("Cheaper box tier", { created_by: orchestrator, created_at: NOW - 60, updated_at: NOW - 200 }),
  idea("Café menu for the demo", { project: API, status: "done", updated_at: NOW - 300 }),
  idea("Rewrite the installer", { status: "dropped", updated_at: NOW - 400 }),
]);

/** A page element that keeps what the script sets on it, with children it is handed. */
const el = (extra = {}) => ({ focus() {}, hidden: false, textContent: "", innerHTML: "", value: "", children: [], dataset: {}, replaceChildren(...c) { this.children = c; }, ...extra });

/** gui-ideas.js in a page whose hub holds `rows`. `calls` lists what the page sent, in order. */
function page(rows = SEED(), { signedIn = true } = {}) {
  const calls = [];
  const store = { rows: [...rows] };
  const handlers = {};
  const timers = [];
  const parts = {};
  let shown = null;
  const filters = el({ elements: { project: el(), status: el({ value: "open" }), query: el() } });
  const part = (sel) => {
    if (sel === ".ideas-filters") return filters;
    if (sel === '.ideas-filters [name="project"]') return filters.elements.project;
    if (sel === ".ideas-add [name='title']") return (parts[sel] ??= el());
    return (parts[sel] ??= el());
  };
  const context = {
    console,
    SHELL_PAGES: {},
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    icon: (name) => `<i data-icon="${name}"></i>`,
    ago: (unix) => (Math.max(0, NOW - unix) < 3600 ? "now" : Math.max(0, NOW - unix) < DAY ? `${Math.floor((NOW - unix) / 3600)}h` : `${Math.floor((NOW - unix) / DAY)}d`),
    headHtml: (title) => `<h1>${title}</h1>`,
    setPage: (h) => (shown = h),
    shellPage: () => "ideas",
    pageView: { addEventListener: (type, f) => (handlers[type] = f), querySelector: (sel) => part(sel) },
    document: {
      activeElement: null,
      createElement: (tag) => (tag === "template" ? { set innerHTML(h) { this.content = { firstElementChild: el({ html: h, dataset: { id: /data-id="([^"]+)"/.exec(h)?.[1] } }) }; } } : editorEl(tag)),
    },
    setTimeout: (f, ms) => timers.push({ f, ms }),
    clearTimeout: () => {},
    modelChanged: () => (context.changed += 1),
    changed: 0,
    projectRepos: new Map([["mini:/work/site", SITE]]),
    hub: signedIn ? {} : null,
    connectorCall: async (cmd, args = {}) => {
      calls.push({ cmd, ...args });
      if (cmd === "list_ideas") return here(store.rows);
      if (cmd === "save_idea") {
        const at = store.rows.findIndex((i) => i.id === args.id);
        const saved = at < 0 ? idea(args.title, { text: args.text ?? "" }) : { ...store.rows[at], title: args.title, text: args.text ?? "", project: args.project ?? null, status: args.status ?? store.rows[at].status, updated_at: NOW };
        if (at < 0) store.rows.unshift(saved);
        else store.rows[at] = saved;
        return here(saved);
      }
      if (cmd === "delete_idea") {
        store.rows = store.rows.filter((i) => i.id !== args.id);
        return null;
      }
      throw new Error(`unexpected ${cmd}`);
    },
    confirmed: [],
    answer: true,
    confirmAction: async (title, about, verb, danger) => {
      context.confirmed.push({ title, verb, danger });
      return context.answer ? true : null;
    },
    menus: [],
    openSideMenu: (items, at) => context.menus.push({ items, at }),
  };
  function editorEl() {
    const fields = {};
    return {
      dataset: {},
      set innerHTML(h) {
        this.html = h;
        fields.title = el({ value: /name="title" value="([^"]*)"/.exec(h)?.[1] ?? "" });
        fields.text = el({ value: />([^<]*)<\/textarea>/.exec(h)?.[1] ?? "" });
        fields.project = el({ value: /<option value="([^"]+)" selected/.exec(h)?.[1] ?? "" });
      },
      querySelector: (sel) => fields[/name="(\w+)"/.exec(sel)?.[1]] ?? (fields[sel] ??= el({ focus() {} })),
    };
  }
  vm.createContext(context);
  vm.runInContext(ideasSrc, context);
  const run = (code) => vm.runInContext(code, context);
  const target = (matches = {}) => ({ closest: (sel) => matches[sel] ?? null, ...matches.props });
  const button = (action, id) => ({ dataset: { idea: action }, closest: (sel) => (sel === "[data-id]" ? { dataset: { id } } : null) });
  const send = (type, event) => handlers[type]({ preventDefault() {}, stopPropagation() {}, ...event });
  return {
    context,
    calls,
    store,
    timers,
    run,
    show: async () => {
      context.SHELL_PAGES.ideas.show();
      await flush();
    },
    get html() {
      return shown;
    },
    list: () => part(".ideas-list").children.map((c) => c.html),
    titles: () => part(".ideas-list").children.map((c) => /class="idea-title">([^<]*)</.exec(c.html)?.[1]),
    empty: () => (part(".ideas-empty").hidden ? "" : part(".ideas-empty").textContent),
    filter: (name, value) => {
      send("change", { target: { name, value, closest: () => null } });
    },
    search: (value) => send("input", { target: { name: "query", value, closest: () => null } }),
    click: async (action, id) => {
      const b = button(action, id);
      send("click", { target: target({ "[data-idea]": b }) });
      await flush();
    },
    menu: async (id) => {
      send("click", { target: target({ "[data-idea]": button("menu", id) }) });
      return Array.from(context.menus.at(-1).items);
    },
    add: async (title) => {
      const form = { elements: { title: { value: title } } };
      send("submit", { target: { closest: (sel) => (sel === ".ideas-add" ? form : null) } });
      await flush();
      await flush();
    },
    type: (editor, field, value) => {
      editor.querySelector(`[name="${field}"]`).value = value;
      send("input", { target: { closest: (sel) => (sel === ".idea-editor" ? editor : null) } });
    },
    part,
  };
}

const sent = (p) => p.calls.filter((c) => c.cmd !== "list_ideas");

test("the page opens with the intro, the quick-add, the filters and one card per open idea", async () => {
  const p = page();
  await p.show();
  assert.match(p.html, /Things you might do\. Agents read them only when asked; facts you want agents to always know go in memory instead\./);
  assert.match(p.html, /placeholder="Quick idea…"/);
  assert.match(p.html, /<option value="open" selected>Open<\/option><option value="done">Done<\/option><option value="dropped">Dropped<\/option><option value="all">All<\/option>/);
  assert.match(p.html, /type="search"/);
  assert.deepEqual(p.titles(), ["iPad layout for the sidebar", "Cheaper box tier"], "the open ones, in the order the hub keeps");
  const [ipad, box] = p.list();
  assert.match(ipad, /class="idea-meta">site · 2d</, "its project and age");
  assert.match(ipad, /class="idea-text">Split view with threads left\.</, "the first line of its text");
  assert.match(box, /class="idea-meta">— · now · by the Orchestrator</, "no project, and who made it");
  assert.doesNotMatch(ipad, /by /, "an idea the user made says nothing of who");
  assert.deepEqual(p.calls, [{ cmd: "list_ideas" }]);
});

test("the page offers no Schedule, Send to Captain or Import, which need Tasks", async () => {
  const p = page();
  await p.show();
  const everything = `${p.html}${p.list().join("")}${(await p.menu("i1")).map((i) => i.label).join("|")}`;
  assert.doesNotMatch(everything, /Schedul|Send to Captain|Import from/);
  assert.equal(p.part(".ideas-empty").hidden, true);
});

test("a title is escaped on its card", async () => {
  const p = page([idea("<b>bold</b> & \"quoted\"")]);
  await p.show();
  assert.match(p.list()[0], /&#60;b&#62;bold&#60;\/b&#62; &#38; &#34;quoted&#34;/);
});

test("the status filter shows Open, Done, Dropped or All", async () => {
  const p = page();
  await p.show();
  p.filter("status", "done");
  assert.deepEqual(p.titles(), ["Café menu for the demo"]);
  assert.match(p.list()[0], /class="idea-state">Done</);
  p.filter("status", "dropped");
  assert.deepEqual(p.titles(), ["Rewrite the installer"]);
  p.filter("status", "all");
  assert.equal(p.titles().length, 4);
  p.filter("status", "open");
  assert.equal(p.titles().length, 2);
});

test("the project filter lists the projects ideas have and narrows to one", async () => {
  const p = page();
  await p.show();
  p.filter("status", "all");
  assert.match(p.part('.ideas-filters [name="project"]').innerHTML, /^<option value="">All projects<\/option><option value="github.com\/acme\/api">api<\/option><option value="github.com\/acme\/site">site<\/option>$/);
  p.filter("project", SITE);
  assert.deepEqual(p.titles(), ["iPad layout for the sidebar"]);
  p.filter("project", API);
  assert.deepEqual(p.titles(), ["Café menu for the demo"]);
  p.filter("project", "");
  assert.equal(p.titles().length, 4);
});

test("the search looks in the title, the text and the project, ignoring case and accents", async () => {
  const p = page();
  await p.show();
  p.filter("status", "all");
  p.search("CAFE");
  assert.deepEqual(p.titles(), ["Café menu for the demo"]);
  p.search("threads left");
  assert.deepEqual(p.titles(), ["iPad layout for the sidebar"]);
  p.search("site ipad");
  assert.deepEqual(p.titles(), ["iPad layout for the sidebar"], "every word must match, in any field");
  p.search("nothing like this");
  assert.deepEqual(p.titles(), []);
  assert.equal(p.empty(), "No ideas match.");
  p.search("");
  assert.equal(p.titles().length, 4);
});

test("with no ideas the page says how to add one; without a hub it says the hub keeps them", async () => {
  const none = page([]);
  await none.show();
  assert.match(none.empty(), /^No ideas yet\. Type one above/);
  const lone = page([], { signedIn: false });
  await lone.show();
  assert.match(lone.empty(), /kept on your pinned hub/);
  assert.equal(lone.part(".ideas-add").hidden, true);
  assert.deepEqual(lone.calls, [], "it asks the hub for nothing while there is none");
});

test("quick-add sends save_idea with the title alone and shows the new card first", async () => {
  const p = page();
  await p.show();
  await p.add("  Dark mode for the usage page ");
  assert.deepEqual(sent(p), [{ cmd: "save_idea", title: "Dark mode for the usage page" }]);
  assert.equal(p.titles()[0], "Dark mode for the usage page");
  assert.equal(p.run("ideasOpenText()"), "3 open");
});

test("quick-add ignores a blank title and shows the hub's refusal", async () => {
  const p = page();
  await p.show();
  await p.add("   ");
  assert.deepEqual(sent(p), []);
  p.context.connectorCall = async () => {
    throw "an idea needs a title";
  };
  await p.add("x");
  assert.equal(p.part(".ideas-error").textContent, "an idea needs a title");
  assert.equal(p.part(".ideas-error").hidden, false);
});

test("a new idea shows even when the filters would hide it", async () => {
  const p = page();
  await p.show();
  p.filter("status", "done");
  p.filter("project", API);
  await p.add("Another one");
  assert.equal(p.titles()[0], "Another one");
});

test("the ⋯ menu holds Edit, Mark done, Drop and Delete for an open idea", async () => {
  const p = page();
  await p.show();
  assert.deepEqual((await p.menu("i1")).map((i) => i.label), ["Edit", "Mark done", "Drop", "Delete"]);
  assert.deepEqual((await p.menu("i3")).map((i) => i.label), ["Edit", "Drop", "Reopen", "Delete"], "a done idea can be reopened");
});

test("Mark done sends save_idea with the status and the rest of the idea as it is", async () => {
  const p = page();
  await p.show();
  const items = await p.menu("i1");
  await items.find((i) => i.label === "Mark done").run();
  assert.deepEqual(sent(p), [{ cmd: "save_idea", id: "i1", title: "iPad layout for the sidebar", text: "Split view with threads left.\nSecond line.", project: SITE, status: "done" }]);
  assert.deepEqual(p.titles(), ["Cheaper box tier"], "it leaves the Open list");
  assert.equal(p.run("ideasOpenText()"), "1 open");
});

test("Drop sends save_idea with the status dropped, and no project when it has none", async () => {
  const p = page();
  await p.show();
  await (await p.menu("i2")).find((i) => i.label === "Drop").run();
  assert.deepEqual(sent(p), [{ cmd: "save_idea", id: "i2", title: "Cheaper box tier", text: "", status: "dropped" }]);
  p.filter("status", "dropped");
  assert.deepEqual(p.titles(), ["Cheaper box tier", "Rewrite the installer"]);
});

test("Delete asks once and sends delete_idea only after the confirm", async () => {
  const p = page();
  await p.show();
  p.context.answer = false;
  await (await p.menu("i2")).find((i) => i.label === "Delete").run();
  assert.equal(p.context.confirmed.length, 1);
  assert.deepEqual(sent(p), [], "a cancelled confirm deletes nothing");
  assert.equal(p.titles().length, 2);
  p.context.answer = true;
  await (await p.menu("i2")).find((i) => i.label === "Delete").run();
  assert.equal(p.context.confirmed.length, 2);
  assert.deepEqual(p.context.confirmed[1], { title: "Delete “Cheaper box tier”?", verb: "Delete", danger: true });
  assert.deepEqual(sent(p), [{ cmd: "delete_idea", id: "i2" }]);
  assert.deepEqual(p.titles(), ["iPad layout for the sidebar"]);
});

test("a refused save leaves the idea as it was and says why", async () => {
  const p = page();
  await p.show();
  const mark = (await p.menu("i1")).find((i) => i.label === "Mark done");
  p.context.connectorCall = async () => {
    throw "this account has no idea i1";
  };
  await mark.run();
  assert.equal(p.part(".ideas-error").textContent, "this account has no idea i1");
  assert.equal(p.titles().length, 2);
});

test("the editor autosaves the title and text after typing, and not before", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i1");
  const editor = p.run("ideasUi.editor");
  assert.match(editor.html, /name="title" value="iPad layout for the sidebar"/);
  p.type(editor, "text", "Split view, threads left, chat right.");
  assert.deepEqual(sent(p), [], "typing alone sends nothing");
  const timer = p.timers.at(-1);
  assert.equal(timer.ms, 700);
  await timer.f();
  assert.deepEqual(sent(p), [{ cmd: "save_idea", id: "i1", title: "iPad layout for the sidebar", text: "Split view, threads left, chat right.", project: SITE }]);
  assert.equal(p.part(".idea-saving").textContent, "Saved");
  assert.equal(p.store.rows.find((i) => i.id === "i1").text, "Split view, threads left, chat right.");
});

test("an emptied title is not saved, and an unchanged editor sends nothing", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i2");
  const editor = p.run("ideasUi.editor");
  p.type(editor, "title", "  ");
  await p.timers.at(-1).f();
  assert.deepEqual(sent(p), []);
  assert.equal(p.part(".idea-saving").textContent, "A title is needed");
  p.type(editor, "title", "Cheaper box tier");
  await p.timers.at(-1).f();
  assert.deepEqual(sent(p), [], "nothing changed from what is saved");
});

test("closing the editor saves what is typed at once", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i2");
  const editor = p.run("ideasUi.editor");
  p.type(editor, "title", "Cheaper box tier, Pro");
  await p.click("close", "i2");
  assert.deepEqual(sent(p), [{ cmd: "save_idea", id: "i2", title: "Cheaper box tier, Pro", text: "" }]);
  assert.equal(p.run("ideasUi.editing"), null);
  assert.equal(p.titles()[0], "Cheaper box tier, Pro", "the edited idea is now the most recent");
});

test("a settings push, which carries no ideas, makes the page read them again", async () => {
  const p = page();
  await p.show();
  assert.equal(p.titles().length, 2);
  p.store.rows.unshift(idea("Added by an agent meanwhile", { created_by: orchestrator, updated_at: NOW }));
  p.run("ideasRefresh()");
  p.run("ideasRefresh()");
  assert.equal(p.timers.length, 2, "each push waits a moment, so a burst reads once");
  await p.timers.at(-1).f();
  await flush();
  assert.equal(p.titles()[0], "Added by an agent meanwhile");
  assert.equal(p.calls.filter((c) => c.cmd === "list_ideas").length, 2);
});

test("the settings handler asks the Ideas script to read again, and the push keeps no list", () => {
  const settings = read("settings.js");
  assert.match(fn(settings, "onHubSettings"), /if \(typeof ideasRefresh === "function"\) ideasRefresh\(\);/);
  const rust = readFileSync(new URL("../crates/core/src/connectors.rs", import.meta.url), "utf8");
  assert.ok(!/pub ideas: Vec/.test(rust), "ideas ride on list_ideas, so the settings push stays bounded");
});

/** drawNav on a page whose Ideas script holds `rows`, with every other part stubbed. */
async function nav(rows, { withScript = true } = {}) {
  const p = page(rows);
  const parts = {};
  const part = () => ({ hidden: false, textContent: "", title: "", style: {}, classList: { toggle() {}, add() {}, remove() {} }, attributes: {}, setAttribute() {}, removeAttribute() {}, querySelector(sel) { return (this.parts[sel] ??= part()); }, parts: {} });
  const context = {
    machines: new Map(),
    LOCAL: "local",
    active: null,
    globalSettings: { name: "Orchestrator" },
    askGlobalSettings() {},
    navCounts: () => ({ needsYou: 0, orchestrator: 0, reviews: 0, usage: null, machinesOnline: 0, machinesTotal: 0 }),
    shellPage: () => null,
    navRow: (name) => (parts[name] ??= part()),
    isGlobalOrchestratorTab: () => false,
    machineName: () => "machine",
    globalHostMachine: () => null,
    document: { getElementById: () => part() },
    accountButton: part(),
    bindingLabel: () => null,
    ...(withScript ? { ideasOpenText: () => p.run("ideasOpenText()") } : {}),
  };
  if (withScript) await p.show();
  vm.createContext(context);
  vm.runInContext(`${line(sidebarSrc, "const setText")}\n${fn(sidebarSrc, "drawCount")}\n${fn(sidebarSrc, "drawNav")}\ndrawNav();`, context);
  return parts.ideas.querySelector(".meta");
}

test("the sidebar's Ideas row says how many are open", async () => {
  const meta = await nav(SEED());
  assert.deepEqual([meta.hidden, meta.textContent], [false, "2 open"], "done and dropped ideas do not count");
});

test("the sidebar's count is the whole list, past any size a settings push could carry", async () => {
  const many = Array.from({ length: 700 }, (_, i) => idea(`Idea ${i}`));
  const meta = await nav([...many, ...SEED()]);
  assert.equal(meta.textContent, "702 open");
});

test("with no open idea, or no Ideas script, the row shows no count", async () => {
  assert.equal((await nav([idea("Done one", { status: "done" })])).hidden, true);
  assert.equal((await nav([], { withScript: false })).hidden, true);
});

test("the sidebar has an Ideas row between Needs you and Pull requests, and no Tasks row", () => {
  const rows = [...html.matchAll(/data-nav="([a-z-]+)"/g)].map((m) => m[1]);
  assert.deepEqual(rows.slice(rows.indexOf("needs-you"), rows.indexOf("pull-requests") + 1), ["needs-you", "ideas", "pull-requests"]);
  assert.ok(!rows.includes("tasks"));
  assert.match(html, /data-nav="ideas" data-open-page="ideas">.*<span class="meta" hidden><\/span>/);
  assert.ok(html.indexOf("gui-ideas.js") > html.indexOf("gui-shell.js"), "the page registers after the shell exists");
});

test("More lists Ideas first, with the open count as its hint", async () => {
  const first = phoneSrc.slice(phoneSrc.indexOf("const MORE_LISTS = ["));
  assert.deepEqual([...first.matchAll(/\{ id: "([a-z-]+)"/g)].map((m) => m[1]).slice(0, 3), ["ideas", "pull-requests", "projects"]);
  assert.match(first, /\{ id: "ideas", label: "Ideas", hint: moreHints\.ideas, open: \(\) => openPage\("ideas"\) \}/);
  const p = page();
  await p.show();
  const context = { ideasOpenText: () => p.run("ideasOpenText()") };
  vm.createContext(context);
  vm.runInContext(`const moreHints = {\n${line(phoneSrc, "  ideas: () =>")}\n};`, context);
  assert.equal(vm.runInContext("moreHints.ideas()", context), "2 open");
});

test("on a phone the quick-add is pinned at the foot of the page", () => {
  const css = read("gui-ideas.css");
  assert.match(css, /@media \(max-width: 768px\)[\s\S]*\.ideas-add \{[^}]*position: sticky; bottom: 0;/);
});

test("the shell's stylesheet shows the page view for Ideas wherever it does for Needs you", () => {
  const css = read("gui-shell.css");
  const count = (page) => css.split(`[data-page="${page}"]`).length - 1;
  assert.ok(count("needs-you") > 0);
  assert.equal(count("ideas"), count("needs-you"));
});
