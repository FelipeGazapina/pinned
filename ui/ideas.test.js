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

const doc = { activeElement: null };
/** Puts `nodes` among `parent`'s children from `index`, taking each out of where it was, as the DOM does. */
const attach = (parent, nodes, index) => {
  for (const node of nodes) node.remove?.();
  parent.children.splice(index, 0, ...nodes);
  for (const node of nodes) node.parentNode = parent;
};
/** A page element that keeps what the script sets on it, and a small DOM: a node taken out of its parent
 * loses the focus it had and counts a `removal`, as in a browser, so a test can tell an element that was
 * redrawn from one that stayed. */
const el = (extra = {}) => ({
  focus() {
    doc.activeElement = this;
  },
  hidden: false,
  textContent: "",
  innerHTML: "",
  value: "",
  children: [],
  dataset: {},
  parentNode: null,
  removals: 0,
  remove() {
    if (!this.parentNode) return;
    this.parentNode.children = this.parentNode.children.filter((c) => c !== this);
    this.parentNode = null;
    this.removals += 1;
    if (doc.activeElement === this) doc.activeElement = null;
  },
  get firstChild() {
    return this.children[0] ?? null;
  },
  get nextSibling() {
    return this.parentNode?.children[this.parentNode.children.indexOf(this) + 1] ?? null;
  },
  insertBefore(node, ref) {
    attach(this, [node], ref ? this.children.indexOf(ref) - (node.parentNode === this && this.children.indexOf(node) < this.children.indexOf(ref) ? 1 : 0) : this.children.length - (node.parentNode === this ? 1 : 0));
  },
  after(...nodes) {
    attach(this.parentNode, nodes, this.parentNode.children.indexOf(this) + 1);
  },
  prepend(...nodes) {
    attach(this, nodes, 0);
  },
  replaceChildren(...nodes) {
    for (const child of [...this.children]) child.remove();
    attach(this, nodes, 0);
  },
  ...extra,
});

