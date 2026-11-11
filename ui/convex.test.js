import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

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
    document: {
      body: { dataset: {} },
      getElementById: byId,
      querySelector: () => element(),
      querySelectorAll: () => [],
    },
  };
  context.globalThis = context;
  vm.runInNewContext(
    `${settingsSrc}\n${convexSrc}\nglobalThis.__settings = settings; globalThis.__actions = actions; globalThis.__sections = settingsSections; globalThis.__ui = convexUi;`,
    context,
  );
  context.__settings.convex = connections;
  return { context, calls, byId, listeners: byId("settings-page").listeners };
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
    assert.match(html, new RegExp(`<li><code>${permission}</code></li>`));
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
