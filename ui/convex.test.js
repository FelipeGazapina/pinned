import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import * as PinnedGraph from "./graph.js";

const settingsSrc = readFileSync(new URL("./settings.js", import.meta.url), "utf8");
const convexSrc = readFileSync(new URL("./convex.js", import.meta.url), "utf8");

/** A value made inside the page, as plain JSON, since its objects carry the page's own prototypes. */
const plain = (value) => JSON.parse(JSON.stringify(value));

function element(id) {
  const listeners = {};
  return {
    id,
    listeners,
    addEventListener(type, fn) {
      (listeners[type] ??= []).push(fn);
    },
    dataset: {},
    hidden: false,
    innerHTML: "",
    outerHTML: "",
    textContent: "",
    value: "",
    classList: { contains: () => false, add() {}, remove() {}, toggle() {} },
    focus() {},
    scrollIntoView() {},
  };
}

const SHOP = { path: "/Users/ana/code/shop", name: "shop", repo: "github.com/acme/shop", convex: "dev:quick-lemur-789" };
const SITE = { path: "/Users/ana/code/site", name: "site", repo: "github.com/acme/site", convex: "dev:joyful-jaguar-123" };
const BILLING = { path: "/Users/ana/code/billing", name: "billing", repo: "github.com/acme/billing" };
const SITE_CONNECTION = { id: "joyful-jaguar-123", kind: "dev", url: "https://joyful-jaguar-123.convex.cloud", team: "acme", project: "site", key_hint: "…a1b2", repos: ["github.com/acme/site"] };
const BILLING_CONNECTION = { id: "happy-otter-456", kind: "prod", url: "https://happy-otter-456.convex.cloud", team: "acme", project: "billing", key_hint: "…9f3c", repos: ["github.com/acme/billing"] };

/** settings.js and convex.js in one page, with the globals of the scripts around them stubbed. */
function load({ signedIn = true, projects = [SHOP, SITE, BILLING], connections = [], reply = (call) => call } = {}) {
  const elements = new Map();
  const byId = (id) => {
    if (!elements.has(id)) elements.set(id, element(id));
    return elements.get(id);
  };
  const calls = [];
  const context = {
    console,
    addEventListener() {},
    setTimeout() {},
    TAURI: false,
    LOCAL: "local",
    account: signedIn ? { email: "ana@acme.dev" } : null,
    hub: {
      async hub(call) {
        calls.push(call);
        return reply(call);
      },
    },
    machines: new Map([["local", { view: { online: true }, projects }]]),
    load: () => null,
    save() {},
    listen() {},
    tilde: (path) => path.replace(/^\/Users\/[^/]+/, "~"),
    agentApproval: () => null,
    agentAsking: () => null,
    agentAskingHtml: () => "",
    agentConsentHtml: () => "",
    externalAgentsCardHtml: () => "",
    icon: (name) => `<svg data-icon="${name}"></svg>`,
    PinnedGraph,
    projectPathOf: (tab) => tab.view.cwd,
    opened: [],
    async openSettings(name) {
      context.opened.push(name);
    },
    document: {
      body: { dataset: {} },
      getElementById: byId,
      querySelector: () => element(),
      querySelectorAll: () => [],
    },
  };
  context.globalThis = context;
  vm.runInNewContext(
    `${settingsSrc}\n${convexSrc}\nglobalThis.__settings = settings; globalThis.__actions = actions; globalThis.__sections = settingsSections; globalThis.__ui = convexUi; globalThis.__setView = (next) => { view = next; }; globalThis.__dismissed = convexDismissed;`,
    context,
  );
  context.__settings.convex = connections;
  const fire = (type, target) => byId("settings-page").listeners[type].forEach((fn) => fn({ target: { dataset: {}, ...target } }));
  return { context, calls, byId, fire, listeners: byId("settings-page").listeners };
}

