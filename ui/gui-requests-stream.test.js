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

/** gui-requests.js and gui-requests-stream.js in a page: the app's globals stubbed, the real esc and the real model. */
function harness({ tabs = [], reply = () => [], host = "mac" } = {}) {
  const log = { calls: [], told: [], activated: [], revealed: [], newRequest: 0 };
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
    openNewRequest: () => log.newRequest++,
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
  for (const file of ["composer-tags.js", "requests-model.js", "gui-requests.js", "gui-requests-stream.js"]) run(read(file));
  const overlay = run("requestsStream");
  const click = (selectors) => overlay.listeners.click({ target: { closest: (sel) => selectors[sel] ?? null } });
  const entry = (id) => ({ "[data-rqs-entry]": { dataset: { rqsEntry: id } } });
  /** A scroll of the list to its end (or not). */
  const scroll = (atEnd = true) => overlay.listeners.scroll({ target: { closest: (sel) => (sel === "[data-rqs-list]" ? {} : null), scrollTop: atEnd ? 900 : 0, clientHeight: 100, scrollHeight: 1000 } });
  return { context, run, log, els, overlay, click, entry, scroll };
}

const anchorTab = (requests) => ({ key: "mac:tab-1", machine: "mac", view: { id: "tab-1", orchestrator_global: true, requests, plans: [] } });
const requestTab = (id, tabId) => ({ key: `mac:${tabId}`, machine: "mac", view: { id: tabId, orchestrator_global: true, request: id, plans: [] } });

const tagNames = (html) => [...html.matchAll(/<([a-z][a-z0-9-]*)/gi)].map((m) => m[1].toLowerCase());
const hrefs = (html) => [...html.matchAll(/\bhref\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi)].map((m) => m[1]);
const local = (y, m, d, hh = 0, mm = 0) => new Date(y, m - 1, d, hh, mm).getTime();
const open = (h) => h.run("openAllActivity()");

test("requests group by their local day, newest first, and midnight splits a day", () => {
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
    ["Today", ["noon", "early"]],
    ["Yesterday", ["late"]],
    ["Nov 3", ["old"]],
  ]);
});

test("an entry shows the ask at its opening time, the state, project, reply count and last line", () => {
  const tabs = [anchorTab([
    glance("q-two", { title: "Why are login codes late?", opened_at: local(2026, 11, 18, 9, 5), replies: 2, last_who: "captain", last_text: "Tests are running", last_at: 5, captains: [{ machine: "mac", path: "/s", project: "storefront", machine_name: "Mac" }] }),
    glance("q-one", { opened_at: local(2026, 11, 18, 8, 0), replies: 1, last_who: "orchestrator", last_text: "On it", closed_at: 9 }),
    glance("q-none", { opened_at: local(2026, 11, 18, 7, 0), needs: "Pick one." }),
  ])];
  const h = harness({ tabs });
  open(h);
  const html = h.overlay.innerHTML;
  const two = html.slice(html.indexOf('data-rqs-entry="q-two"') - 400, html.indexOf('data-rqs-entry="q-one"'));
  assert.match(two, /Why are login codes late\?/);
  assert.match(two, /09:05/);
  assert.match(two, /Working/);
  assert.match(two, /#storefront/);
  assert.match(two, /2 replies/);
  assert.match(two, /Captain storefront: Tests are running/);
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

test("scrolling to the end asks for older requests before the oldest opened_at, and stops on an empty page", async () => {
  const tabs = [anchorTab([glance("a", { opened_at: 3000 }), glance("b", { opened_at: 2000 }), glance("c", { opened_at: 1000 })])];
  const pages = [[glance("d", { opened_at: 500 }), glance("e", { opened_at: 400 })], []];
  const h = harness({ tabs, reply: (call) => (call.cmd === "user_requests" ? pages.shift() : null) });
  open(h);
  h.scroll(false);
  await flush();
  assert.deepEqual(h.log.calls, [], "not at the end yet");
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
  assert.deepEqual(h.log.calls.at(-1), ["mac", { cmd: "user_request_action", id: "q-none", action: "open" }]);
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