/** gui-ideas.js in a page whose hub holds `rows`. `calls` lists what the page sent, in order. */
function page(rows = SEED(), { signedIn = true } = {}) {
  doc.activeElement = null;
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
    pageHandlers: handlers,
    pageView: { addEventListener: (type, f) => (handlers[type] = f), querySelector: (sel) => part(sel) },
    document: {
      get activeElement() {
        return doc.activeElement;
      },
      set activeElement(node) {
        doc.activeElement = node;
      },
      createElement: (tag) => (tag === "template" ? { set innerHTML(h) { this.content = { firstElementChild: el({ html: h, dataset: { id: /data-id="([^"]+)"/.exec(h)?.[1] } }) }; } } : editorEl(tag)),
    },
    setTimeout: (f, ms) => {
      const timer = { f, ms, cancelled: false };
      timers.push(timer);
      return timer;
    },
    clearTimeout: (timer) => {
      if (timer) timer.cancelled = true;
    },
    modelChanged: () => (context.changed += 1),
    changed: 0,
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
    const node = el();
    Object.defineProperty(node, "innerHTML", {
      set(h) {
        this.html = h;
        fields.title = el({ value: /name="title" value="([^"]*)"/.exec(h)?.[1] ?? "" });
        fields.text = el({ value: />([^<]*)<\/textarea>/.exec(h)?.[1] ?? "" });
      },
    });
    node.querySelector = (sel) => fields[/name="(\w+)"/.exec(sel)?.[1] ?? /idea-edit-(\w+)/.exec(sel)?.[1]] ?? (fields[sel] ??= el());
    // A field inside an editor that leaves the page loses the focus with it.
    const leave = node.remove;
    node.remove = function () {
      const had = this.parentNode;
      leave.call(this);
      if (had && Object.values(fields).includes(doc.activeElement)) doc.activeElement = null;
    };
    return node;
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
    onEditor: (type, editor, extra = {}) => send(type, { target: { closest: (sel) => (sel === ".idea-editor" ? editor : null) }, relatedTarget: null, ...extra }),
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

const reads = (p) => p.calls.filter((c) => c.cmd === "list_ideas").length;
const typed = (p, field) => p.run("ideasUi.editor").querySelector(`[name="${field}"]`);

test("an ideas_changed signal makes the page read them again, a burst of five reading once", async () => {
  const p = page();
  await p.show();
  assert.equal(p.titles().length, 2);
  p.store.rows.unshift(idea("Added by an agent meanwhile", { created_by: orchestrator, updated_at: NOW }));
  const before = reads(p);
  for (let i = 0; i < 5; i += 1) p.run("ideasRefresh()");
  const live = p.timers.filter((t) => !t.cancelled);
  assert.equal(live.length, 1, "each signal replaces the wait of the one before");
  await live[0].f();
  await flush();
  assert.equal(reads(p) - before, 1, "five signals, one read");
  assert.equal(p.titles()[0], "Added by an agent meanwhile");
});

test("a signal that arrives while the page is reading queues one more read, however many arrive", async () => {
  const p = page();
  await p.show();
  const before = reads(p);
  let release;
  const gate = new Promise((r) => (release = r));
  const call = p.context.connectorCall;
  p.context.connectorCall = async (cmd, args) => (cmd === "list_ideas" ? (await gate, call(cmd, args)) : call(cmd, args));
  const first = p.run("ideasLoad()");
  p.run("ideasLoad()");
  p.run("ideasLoad()");
  p.run("ideasLoad()");
  release();
  await first;
  await flush();
  assert.equal(reads(p) - before, 2, "the read in flight, and one after it");
});

test("a settings push alone reads no ideas", async () => {
  const p = page();
  await p.show();
  const before = reads(p);
  // What the app does for a settings push, whatever it is wired to.
  const settings = read("settings.js");
  const context = { settings: null, settingsOpen: () => false, ideasRefresh: () => p.run("ideasRefresh()") };
  vm.createContext(context);
  vm.runInContext(fn(settings, "onHubSettings"), context);
  for (let i = 0; i < 3; i += 1) vm.runInContext(`onHubSettings({ connectors: [] })`, context);
  assert.deepEqual(p.timers.filter((t) => !t.cancelled), [], "no read is scheduled");
  await flush();
  assert.equal(reads(p) - before, 0);
  assert.ok(!/ideas/.test(fn(settings, "onHubSettings")), "the settings handler knows nothing of ideas");
  const rust = readFileSync(new URL("../crates/core/src/connectors.rs", import.meta.url), "utf8");
  assert.ok(!/pub ideas: Vec/.test(rust), "ideas ride on list_ideas, so the settings push stays bounded");
});

test("the app reads ideas once after the hub says hello and on ideas_changed, and a push wires to neither", () => {
  const main = read("main.js");
  const hello = main.slice(main.indexOf("    hello(frame) {"), main.indexOf("    machines: (list, tabs)"));
  assert.match(hello, /if \(typeof ideasLoad === "function"\) ideasLoad\(\);/);
  assert.match(main, /ideasChanged: \(\) => typeof ideasRefresh === "function" && ideasRefresh\(\),/);
  assert.match(main, /settings: \(latest\) => onHubSettings\(latest\),/);
});

test("opening a second idea saves the first and leaves the second open", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i1");
  p.type(p.run("ideasUi.editor"), "text", "Typed but not yet saved");
  await p.click("edit", "i2");
  assert.equal(p.run("ideasUi.editing"), "i2");
  assert.equal(p.run("ideasUi.editor.dataset.id"), "i2");
  assert.deepEqual(sent(p), [{ cmd: "save_idea", id: "i1", title: "iPad layout for the sidebar", text: "Typed but not yet saved", project: SITE }]);
  assert.equal(p.store.rows.find((i) => i.id === "i1").text, "Typed but not yet saved");
  assert.deepEqual(p.list().map((h) => h ?? "editor").length, 2, "its editor is in the list");
  assert.equal(p.part(".ideas-list").children.some((c) => c === p.run("ideasUi.editor")), true);
});

test("a draft that cannot be saved on close keeps the editor open with its text and says why, offline", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i1");
  const editor = p.run("ideasUi.editor");
  p.type(editor, "text", "Draft that must not be lost");
  p.context.connectorCall = async () => {
    throw "not connected to the hub";
  };
  await p.click("close", "i1");
  assert.equal(p.run("ideasUi.editing"), "i1", "still open");
  assert.equal(p.run("ideasUi.editor"), editor, "the same editor");
  assert.equal(editor.querySelector('[name="text"]').value, "Draft that must not be lost");
  assert.equal(p.part(".idea-saving").textContent, "not connected to the hub");
  assert.equal(p.part(".idea-saving").dataset.state, "error");
  await p.click("edit", "i2");
  assert.equal(p.run("ideasUi.editing"), "i1", "another idea does not open over a draft that is not saved");
});

test("a draft with an emptied title keeps the editor open on close, and a fixed title closes it", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i2");
  const editor = p.run("ideasUi.editor");
  p.type(editor, "title", "   ");
  p.type(editor, "text", "Notes I wrote");
  await p.click("close", "i2");
  assert.equal(p.run("ideasUi.editing"), "i2");
  assert.equal(editor.querySelector('[name="text"]').value, "Notes I wrote");
  assert.equal(p.part(".idea-saving").textContent, "A title is needed");
  assert.equal(p.part(".idea-saving").dataset.state, "error");
  assert.deepEqual(sent(p), []);
  p.type(editor, "title", "Cheaper box tier, Pro");
  await p.click("close", "i2");
  assert.equal(p.run("ideasUi.editing"), null);
  assert.deepEqual(sent(p), [{ cmd: "save_idea", id: "i2", title: "Cheaper box tier, Pro", text: "Notes I wrote" }]);
});

test("the open editor stays the same element, with its focus, through every redraw", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i2");
  const editor = p.run("ideasUi.editor");
  const field = typed(p, "text");
  assert.equal(p.run("document.activeElement"), field, "the text field has the focus");
  p.type(editor, "text", "first part");
  await p.timers.at(-1).f(); // the autosave: a redraw when it starts and one when it ends
  p.type(editor, "text", "first part, second part");
  assert.equal(p.run("document.activeElement"), field, "focus kept after the autosave");
  // An agent saves an idea that sorts above the open one, and the page reads it.
  p.store.rows.unshift(idea("Added above", { created_by: orchestrator, updated_at: NOW + 5 }));
  p.run("ideasLoad()");
  await flush();
  assert.equal(p.titles()[0], "Added above");
  assert.equal(p.run("ideasUi.editor"), editor);
  assert.equal(p.part(".ideas-list").children.indexOf(editor), 2, "it sits after the new card and the one above it");
  assert.equal(editor.removals, 0, "the editor was never taken out of the page");
  assert.equal(p.run("document.activeElement"), field);
  assert.equal(field.value, "first part, second part", "the text typed during the save is still there");
  p.filter("status", "all");
  assert.equal(editor.removals, 0, "a filter change does not take it out either");
});

test("a redraw keeps the cards that did not change, so a click that began before it still lands", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i1");
  p.type(p.run("ideasUi.editor"), "text", "Typed, not saved yet");
  const cardOf = (id) => p.part(".ideas-list").children.find((c) => c.dataset.id === id);
  const second = cardOf("i2");
  // Pressing the mouse on the second card takes the focus out of the editor, which saves and redraws the page.
  p.onEditor("focusout", p.run("ideasUi.editor"));
  await flush();
  assert.equal(sent(p).length, 1, "the blur saved the first idea");
  assert.equal(cardOf("i2"), second, "the second card is the element the mouse went down on");
  assert.equal(second.removals, 0, "and it was never taken out of the page");
  await p.click("edit", "i2");
  assert.equal(p.run("ideasUi.editing"), "i2", "so the click opens it");
});

test("pressing Done or Save keeps the focus in the field, so the tab bar that returns on a phone cannot cover the button", async () => {
  const p = page();
  await p.show();
  const press = (button) => {
    let prevented = false;
    p.context.pageHandlers.mousedown({ target: { closest: (sel) => (sel.includes(button) ? {} : null) }, preventDefault: () => (prevented = true) });
    return prevented;
  };
  assert.equal(press(".idea-close"), true, "Done");
  assert.equal(press(".ideas-save"), true, "Save");
  assert.equal(p.context.pageHandlers.mousedown({ target: { closest: () => null }, preventDefault: () => assert.fail("another press is left alone") }), undefined);
});

test("closing an editor the user did not touch sends nothing, and an agent's edit stays", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i2");
  const row = p.store.rows.find((i) => i.id === "i2");
  p.store.rows = p.store.rows.map((i) => (i.id === "i2" ? { ...row, title: "Cheaper box tier (agent)", text: "The agent added a note.", updated_at: NOW + 9 } : i));
  p.run("ideasLoad()");
  await flush();
  await p.click("close", "i2");
  assert.deepEqual(sent(p), [], "no save_idea");
  assert.equal(p.store.rows.find((i) => i.id === "i2").text, "The agent added a note.");
  assert.equal(p.run("ideasUi.editing"), null);
});