test("signed out, the Convex card asks to sign in and has no key field", () => {
  const { context } = load({ signedIn: false });
  const html = context.convexCardHtml();
  assert.match(html, /<div class="card" id="convex">/);
  assert.match(html, /Sign in to pinned to connect Convex\. The hub keeps the deploy keys\./);
  assert.match(html, /<span class="badge" data-on="false">Not connected<\/span>/);
  assert.doesNotMatch(html, /convex-key/);
});

test("the card sits after Upstash on Connectors, where the settings search finds it", () => {
  const { context } = load();
  const html = context.integrationsHtml();
  assert.ok(html.indexOf("<b>Upstash Box</b>") < html.indexOf("<b>Convex</b>"), "Convex follows Upstash");
  const entry = context.__sections.find((s) => s.page === "integrations").search.find((item) => item.title === "Convex");
  assert.match(html, new RegExp(`<div class="card" id="${entry.id}">`));
});

test("each connection shows its project, deployment, kind and key hint, never a key", () => {
  const { context } = load({ connections: [SITE_CONNECTION, BILLING_CONNECTION] });
  const html = context.convexCardHtml();
  assert.match(html, /<span class="badge" data-on="true">2 connected<\/span>/);
  assert.match(html, /<b>site<\/b> <code>joyful-jaguar-123<\/code> <span class="badge convex-kind" data-kind="dev">dev<\/span> <span class="hint">key …a1b2<\/span>/);
  assert.match(html, /<b>billing<\/b> <code>happy-otter-456<\/code> <span class="badge convex-kind" data-kind="prod">prod<\/span> <span class="hint">key …9f3c<\/span>/);
  assert.match(html, /title="github\.com\/acme\/site">site<button type="button" data-action="convex-remove-repo" data-id="joyful-jaguar-123" data-repo="github\.com\/acme\/site"/);
  assert.match(html, /<select data-convex-add="joyful-jaguar-123" aria-label="Add a project"><option value="" selected>Add a project…<\/option><option value="github\.com\/acme\/shop">shop · ~\/code\/shop<\/option><option value="github\.com\/acme\/billing">billing · ~\/code\/billing<\/option><\/select>/);
  assert.match(html, /data-action="disconnect-convex" data-id="happy-otter-456">Disconnect/);
  assert.match(html, /<input id="convex-key" type="password" placeholder="[^"]*" autocomplete="off" spellcheck="false">/, "the key field starts empty");
});

test("a project whose dotenv names an unconnected deployment is listed and picked for the form", () => {
  const { context } = load({ connections: [SITE_CONNECTION] });
  const html = context.convexCardHtml();
  assert.match(html, /Pinned found <code>quick-lemur-789<\/code> in <code>~\/code\/shop<\/code>\.<\/p>/);
  assert.doesNotMatch(html, /Pinned found <code>joyful-jaguar-123<\/code>/, "a served project is not listed");
  assert.match(html, /<option value="github\.com\/acme\/shop" selected>shop · ~\/code\/shop<\/option>/);
  assert.match(html, /placeholder="dev:quick-lemur-789\|…"/);
  assert.match(html, /data-action="open-convex-keys">Open Convex ↗/);
  for (const permission of ["deployment:data:view", "deployment:functions:runInternalQueries", "deployment:functions:runTestQuery", "deployment:logs:view"]) {
    assert.match(html, new RegExp(`<span class="convex-permissions">.*<code>${permission}</code>`));
  }
});

test("a found project whose deployment is already connected joins it in one click", async () => {
  const shopOnSite = { ...SHOP, convex: "dev:joyful-jaguar-123" };
  const { context, calls } = load({ projects: [shopOnSite, SITE], connections: [SITE_CONNECTION], reply: (call) => ({ ...SITE_CONNECTION, repos: call.repos }) });
  const html = context.convexCardHtml();
  assert.match(html, /data-action="convex-add-repo" data-id="joyful-jaguar-123" data-repo="github\.com\/acme\/shop">Add shop to it/);
  await context.__actions["convex-add-repo"]({ dataset: { id: "joyful-jaguar-123", repo: "github.com/acme/shop" } });
  assert.deepEqual(plain(calls), [{ cmd: "set_convex_repos", id: "joyful-jaguar-123", repos: ["github.com/acme/site", "github.com/acme/shop"] }]);
  assert.doesNotMatch(context.convexCardHtml(), /Pinned found/);
});

