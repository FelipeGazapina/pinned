import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const line = (src, from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
/** Plain copies of what the page's own realm builds, for deepStrictEqual. */
const plain = (value) => JSON.parse(JSON.stringify(value));

function element() {
  return {
    hidden: false,
    dataset: {},
    innerHTML: "",
    textContent: "",
    value: "",
    attrs: {},
    style: {},
    listeners: {},
    setAttribute(name, value) {
      this.attrs[name] = String(value);
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
    append() {},
    focus() {},
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

const catalogOf = (names = {}) => ({
  name: "Orchestrator",
  machines: [
    { slug: "laptop", name: names.laptop ?? "laptop", online: true },
    { slug: "studio", name: names.studio ?? "studio", online: true },
  ],
  projects: [
    { slug: "storefront", name: names.storefront ?? "storefront", repo: "", machines: [{ id: "mac", slug: "laptop", name: names.laptop ?? "laptop", online: true, captain: "working" }] },
    { slug: "billing-api", name: "billing-api", repo: "", machines: [{ id: "studio", slug: "studio", name: names.studio ?? "studio", online: true, captain: "idle" }] },
  ],
});
const factsOf = (over = {}) => [
  { slug: "storefront", name: "storefront", machine: { id: "mac", name: "laptop" }, state: "working", ...over.storefront },
  { slug: "billing-api", name: "billing-api", machine: { id: "studio", name: "studio" }, state: "idle", ...over.billing },
];

const anchorTab = (over = {}) => ({ key: "mac:tab-1", machine: "mac", view: { id: "tab-1", orchestrator_global: true, plans: [], ...over } });
const requestTab = (id, tabId) => ({ key: `mac:${tabId}`, machine: "mac", view: { id: tabId, orchestrator_global: true, request: id, plans: [] } });

/** gui-requests-new.js in a page: the app's globals stubbed, the real esc, composer-tags and model. */
function harness({ tabs = [anchorTab()], glances = [], catalog = catalogOf(), facts = factsOf(), reply = () => [], host = "mac" } = {}) {
  const log = { calls: [], told: [], activated: [], revealed: [], threads: 0 };
  const els = { chat: element(), crumb: element() };
  const made = [];
  const models = [];
  let uuid = 0;
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
    crypto: { randomUUID: () => `key-${++uuid}` },
    tabs,
    active: tabs[0] ?? null,
    glances,
    catalog,
    facts,
    globalSettings: { name: "Orchestrator", busy: "queue" },
    requestHost: () => host,
    requestGlances: () => context.glances,
    isGlobalOrchestratorTab: (tab) => tab?.view?.orchestrator_global === true,
    composerTagCatalog: () => context.catalog,
    captainFacts: () => context.facts,
    icon: (name) => `<svg data-icon="${name}"></svg>`,
    renderThread: () => log.threads++,
    onModelChange: (fn) => models.push(fn),
    tell: (html) => log.told.push(html),
    activate: (tab) => {
      context.active = tab;
      log.activated.push(tab.key);
    },
    revealTab: async (machine, id) => {
      log.revealed.push(`${machine}:${id}`);
      const tab = context.tabs.find((t) => t.key === `${machine}:${id}`) ?? { key: `${machine}:${id}`, machine, view: { id } };
      context.activate(tab);
      return tab;
    },
    callOn: async (machine, call) => {
      log.calls.push([machine, plain(call)]);
      return context.reply(call);
    },
    reply,
    document: {
      getElementById: (id) => (id === "chat" ? els.chat : null),
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
  for (const file of ["composer-tags.js", "requests-model.js", "gui-requests-new.js"]) run(read(file));
  const panel = run("newRequestPanel");
  const click = (selectors) => panel.listeners.click({ target: { closest: (sel) => selectors[sel] ?? null } });
  const type = (text) => run(`setNewRequestText(${JSON.stringify(text)})`);
  const items = () => plain(run("newRequestItems()"));
  const cards = () => run("newRequestCardsHtml()");
  const send = () => run("sendNewRequest()");
  const created = () => log.calls.filter(([, call]) => call.cmd === "new_user_requests");
  const prompts = () => log.calls.filter(([, call]) => call.cmd === "prompt");
  return { context, run, log, els, made, panel, models, click, type, items, cards, send, created, prompts };
}

const TWO = "#storefront show the delivery estimate on the cart page, before the customer reaches checkout.\n\n#billing-api @studio the monthly export is missing refunds. Find out why.";
const cardCount = (html) => (html.match(/data-nr-card=/g) ?? []).length;
const tagNames = (html) => [...html.matchAll(/<([a-z][a-z0-9-]*)/gi)].map((m) => m[1].toLowerCase());
const hrefs = (html) => [...html.matchAll(/\bhref\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi)].map((m) => m[1]);
const created = (h, n = 0) => h.created()[n]?.[1].items ?? [];
const sent = (reply) => (call) => (call.cmd === "new_user_requests" ? reply(call) : null);
const opened = (call) => call.items.map((item, i) => ({ key: item.key, id: `q${i + 1}`, tab: `tab-${i + 5}`, handoff: "started" }));

test("two #projects make two cards, each with its title and project", () => {
  const h = harness();
  h.type(TWO);
  const items = h.items();
  assert.equal(items.length, 2);
  assert.deepEqual(items.map((i) => i.project), ["storefront", "billing-api"]);
  assert.equal(items[0].title, "show the delivery estimate on the cart page, before the…");
  assert.equal(items[1].title, "the monthly export is missing refunds");
  const html = h.cards();
  assert.equal(cardCount(html), 2);
  assert.match(html, /Show the delivery estimate on the cart page, before the…/, "9r7: the card shows the title with a capital; the item keeps the paragraph's own text");
  assert.match(html, /This opens 2 threads/);
});

test("an untagged paragraph joins the item above it", () => {
  const h = harness();
  h.type("#storefront Add the estimate.\n\nIt should read days, not dates.\n\n#billing-api Fix the export.");
  const items = h.items();
  assert.equal(items.length, 2);
  assert.equal(items[0].text, "#storefront Add the estimate.\n\nIt should read days, not dates.");
  assert.equal(items[1].text, "#billing-api Fix the export.");
  assert.equal(cardCount(h.cards()), 2);
});

test("the routing preview says where each card goes: working under Queue, idle, no Captain, offline", () => {
  const h = harness();
  h.type(TWO);
  const [store, billing] = [0, 1].map((i) => `newRequestPreview(newRequestItems()[${i}])`);
  assert.equal(h.run(store), "Goes to Captain storefront on laptop, working now, so it waits in the queue");
  assert.equal(h.run(billing), "Goes to Captain billing-api on studio, free now");
  h.context.facts = factsOf({ storefront: { state: "none" } });
  assert.equal(h.run(store), "Goes to Captain storefront on laptop. It has no Captain yet, so one starts.");
  h.context.facts = factsOf({ storefront: { state: "offline" } });
  assert.equal(h.run(store), "Goes to Captain storefront on laptop, which is offline, so it waits until it is back");
  h.context.facts = factsOf();
  h.context.globalSettings.busy = "refuse";
  assert.equal(h.run(store), "Goes to Captain storefront on laptop, working now, and may be turned away until it is free");
  h.type("Look at the logs.\n\n#billing-api Check the export.");
  assert.equal(h.run("newRequestPreview(newRequestItems()[0])"), "No project: Orchestrator decides");
  assert.deepEqual(h.log.calls, [], "the preview calls nothing");
  const html = h.cards();
  assert.match(html, /No project: Orchestrator decides/);
  assert.match(html, /Goes to Captain billing-api on studio, free now/);
});

test("the preview shows only with two or more items or a close match; one plain item sends straight through", () => {
  const h = harness({ glances: [glance("q-gift", { title: "Add gift cards to checkout", tab: "tab-12" })] });
  h.type("#storefront Show the delivery estimate on the cart page.");
  assert.equal(cardCount(h.cards()), 0, "one item and no match draws no cards");
  assert.match(h.run("newRequestSendLabel()"), /^Open thread$/);
  h.type(TWO);
  assert.equal(cardCount(h.cards()), 2);
  assert.match(h.run("newRequestSendLabel()"), /^Open 2 threads$/);
  h.type("#storefront add gift cards to the checkout page");
  assert.equal(cardCount(h.cards()), 1, "a close match brings the card back");
});

test("Rename changes the title that is sent, capped at 60 characters", async () => {
  const h = harness({ reply: sent(opened) });
  h.type(TWO);
  h.click({ "[data-nr-rename]": { dataset: { nrRename: "0" } } });
  assert.match(h.cards(), /<input[^>]*data-nr-title="0"/, "the title becomes an input");
  h.panel.listeners.focusout({ target: { closest: (sel) => (sel === "[data-nr-title]" ? { dataset: { nrTitle: "0" }, value: "Delivery estimate on the cart page" } : null) } });
  assert.doesNotMatch(h.cards(), /<input/, "renaming ends");
  assert.match(h.cards(), /Delivery estimate on the cart page/);
  h.click({ "[data-nr-rename]": { dataset: { nrRename: "1" } } });
  h.panel.listeners.focusout({ target: { closest: (sel) => (sel === "[data-nr-title]" ? { dataset: { nrTitle: "1" }, value: "x".repeat(100) } : null) } });
  await h.send();
  const items = created(h);
  assert.equal(items[0].title, "Delivery estimate on the cart page");
  assert.equal(items[1].title, "x".repeat(60));
  assert.equal(items[0].text, TWO.split("\n\n")[0], "the text is not renamed");
});

test("Send makes one new_user_requests call with a key per item, and opens the first new tab", async () => {
  const h = harness({ reply: sent(opened) });
  h.type(TWO);
  await h.send();
  assert.equal(h.log.calls.length, 1);
  const [machine, call] = h.created()[0];
  assert.equal(machine, "mac");
  assert.deepEqual(Object.keys(call), ["cmd", "items"]);
  assert.deepEqual(
    call.items.map(({ title, text, project, machine }) => ({ title, text, project, machine })),
    [
      { title: "show the delivery estimate on the cart page, before the…", text: TWO.split("\n\n")[0], project: "storefront", machine: "laptop" },
      { title: "the monthly export is missing refunds", text: TWO.split("\n\n")[1], project: "billing-api", machine: "studio" },
    ],
  );
  assert.equal(new Set(call.items.map((i) => i.key)).size, 2, "one key each");
  assert.deepEqual(h.log.revealed, ["mac:tab-5"], "the first new tab opens");
  assert.equal(h.run("nr.text"), "", "the draft is gone");
  assert.equal(h.panel.hidden, true);
});

test("an untagged item is sent without project or machine", async () => {
  const h = harness({ reply: sent(opened) });
  h.type("Look at the logs.\n\n#billing-api Check the export.");
  await h.send();
  const [first] = created(h);
  assert.equal("project" in first, false);
  assert.equal("machine" in first, false);
});

test("a retry after an error sends the same keys", async () => {
  const h = harness();
  h.context.reply = () => {
    throw new Error("socket closed");
  };
  h.type(TWO);
  await h.send();
  await h.send();
  const [first, second] = h.created().map(([, call]) => call.items.map((i) => i.key));
  assert.equal(first.length, 2);
  assert.deepEqual(second, first, "the retry keeps the keys");
  assert.equal(h.run("nr.text"), TWO, "the draft is kept while nothing went through");
  h.context.reply = sent(opened);
  await h.send();
  assert.deepEqual(h.created()[2][1].items.map((i) => i.key), first);
  h.type(TWO);
  await h.send();
  assert.notDeepEqual(h.created()[3][1].items.map((i) => i.key), first, "a new draft gets new keys");
});

test("a close match sends the paragraph as a Prompt in that request's tab, and no new_user_requests", async () => {
  const gift = glance("q-gift", { title: "Add gift cards to checkout", last_text: "Tests are running", tab: "tab-12" });
  const tabs = [anchorTab(), requestTab("q-gift", "tab-12")];
  const h = harness({ tabs, glances: [gift, glance("q-closed", { title: "Add gift cards to checkout", closed_at: 5, tab: "tab-13" })] });
  h.type("#storefront add gift cards to the checkout page");
  const html = h.cards();
  assert.match(html, /Looks close to an open thread: <b>Add gift cards to checkout<\/b>/);
  h.click({ "[data-nr-join]": { dataset: { nrJoin: "0" } } });
  assert.match(h.cards(), /data-nr-join="0"[^>]*aria-pressed="true"/);
  assert.match(h.run("newRequestSendLabel()"), /^Send$/);
  await h.send();
  assert.deepEqual(h.log.calls, [["mac", { cmd: "prompt", tab: "tab-12", text: "#storefront add gift cards to the checkout page" }]]);
  assert.deepEqual(h.log.activated, ["mac:tab-12"], "the matching tab opens");
});

test("without choosing the close match a new request opens", async () => {
  const h = harness({ glances: [glance("q-gift", { title: "Add gift cards to checkout", tab: "tab-12" })], reply: sent(opened) });
  h.type("#storefront add gift cards to the checkout page");
  await h.send();
  assert.equal(h.created().length, 1);
  assert.deepEqual(h.prompts(), []);
});

test("a close match whose request rests with no tab asks the host to open one", async () => {
  const h = harness({
    glances: [glance("q-gift", { title: "Add gift cards to checkout", tab: null })],
    reply: (call) => (call.cmd === "user_request_action" ? { tab: "tab-21" } : null),
  });
  h.type("#storefront add gift cards to the checkout page");
  h.click({ "[data-nr-join]": { dataset: { nrJoin: "0" } } });
  await h.send();
  assert.deepEqual(h.log.calls.map(([, call]) => call), [
    { cmd: "user_request_action", id: "q-gift", action: "open" },
    { cmd: "prompt", tab: "tab-21", text: "#storefront add gift cards to the checkout page" },
  ]);
});

test("one joined item and one new: only the new one is created", async () => {
  const h = harness({
    tabs: [anchorTab(), requestTab("q-gift", "tab-12")],
    glances: [glance("q-gift", { title: "Add gift cards to checkout", tab: "tab-12" })],
    reply: sent(opened),
  });
  h.type("#storefront add gift cards to the checkout page\n\n#billing-api Fix the export.");
  h.click({ "[data-nr-join]": { dataset: { nrJoin: "0" } } });
  await h.send();
  assert.equal(created(h).length, 1);
  assert.equal(created(h)[0].project, "billing-api");
  assert.deepEqual(h.prompts().map(([, call]) => call.tab), ["tab-12"]);
});

test("when new_user_requests errors the whole text goes to the single chat unchanged, with no banner", async () => {
  const text = "  #storefront Show the estimate.\n\n\n#billing-api Fix the export.  ";
  const h = harness({
    reply: (call) => {
      if (call.cmd === "new_user_requests") throw new Error("unknown variant `new_user_requests`");
      return null;
    },
  });
  h.type(text);
  await h.send();
  assert.deepEqual(h.log.calls.map(([, call]) => call.cmd), ["new_user_requests", "prompt"]);
  assert.deepEqual(h.prompts()[0], ["mac", { cmd: "prompt", tab: "tab-1", text }]);
  assert.deepEqual(h.log.told, [], "nothing is told");
  assert.equal(h.run("nr.text"), "", "the draft goes once the send succeeded");
  assert.deepEqual(h.log.activated, ["mac:tab-1"], "the single chat opens");
});

test("when the single chat cannot take the text either, the draft stays", async () => {
  const text = "#storefront Show the estimate.\n\n#billing-api Fix the export.";
  const h = harness({
    reply: () => {
      throw new Error("unknown variant");
    },
  });
  h.run("openNewRequest()");
  h.type(text);
  await h.send();
  assert.equal(h.run("nr.text"), text);
  assert.equal(h.panel.hidden, false, "the screen stays open with the draft");
  assert.equal(h.log.told.length, 1);
});

test("with no anchor tab the single chat is opened first", async () => {
  const h = harness({
    tabs: [requestTab("q1", "tab-9")],
    reply: (call) => {
      if (call.cmd === "new_user_requests") throw new Error("unknown variant");
      return call.cmd === "open_global_orchestrator" ? "tab-1" : null;
    },
  });
  h.type("#storefront Show the estimate.");
  await h.send();
  assert.deepEqual(h.log.calls.map(([, call]) => call.cmd), ["new_user_requests", "open_global_orchestrator", "prompt"]);
  assert.equal(h.prompts()[0][1].tab, "tab-1");
});

test("New request opens with the draft, an empty draft cannot send, and another tab closes it", async () => {
  const h = harness({ reply: sent(opened) });
  h.run("openNewRequest()");
  assert.equal(h.panel.hidden, false);
  await h.send();
  assert.deepEqual(h.log.calls, [], "nothing to send");
  h.type("#storefront Show the estimate.");
  h.click({ "[data-nr-close]": {} });
  assert.equal(h.panel.hidden, true);
  assert.equal(h.run("nr.text"), "#storefront Show the estimate.", "the draft survives closing");
  h.run("openNewRequest()");
  assert.equal(h.run("nr.box.value"), "#storefront Show the estimate.");
  h.context.active = requestTab("q1", "tab-2");
  for (const fn of h.models) fn();
  assert.equal(h.panel.hidden, true, "choosing another thread closes the screen");
});

test("markup in any drawn field stays text: no img element and no javascript: href", async () => {
  const bad = "<img src=x onerror=alert(1)>";
  const link = "[x](javascript:alert(1))";
  const catalog = catalogOf({ storefront: bad, laptop: bad, studio: bad });
  const h = harness({
    catalog,
    facts: [{ slug: "", name: bad, machine: { id: "mac", name: bad }, state: "working" }],
    glances: [glance("q-hostile", { title: `${bad} ${link}`, project: bad, last_text: `${bad} ${link}`, tab: bad })],
  });
  const tag = h.run(`tagSlug(${JSON.stringify(bad)})`);
  catalog.projects[0].slug = tag;
  h.context.facts[0].slug = tag;
  h.type(`#${tag} ${bad} ${link}\n\n#billing-api ${bad} ${link}`);
  h.click({ "[data-nr-rename]": { dataset: { nrRename: "0" } } });
  h.context.reply = () => {
    throw new Error(`${bad} ${link}`);
  };
  await h.send();
  const pieces = { cards: h.cards(), label: h.run("newRequestSendLabel()"), told: h.log.told.join(""), panel: h.panel.innerHTML };
  assert.ok(cardCount(pieces.cards) === 2, "the cards draw");
  assert.match(pieces.cards, /Looks close/, "the close match draws");
  for (const [name, html] of Object.entries(pieces)) {
    assert.deepEqual(tagNames(html).filter((t) => t === "img"), [], `${name}: no img element`);
    assert.deepEqual(hrefs(html).filter((href) => /javascript:/i.test(href)), [], `${name}: no javascript: href`);
    assert.doesNotMatch(html, /<img/i, `${name}: no raw <img`);
  }
  assert.ok(pieces.cards.includes("&#60;img src=x onerror=alert(1)&#62;"), "the markup shows as text");
  assert.ok(pieces.told.includes("&#60;img src=x onerror=alert(1)&#62;"), "the error shows as text");
  assert.match(pieces.cards, /<input[^>]*value="[^"]*&#60;img/, "the rename input holds the markup as text");
});

const failWith = (message) => (call) => {
  if (call.cmd === "new_user_requests") throw new Error(message);
  return null;
};

test("only an error that says the host does not know the command falls back to the single chat", async () => {
  for (const message of [
    "unknown variant `new_user_requests`, expected one of `agents`, `work_cost`",
    "this hub runs pinned 0.17.19, which can't answer that: unknown variant `new_user_requests`",
    "unknown command",
    "unknown cmd new_user_requests",
  ]) {
    const h = harness({ reply: failWith(message) });
    h.type("#storefront Show the estimate.");
    await h.send();
    assert.equal(h.prompts().length, 1, `falls back on: ${message}`);
  }
  for (const message of ["timed out", "socket closed", "host is offline", "not a list of requests"]) {
    const h = harness({ reply: failWith(message) });
    h.type("#storefront Show the estimate.");
    await h.send();
    assert.deepEqual(h.prompts(), [], `no prompt on: ${message}`);
  }
});

test("a timeout keeps the draft and shows the error, and the retry sends the same keys", async () => {
  const h = harness({ reply: failWith("timed out waiting for the host") });
  h.run("openNewRequest()");
  h.type(TWO);
  await h.send();
  assert.deepEqual(h.prompts(), [], "no prompt reached the single chat");
  assert.equal(h.run("nr.text"), TWO, "the draft stays");
  assert.equal(h.panel.hidden, false, "the screen stays open");
  assert.equal(h.log.told.length, 1);
  assert.match(h.log.told[0], /timed out waiting for the host/);
  h.context.reply = sent(opened);
  await h.send();
  const [first, retry] = h.created().map(([, call]) => call.items.map((i) => i.key));
  assert.equal(first.length, 2);
  assert.deepEqual(retry, first, "the retry reuses the keys");
  assert.equal(h.run("nr.text"), "", "the draft goes once it was sent");
  assert.deepEqual(h.prompts(), []);
});

test("the header is the chat head's crumb, not a second bar under it", () => {
  const h = harness();
  assert.equal(h.run("newRequestCrumbHtml()"), "", "closed: no crumb");
  h.run("openNewRequest()");
  const crumb = h.run("newRequestCrumbHtml()");
  assert.match(crumb, /<button[^>]*data-nr-close[^>]*>Requests<\/button>/);
  assert.match(crumb, /<span class="nr-sep">\/<\/span>/);
  assert.match(crumb, /<h2[^>]*>New request<\/h2>/);
  assert.equal(h.made.some((el) => el.className === "nr-head"), false, "the panel draws no head bar of its own");
  assert.ok(h.log.threads > 0, "opening redraws the chat head");
  const before = h.log.threads;
  h.els.crumb.listeners.click({ target: { closest: (sel) => (sel === "[data-nr-close]" ? {} : null) } });
  assert.equal(h.panel.hidden, true, "the crumb's Requests link closes the screen");
  assert.ok(h.log.threads > before, "closing redraws the chat head");
  assert.equal(h.run("newRequestCrumbHtml()"), "");
});

test("a card draws no project chip: the routing preview names the Captain", () => {
  const h = harness();
  h.type(TWO);
  const html = h.cards();
  assert.doesNotMatch(html, /nr-project/);
  assert.match(html, /Goes to Captain storefront on laptop/);
});

test("the close match is a dashed row under the cards with Reply there instead", () => {
  const h = harness({ glances: [glance("q-gift", { title: "Add gift cards to checkout", tab: "tab-12" })] });
  h.type("#storefront add gift cards to the checkout page");
  const html = h.cards();
  const row = html.slice(html.indexOf('class="nr-close-row"'));
  assert.ok(html.includes('class="nr-close-row"'), "a separate row");
  assert.ok(html.indexOf('class="nr-close-row"') > html.lastIndexOf("data-nr-card="), "after the cards");
  assert.doesNotMatch(html.slice(0, html.indexOf('class="nr-close-row"')), /Looks close/, "not inside a card");
  assert.match(row, /Looks close to an open thread: <b>Add gift cards to checkout<\/b>/);
  assert.match(row, /<button[^>]*data-nr-join="0"[^>]*>Reply there instead<\/button>/);
  h.click({ "[data-nr-join]": { dataset: { nrJoin: "0" } } });
  assert.match(h.cards(), /data-nr-join="0"[^>]*aria-pressed="true"/, "choosing it joins the thread");
});

test("the text box paints #project and @machine with the composer's tag painting", () => {
  const h = harness();
  h.type("#storefront @studio check #nope and <img src=x onerror=alert(1)>");
  const mirror = h.run("nrMirror").innerHTML;
  assert.match(mirror, /<span class="tag project">#storefront<\/span>/);
  assert.match(mirror, /<span class="tag machine">@studio<\/span>/);
  assert.match(mirror, /<span class="tag project unknown">#nope<\/span>/);
  assert.doesNotMatch(mirror, /<img/i);
  assert.match(mirror, /&#60;img/, "the markup shows as text");
  h.type("");
  assert.equal(h.run("nrMirror").innerHTML, "", "an empty draft paints nothing");
});

const cssOfNew = (name) => read(name).replace(/\/\*[\s\S]*?\*\//g, "");
const ruleOfNew = (css, selector) => {
  const at = css.indexOf(`${selector} {`);
  assert.ok(at >= 0, `${selector} is a rule`);
  return css.slice(at, css.indexOf("}", at));
};

test("9r5 item 2: a mirror line is a block of its own, so tags stay inline and the sentence stays on the line", () => {
  const h = harness();
  h.type("#billing-api @studio the monthly export is missing refunds. Find out why.");
  const mirror = h.run("nrMirror").innerHTML;
  assert.equal(mirror, '<div class="pl"><span class="tag project">#billing-api</span> <span class="tag machine">@studio</span> the monthly export is missing refunds. Find out why.</div>');
  const css = cssOfNew("gui-requests-new.css");
  assert.doesNotMatch(css, /\.nr-mirror \.pl\b(?!-)/, "9r6: no per-screen override of the mirror's .pl line is left");
  const plan = cssOfNew("gui-plan.css");
  assert.match(ruleOfNew(plan, ".plan-root"), /display:\s*flex;\s*flex-direction:\s*column/, "the flex column is the plan panel's own class, so the mirror's .pl lines stay blocks");
});

test("9r5 item 3: the preview header carries no New chip", () => {
  const h = harness();
  h.type(TWO);
  const html = h.cards();
  assert.match(html, /This opens 2 threads/);
  assert.doesNotMatch(html, />New</);
  assert.doesNotMatch(html, /nr-new/);
});

// ---- 0.18.8 unit 9r7 ----
test("9r7 item 5: opening and closing New request redraws the Requests column, which hides while the screen is open", () => {
  const h = harness();
  let redraws = 0;
  h.context.renderRequests = () => redraws++;
  h.run("openNewRequest()");
  assert.ok(redraws > 0, "opening asks the column to redraw");
  const opened = redraws;
  h.run("closeNewRequest()");
  assert.ok(redraws > opened, "closing asks it to redraw");
});

test("9r7 item 5: the screen no longer leaves 340 px for the column", () => {
  assert.doesNotMatch(cssOfNew("gui-requests-new.css"), /#chat\[data-requests\] #new-request/);
});

test("9r7 item 6: the text box grows with its text from a small minimum and has no resize handle", () => {
  const rule = ruleOfNew(cssOfNew("gui-requests-new.css"), ":is(#new-request, #all-activity) .nr-text");
  assert.match(rule, /resize:\s*none/);
  assert.doesNotMatch(rule, /min-height:\s*110px/);
  assert.match(rule, /min-height:\s*(?:[1-5]\d|6[0-4])px/, "a small minimum, at most 64 px");
  const h = harness();
  h.run("openNewRequest()");
  h.run("nr.box.scrollHeight = 137");
  h.type("#storefront add gift cards.\n\nSecond paragraph.");
  assert.equal(h.run("nr.box.style.height"), "137px", "the box takes the height of its text");
});

test("9r7 item 7: Reply there instead is a plain text button in the link colour, with no border", () => {
  const rule = ruleOfNew(cssOfNew("gui-requests-new.css"), ":is(#new-request, #all-activity) .nr-join");
  assert.match(rule, /border:\s*0/);
  assert.match(rule, /color:\s*var\(--g-link\)/);
  assert.doesNotMatch(rule, /border-radius/);
  const h = harness({ glances: [glance("q-gift", { title: "Add gift cards to checkout", tab: "tab-12" })] });
  h.type("#storefront add gift cards to the checkout page");
  assert.match(h.cards(), /<button type="button" class="nr-join" data-nr-join="0" aria-pressed="false">Reply there instead<\/button>/);
});

test("9r7 item 8: a card title starts with a capital letter; the title that is sent keeps the paragraph's own text", async () => {
  const h = harness({ reply: sent(opened) });
  h.type(TWO);
  const titles = [...h.cards().matchAll(/<span class="nr-title">([^<]*)<\/span>/g)].map((m) => m[1]);
  assert.deepEqual(titles, ["Show the delivery estimate on the cart page, before the…", "The monthly export is missing refunds"]);
  await h.send();
  assert.deepEqual(created(h).map((item) => item.title), ["show the delivery estimate on the cart page, before the…", "the monthly export is missing refunds"]);
});

// ---- 0.18.8 unit 9r8 ----
test("9r8 item 1: the New request column is 720 px of content, with the 24 px side room kept outside it", () => {
  const rule = ruleOfNew(cssOfNew("gui-requests-new.css"), ":is(#new-request, #all-activity) .nr-main");
  assert.match(rule, /box-sizing:\s*border-box/);
  assert.match(rule, /width:\s*768px/, "720 px of content plus 24 px on each side");
  assert.match(rule, /padding:\s*24px/);
  assert.match(rule, /max-width:\s*100%/);
  const stream = cssOfNew("gui-requests-stream.css");
  assert.match(ruleOfNew(stream, "#all-activity .rqs-list > *"), /width:\s*680px/, "All activity keeps the Stream board's 680 px column");
});