test("a change in one field keeps the other field as the agent last edited it", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i2");
  const row = p.store.rows.find((i) => i.id === "i2");
  p.store.rows = p.store.rows.map((i) => (i.id === "i2" ? { ...row, text: "The agent added a note.", updated_at: NOW + 9 } : i));
  p.run("ideasLoad()");
  await flush();
  p.type(p.run("ideasUi.editor"), "title", "A better title");
  await p.click("close", "i2");
  assert.deepEqual(sent(p), [{ cmd: "save_idea", id: "i2", title: "A better title", text: "The agent added a note." }]);
});

test("a status change goes out on the idea as it is now, not as the menu saw it", async () => {
  const p = page();
  await p.show();
  const items = await p.menu("i1");
  p.store.rows = p.store.rows.map((i) => (i.id === "i1" ? { ...i, title: "iPad layout (agent rename)", text: "Agent text.", updated_at: NOW + 9 } : i));
  p.run("ideasLoad()");
  await flush();
  await items.find((i) => i.label === "Mark done").run();
  assert.deepEqual(sent(p), [{ cmd: "save_idea", id: "i1", title: "iPad layout (agent rename)", text: "Agent text.", project: SITE, status: "done" }]);
});

test("one change in a field sends one save, though the change, the blur and Done all ask", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i1");
  const editor = p.run("ideasUi.editor");
  p.type(editor, "title", "iPad layout, wider");
  p.onEditor("change", editor);
  p.onEditor("focusout", editor);
  await p.click("close", "i1");
  assert.deepEqual(sent(p).map((c) => c.cmd), ["save_idea"], "exactly one");
  assert.equal(sent(p)[0].title, "iPad layout, wider");
  assert.equal(p.run("ideasUi.editing"), null);
});