test("Enter in the key field connects the picked project, and the redrawn card holds no key", async () => {
  const connected = { id: "quick-lemur-789", kind: "dev", url: "https://quick-lemur-789.convex.cloud", team: "acme", project: "shop", key_hint: "…7d3e", repos: ["github.com/acme/shop"] };
  const { context, calls, byId, listeners } = load({ connections: [SITE_CONNECTION], reply: () => connected });
  byId("convex-key").value = "  dev:quick-lemur-789|eyJ2c2VjcmV0dG9rZW4  ";
  byId("convex-project").value = "github.com/acme/shop";
  for (const fn of listeners.keydown) fn({ key: "Enter", target: { id: "convex-key", closest: () => null } });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(plain(calls), [{ cmd: "connect_convex", key: "dev:quick-lemur-789|eyJ2c2VjcmV0dG9rZW4", repos: ["github.com/acme/shop"] }]);
  assert.deepEqual(plain(context.__settings.convex.map((c) => c.id)), ["joyful-jaguar-123", "quick-lemur-789"]);
  const card = byId("convex").outerHTML;
  assert.match(card, /<b>shop<\/b> <code>quick-lemur-789<\/code>/);
  assert.match(card, /key …7d3e/);
  assert.doesNotMatch(card, /eyJ2c2VjcmV0dG9rZW4/);
  assert.doesNotMatch(card, /Pinned found/);
});

test("a refused key is reported under the form", async () => {
  const { context, byId } = load({
    reply: () => {
      throw "Convex refused the deploy key of quick-lemur-789.";
    },
  });
  byId("convex-key").value = "dev:quick-lemur-789|bad";
  await context.__actions["connect-convex"]();
  assert.equal(byId("convex-error").textContent, "Convex refused the deploy key of quick-lemur-789.");
});

test("a connection that serves a repo takes it off the connection that had it", async () => {
  const moved = { ...BILLING_CONNECTION, repos: ["github.com/acme/billing", "github.com/acme/site"] };
  const { context } = load({ connections: [SITE_CONNECTION, BILLING_CONNECTION], reply: () => moved });
  await context.__actions["convex-add-repo"]({ dataset: { id: "happy-otter-456", repo: "github.com/acme/site" } });
  assert.deepEqual(plain(context.__settings.convex.map((c) => [c.id, c.repos])), [
    ["joyful-jaguar-123", []],
    ["happy-otter-456", ["github.com/acme/billing", "github.com/acme/site"]],
  ]);
  assert.match(context.convexCardHtml(), /No project reads it yet\./);
});

test("Disconnect takes a second click", async () => {
  const { context, calls } = load({ connections: [SITE_CONNECTION], reply: () => null });
  const button = { dataset: { id: "joyful-jaguar-123" }, textContent: "Disconnect" };
  await context.__actions["disconnect-convex"](button);
  assert.deepEqual(plain(calls), []);
  assert.equal(button.textContent, "Click again to disconnect");
  await context.__actions["disconnect-convex"](button);
  assert.deepEqual(plain(calls), [{ cmd: "disconnect_convex", id: "joyful-jaguar-123" }]);
  assert.deepEqual(plain(context.__settings.convex), []);
});

test("while an agent's request to connect waits, the form keeps the project it asked for", () => {
  const { context } = load();
  context.agentAsking = (action) => (action === "connect_convex" ? { args: { repos: ["github.com/acme/billing"] } } : null);
  const html = context.convexCardHtml();
  assert.match(html, /<select id="convex-project" aria-label="Project" disabled><option value="">No project yet<\/option>/);
  assert.match(html, /<option value="github\.com\/acme\/billing" selected>billing · ~\/code\/billing<\/option>/);
});

