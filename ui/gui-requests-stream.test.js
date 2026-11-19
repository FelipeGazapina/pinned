import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const line = (src, from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
const flush = () => new Promise((resolve) => setImmediate(resolve));
const plain = (value) => JSON.parse(JSON.stringify(value));

function element() {
  return {
    hidden: false,
    dataset: {},
    innerHTML: "",
    textContent: "",
    attrs: {},
    style: {},
    listeners: {},
    setAttribute(name, value) {
      this.attrs[name] = String(value);
    },
    removeAttribute(name) {
      delete this.attrs[name];
    },
    toggleAttribute(name, force) {
      if (force) this.attrs[name] = "";
      else delete this.attrs[name];
      return !!force;
    },
    hasAttribute(name) {
      return name in this.attrs;
    },
    addEventListener(type, fn) {
      this.listeners[type] = fn;
    },
    after() {},
    append() {},
    querySelector: () => null,
    querySelectorAll: () => [],
    classList: { toggle() {}, add() {}, remove() {} },
  };
}

const glance = (id, over = {}) => ({
  id,
  title: `Request ${id}`,
  project: "storefront",
  opened_at: 1000,
  kind: "asked",
  closed_at: null,
  needs: "",
  needs_at: null,
  last_user_at: null,
  last_who: null,
  last_text: "",
  last_at: null,
  replies: 0,
  captains: [],
  tab: null,
  ...over,
});
const stands = (over = {}) => ({ asked: "Find out why codes are late.", done: "Cause found.", left: "Tell support.", needs: "", links: ["PR #212 merged"], at: 5000, ...over });
const userView = (id, over = {}) => ({ glance: glance(id), stands: stands(), items: [], plans: [], ...over });

const CATALOG = {
  name: "Orchestrator",
  machines: [{ slug: "laptop", name: "laptop", online: true }, { slug: "studio", name: "studio", online: true }],
  projects: [
    { slug: "storefront", name: "storefront", repo: "", machines: [{ id: "mac", slug: "laptop", name: "laptop", online: true, captain: "working" }] },
    { slug: "billing-api", name: "billing-api", repo: "", machines: [{ id: "studio", slug: "studio", name: "studio", online: true, captain: "idle" }] },
  ],
};
const FACTS = [
  { slug: "storefront", name: "storefront", machine: { id: "mac", name: "laptop" }, state: "working" },
  { slug: "billing-api", name: "billing-api", machine: { id: "studio", name: "studio" }, state: "idle" },
];
/** The app's `markdown()` is gui-timeline.js's, which this page does not load: the same vendored marked, with raw HTML and unsafe links and images drawn as text. */
const MARKDOWN_STUB = `const markdownParser = new marked.Marked({ gfm: true, breaks: true, renderer: {
  html: ({ text }) => esc(text),
  image: ({ text }) => esc(text),
  link({ href, tokens }) { const inner = this.parser.parseInline(tokens); return /^https?:/i.test(href) ? '<a href="' + esc(href) + '">' + inner + '</a>' : inner; },
} });
function markdown(text) { return markdownParser.parse(text).trim(); }`;
const TWO = "#storefront show the delivery estimate on the cart page.\n\n#billing-api @studio the monthly export is missing refunds.";
const made = (call) => call.items.map((item, i) => ({ key: item.key, id: `q${i + 1}`, tab: `tab-${i + 5}`, handoff: "started" }));
const typeAsk = (h, value) => h.overlay.listeners.input({ target: { closest: (sel) => (sel === "[data-rqs-ask]" ? { value } : null) } });
const typeReply = (h, id, value) => h.overlay.listeners.input({ target: { closest: (sel) => (sel === "[data-rqs-reply]" ? { value, dataset: { rqsReply: id } } : null) } });
const enter = (h, sel, extra = {}) => h.overlay.listeners.keydown({ key: "Enter", shiftKey: false, isComposing: false, preventDefault() {}, target: { closest: (s) => (s === sel ? extra : null) } });

/** gui-requests.js and gui-requests-stream.js in a page: the app's globals stubbed, the real esc and the real model. */
function harness({ tabs = [], reply = () => [], host = "mac" } = {}) {
  const log = { calls: [], told: [], activated: [], revealed: [] };
  const els = { chat: element(), crumb: element(), footer: element(), head: element() };
  const made = [];
  const context = {
    console,
    Date,
    Math,
    JSON,
    Promise,
    Map,
    Set,
    Array,
    Object,
    String,
    Number,
    setTimeout,
    clearTimeout,
    tabs,
    active: null,
    machines: new Map([[host, { view: { online: true } }]]),
    globalHostMachine: () => host,
    isGlobalOrchestratorTab: (tab) => tab?.view?.orchestrator_global === true,
    guiOn: () => true,
    narrow: () => false,
    shellPage: () => null,
    ago: () => "12m",
    icon: (name) => `<svg data-icon="${name}"></svg>`,
    globalOrchestratorName: () => "Orchestrator",
    waitingKindOf: () => null,
    onModelChange() {},
    visibleInterval: () => {},
    renderThread() {},
    tell: (html) => log.told.push(html),
    crypto: { randomUUID: (() => { let n = 0; return () => `key-${++n}`; })() },
    globalSettings: { name: "Orchestrator", busy: "queue" },
    composerTagCatalog: () => CATALOG,
    captainFacts: () => FACTS,
    activate: (tab) => {
      context.active = tab;
      log.activated.push(tab.key);
    },
    revealTab: async (machine, id) => {
      log.revealed.push(`${machine}:${id}`);
      const tab = context.tabs.find((t) => t.key === `${machine}:${id}`);
      context.activate(tab);
      return tab;
    },
    callOn: async (machine, call) => {
      log.calls.push([machine, plain(call)]);
      return context.reply(call);
    },
    reply,
    localStorage: { getItem: () => null, setItem() {} },
    document: {
      getElementById: (id) => ({ chat: els.chat, "chat-head": els.head, "composer-footer": els.footer })[id] ?? null,
      querySelector: (sel) => (sel === "#chat-head .crumb" ? els.crumb : null),
      createElement: () => {
        const el = element();
        made.push(el);
        return el;
      },
    },
  };
  vm.createContext(context);
  const run = (code) => vm.runInContext(code, context);
  run(line(settingsSrc, "const esc = "));
  run(read("vendor/marked.js"));
  run(MARKDOWN_STUB);
  for (const file of ["composer-tags.js", "requests-model.js", "gui-requests.js", "gui-requests-new.js", "gui-requests-stream.js"]) run(read(file));
  const overlay = run("requestsStream");
  const click = (selectors) => overlay.listeners.click({ target: { closest: (sel) => selectors[sel] ?? null } });
  const entry = (id) => ({ "[data-rqs-entry]": { dataset: { rqsEntry: id } } });
  /** The list element: a fresh one (scrolled to 0) on each redraw, 100 high per entry plus 200, in a 100 high box. */
  const list = { scrollTop: 0, clientHeight: 100, scrollHeight: 0, closest: (sel) => (sel === "[data-rqs-list]" ? list : null) };
  let drawn = "";
  Object.defineProperty(overlay, "innerHTML", {
    get: () => drawn,
    set: (value) => {
      drawn = value;
      list.scrollTop = 0;
      list.scrollHeight = (value.match(/data-rqs-entry=/g) ?? []).length * 100 + 200;
    },
  });
  overlay.querySelector = (sel) => (sel === "[data-rqs-list]" && drawn ? list : null);
  /** A scroll of the list to its top (or to the middle). */
  const scroll = (atTop = true) => {
    list.scrollTop = atTop ? 10 : 250;
    overlay.listeners.scroll({ target: list });
  };
  return { context, run, log, els, overlay, click, entry, scroll, list };
}

const anchorTab = (requests) => ({ key: "mac:tab-1", machine: "mac", view: { id: "tab-1", orchestrator_global: true, requests, plans: [] } });
const requestTab = (id, tabId) => ({ key: `mac:${tabId}`, machine: "mac", view: { id: tabId, orchestrator_global: true, request: id, plans: [] } });

const tagNames = (html) => [...html.matchAll(/<([a-z][a-z0-9-]*)/gi)].map((m) => m[1].toLowerCase());
const hrefs = (html) => [...html.matchAll(/\bhref\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi)].map((m) => m[1]);
const local = (y, m, d, hh = 0, mm = 0) => new Date(y, m - 1, d, hh, mm).getTime();
const open = (h) => h.run("openAllActivity()");

test("9r5 item 6: requests group by their local day, oldest first like a chat, and midnight splits a day", () => {
  const h = harness();
  const now = local(2026, 11, 18, 12);
  const glances = [
    glance("late", { opened_at: local(2026, 11, 17, 23, 30) }),
    glance("early", { opened_at: local(2026, 11, 18, 0, 30) }),
    glance("noon", { opened_at: local(2026, 11, 18, 11, 5) }),
    glance("old", { opened_at: local(2026, 11, 3, 9) }),
  ];
  const days = plain(h.run(`requestDays(${JSON.stringify(glances)}, ${now})`));
  assert.deepEqual(days.map((d) => [d.label, d.glances.map((g) => g.id)]), [
    ["Nov 3", ["old"]],
    ["Yesterday", ["late"]],
    ["Today", ["early", "noon"]],
  ]);
});

test("an entry shows the ask with no time, the state, project, reply count and last line", () => {
  const tabs = [anchorTab([
    glance("q-two", { title: "Why are login codes late?", opened_at: local(2026, 11, 18, 9, 5), replies: 2, last_who: "captain", last_text: "Tests are running", last_at: 5, captains: [{ machine: "mac", path: "/s", project: "storefront", machine_name: "Mac" }] }),
    glance("q-one", { opened_at: local(2026, 11, 18, 8, 0), replies: 1, last_who: "orchestrator", last_text: "On it", closed_at: 9 }),
    glance("q-none", { opened_at: local(2026, 11, 18, 7, 0), needs: "Pick one." }),
  ])];
  const h = harness({ tabs });
  open(h);
  const html = h.overlay.innerHTML;
  const two = html.slice(html.lastIndexOf('<div class="rqs-entry">'));
  assert.match(two, /Why are login codes late\?/);
  assert.doesNotMatch(two, /09:05/, "the ask bubble carries no time");
  assert.doesNotMatch(two, /<time/, "the ask bubble carries no time element");
  assert.match(two, /Working/);
  assert.match(two, /#storefront/);
  assert.match(two, /2 replies/);
  assert.match(two, /<span class="last">Tests are running<\/span>/, "the board shows the text, with no speaker");
  assert.match(html, /1 reply[^s]/);
  assert.match(html, /Done/);
  assert.match(html, /No replies yet/);
  assert.match(html, /Needs you/);
});

test("the earlier conversation shows as one entry named Earlier conversation, Done", () => {
  const tabs = [anchorTab([glance("q-old", { kind: "earlier", title: "whatever the host named it", project: null, closed_at: 5, replies: 40, tab: null })])];
  const h = harness({ tabs });
  open(h);
  const html = h.overlay.innerHTML;
  assert.equal(html.match(/data-rqs-entry="/g).length, 1);
  assert.match(html, /Earlier conversation/);
  assert.match(html, /Done/);
  assert.doesNotMatch(html, /whatever the host named it/);
});

test("scrolling to the top asks for older requests before the oldest opened_at, and stops on an empty page", async () => {
  const tabs = [anchorTab([glance("a", { opened_at: 3000 }), glance("b", { opened_at: 2000 }), glance("c", { opened_at: 1000 })])];
  const pages = [[glance("d", { opened_at: 500 }), glance("e", { opened_at: 400 })], []];
  const h = harness({ tabs, reply: (call) => (call.cmd === "user_requests" ? pages.shift() : null) });
  open(h);
  h.scroll(false);
  await flush();
  assert.deepEqual(h.log.calls, [], "not at the top yet");
  h.scroll();
  await flush();
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_requests", before: 1000 }]]);
  assert.match(h.overlay.innerHTML, /data-rqs-entry="e"/);
  h.scroll();
  await flush();
  assert.deepEqual(h.log.calls.at(-1), ["mac", { cmd: "user_requests", before: 400 }]);
  assert.equal(h.log.calls.length, 2);
  h.scroll();
  await flush();
  assert.equal(h.log.calls.length, 2, "an empty page ended the list");
});

test("an error while paging ends the list with no banner", async () => {
  const tabs = [anchorTab([glance("a", { opened_at: 3000 })])];
  const h = harness({ tabs, reply: () => { throw new Error("unknown command"); } });
  open(h);
  h.scroll();
  await flush();
  h.scroll();
  await flush();
  assert.equal(h.log.calls.length, 1);
  assert.deepEqual(h.log.told, []);
});

const threeTabs = () => [anchorTab([glance("a", { opened_at: 3000 }), glance("b", { opened_at: 2000 }), glance("c", { opened_at: 1000 })])];

test("9r6 item 2: opening All activity scrolls the list to its newest end", () => {
  const h = harness({ tabs: threeTabs() });
  open(h);
  assert.equal(h.list.scrollHeight, 500);
  assert.equal(h.list.scrollTop, 500);
  assert.deepEqual(h.log.calls, [], "opening at the bottom pages nothing");
});

test("9r6 item 2: a request that arrives while the user is at the bottom keeps the list at the bottom, and one that arrives while reading higher leaves the place", () => {
  const tabs = threeTabs();
  const h = harness({ tabs });
  open(h);
  tabs[0].view.requests.unshift(glance("d", { opened_at: 4000 }));
  h.run("renderStream()");
  assert.equal(h.list.scrollHeight, 600);
  assert.equal(h.list.scrollTop, 600, "followed the new request");
  h.list.scrollTop = 250;
  tabs[0].view.requests.unshift(glance("e", { opened_at: 5000 }));
  h.run("renderStream()");
  assert.equal(h.list.scrollHeight, 700);
  assert.equal(h.list.scrollTop, 250, "the reader's place is kept");
});

test("9r6 item 2: older requests page in at the top and the reader's place does not jump", async () => {
  const pages = [[glance("d", { opened_at: 500 }), glance("e", { opened_at: 400 })]];
  const h = harness({ tabs: threeTabs(), reply: (call) => (call.cmd === "user_requests" ? pages.shift() : null) });
  open(h);
  h.scroll(false);
  await flush();
  assert.deepEqual(h.log.calls, [], "the middle of the list pages nothing");
  h.scroll(true);
  await flush();
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_requests", before: 1000 }]]);
  assert.equal(h.list.scrollHeight, 700, "two older entries were added above");
  assert.equal(h.list.scrollTop, 210, "10 above the old top, now 200 lower");
});

test("an old host that errors on user_requests leaves All activity doing nothing, with no banner", async () => {
  const h = harness({ reply: () => { throw new Error("unknown command"); } });
  await h.run("refreshRequests()");
  open(h);
  assert.equal(h.overlay.hidden, true);
  assert.equal(h.overlay.innerHTML, "");
  assert.deepEqual(h.log.told, []);
});

test("choosing an entry opens the side panel with the card and the compact items of its user_request", async () => {
  const g = glance("q1", { title: "Codes", needs: "Choose how support is told." });
  const reply = (call) =>
    call.cmd === "user_request"
      ? userView("q1", {
          stands: stands({ needs: "Choose how support is told.", links: ["PR #212 merged"] }),
          items: [
            { who: "user", text: "Find out why.", at: local(2026, 11, 18, 13, 48), captain: null },
            { who: "captain", text: "The code expires after 10 minutes.", at: local(2026, 11, 18, 14, 2), captain: "storefront" },
            { who: "orchestrator", text: "One thing is left for you.", at: local(2026, 11, 18, 14, 3), captain: null },
          ],
        })
      : null;
  const h = harness({ tabs: [anchorTab([g])], reply });
  open(h);
  assert.doesNotMatch(h.overlay.innerHTML, /Where this stands/, "no panel before a choice");
  h.click(h.entry("q1"));
  await flush();
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_request", id: "q1", items: 20 }]]);
  const html = h.overlay.innerHTML;
  assert.match(html, /aria-label="Thread"/);
  assert.match(html, /Where this stands/);
  assert.match(html, /Cause found\./);
  assert.match(html, /Tell support\./);
  assert.match(html, /data-needs="true"[^>]*>Choose how support is told\./);
  assert.match(html, /PR #212 merged/);
  assert.match(html, /You[\s\S]*Find out why\./);
  assert.match(html, /Captain storefront[\s\S]*The code expires after 10 minutes\./);
  assert.match(html, /Orchestrator[\s\S]*One thing is left for you\./);
  assert.match(html, /13:48/);
  assert.equal(h.run("rq.views.size"), 0, "the card's cache of the open thread is not touched");
});

test("Open thread opens the request's own tab, or asks the host to open one when it has none", async () => {
  const own = requestTab("q-tab", "tab-12");
  const fresh = requestTab("q-none", "tab-40");
  const tabs = [anchorTab([glance("q-tab", { tab: "tab-12" }), glance("q-none", { tab: null })]), own];
  const h = harness({ tabs, reply: (call) => (call.cmd === "user_request" ? userView(call.id) : call.cmd === "user_request_action" ? glance("q-none", { tab: "tab-40" }) : null) });
  tabs.push(fresh);
  open(h);
  h.click(h.entry("q-tab"));
  await flush();
  h.click({ "[data-rqs-open]": { dataset: { rqsOpen: "q-tab" } } });
  await flush();
  assert.deepEqual(h.log.activated, ["mac:tab-12"]);
  assert.equal(h.log.calls.some(([, c]) => c.cmd === "user_request_action"), false);

  open(h);
  h.click(h.entry("q-none"));
  await flush();
  h.click({ "[data-rqs-open]": { dataset: { rqsOpen: "q-none" } } });
  await flush();
  assert.deepEqual(h.log.calls.filter(([, c]) => c.cmd === "user_request_action").at(-1), ["mac", { cmd: "user_request_action", id: "q-none", action: "open" }]);
  assert.deepEqual(h.log.revealed, ["mac:tab-40"]);
  assert.deepEqual(h.log.activated, ["mac:tab-12", "mac:tab-40"]);
});

test("closing the panel and All activity hides them", async () => {
  const h = harness({ tabs: [anchorTab([glance("q1")])], reply: (call) => userView(call.id) });
  open(h);
  h.click(h.entry("q1"));
  await flush();
  assert.match(h.overlay.innerHTML, /Where this stands/);
  h.click({ "[data-rqs-panel-close]": {} });
  assert.doesNotMatch(h.overlay.innerHTML, /Where this stands/);
  h.click({ "[data-rqs-close]": {} });
  assert.equal(h.overlay.hidden, true);
});

test("every field drawn from data is escaped: no img element and no javascript: link", async () => {
  const bad = "<img src=x onerror=alert(1)> [x](javascript:alert(1))";
  const g = glance("q-x", {
    title: bad,
    project: bad,
    last_who: "captain",
    last_text: bad,
    needs: bad,
    captains: [{ machine: "mac", path: "/s", project: bad, machine_name: bad }],
    tab: bad,
  });
  const reply = () =>
    userView("q-x", {
      glance: g,
      stands: stands({ asked: bad, done: bad, left: bad, needs: bad, links: [bad] }),
      items: [{ who: "captain", text: bad, at: 1, captain: bad }, { who: "orchestrator", text: bad, at: 2, captain: null }, { who: bad, text: bad, at: 3, captain: null }],
    });
  const h = harness({ tabs: [anchorTab([g, glance("q-e", { kind: "earlier", title: bad, project: bad })])], reply });
  open(h);
  h.click(h.entry("q-x"));
  await flush();
  const html = h.overlay.innerHTML;
  assert.match(html, /Where this stands/);
  assert.equal(tagNames(html).includes("img"), false);
  assert.deepEqual(hrefs(html).filter((href) => /javascript:/i.test(href)), []);
  assert.doesNotMatch(html, /<img/i);
});

test("the header is the Orchestrator title with Requests and All activity tabs, and Requests returns to the column", () => {
  const h = harness({ tabs: [anchorTab([glance("q1")])] });
  open(h);
  const html = h.overlay.innerHTML;
  assert.match(html, /<h1>Orchestrator<\/h1>/);
  assert.match(html, /<button[^>]*data-rqs-close[^>]*>Requests<\/button>/);
  assert.match(html, /<[a-z]+[^>]*aria-current="page"[^>]*>All activity<\//);
  assert.doesNotMatch(html, /<h1>All activity<\/h1>/);
  h.click({ "[data-rqs-close]": {} });
  assert.equal(h.overlay.hidden, true);
});

test("the footer is the Ask for something new composer, with no New request button", () => {
  const h = harness({ tabs: [anchorTab([glance("q1")])] });
  open(h);
  const html = h.overlay.innerHTML;
  assert.match(html, /<label[^>]*for="rqs-ask"[^>]*>Ask for something new<\/label>/);
  assert.match(html, /<input[^>]*id="rqs-ask"[^>]*data-rqs-ask[^>]*placeholder="Ask for something new"/);
  assert.match(html, /Each request opens its own thread/);
  assert.match(html, /<button[^>]*data-rqs-send[^>]*disabled/, "nothing typed: Send is off");
  assert.doesNotMatch(html, /data-rqs-new/);
});

test("the ask bubble holds the asked text and no time", () => {
  const h = harness({ tabs: [anchorTab([glance("q-ask", { title: "Codes", asked: "Why are login codes late?", opened_at: local(2026, 11, 18, 9, 5) })])] });
  open(h);
  const html = h.overlay.innerHTML;
  const ask = html.slice(html.indexOf('class="rqs-ask"'), html.indexOf("</div>", html.indexOf('class="rqs-ask"')));
  assert.match(ask, /Why are login codes late\?/);
  assert.doesNotMatch(ask, /<time|09:05/);
});

test("the footer and the panel head carry no New chip", async () => {
  const g = glance("q1", { title: "Codes" });
  const h = harness({ tabs: [anchorTab([g])], reply: (call) => (call.cmd === "user_request" ? userView("q1", { items: [] }) : null) });
  open(h);
  assert.match(h.overlay.innerHTML, /Each request opens its own thread/);
  assert.doesNotMatch(h.overlay.innerHTML, /rqs-new|>New</, "footer");
  h.click(h.entry("q1"));
  await flush();
  const html = h.overlay.innerHTML;
  const head = html.slice(html.indexOf('class="rqs-phead"'), html.indexOf("</div>", html.indexOf('class="rqs-phead"')));
  assert.match(head, /<h2>Codes<\/h2>/);
  assert.doesNotMatch(head, /rqs-new|>New</, "panel head");
  assert.doesNotMatch(html, /rqs-new|>New</, "anywhere");
});

test("the footer shows the preview only with two or more items or a close match", () => {
  const h = harness({ tabs: [anchorTab([glance("q-gift", { title: "Add gift cards to checkout", tab: "tab-12" })])] });
  open(h);
  typeAsk(h, "#storefront show the estimate on the cart page.");
  assert.doesNotMatch(h.overlay.innerHTML, /data-nr-card=/, "one plain item: no preview");
  assert.doesNotMatch(h.overlay.innerHTML, /<button[^>]*data-rqs-send[^>]*disabled/, "Send is on");
  typeAsk(h, TWO);
  let html = h.overlay.innerHTML;
  assert.equal(html.match(/data-nr-card=/g).length, 2);
  assert.match(html, /This opens 2 threads/);
  assert.match(html, /Goes to Captain billing-api on studio, free now/);
  assert.doesNotMatch(html, /data-nr-rename/, "the stream's preview has no Rename, as its board has none");
  typeAsk(h, "#storefront add gift cards to the checkout page");
  html = h.overlay.innerHTML;
  assert.match(html, /Looks close to an open thread: <b>Add gift cards to checkout<\/b>/);
  assert.equal(h.log.calls.length, 0, "the preview calls nothing");
});

test("the footer sends through the same path as New request: one new_user_requests with a key per item", async () => {
  const tabs = [anchorTab([glance("q0")]), requestTab("q1", "tab-5")];
  const h = harness({ tabs, reply: (call) => (call.cmd === "new_user_requests" ? made(call) : null) });
  open(h);
  typeAsk(h, TWO);
  h.click({ "[data-rqs-send]": {} });
  await flush();
  const sent = h.log.calls.filter(([, call]) => call.cmd === "new_user_requests");
  assert.equal(sent.length, 1);
  assert.deepEqual(Object.keys(sent[0][1]), ["cmd", "items"]);
  assert.deepEqual(
    sent[0][1].items.map(({ title, text, project, machine }) => ({ title, text, project, machine })),
    [
      { title: "show the delivery estimate on the cart page", text: TWO.split("\n\n")[0], project: "storefront", machine: "laptop" },
      { title: "the monthly export is missing refunds", text: TWO.split("\n\n")[1], project: "billing-api", machine: "studio" },
    ],
  );
  assert.equal(new Set(sent[0][1].items.map((i) => i.key)).size, 2);
  assert.deepEqual(h.log.revealed, ["mac:tab-5"], "the first new thread opens");
  assert.equal(h.run("rqs.ask.text"), "", "the draft is gone");
  assert.equal(h.run("nr.text"), "", "New request's own draft was never touched");
});

test("Enter in the footer sends too, and a timeout keeps the ask and sends the same keys again", async () => {
  let fail = true;
  const tabs = [anchorTab([glance("q0")]), requestTab("q1", "tab-5")];
  const h = harness({
    tabs,
    reply: (call) => {
      if (call.cmd !== "new_user_requests") return null;
      if (fail) throw new Error("timed out");
      return made(call);
    },
  });
  open(h);
  typeAsk(h, TWO);
  enter(h, "[data-rqs-ask]");
  await flush();
  assert.deepEqual(h.log.calls.filter(([, c]) => c.cmd === "prompt"), [], "nothing reaches the single chat");
  assert.equal(h.run("rqs.ask.text"), TWO, "the ask stays");
  assert.match(h.log.told.join(""), /timed out/);
  fail = false;
  h.click({ "[data-rqs-send]": {} });
  await flush();
  const [first, retry] = h.log.calls.filter(([, c]) => c.cmd === "new_user_requests").map(([, c]) => c.items.map((i) => i.key));
  assert.deepEqual(retry, first);
});

test("a close match chosen in the footer sends a Prompt in that request's tab", async () => {
  const gift = glance("q-gift", { title: "Add gift cards to checkout", tab: "tab-12" });
  const tabs = [anchorTab([gift]), requestTab("q-gift", "tab-12")];
  const h = harness({ tabs });
  open(h);
  typeAsk(h, "#storefront add gift cards to the checkout page");
  h.click({ "[data-nr-join]": { dataset: { nrJoin: "0" } } });
  h.click({ "[data-rqs-send]": {} });
  await flush();
  assert.deepEqual(h.log.calls, [["mac", { cmd: "prompt", tab: "tab-12", text: "#storefront add gift cards to the checkout page" }]]);
});

test("the side panel has Reply in this thread and Open full, and no Open thread", async () => {
  const h = harness({ tabs: [anchorTab([glance("q1", { tab: "tab-12" })])], reply: (call) => (call.cmd === "user_request" ? userView("q1") : null) });
  open(h);
  h.click(h.entry("q1"));
  await flush();
  const html = h.overlay.innerHTML;
  assert.match(html, /<label[^>]*for="rqs-reply"[^>]*>Reply in this thread<\/label>/);
  assert.match(html, /<input[^>]*id="rqs-reply"[^>]*data-rqs-reply="q1"[^>]*placeholder="Reply in this thread"/);
  assert.match(html, /<button[^>]*data-rqs-open="q1"[^>]*>Open full<\/button>/);
  assert.doesNotMatch(html, /Open thread/);
  assert.doesNotMatch(html, /data-rqs-reply-send/, "the board's box has no send button: Enter sends");
});

test("Enter in the panel's reply box sends a Prompt to the request's tab, then the panel reads the thread again", async () => {
  const tabs = [anchorTab([glance("q1", { tab: "tab-12" })]), requestTab("q1", "tab-12")];
  const h = harness({ tabs, reply: (call) => (call.cmd === "user_request" ? userView("q1") : null) });
  open(h);
  h.click(h.entry("q1"));
  await flush();
  typeReply(h, "q1", "Tell support: draft it.");
  enter(h, "[data-rqs-reply]", { dataset: { rqsReply: "q1" } });
  await flush();
  assert.deepEqual(h.log.calls.map(([, c]) => c.cmd), ["user_request", "prompt", "user_request"]);
  assert.deepEqual(h.log.calls[1], ["mac", { cmd: "prompt", tab: "tab-12", text: "Tell support: draft it." }]);
  assert.equal(h.run("rqs.reply.get('q1') ?? ''"), "", "the box is empty after it went");
  assert.equal(h.log.activated.length, 0, "the panel stays; no tab is opened");
});

test("a reply to a request with no tab asks the host to open one first", async () => {
  const h = harness({
    tabs: [anchorTab([glance("q1", { tab: null })])],
    reply: (call) => (call.cmd === "user_request" ? userView("q1") : call.cmd === "user_request_action" ? { tab: "tab-40" } : null),
  });
  open(h);
  h.click(h.entry("q1"));
  await flush();
  typeReply(h, "q1", "Go on.");
  enter(h, "[data-rqs-reply]", { dataset: { rqsReply: "q1" } });
  await flush();
  assert.deepEqual(h.log.calls.slice(1, 3).map(([, c]) => c), [{ cmd: "user_request_action", id: "q1", action: "open" }, { cmd: "prompt", tab: "tab-40", text: "Go on." }]);
});

test("a reply that fails keeps the text and says why", async () => {
  const h = harness({
    tabs: [anchorTab([glance("q1", { tab: "tab-12" })]), requestTab("q1", "tab-12")],
    reply: (call) => {
      if (call.cmd === "prompt") throw new Error("host is offline");
      return call.cmd === "user_request" ? userView("q1") : null;
    },
  });
  open(h);
  h.click(h.entry("q1"));
  await flush();
  typeReply(h, "q1", "Go on.");
  enter(h, "[data-rqs-reply]", { dataset: { rqsReply: "q1" } });
  await flush();
  assert.equal(h.run("rqs.reply.get('q1')"), "Go on.");
  assert.match(h.log.told.join(""), /host is offline/);
});

test("the panel's item text goes through markdown, and unsafe markup in it stays text", async () => {
  const h = harness({
    tabs: [anchorTab([glance("q1")])],
    reply: () => userView("q1", { items: [{ who: "captain", text: "**Cause** found, see [the PR](https://example.com/pr/1). <img src=x onerror=alert(1)> [x](javascript:alert(1))", at: 1, captain: "storefront" }] }),
  });
  open(h);
  h.click(h.entry("q1"));
  await flush();
  const html = h.overlay.innerHTML;
  assert.match(html, /<strong>Cause<\/strong> found/);
  assert.match(html, /<a href="https:\/\/example\.com\/pr\/1">the PR<\/a>/);
  assert.equal(tagNames(html).includes("img"), false);
  assert.deepEqual(hrefs(html).filter((href) => /javascript:/i.test(href)), []);
});

test("the ask bubble shows glance.asked, and the title when asked is empty or missing", () => {
  const tabs = [anchorTab([
    glance("q-a", { title: "Why are codes late?", asked: "Customers say the login code takes forever to arrive. Find out why." }),
    glance("q-b", { title: "Pause the workers", asked: "" }),
    glance("q-c", { title: "Compare v2.4 with the roadmap" }),
    glance("q-d", { title: "Hostile", asked: "<img src=x onerror=alert(1)>" }),
  ])];
  const h = harness({ tabs });
  open(h);
  const html = h.overlay.innerHTML;
  const ask = (id) => html.slice(html.indexOf(`data-rqs-entry="${id}"`) - 330, html.indexOf(`data-rqs-entry="${id}"`));
  assert.match(ask("q-a"), /Customers say the login code takes forever to arrive\. Find out why\./);
  assert.doesNotMatch(ask("q-a"), /Why are codes late/);
  assert.match(ask("q-b"), /Pause the workers/);
  assert.match(ask("q-c"), /Compare v2.4 with the roadmap/);
  assert.doesNotMatch(html, /<img/i);
});

test("text typed into the footer is drawn as text: no img element and no javascript: link", () => {
  const bad = "<img src=x onerror=alert(1)> [x](javascript:alert(1))";
  const h = harness({ tabs: [anchorTab([glance("q1", { title: bad })])] });
  open(h);
  typeAsk(h, `#storefront ${bad}\n\n#billing-api ${bad}`);
  const html = h.overlay.innerHTML;
  assert.match(html, /This opens 2 threads/);
  assert.equal(tagNames(html).includes("img"), false);
  assert.deepEqual(hrefs(html).filter((href) => /javascript:/i.test(href)), []);
});

test("9r5 item 6: the list runs oldest first, newest at the bottom, with the day heads in that order", () => {
  const tabs = [anchorTab([
    glance("q-new", { title: "Newest", asked: "Newest", opened_at: local(2026, 11, 18, 9) }),
    glance("q-old", { title: "Oldest", asked: "Oldest", opened_at: local(2026, 11, 17, 9) }),
    glance("q-mid", { title: "Middle", asked: "Middle", opened_at: local(2026, 11, 18, 8) }),
  ])];
  const h = harness({ tabs });
  open(h);
  const html = h.overlay.innerHTML;
  const at = (s) => html.indexOf(s);
  assert.ok(at('data-rqs-entry="q-old"') < at('data-rqs-entry="q-mid"') && at('data-rqs-entry="q-mid"') < at('data-rqs-entry="q-new"'), "q-old, q-mid, q-new");
  const heads = [...html.matchAll(/rqs-dayhead/g)].map((m) => m.index);
  assert.equal(heads.length, 2);
  assert.ok(heads[0] < at('data-rqs-entry="q-old"') && at('data-rqs-entry="q-old"') < heads[1] && heads[1] < at('data-rqs-entry="q-mid"'), "the older day head comes first");
});

test("9r5 item 6: the panel's card has no You asked row in All activity, and keeps it elsewhere", async () => {
  const reply = (call) => (call.cmd === "user_request" ? userView("q1", { stands: stands({ asked: "Find out why codes are late." }) }) : null);
  const h = harness({ tabs: [anchorTab([glance("q1", { title: "Codes" })])], reply });
  open(h);
  h.click(h.entry("q1"));
  await flush();
  const html = h.overlay.innerHTML;
  assert.match(html, /<span class="rc-k">Done<\/span>/);
  assert.match(html, /<span class="rc-k">Left<\/span>/);
  assert.match(html, /<span class="rc-k">Needs you<\/span>/);
  assert.doesNotMatch(html, /You asked/);
  assert.match(h.run("requestCardFrom(rqs.views.get('q1').view.glance ?? { id: 'q1', captains: [] }, { view: { stands: { asked: 'x', done: '', left: '', needs: '', links: [] } } })"), /You asked/);
});

test("9r5 item 6: All activity hides the Requests column and gives the stream its room", () => {
  const tabs = [anchorTab([glance("q1")])];
  const h = harness({ tabs });
  h.context.active = tabs[0];
  h.run("renderRequests()");
  assert.equal(h.run("requestsColumn.hidden"), false, "the column is beside the thread");
  open(h);
  assert.equal(h.run("requestsColumn.hidden"), true, "All activity open: no column");
  assert.equal("data-requests" in h.els.chat.attrs, false);
  h.run("closeAllActivity()");
  assert.equal(h.run("requestsColumn.hidden"), false, "closed: the column is back");
  const css = read("gui-requests-stream.css").replace(/\/\*[\s\S]*?\*\//g, "");
  assert.match(css, /#all-activity \{[^}]*left:\s*0/);
  assert.match(css, /#chat\[data-requests\] #all-activity \{[^}]*left:\s*340px/);
});