test("a save that failed is not sent again by the blur that follows it, but Done tries once more", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i1");
  const editor = p.run("ideasUi.editor");
  p.type(editor, "text", "Draft");
  let down = true;
  const call = p.context.connectorCall;
  p.context.connectorCall = async (cmd, args) => {
    if (cmd === "save_idea" && down) {
      p.calls.push({ cmd, ...args });
      throw "not connected to the hub";
    }
    return call(cmd, args);
  };
  p.onEditor("change", editor);
  p.onEditor("focusout", editor);
  await flush();
  assert.equal(sent(p).length, 1);
  down = false;
  await p.click("close", "i1");
  assert.equal(sent(p).length, 2, "Done sends the draft again");
  assert.equal(p.run("ideasUi.editing"), null, "and closes once it is saved");
  assert.equal(p.store.rows.find((i) => i.id === "i1").text, "Draft");
});

test("the editor has no project picker, and a save keeps the idea's project", async () => {
  const p = page();
  await p.show();
  await p.click("edit", "i1");
  assert.doesNotMatch(p.run("ideasUi.editor").html, /<select|name="project"/);
  assert.doesNotMatch(ideasSrc, /ideaProjectOptions/);
  assert.doesNotMatch(read("gui-ideas.css"), /idea-edit-foot select|idea-editor :is\(input, select/);
  p.type(p.run("ideasUi.editor"), "title", "iPad layout, wider");
  await p.click("close", "i1");
  assert.equal(sent(p)[0].project, SITE);
  assert.equal(p.store.rows.find((i) => i.id === "i1").project, SITE);
});

test("the project filter's options are written once, not on every draw", async () => {
  const p = page();
  let writes = 0;
  const select = p.part('.ideas-filters [name="project"]');
  let html = "";
  Object.defineProperty(select, "innerHTML", { get: () => html.replace(' selected', ' selected=""'), set: (v) => ((html = v), (writes += 1)) });
  await p.show();
  p.filter("project", SITE);
  p.search("ipad");
  p.search("");
  p.run("drawIdeas()");
  p.run("drawIdeas()");
  assert.equal(writes, 3, "when the page opens, when the list arrives, and when a project is chosen; no draw after that writes them");
});

test("Reopen on a done idea sends save_idea with status open, and it returns to the Open list", async () => {
  const p = page();
  await p.show();
  p.filter("status", "done");
  assert.deepEqual(p.titles(), ["Café menu for the demo"]);
  await (await p.menu("i3")).find((i) => i.label === "Reopen").run();
  assert.deepEqual(sent(p), [{ cmd: "save_idea", id: "i3", title: "Café menu for the demo", text: "", project: API, status: "open" }]);
  assert.deepEqual(p.titles(), [], "it left the Done list");
  p.filter("status", "open");
  assert.ok(p.titles().includes("Café menu for the demo"), "it is in the Open list");
  assert.equal(p.run("ideasOpenText()"), "3 open");
});

test("⌘K lists Open Ideas for “ideas”, and running it opens the Ideas page", () => {
  const shell = read("gui-shell.js");
  const opened = [];
  const context = {
    active: null,
    contextProject: () => null,
    paletteProjects: () => [],
    onlineMachines: () => [],
    promptLinkItems: () => [],
    themeActionItems: () => [],
    openPage: (name) => opened.push(name),
    esc: (t) => t,
    icon: () => "",
    document: { createElement: () => ({ set innerHTML(h) { this.content = { textContent: h.replace(/<[^>]*>/g, "") }; } }) },
  };
  vm.createContext(context);
  vm.runInContext([line(shell, "const normalize"), fn(shell, "searchFields"), fn(shell, "rankItem"), fn(shell, "filterGroups"), fn(shell, "actionItems"), shell.slice(shell.indexOf("const startsWord"), shell.indexOf("\n};\n", shell.indexOf("const startsWord")) + 4)].join("\n"), context);
  const groups = vm.runInContext(`filterGroups([{ label: "Actions", items: actionItems() }], "ideas")`, context);
  assert.deepEqual(here(groups.map((g) => g.items.map((i) => i.title))), [["Open Ideas"]]);
  groups[0].items[0].run();
  assert.deepEqual(opened, ["ideas"]);
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

test("the idea being edited keeps its place when its own autosave's ideas_changed read lists it first", async () => {
  n = 0;
  const five = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"].map((t, i) => idea(t, { updated_at: NOW - 100 * (i + 1) }));
  const p = page(five);
  await p.show();
  assert.deepEqual(p.titles(), ["Alpha", "Bravo", "Charlie", "Delta", "Echo"]);
  await p.click("edit", "i4");
  const editor = p.run("ideasUi.editor");
  const field = typed(p, "text");
  p.type(editor, "text", "typed while the hub saves");
  await p.timers.at(-1).f(); // the autosave
  assert.equal(p.calls.filter((c) => c.cmd === "save_idea").length, 1);
  // The hub lists newest edit first, so the idea just saved is now the first row.
  const saved = p.store.rows.find((i) => i.id === "i4");
  p.store.rows = [{ ...saved, updated_at: NOW + 1 }, ...p.store.rows.filter((i) => i.id !== "i4")];
  p.run("ideasRefresh()"); // the autosave's own ideas_changed, about 300 ms later
  await p.timers.filter((t) => !t.cancelled).at(-1).f();
  await flush();
  assert.deepEqual(p.titles().filter((t) => t !== undefined), ["Alpha", "Bravo", "Charlie", "Echo"], "the other cards keep the hub's order, and the editor is not a card");
  assert.equal(p.part(".ideas-list").children.indexOf(editor), 3, "the open card is still the fourth");
  assert.equal(p.run("ideasUi.editor"), editor, "the editor element is the same");
  assert.equal(editor.removals, 0, "it was never taken out of the page");
  assert.equal(field.value, "typed while the hub saves", "the text is unchanged");
  // Closing it sorts as before: the saved idea is the newest.
  await p.click("close", "i4");
  assert.equal(p.titles()[0], "Delta");
});

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
