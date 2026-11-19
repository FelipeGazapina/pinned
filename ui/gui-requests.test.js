import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const line = (src, from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
const flush = () => new Promise((resolve) => setImmediate(resolve));
/** Plain copies of what the page's own realm builds, for deepStrictEqual. */
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
const planGlance = (id, over = {}) => ({ id, title: id, state: "working", done: 1, total: 2, strips: [], requests: [], ...over });
const stands = (over = {}) => ({ asked: "Find out why codes are late.", done: "Cause found.", left: "Tell support.", needs: "", links: ["Captain storefront", "PR #212 merged"], at: 5000, ...over });
const view = (id, over = {}) => ({ glance: glance(id), stands: stands(), items: [], plans: [], ...over });

/** gui-requests.js in a page: the app's globals stubbed, the real esc and the real model. */
function harness({ tabs = [], reply = () => [], host = "mac", online = true } = {}) {
  const log = { calls: [], told: [], activated: [], revealed: [], threads: 0, newRequest: 0, allActivity: 0 };
  const els = { chat: element(), crumb: element(), footer: element(), head: element() };
  const models = [];
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
    machines: new Map([[host, { view: { online } }]]),
    globalHostMachine: () => host,
    isGlobalOrchestratorTab: (tab) => tab?.view?.orchestrator_global === true,
    guiOn: () => true,
    narrow: () => false,
    shellPage: () => null,
    ago: () => "12m",
    icon: (name) => `<svg data-icon="${name}"></svg>`,
    globalOrchestratorName: () => "Orchestrator",
    waitingKindOf: () => null,
    onModelChange: (fn) => models.push(fn),
    visibleInterval: () => {},
    renderThread: () => log.threads++,
    tell: (html) => log.told.push(html),
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
      createElement: () => element(),
    },
  };
  vm.createContext(context);
  const run = (code) => vm.runInContext(code, context);
  run(line(settingsSrc, "const esc = "));
  for (const file of ["composer-tags.js", "requests-model.js", "gui-requests.js"]) run(read(file));
  const column = run("requestsColumn");
  const click = (el, selectors) => el.listeners.click({ target: { closest: (sel) => selectors[sel] ?? null } });
  return { context, run, log, els, column, models, click };
}

/** A global tab as the host lists it: the anchor carries `requests`, a request tab carries `request`. */
const anchorTab = (requests, over = {}) => ({ key: "mac:tab-1", machine: "mac", view: { id: "tab-1", orchestrator_global: true, requests, plans: [], ...over } });
const requestTab = (id, tabId, over = {}) => ({ key: `mac:${tabId}`, machine: "mac", view: { id: tabId, orchestrator_global: true, request: id, plans: [], ...over } });

const tagNames = (html) => [...html.matchAll(/<([a-z][a-z0-9-]*)/gi)].map((m) => m[1].toLowerCase());
const hrefs = (html) => [...html.matchAll(/\bhref\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi)].map((m) => m[1]);
const rowIds = (html) => [...html.matchAll(/data-request="([^"]*)"/g)].map((m) => m[1]);

/** requestsColumnHtml with the page's defaults filled in. */
const draw = (h, over = {}) => h.run(`requestsColumnHtml(${JSON.stringify({ glances: [], filter: "open", seen: {}, current: null, name: "Orchestrator", tabViews: {}, ...over })})`);

const board = () => [
  glance("q-done", { title: "Pause the workers", closed_at: 900, last_who: "orchestrator", last_text: "Paused, nothing was lost", last_at: 900 }),
  glance("q-work", { title: "Add gift cards", replies: 2, last_who: "captain", last_text: "Tests are running in a box", last_at: 700, captains: [{ machine: "mac", path: "/repo/store", project: "storefront", machine_name: "MacBook" }] }),
  glance("q-needs", { title: "Why are login codes late?", replies: 3, needs: "Choose how support is told.", needs_at: 800, last_who: "orchestrator", last_text: "Two options, pick one", last_at: 800 }),
];
const planTabs = { mac: [{ id: "tab-30", cwd: "/repo/store", plans: [planGlance("p1", { done: 7, total: 8, requests: ["q-work"] })] }] };

