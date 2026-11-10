import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

/** Runs agents-mcp.js against the few globals of main.js and settings.js it reads. */
function load({ framed = false, signedIn = true } = {}) {
  const window = {};
  window.self = window;
  window.top = framed ? {} : window;
  const context = {
    console,
    URL,
    TAURI: true,
    APP_LOCALE: "en-US",
    window,
    location: { pathname: "/", search: "" },
    sessionStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    history: { replaceState() {} },
    account: signedIn ? { email: "ana@acme.dev", machine_id: "m-local" } : null,
    hub: signedIn ? {} : null,
    machines: new Map([["m1", { view: { name: "mac", online: true } }]]),
    machineName: (id) => (id === "m1" ? "mac" : "machine"),
    localIdOf: (id) => id,
    hubIdOf: (id) => id,
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    actions: {},
  };
  context.globalThis = context;
  const source = readFileSync(new URL("./agents-mcp.js", import.meta.url), "utf8");
  vm.runInNewContext(`${source}\nglobalThis.__ui = agentsUi;\nglobalThis.__consent = agentConsentHtml;\nglobalThis.__card = externalAgentsCardHtml;`, context);
  return context;
}

const pending = (redirect = "https://grok.example/oauth/callback") => ({
  request: "r1",
  view: { request: "r1", client: "<b>Grok</b>", redirect_uri: redirect, scopes: ["orchestrator"], expires_at: 1792400000 },
  projects: [{ machine: "m1", machineName: "mac", path: "/w/app", name: "app" }],
  error: "",
});

test("the consent card names the client and its redirect, and offers each project and scope", () => {
  const ui = load();
  ui.__ui.consent = pending();
  const html = ui.__consent();
  assert.match(html, /Connect “&#60;b&#62;Grok&#60;\/b&#62;”\?/, "the client's own name is escaped");
  assert.match(html, /<code>https:\/\/grok\.example\/oauth\/callback<\/code>/);
  assert.match(html, /name="agent-scope" value="orchestrator" checked/);
  assert.match(html, /name="agent-scope" value="sessions">/, "a scope the client didn't ask for starts unticked");
  assert.match(html, /name="agent-project" value="0"> <b>app<\/b> <span class="hint">mac · \/w\/app/);
  assert.doesNotMatch(html, /name="agent-project" value="0" checked/, "no project is picked for the user");
  assert.match(html, /data-action="approve-agent">Approve/);
  assert.doesNotMatch(html, /an app on this device/);
});

test("a framed page can't approve", () => {
  const ui = load({ framed: true });
  ui.__ui.consent = pending();
  const html = ui.__consent();
  assert.match(html, /data-action="approve-agent" disabled/);
  assert.match(html, /inside another page/);
});

test("an app scheme redirect is said to go to an app", () => {
  const ui = load();
  ui.__ui.consent = pending("cursor://anysphere.cursor-mcp/oauth/callback");
  assert.match(ui.__consent(), /an app on this device/);
});

test("the Connectors card shows the endpoint, warns off a non-public hub and lists agents to revoke", () => {
  const ui = load();
  ui.__ui.endpoint = "http://127.0.0.1:8797/mcp";
  ui.__ui.list = [{ id: "g1", name: "Grok", redirect: "https://grok.example/cb", scopes: ["sessions"], projects: [{ machine: "m1", path: "/w/app", name: "app" }], created_at: 1, last_used: null }];
  const html = ui.__card();
  assert.match(html, /id="mcp-endpoint" readonly value="http:\/\/127\.0\.0\.1:8797\/mcp"/);
  assert.match(html, /not public HTTPS/);
  assert.match(html, /<span class="badge" data-on="true">1 connected<\/span>/);
  assert.match(html, /via grok\.example/);
  assert.match(html, /sessions · app on mac/);
  assert.match(html, /data-action="revoke-agent" data-agent="g1"/);
  ui.__ui.endpoint = "https://hub.example/mcp";
  assert.doesNotMatch(ui.__card(), /not public HTTPS/);
});

test("a signed-out app only says to sign in", () => {
  const html = load({ signedIn: false }).__card();
  assert.match(html, /Sign in to pinned to let external agents in/);
  assert.doesNotMatch(html, /mcp-endpoint/);
});

test("no top-level name of agents-mcp.js is declared by another page script", async () => {
  const { readdirSync } = await import("node:fs");
  const declared = (source) => new Set([...source.matchAll(/^(?:const|let|var|class|function|async function)\s+([A-Za-z_$][\w$]*)/gm)].map((m) => m[1]));
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  const scripts = [...html.matchAll(/<script src="([^"]+\.js)"><\/script>/g)].map((m) => m[1]).filter((s) => !s.startsWith("vendor/") && s !== "agents-mcp.js");
  assert.ok(scripts.includes("settings.js") && readdirSync(new URL(".", import.meta.url)).includes("agents-mcp.js"));
  const ours = declared(readFileSync(new URL("./agents-mcp.js", import.meta.url), "utf8"));
  for (const script of scripts) {
    const theirs = declared(readFileSync(new URL(`./${script}`, import.meta.url), "utf8"));
    const clash = [...ours].filter((name) => theirs.has(name));
    assert.deepEqual(clash, [], `${script} declares ${clash.join(", ")} too, which stops one of the two scripts from loading`);
  }
});