/** A workflow draft whose graph is a trigger, then `step` when given, open in the editor. */
function editing(context, { repo = "", step = null } = {}) {
  const trigger = { id: "trigger-1", name: "Trigger", position: [0, 0], type: "trigger", watch: { kind: "called" } };
  const draft = { id: "w1", name: "Support triage", repo, folder: "", model: "", effort: "", permissionMode: "", columns: {}, graph: { nodes: step ? [trigger, step] : [trigger], edges: [] } };
  context.__setView({ name: "edit", draft, open: step?.id ?? null, graphPick: null });
  context.redraw = () => {};
  context.refresh = () => {};
  return draft;
}

const step = (fields = {}) => ({ id: "convex-data-1", name: "convex-data", position: [230, 0], type: "convex-data", connection: "joyful-jaguar-123", tables: null, ...fields });

test("the add-step menu offers Read Convex data with its own tile", () => {
  const { context } = load({ connections: [SITE_CONNECTION] });
  const draft = editing(context);
  context.__setView({ name: "edit", draft, open: null, graphPick: { mode: "append", from: "trigger-1", port: "out", x: 0, y: 0 } });
  const html = context.graphPickerHtml(draft, { x: 0, y: 0 }, 800);
  assert.match(html, /data-type="convex-data"><span class="icon" data-tone="convex" data-logo="convex" title="Convex: Read Convex data"><svg viewBox="0 0 24 24">/);
  assert.match(html, /<b>Read Convex data<\/b><small>Lets this run&#39;s session read the project&#39;s Convex data\.<\/small>/);
});

test("a new step reads the connection serving the workflow's repository, else the only one", () => {
  const pick = (connections, repo) => {
    const { context } = load({ connections });
    const draft = editing(context, { repo });
    context.__setView({ name: "edit", draft, open: null, graphPick: { mode: "append", from: "trigger-1", port: "out", x: 0, y: 0 } });
    context.applyGraphPick("convex-data");
    return draft.graph.nodes.find((n) => n.type === "convex-data").connection;
  };
  assert.equal(pick([SITE_CONNECTION, BILLING_CONNECTION], "git@github.com:Acme/billing.git"), "happy-otter-456");
  assert.equal(pick([SITE_CONNECTION, BILLING_CONNECTION], "https://github.com/acme/site"), "joyful-jaguar-123");
  assert.equal(pick([SITE_CONNECTION, BILLING_CONNECTION], "https://github.com/acme/other.git"), "");
  assert.equal(pick([BILLING_CONNECTION], ""), "happy-otter-456");
});

test("the step's ! names what blocks the run, and its summary what it reads", () => {
  const { context } = load({ connections: [SITE_CONNECTION] });
  const issues = (node) => plain(context.graphNodeIssues(node, editing(context, { step: node })));
  const summary = (node) => context.graphNodeSummary(node, editing(context, { step: node }));
  assert.deepEqual(issues(step()), []);
  assert.equal(summary(step()), "site · all tables");
  assert.deepEqual(issues(step({ tables: ["invoices", "customers"] })), []);
  assert.equal(summary(step({ tables: ["invoices", "customers"] })), "site · invoices, customers");
  assert.deepEqual(issues(step({ tables: [] })), ["Pick at least one table, or read all tables."]);
  assert.equal(summary(step({ tables: [] })), "site · pick tables");
  assert.deepEqual(issues(step({ connection: "quick-lemur-789" })), ["quick-lemur-789 is no longer connected. Choose another connection."]);
  assert.equal(summary(step({ connection: "quick-lemur-789" })), "Missing connection");
  assert.deepEqual(issues(step({ connection: "" })), ["Choose the Convex connection this run reads."]);
  context.__settings.convex = [];
  assert.deepEqual(issues(step()), ["Connect Convex under Connectors."]);
  assert.equal(summary(step()), "Connect Convex");
});

test("with no connection, the step's panel offers the Convex card", () => {
  const { context } = load();
  const draft = editing(context, { step: step({ connection: "" }) });
  const html = context.graphNodePanel(draft);
  assert.match(html, /<span class="app">Convex<\/span><b>convex-data<\/b>/);
  assert.match(html, /data-action="convex-open-card">Connect Convex/);
  assert.doesNotMatch(html, /data-convex-connection/);
});

test("the panel shows the connection, Only these, and the picked tables ticked in the list", () => {
  const { context, calls } = load({ connections: [SITE_CONNECTION, BILLING_CONNECTION] });
  context.__ui.tables["joyful-jaguar-123"] = ["invoices", "customers", "events"];
  const html = context.graphNodePanel(editing(context, { step: step({ tables: ["invoices"] }) }));
  assert.match(html, /<select data-convex-connection><option value="">Choose a connection<\/option><option value="joyful-jaguar-123" selected>site · joyful-jaguar-123<\/option><option value="happy-otter-456">billing · happy-otter-456<\/option><\/select>/);
  assert.match(html, /class="chip" data-action="convex-all-tables">All tables<\/button><button type="button" class="chip active" data-action="convex-only-tables">Only these/);
  assert.match(html, /<span class="chip convex-table" title="invoices">invoices<button type="button" data-action="convex-remove-table" data-table="invoices"/);
  assert.match(html, /data-convex-table="invoices" checked> <code>invoices<\/code>/);
  assert.match(html, /data-convex-table="customers"> <code>customers<\/code>/);
  assert.match(html, /With a table list, queries and logs are off for this run\./);
  assert.deepEqual(calls, [], "known names are not fetched again");
  const all = context.graphNodePanel(editing(context, { step: step() }));
  assert.match(all, /class="chip active" data-action="convex-all-tables"/);
  assert.doesNotMatch(all, /convex-step-tables/);
  assert.match(all, /<p class="hint">All tables also lets this run execute queries and read logs\. A query can read the deployment's environment variables by name\. A table list turns both off\.<\/p>/);
  assert.doesNotMatch(html, /environment variables/);
});

test("with 128 tables the filter narrows the list and picked tables stay as chips", () => {
  const names = Array.from({ length: 128 }, (_, i) => `table_${String(i + 1).padStart(3, "0")}`);
  const { context, byId, fire } = load({ connections: [SITE_CONNECTION] });
  context.__ui.tables["joyful-jaguar-123"] = names;
  const node = step({ tables: ["table_007", "table_100"] });
  const html = context.graphNodePanel(editing(context, { step: node }));
  assert.equal([...html.matchAll(/data-convex-table="/g)].length, 128);
  assert.match(html, /<input type="search" data-convex-filter value="" placeholder="Filter 128 tables"/);
  assert.match(html, /<div class="chips" id="convex-chosen"><span class="chip convex-table" title="table_007">table_007<button[^>]*data-table="table_007"[^>]*>×<\/button><\/span><span class="chip convex-table" title="table_100">table_100<button/);

  fire("input", { dataset: { convexFilter: "" }, value: "10" });
  const listed = (html) => [...html.matchAll(/data-convex-table="([^"]+)"( checked)?/g)].map(([, name, on]) => (on ? `${name} ✓` : name));
  assert.deepEqual(listed(byId("convex-table-list").innerHTML), ["table_010", "table_100 ✓", "table_101", "table_102", "table_103", "table_104", "table_105", "table_106", "table_107", "table_108", "table_109", "table_110"]);

  fire("change", { dataset: { convexTable: "table_105" }, checked: true });
  assert.deepEqual(plain(node.tables), ["table_007", "table_100", "table_105"]);
  assert.match(byId("convex-chosen").innerHTML, /title="table_105">table_105/);

  context.__actions["convex-remove-table"]({ dataset: { table: "table_100" } });
  assert.deepEqual(plain(node.tables), ["table_007", "table_105"]);
  assert.doesNotMatch(byId("convex-chosen").innerHTML, /table_100/);
  assert.ok(listed(byId("convex-table-list").innerHTML).includes("table_100"), "the filter still shows it, now unticked");

  fire("input", { dataset: { convexFilter: "" }, value: "nope" });
  assert.equal(byId("convex-table-list").innerHTML, "<p class=\"hint\">No table matches “nope”.</p>");
});

test("the panel fetches the names once, and types them when the fetch fails", async () => {
  const { context, calls, byId, fire } = load({
    connections: [SITE_CONNECTION],
    reply: () => {
      throw "the hub is busy";
    },
  });
  const node = step({ tables: ["invoices", "customers"] });
  const html = context.graphNodePanel(editing(context, { step: node }));
  assert.match(html, /<div id="convex-step-tables"><p class="hint">Listing the tables…<\/p><\/div>/);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(plain(calls), [{ cmd: "convex_tables", id: "joyful-jaguar-123" }]);
  const box = byId("convex-step-tables").innerHTML;
  assert.match(box, /<input data-convex-tables-text value="invoices, customers" placeholder="invoices, customers"/);
  assert.match(box, /Pinned could not list the tables\. Type their names, separated by commas\./);

  fire("input", { dataset: { convexTablesText: "" }, value: "invoices, , events " });
  assert.deepEqual(plain(node.tables), ["invoices", "events"]);
  context.graphNodePanel(editing(context, { step: node }));
  assert.equal(calls.length, 1, "a failed fetch is not retried on every draw");
});

const thread = (path, machine = "local") => ({ key: `${machine}:t1`, machine, view: { cwd: path } });

test("a thread whose project names an unconnected deployment offers to connect it", () => {
  const { context } = load({ connections: [SITE_CONNECTION] });
  const banner = context.convexBannerFor(thread(SHOP.path));
  assert.deepEqual(plain(banner), { repo: "github.com/acme/shop", deployment: "quick-lemur-789" });
  const html = context.convexBannerHtml(banner);
  assert.match(html, /<b>This project uses Convex\.<\/b>\s*<span>Connect quick-lemur-789 so agents here can read its data\.<\/span>/);
  assert.match(html, /<button type="button" class="primary" data-convex-connect="github\.com\/acme\/shop">Connect Convex<\/button>/);
  assert.match(html, /data-dismiss-convex="github\.com\/acme\/shop" aria-label="Dismiss"/);
});

test("no banner once the repo is served, dismissed, signed out, or without Convex", () => {
  const { context } = load({ connections: [SITE_CONNECTION] });
  assert.equal(context.convexBannerFor(thread(SITE.path)), null, "served");
  assert.equal(context.convexBannerFor(thread(BILLING.path)), null, "no Convex in its dotenv");
  assert.equal(context.convexBannerFor(thread("/Users/ana/elsewhere")), null, "not a listed project");
  assert.ok(context.convexBannerFor(thread(SHOP.path)));
  context.__dismissed.add("github.com/acme/shop");
  assert.equal(context.convexBannerFor(thread(SHOP.path)), null, "dismissed");
  const signedOut = load({ signedIn: false });
  assert.equal(signedOut.context.convexBannerFor(thread(SHOP.path)), null, "signed out");
});

test("Connect on the banner opens Connectors with that project picked", async () => {
  const admin = { path: "/Users/ana/code/admin", name: "admin", repo: "github.com/acme/admin", convex: "prod:brave-fox-321" };
  const { context, byId } = load({ projects: [SHOP, admin] });
  const focused = [];
  byId("convex-key").focus = () => focused.push("convex-key");
  await context.convexOpenCard("github.com/acme/admin");
  assert.deepEqual(plain(context.opened), ["integrations"]);
  assert.deepEqual(focused, ["convex-key"]);
  const html = context.convexCardHtml();
  assert.match(html, /<option value="github\.com\/acme\/admin" selected>admin · ~\/code\/admin<\/option>/);
  assert.match(html, /placeholder="prod:brave-fox-321\|…"/);
});