test("the column draws the filters with their counts and one row per request", () => {
  const h = harness();
  const html = draw(h, { glances: board(), tabViews: planTabs });
  assert.match(html, />\s*Requests\s*</, "the heading");
  assert.deepEqual([...html.matchAll(/data-rq-filter="([^"]*)"[^>]*>([^<]*?)\s*<span[^>]*>(\d+)</g)].map((m) => [m[1], m[2], m[3]]), [["open", "Open", "2"], ["needs", "Needs you", "1"], ["done", "Done", "1"]]);
  assert.match(html, /aria-pressed="true"[^>]*>Open/);
  const work = html.slice(html.indexOf('data-request="q-work"'), html.indexOf('data-request="q-done"'));
  assert.match(work, /Add gift cards/);
  assert.match(work, /#storefront/);
  assert.match(work, /Working/);
  assert.match(work, /Captain storefront: Tests are running in a box/);
  assert.match(work, /7 of 8 steps/);
  assert.match(work, /12m/);
  assert.match(work, />2<\/span>/, "the unread count");
  assert.match(html, /New request/);
  assert.match(html, /All activity/);
});

test("Open shows needs you then working, Needs you and Done one each", () => {
  const h = harness();
  const ids = (filter) => rowIds(draw(h, { glances: board(), filter }));
  assert.deepEqual(ids("open"), ["q-needs", "q-work"]);
  assert.deepEqual(ids("needs"), ["q-needs"]);
  assert.deepEqual(ids("done"), ["q-done"]);
  assert.match(draw(h, { glances: board(), filter: "needs" }), /aria-pressed="true"[^>]*>Needs you/);
});

test("a plan that needs you, or a request tab that waits, puts the request first", () => {
  const h = harness();
  const glances = [glance("q-a", { opened_at: 1, last_at: 500 }), glance("q-b", { opened_at: 2, last_at: 600 }), glance("q-c", { opened_at: 3, last_at: 100 })];
  const tabViews = {
    mac: [
      { id: "tab-30", cwd: "/x", plans: [planGlance("p1", { state: "needs_you", requests: ["q-c"] })] },
      { id: "tab-31", orchestrator_global: true, request: "q-a", status: "needs-input", running: true, interrupted: true, plans: [] },
    ],
  };
  assert.deepEqual(rowIds(draw(h, { glances, tabViews })), ["q-a", "q-c", "q-b"]);
});

test("the unread count is the replies not yet seen on this device, and a seen request draws none", () => {
  const h = harness();
  const glances = [glance("q1", { replies: 3 })];
  assert.match(draw(h, { glances }), />3<\/span>/);
  assert.match(draw(h, { glances, seen: { q1: 1 } }), />2<\/span>/);
  assert.doesNotMatch(draw(h, { glances, seen: { q1: 3 } }), /unread/);
});

test("the open request's row is the current one", () => {
  const h = harness();
  const html = draw(h, { glances: board(), current: "q-work" });
  assert.match(html, /data-request="q-work"[^>]*aria-current="true"/);
  assert.match(html, /data-request="q-needs"[^>]*aria-current="false"/);
});

test("an empty list says so", () => {
  const h = harness();
  const html = draw(h);
  assert.match(html, /Nothing here\. A request you send shows up as a thread\./);
});

test("the glances come from the host's tab that carries them, and nothing is asked", async () => {
  const tabs = [requestTab("q1", "tab-12"), anchorTab(board())];
  const h = harness({ tabs });
  await h.run("refreshRequests()");
  assert.deepEqual(h.log.calls, []);
  assert.equal(h.run("requestsOn()"), true);
  assert.deepEqual(plain(h.run("requestGlances()")).map((g) => g.id), ["q-done", "q-work", "q-needs"]);
});

test("a tab of another machine that carries requests is not the host's", async () => {
  const stray = { key: "mini:tab-9", machine: "mini", view: { id: "tab-9", orchestrator_global: true, requests: board() } };
  const h = harness({ tabs: [stray], reply: () => [] });
  await h.run("refreshRequests()");
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_requests" }]]);
});

test("with no tab carrying requests the screen asks user_requests, and draws its answer", async () => {
  const h = harness({ tabs: [requestTab("q1", "tab-12")], reply: (call) => (call.cmd === "user_requests" ? board() : null) });
  assert.equal(h.run("requestsOn()"), false, "before the answer the single chat stays");
  await h.run("refreshRequests()");
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_requests" }]]);
  assert.equal(h.run("requestsOn()"), true);
  h.context.active = h.context.tabs[0];
  h.run("renderRequests()");
  assert.equal(h.column.hidden, false);
  assert.deepEqual(rowIds(h.column.innerHTML), ["q-needs", "q-work"]);
});

test("when user_requests errors the single chat shows, with no banner and no message", async () => {
  const anchor = anchorTab(undefined);
  const h = harness({ tabs: [anchor] });
  h.context.reply = () => {
    throw new Error("unknown variant `user_requests`");
  };
  h.context.active = anchor;
  await h.run("refreshRequests()");
  h.run("renderRequests()");
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_requests" }]]);
  assert.equal(h.run("requestsOn()"), false);
  assert.equal(h.column.hidden, true, "the column stays away");
  assert.equal(h.column.innerHTML, "", "and holds nothing, not even an error");
  assert.equal("data-requests" in h.els.chat.attrs, false, "the chat keeps its width");
  assert.deepEqual(h.log.told, [], "nothing is told");
  assert.equal(h.run("requestsBadge()"), null, "the Orchestrator row keeps counting reports");
  assert.equal(h.run("requestCardHtml(active)"), "");
  assert.equal(h.run("requestFooterHtml(active)"), "");
  await h.run("refreshRequests()");
  assert.equal(h.log.calls.length, 1, "an old host is not asked again at once");
});

test("opening the Orchestrator on an old host lands on the first global tab, as before", async () => {
  const first = anchorTab(undefined);
  const second = requestTab("q1", "tab-12");
  const h = harness({ tabs: [first, second] });
  h.context.reply = () => {
    throw new Error("unknown variant");
  };
  await h.run("refreshRequests()");
  assert.equal(h.run("requestLandingTab('mac')").key, "mac:tab-1");
});

test("opening the Orchestrator lands on the request that needs you, else the anchor", () => {
  const anchor = anchorTab(board());
  const asking = requestTab("q-needs", "tab-20");
  const working = requestTab("q-work", "tab-21");
  const h = harness({ tabs: [anchor, working, asking] });
  const g = board();
  g[2].tab = "tab-20";
  g[1].tab = "tab-21";
  anchor.view.requests = g;
  assert.equal(h.run("requestLandingTab('mac')").key, "mac:tab-20");
  g[2].needs = "";
  assert.equal(h.run("requestLandingTab('mac')").key, "mac:tab-1", "with nothing waiting the anchor is where it lands");
  g[2].needs = "Choose how support is told.";
  h.context.active = working;
  h.run("renderRequests()");
  assert.equal(h.run("requestLandingTab('mac')").key, "mac:tab-21", "the request last open is where it comes back to, even with another waiting");
});

test("the Orchestrator's badge counts the requests in needs", () => {
  const h = harness({ tabs: [anchorTab(board())] });
  assert.equal(h.run("requestsBadge()"), 1);
  const more = board();
  more[1].needs = "Approve the plan";
  more[1].needs_at = 10;
  h.context.tabs[0].view.requests = more;
  assert.equal(h.run("requestsBadge()"), 2);
  h.context.tabs[0].view.requests = more.map((g) => ({ ...g, closed_at: 5 }));
  assert.equal(h.run("requestsBadge()"), 0);
});

test("a row opens its request's own tab", async () => {
  const g = board();
  g[2].tab = "tab-20";
  const asking = requestTab("q-needs", "tab-20");
  const h = harness({ tabs: [anchorTab(g), asking] });
  h.click(h.column, { "[data-request]": { dataset: { request: "q-needs" } } });
  await flush();
  assert.deepEqual(h.log.activated, ["mac:tab-20"]);
  assert.deepEqual(h.log.calls, []);
});

test("a row whose request has no tab asks the host to open one", async () => {
  const g = board();
  const resting = requestTab("q-done", "tab-40");
  const h = harness({
    tabs: [anchorTab(g), resting],
    reply: (call) => (call.action === "open" ? { ...g[0], tab: "tab-40" } : null),
  });
  h.click(h.column, { "[data-request]": { dataset: { request: "q-done" } } });
  await flush();
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_request_action", id: "q-done", action: "open" }]]);
  assert.deepEqual(h.log.revealed, ["mac:tab-40"]);
});

test("the column shows only beside a global tab, on a wide screen, in the app view", () => {
  const anchor = anchorTab(board());
  const plainTab = { key: "mac:tab-3", machine: "mac", view: { id: "tab-3", plans: [] } };
  const h = harness({ tabs: [anchor, plainTab] });
  h.context.active = plainTab;
  h.run("renderRequests()");
  assert.equal(h.column.hidden, true, "an ordinary thread has no column");
  h.context.active = anchor;
  h.context.narrow = () => true;
  h.run("renderRequests()");
  assert.equal(h.column.hidden, true, "a phone is unit 12's");
  h.context.narrow = () => false;
  h.context.guiOn = () => false;
  h.run("renderRequests()");
  assert.equal(h.column.hidden, true, "the terminal view has none");
  h.context.guiOn = () => true;
  h.run("renderRequests()");
  assert.equal(h.column.hidden, false);
});

test("the column opens on the Open filter, as the board does", () => {
  const anchor = anchorTab(board());
  const h = harness({ tabs: [anchor] });
  h.context.active = anchor;
  h.run("renderRequests()");
  assert.deepEqual(rowIds(h.column.innerHTML), ["q-needs", "q-work"]);
  assert.match(h.column.innerHTML, /aria-pressed="true"[^>]*>Open/);
});

test("a filter button redraws the column with that filter", () => {
  const anchor = anchorTab(board());
  const h = harness({ tabs: [anchor] });
  h.context.active = anchor;
  h.run("renderRequests()");
  assert.equal(h.column.hidden, false);
  assert.equal(h.els.chat.attrs["data-requests"], "");
  h.click(h.column, { "[data-rq-filter]": { dataset: { rqFilter: "done" } } });
  assert.deepEqual(rowIds(h.column.innerHTML), ["q-done"]);
});

test("New request and All activity call their hooks", () => {
  const anchor = anchorTab(board());
  const h = harness({ tabs: [anchor] });
  h.context.openNewRequest = () => h.log.newRequest++;
  h.context.openAllActivity = () => h.log.allActivity++;
  h.context.active = anchor;
  h.run("renderRequests()");
  h.click(h.column, { "[data-rq-new]": { dataset: {} } });
  h.click(h.column, { "[data-rq-all]": { dataset: {} } });
  assert.deepEqual([h.log.newRequest, h.log.allActivity], [1, 1]);
});

test("the hooks unit 10 and unit 11 fill in exist as stubs, and a later definition wins", () => {
  const h = harness({ tabs: [] });
  assert.equal(h.run("typeof openNewRequest"), "function");
  assert.equal(h.run("typeof openAllActivity"), "function");
});

test("the header names the request, its project and state, and offers Mark done", () => {
  const g = board();
  g[2].tab = "tab-20";
  const asking = requestTab("q-needs", "tab-20");
  const h = harness({ tabs: [anchorTab(g), asking] });
  const html = h.run("requestCrumbHtml(tabs[1])");
  assert.match(html, /Why are login codes late\?/);
  assert.match(html, /#storefront/);
  assert.match(html, /Needs you/);
  assert.match(html, /data-request-action="done"[^>]*>Mark done</);
});

test("while New request is open the header is its crumb, in any global tab", () => {
  const h = harness({ tabs: [anchorTab(board())] });
  assert.equal(h.run("requestCrumbHtml(tabs[0])"), "");
  h.run('globalThis.newRequestCrumbHtml = () => \'<h2 class="nr-title-crumb">New request</h2>\'');
  assert.equal(h.run("requestCrumbHtml(tabs[0])"), '<h2 class="nr-title-crumb">New request</h2>');
});

test("Mark done sends user_request_action done for the request", async () => {
  const g = board();
  g[1].tab = "tab-21";
  const working = requestTab("q-work", "tab-21");
  const h = harness({ tabs: [anchorTab(g), working], reply: () => ({ ...g[1], closed_at: 5000 }) });
  h.click(h.els.crumb, { "[data-request-action]": { dataset: { requestAction: "done", requestId: "q-work" } } });
  await flush();
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_request_action", id: "q-work", action: "done" }]]);
  assert.equal(h.run("requestGlances().find((g) => g.id === 'q-work').closed_at"), 5000, "the reply shows before the next tabs push");
});

test("a done request offers Reopen, which sends user_request_action reopen", async () => {
  const g = board();
  g[0].tab = "tab-40";
  const done = requestTab("q-done", "tab-40");
  const h = harness({ tabs: [anchorTab(g), done], reply: () => ({ ...g[0], closed_at: null }) });
  const html = h.run("requestCrumbHtml(tabs[1])");
  assert.match(html, /data-request-action="reopen"[^>]*>Reopen</);
  assert.match(html, /Done/);
  h.click(h.els.crumb, { "[data-request-action]": { dataset: { requestAction: "reopen", requestId: "q-done" } } });
  await flush();
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_request_action", id: "q-done", action: "reopen" }]]);
});

test("a failed action tells the user and keeps the request as it was", async () => {
  const g = board();
  g[1].tab = "tab-21";
  const h = harness({ tabs: [anchorTab(g), requestTab("q-work", "tab-21")] });
  h.context.reply = () => {
    throw new Error("the host is busy");
  };
  h.click(h.els.crumb, { "[data-request-action]": { dataset: { requestAction: "done", requestId: "q-work" } } });
  await flush();
  assert.equal(h.log.told.length, 1);
  assert.match(h.log.told[0], /the host is busy/);
  assert.equal(h.run("requestGlances().find((g) => g.id === 'q-work').closed_at"), null);
});

test("the card reads You asked, Done, Left, Needs you and the links from user_request", async () => {
  const g = board();
  g[2].tab = "tab-20";
  const asking = requestTab("q-needs", "tab-20");
  const h = harness({
    tabs: [anchorTab(g), asking],
    reply: (call) => (call.cmd === "user_request" ? view("q-needs", { glance: g[2], stands: stands({ needs: "Choose how support is told." }) }) : null),
  });
  h.context.active = asking;
  assert.match(h.run("requestCardHtml(active)"), /Where this stands/, "the card holds its place while it loads");
  await h.run("ensureRequestView(active)");
  assert.deepEqual(h.log.calls, [["mac", { cmd: "user_request", id: "q-needs" }]]);
  const html = h.run("requestCardHtml(active)");
  for (const [label, text] of [["You asked", "Find out why codes are late."], ["Done", "Cause found."], ["Left", "Tell support."], ["Needs you", "Choose how support is told."]]) {
    assert.match(html, new RegExp(`${label}[\\s\\S]*${text.replace(/[.?]/g, "\\$&")}`));
  }
  assert.match(html, /Captain storefront/);
  assert.match(html, /PR #212 merged/);
  assert.match(html, /data-needs="true"/);
  await h.run("ensureRequestView(active)");
  assert.equal(h.log.calls.length, 1, "an unchanged request is not read again");
});

test("Needs you always shows, and says nothing is needed when nothing is", async () => {
  const g = board();
  g[1].tab = "tab-21";
  const working = requestTab("q-work", "tab-21");
  const h = harness({ tabs: [anchorTab(g), working], reply: () => view("q-work", { glance: g[1], stands: stands({ needs: "" }) }) });
  h.context.active = working;
  await h.run("ensureRequestView(active)");
  const html = h.run("requestCardHtml(active)");
  assert.match(html, /Needs you/);
  assert.match(html, /Nothing right now\./);
  assert.match(html, /data-needs="false"/);
});

test("a stale need reads as nothing needed", async () => {
  const g = board();
  g[2].tab = "tab-20";
  g[2].last_user_at = 900;
  const asking = requestTab("q-needs", "tab-20");
  const h = harness({ tabs: [anchorTab(g), asking], reply: () => view("q-needs", { glance: g[2], stands: stands({ needs: "Choose how support is told." }) }) });
  h.context.active = asking;
  await h.run("ensureRequestView(active)");
  assert.match(h.run("requestCardHtml(active)"), /Nothing right now\./);
});

test("the card is drawn only in a request tab", () => {
  const h = harness({ tabs: [anchorTab(board()), requestTab("q-work", "tab-21")] });
  assert.equal(h.run("requestCardHtml(tabs[0])"), "", "the anchor keeps today's look");
  assert.equal(h.run("requestFooterHtml(tabs[0])"), "");
  assert.equal(h.run("requestCrumbHtml(tabs[0])"), "");
});

test("the composer footer says the reply stays in this thread and offers New request", () => {
  const g = board();
  g[1].tab = "tab-21";
  const h = harness({ tabs: [anchorTab(g), requestTab("q-work", "tab-21")] });
  const html = h.run("requestFooterHtml(tabs[1])");
  assert.match(html, /#storefront/);
  assert.match(html, /stays in this thread/);
  assert.match(html, /data-new-request[^>]*>Another subject\? New request</);
});

test("the footer's New request calls the hook", () => {
  const h = harness({ tabs: [] });
  h.context.openNewRequest = () => h.log.newRequest++;
  h.click(h.els.footer, { "[data-new-request]": { dataset: {} } });
  assert.equal(h.log.newRequest, 1);
});

test("markup in any field stays text: no img element and no javascript: href in any request screen", async () => {
  const bad = "<img src=x onerror=alert(1)>";
  const link = "[x](javascript:alert(1))";
  const hostile = glance(`${bad}`, {
    title: `${bad} ${link}`,
    project: `${bad}${link}`,
    last_who: "captain",
    last_text: `${bad} ${link}`,
    needs: `${bad} ${link}`,
    needs_at: 5,
    replies: 4,
    tab: `${bad}`,
    captains: [{ machine: bad, path: bad, project: bad, machine_name: bad }],
  });
  const anchor = anchorTab([hostile]);
  const own = requestTab(hostile.id, hostile.tab, { plans: [] });
  const h = harness({
    tabs: [anchor, own],
    reply: () => view(hostile.id, { glance: hostile, stands: stands({ asked: `${bad} ${link}`, done: `${bad} ${link}`, left: `${bad} ${link}`, needs: `${bad} ${link}`, links: [`${bad} ${link}`, link, "javascript:alert(1)"] }) }),
  });
  h.context.active = own;
  await h.run("ensureRequestView(active)");
  const pieces = {
    column: draw(h, { glances: [hostile], current: hostile.id, name: bad }),
    crumb: h.run("requestCrumbHtml(tabs[1])"),
    card: h.run("requestCardHtml(tabs[1])"),
    footer: h.run("requestFooterHtml(tabs[1])"),
  };
  for (const [name, html] of Object.entries(pieces)) {
    assert.ok(html.length > 0, `${name} draws`);
    assert.deepEqual(tagNames(html).filter((tag) => tag === "img"), [], `${name}: no img element`);
    assert.deepEqual(hrefs(html).filter((href) => /javascript:/i.test(href)), [], `${name}: no javascript: href`);
    assert.doesNotMatch(html, /<img/i, `${name}: no raw <img`);
  }
  assert.ok(pieces.card.includes("&#60;img src=x onerror=alert(1)&#62;"), "the card shows the markup as text");
  assert.ok(pieces.column.includes("&#60;img src=x onerror=alert(1)&#62;"), "the row shows the markup as text");
});

// ---- 0.18.8 unit 9r3 ----
const cssOf = (name) => read(name).replace(/\/\*[\s\S]*?\*\//g, "");
const ruleOf = (css, selector) => {
  const at = css.indexOf(`${selector} {`);
  assert.ok(at >= 0, `${selector} is a rule`);
  return css.slice(at, css.indexOf("}", at));
};

test("9r3 item 10: the request header shows the crumb, the dot, the title, the state, the project and Mark done", () => {
  const g = board();
  g[1].tab = "tab-21";
  const h = harness({ tabs: [anchorTab(g), requestTab("q-work", "tab-21")] });
  const html = h.run("requestCrumbHtml(tabs[1])");
  assert.match(html, /^<span class="request-dot"/);
  assert.match(html, /<h2 class="request-title">Add gift cards<\/h2>/);
  assert.match(html, /<button type="button" data-request-action="done" data-request-id="q-work">Mark done<\/button>/);
});

test("9r3 item 10: the title truncates and nothing else in the header shrinks or paints over the buttons", () => {
  const css = cssOf("gui-requests.css");
  assert.match(ruleOf(css, "#chat-head .crumb .request-title"), /min-width:\s*0[^}]*text-overflow:\s*ellipsis/);
  assert.match(ruleOf(css, "#chat-head .crumb:has(.request-dot)"), /overflow:\s*hidden/, "the crumb clips instead of painting over the controls");
  for (const sel of ["#chat-head .crumb .request-back", "#chat-head .crumb .request-state", "#chat-head .crumb .rq-project"]) assert.match(ruleOf(css, sel), /flex:\s*none/, `${sel} keeps its size`);
  assert.match(cssOf("gui-orchestrator.css"), /#chat-head \.crumb \.orchestrator-actions \{[^}]*flex:\s*none/, "the button group does not shrink");
  assert.match(read("gui.css"), /#chat-head \.controls \{ flex: none;/, "nor do the view toggles");
});

test("9r3 item 10: the Requests crumb closes the Plan panel and brings the column back", () => {
  const g = board();
  g[1].tab = "tab-21";
  const tab = requestTab("q-work", "tab-21");
  const h = harness({ tabs: [anchorTab(g), tab] });
  const log = [];
  h.context.panelOf = () => ({ open: true, active: "plan", surfaces: [{ id: "plan", kind: "plan" }] });
  h.context.closePanel = () => log.push("closePanel");
  h.context.shownSurface = () => ({ id: "plan", kind: "plan" });
  h.context.active = tab;
  h.click(h.els.crumb, { "[data-rq-back]": { dataset: {} } });
  assert.deepEqual(log, ["closePanel"]);
});

test("9r3 item 11: the column shows beside a request thread while its Plan is closed, and hides while it is open", () => {
  const g = board();
  g[1].tab = "tab-21";
  const tab = requestTab("q-work", "tab-21");
  const h = harness({ tabs: [anchorTab(g), tab] });
  const panel = { open: false, active: "plan", surfaces: [{ id: "plan", kind: "plan" }] };
  h.context.panelOf = () => panel;
  h.context.shownSurface = (t) => t.panel?.surfaces?.find((s) => s.id === panel.active) ?? panel.surfaces.find((s) => s.id === panel.active);
  h.context.active = tab;
  h.run("renderRequests()");
  assert.equal(h.column.hidden, false, "Plan closed: the column is there");
  assert.equal(h.els.chat.attrs["data-requests"], "");
  panel.open = true;
  h.run("renderRequests()");
  assert.equal(h.column.hidden, true, "Plan open: the column is gone");
  assert.equal("data-requests" in h.els.chat.attrs, false);
  panel.surfaces = [{ id: "diff", kind: "diff" }];
  panel.active = "diff";
  h.run("renderRequests()");
  assert.equal(h.column.hidden, false, "another surface open: the column stays");
});

test("9r3 item 12: the stands card has no 'kept up to date' line and no New mark", () => {
  const html = h0().run("requestCardFrom(requestGlance, { view: { stands: { asked: 'a', done: 'b', left: 'c', needs: '', links: [] } } })");
  assert.doesNotMatch(html, /kept up to date|>New<|class="[^"]*\bnew\b/i);
  assert.match(html, /<span class="rc-title">Where this stands<\/span>/);
});
function h0() {
  const g = board();
  g[1].tab = "tab-21";
  const h = harness({ tabs: [anchorTab(g), requestTab("q-work", "tab-21")] });
  h.run("var requestGlance = requestOf(tabs[1])");
  return h;
}

test("9r3 item 13: a running request tab's composer says 'Reply in this thread'; a stopped one keeps the start-the-agent line", () => {
  const src = read("gui-composer.js");
  const at = src.indexOf("function promptText(");
  const text = src.slice(at, src.indexOf("\n}\n", at) + 2);
  const run = (tab) => {
    const ctx = { matchMedia: () => ({ matches: true }), menuKind: () => null, machines: new Map(), touchComposer: () => false, requestHere: (t) => (t.view.request ? { id: t.view.request } : null) };
    vm.createContext(ctx);
    vm.runInContext(text, ctx);
    return ctx.promptText(tab, null);
  };
  const request = (running) => ({ view: { running, role: "orchestrator", orchestrator_global: true, request: "q1" } });
  assert.equal(run(request(true)), "Reply in this thread");
  assert.equal(run(request(false)), "claude isn't running. Press Enter to start it.");
  assert.equal(run({ view: { running: true, role: "worker" } }), "Ask anything, @ files, $ skills, / commands", "other threads are as before");
});

test("9r4 item 1: a request tab with its Plan open reads user_request once and the card draws its rows", async () => {
  const g = board();
  g[1].tab = "tab-21";
  const tab = requestTab("q-work", "tab-21");
  const h = harness({
    tabs: [anchorTab(g), tab],
    reply: (call) => (call.cmd === "user_request" ? view("q-work", { glance: g[1], stands: stands() }) : null),
  });
  const panel = { open: true, active: "plan", surfaces: [{ id: "plan", kind: "plan" }] };
  h.context.panelOf = () => panel;
  h.context.shownSurface = () => panel.surfaces[0];
  h.context.active = tab;
  h.run("renderRequests()");
  assert.equal(h.column.hidden, true, "Plan open: the column is gone");
  assert.match(h.run("requestCardHtml(active)"), /Loading…/, "while it is read");
  await flush();
  const html = h.run("requestCardHtml(active)");
  assert.doesNotMatch(html, /Loading…/);
  assert.match(html, /You asked[\s\S]*Find out why codes are late\./);
  assert.equal(h.log.threads, 2, "the thread draws the card loading, then once more when the reply lands");
  for (let i = 0; i < 4; i++) h.run("renderRequests()");
  await flush();
  assert.equal(h.log.calls.filter(([, c]) => c.cmd === "user_request").length, 1, "one read, not one per paint");
});

test("9r4 item 3: the Requests crumb is the theme's link colour", () => {
  assert.match(ruleOf(cssOf("gui-requests.css"), "#chat-head .crumb .request-back"), /color:\s*var\(--g-link\)/);
});

test("9r5 item 1: the footer facts are a three-column grid that wraps, Cost under List, and nothing is cut with an ellipsis", () => {
  const css = cssOf("gui-plan.css");
  assert.match(ruleOf(css, ".pl-foot .facts"), /display:\s*grid;[^}]*grid-template-columns:\s*repeat\(3,/);
  assert.doesNotMatch(ruleOf(css, ".pl-foot .facts"), /nowrap/);
  assert.doesNotMatch(ruleOf(css, ".pl-foot .fact b"), /text-overflow|nowrap/);
  assert.doesNotMatch(css, /\.pl-foot \.fact:last-child/);
});

test("9r5 item 5: the request header has a Plan button beside Mark done, and it opens and closes the Plan panel", () => {
  const g = board();
  g[1].tab = "tab-21";
  const tab = requestTab("q-work", "tab-21");
  const h = harness({ tabs: [anchorTab(g), tab] });
  const html = h.run("requestCrumbHtml(tabs[1])");
  assert.match(html, /<span class="orchestrator-actions"><button type="button" class="request-plan" data-request-plan aria-pressed="false">Plan<\/button><button type="button" data-request-action="done"/);
  const log = [];
  const panel = { open: false, active: "plan", surfaces: [{ id: "plan", kind: "plan" }] };
  h.context.panelOf = () => panel;
  h.context.shownSurface = () => panel.surfaces[0];
  h.context.addSurface = (t, s) => log.push(["add", t.key, s.id, s.kind]);
  h.context.closePanel = () => log.push(["close"]);
  h.context.active = tab;
  h.click(h.els.crumb, { "[data-request-plan]": { dataset: {} } });
  assert.deepEqual(log, [["add", "mac:tab-21", "plan", "plan"]]);
  panel.open = true;
  assert.match(h.run("requestCrumbHtml(tabs[1])"), /data-request-plan aria-pressed="true">Plan</);
  h.click(h.els.crumb, { "[data-request-plan]": { dataset: {} } });
  assert.deepEqual(log.at(-1), ["close"]);
});

test("9r5 item 7: a request panel with one block draws no section spacing under the root dot", () => {
  const css = cssOf("gui-plan.css");
  assert.match(ruleOf(css, '.pl-scroll[data-single="true"] .pl-sec-group'), /margin-top:\s*0/);
  assert.match(read("gui-plan.js"), /<div class="pl-scroll" data-single="\${plain}">/);
});

test("9r5 A: the card's pill row is the Captains first, then the links, and empty when there are neither", () => {
  const has = (captains, links) =>
    h0().run(`requestCardFrom(${JSON.stringify(glance("q", { captains }))}, { view: { stands: { asked: 'a', done: 'b', left: 'c', needs: '', links: ${JSON.stringify(links)} } } })`);
  const pills = (html) => [...html.matchAll(/<span class="rc-link">([^<]*)<\/span>/g)].map((m) => m[1]);
  const one = [{ machine: "mac", path: "/s", project: "storefront", machine_name: "Mac" }];
  assert.deepEqual(pills(has(one, ["PR #212 merged", "Ticket SUP-88"])), ["Captain storefront", "PR #212 merged", "Ticket SUP-88"]);
  assert.doesNotMatch(has([], []), /rc-links/);
});

test("9r6 item 1: no stylesheet styles the bare class .pl (the composers' mirror lines carry it), and the plan panel's root has its own class", () => {
  const files = readdirSync(new URL("./", import.meta.url)).filter((f) => f.endsWith(".css") && !/^(gui-)?composer/.test(f));
  assert.ok(files.includes("gui-plan.css") && files.includes("gui-requests-new.css"), "the sweep covers the plan and NewRequest sheets");
  const bare = [];
  for (const file of files) {
    for (const m of cssOf(file).matchAll(/([^{}]+)\{/g)) {
      for (const sel of m[1].split(",")) if (/\.pl(?![\w-])/.test(sel)) bare.push(`${file}: ${sel.trim()}`);
    }
  }
  assert.deepEqual(bare, [], "no selector names the bare .pl class");
  const js = read("gui-plan.js");
  assert.equal((js.match(/showPanel\(`<div class="plan-root"/g) ?? []).length, 3, "the three panel roots carry plan-root");
  assert.doesNotMatch(js, /class="pl"/, "and none carries .pl");
  assert.match(js, /closest\("\.plan-root"\)/, "a click is matched inside the plan-root");
});

test("9r6 item 3: a glance with no captains list draws no Captain pill and no crash", () => {
  const html = h0().run(`requestCardFrom({ id: "q", title: "t", last_who: "captain", opened_at: 1 }, { view: { stands: { asked: 'a', done: 'b', left: 'c', needs: '', links: ["PR #1"] } } })`);
  const pills = [...html.matchAll(/<span class="rc-link">([^<]*)<\/span>/g)].map((m) => m[1]);
  assert.deepEqual(pills, ["PR #1"]);
  assert.equal(h0().run(`requestLastWho({ last_who: "captain" }, "Orchestrator")`), "Captain");
});

test("9r6 item 3: a Captain with no project is named by its machine, and one with neither draws nothing", () => {
  const card = (captains) => {
    const html = h0().run(`requestCardFrom(${JSON.stringify(glance("q", { captains }))}, { view: { stands: { asked: 'a', done: 'b', left: 'c', needs: '', links: [] } } })`);
    return [...html.matchAll(/<span class="rc-link">([^<]*)<\/span>/g)].map((m) => m[1]);
  };
  assert.deepEqual(card([{ machine: "mac", path: "/s", project: "", machine_name: "MacBook" }]), ["Captain on MacBook"]);
  assert.deepEqual(card([{ machine: "mac", path: "/s", project: "", machine_name: "" }]), []);
  assert.deepEqual(card([{ machine: "mac", path: "/s", project: "storefront", machine_name: "MacBook" }, { machine: "mac", path: "/t", project: "", machine_name: "" }]), ["Captain storefront"]);
});

test("9r5 B: 'kept up to date by the Orchestrator' shows in the card's title while the Plan is closed, and hides while it is open", () => {
  const h = h0();
  const panel = { open: false, active: "plan", surfaces: [{ id: "plan", kind: "plan" }] };
  h.context.panelOf = () => panel;
  h.context.shownSurface = () => panel.surfaces[0];
  h.run("var tab = tabs[1]; rq.views.set('q-work', { sig: requestSig(requestHere(tab)), view: { stands: { asked: 'a', done: 'b', left: 'c', needs: '', links: [] } }, at: Date.now() })");
  const closed = h.run("requestCardHtml(tab)");
  assert.match(closed, /<span class="rc-title">Where this stands<\/span><span class="rc-kept">kept up to date by the Orchestrator<\/span>/);
  panel.open = true;
  assert.doesNotMatch(h.run("requestCardHtml(tab)"), /kept up to date/);
});

test("9r5 C: the Requests crumb shows only while the column is hidden: Plan open or a narrow screen", () => {
  const g = board();
  g[1].tab = "tab-21";
  const tab = requestTab("q-work", "tab-21");
  const h = harness({ tabs: [anchorTab(g), tab] });
  const panel = { open: false, active: "plan", surfaces: [{ id: "plan", kind: "plan" }] };
  h.context.panelOf = () => panel;
  h.context.shownSurface = () => panel.surfaces[0];
  assert.match(h.run("requestCrumbHtml(tabs[1])"), /^<span class="request-dot"/, "column beside the thread: no crumb");
  panel.open = true;
  assert.match(h.run("requestCrumbHtml(tabs[1])"), /^<button type="button" class="request-back" data-rq-back>Requests<\/button><span class="sep">\/<\/span><span class="request-dot"/, "Plan open: the crumb");
  panel.open = false;
  h.context.narrow = () => true;
  assert.match(h.run("requestCrumbHtml(tabs[1])"), /^<button type="button" class="request-back"/, "narrow: the crumb");
});

test("9r4 item 5: the minimap sits at the thread's left edge, not over the Requests column", () => {
  assert.match(ruleOf(cssOf("gui-requests.css"), "#chat[data-requests] #minimap"), /left:\s*340px/);
});
