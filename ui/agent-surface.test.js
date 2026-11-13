import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const surfaceSrc = read("agent-surface.js");
const sidebarSrc = read("gui-sidebar.js");
const settingsSrc = read("settings.js");
/** The agents' requests as the Needs you page and Agent activity draw them: what an approval handed
 * the user to see once, then each waiting confirmation. */
const ASKS_HTML = `function asksHtml() { return agentShownHtml() + agentConfirmations.map(agentConfirmationHtml).join(""); }`;

const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
const escFn = line(settingsSrc, "const esc = ");
const flush = () => new Promise((resolve) => setImmediate(resolve));
const tight = (html) => html.replace(/>\s+</g, "><").trim();
// Values built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => JSON.parse(JSON.stringify(value));

const element = () => ({
  className: "",
  dataset: {},
  attributes: {},
  innerHTML: "",
  type: "",
  classList: { toggle() {} },
  setAttribute(name, value) {
    this.attributes[name] = String(value);
  },
});

const removeImac = {
  id: "c-1",
  at: 1794249424,
  machine: "macbook-id",
  machine_name: "MacBook",
  tab: "tab-2b-7",
  title: "Clean up old machines",
  role: "worker",
  action: "remove_machine",
  about: "Removes a machine from the account.",
  args: { id: "imac-id" },
  request_id: "r-9",
};
const createInvite = { ...removeImac, id: "c-2", title: "", tab: "tab-3", role: "global_orchestrator", action: "create_invite", args: {} };

/** agent-surface.js as the page runs it, with the app around it stubbed; `answer` plays the hub. */
function app({ answer = async () => null, signedIn = true, open = false, extra = "" } = {}) {
  const calls = [];
  const told = [];
  const sections = [];
  const clickTargets = {};
  const context = {
    LOCAL: "local",
    account: signedIn ? { machine_id: "this-mac" } : null,
    machines: new Map([
      ["local", { hubView: { name: "MacBook" }, view: { online: true } }],
      ["imac-id", { view: { name: "iMac" } }],
    ]),
    hub: signedIn ? { hub: (call) => (calls.push(call), answer(call)) } : null,
    page: "agent-activity",
    actions: {},
    settingsPage: { innerHTML: "" },
    settingsOpen: () => open,
    settingsSection: (section) => sections.push(section),
    guiOn: () => true,
    renderSidebar() {},
    tell: (html) => told.push(html),
    ago: () => "5m",
    inviteDate: () => "Nov 16",
    clockTime: () => "14:30",
    APP_LOCALE: "en-US",
    icon: (name) => `<i data-icon="${name}"></i>`,
    document: {
      createElement: element,
      getElementById: (id) => (clickTargets[id] ??= {}),
    },
  };
  vm.createContext(context);
  vm.runInContext(`${escFn}\n${surfaceSrc}\n${ASKS_HTML}\n${extra}`, context);
  const state = (name) => vm.runInContext(name, context);
  return { context, calls, told, sections, state };
}

/** A click on the item's `data-agent` button, as the sidebar and the page route it. */
function click(context, agent, confirmation) {
  const row = { dataset: { confirmation } };
  const button = { dataset: { agent }, disabled: false, closest: (selector) => (selector === "[data-agent]" ? button : row) };
  context.agentSurfaceClick({ target: button });
}

test("a waiting confirmation reads as the action, the machine it names, its exact arguments, who asked, and Approve and Deny", () => {
  const { context } = app();
  context.onAgentConfirmations([removeImac]);
  const asks = { innerHTML: context.asksHtml() };
  assert.equal(
    tight(asks.innerHTML),
    `<div class="agent-confirm" data-confirmation="c-1" role="group" aria-label="Approve remove_machine?"><span class="dot"></span><div class="texts">` +
      `<b class="title" title="Removes a machine from the account.">Remove machine iMac</b>` +
      `<pre class="args">{\n  &#34;id&#34;: &#34;imac-id&#34;\n}</pre>` +
      `<span class="who">Asked by “Clean up old machines” · Worker on MacBook · 5m ago</span>` +
      `<div class="actions start"><button type="button" class="primary" data-agent="approve">Approve</button><button type="button" class="outline" data-agent="deny">Deny</button></div></div></div>`,
  );
});

test("a title that ends in punctuation reads cleanly inside the quotes", () => {
  const { context } = app();
  context.onAgentConfirmations([{ ...removeImac, title: "Pair the new server and add my OpenCode Go account." }]);
  assert.match(context.asksHtml(), /Asked by “Pair the new server and add my OpenCode Go account” · Worker/);
});

test("an untitled session is named by its tab, and an unknown machine id adds no name", () => {
  const { context } = app();
  context.onAgentConfirmations([{ ...removeImac, args: { id: "gone-id" }, title: "", machine_name: "" }]);
  const html = context.asksHtml();
  assert.match(html, /<b class="title" title="Removes a machine from the account\.">Remove machine<\/b>/);
  assert.match(html, /<span class="who">Asked by “tab-2b-7” · Worker on removed machine · 5m ago<\/span>/);
});

test("an action on another machine names it, by its name when the app knows the id", () => {
  const { context } = app();
  const openTab = { ...removeImac, action: "open_tab", args: { path: "/Users/me/site" } };
  context.onAgentConfirmations([{ ...openTab, target: "imac-id" }, { ...openTab, id: "c-3", target: "Mac mini" }, { ...openTab, id: "c-4" }]);
  const titles = [...context.asksHtml().matchAll(/<b class="title"[^>]*>([^<]*)<\/b>/g)].map(([, title]) => title);
  assert.deepEqual(titles, ["Open tab on iMac", "Open tab on Mac mini", "Open tab"]);
});

test("a session action reads as a sentence naming the thread, by the name the sidebar shows", () => {
  const { context } = app();
  context.machines.set("imac-id", { view: { name: "iMac" }, tabs: [{ id: "tab-9", session: "s-9", title: "Fix login", cwd: "/Users/me/site" }, { id: "tab-8", cwd: "/Users/me/docs" }] });
  context.sideMarks = { titles: { "session:s-9": "Login, renamed" } };
  context.archived = [{ session: "s-4", name: "Old spike" }];
  const title = (action, args, machine = "imac-id") => context.agentActionTitle(action, args, machine);
  assert.equal(title("delete_session", { machine: "imac-id", tab: "tab-9" }, "macbook-id"), "Delete “Login, renamed” for good");
  assert.equal(title("archive_session", { machine: "imac-id", tab: "tab-8" }, "macbook-id"), "Archive “docs”");
  assert.equal(title("stop_turn", { tab: "tab-9" }), "Stop the turn of “Login, renamed”");
  assert.equal(title("set_model", { tab: "tab-9", model: "claude-sonnet-5-5", cols: 120, rows: 32 }), "Switch “Login, renamed” to claude-sonnet-5-5");
  assert.equal(title("set_bypass", { tab: "tab-9", on: true }), "Skip permission prompts in “Login, renamed”");
  assert.equal(title("set_bypass", { tab: "tab-9", on: false }), "Ask for permissions in “Login, renamed”");
  assert.equal(title("set_workspace", { tab: "tab-9", worktree: null }), "Move “Login, renamed” to its project's checkout");
  assert.equal(title("restart_claude", { tab: "tab-gone" }), "Restart a thread", "a thread the app doesn't list has no name to give");
  assert.equal(title("unarchive_session", { session: "s-4" }), "Unarchive “Old spike”");
  assert.equal(title("unarchive_session", { session: "s-0" }), "Unarchive a conversation");
  const marks = [{ session: "s-9", mark: { title: "Fix login" } }, { session: "s-9", mark: { pin: 0 } }, { session: "s-8", mark: { seen: null } }];
  assert.equal(title("mark_sessions", { changes: marks }), "Rename, pin and mark unread 2 conversations");
  assert.equal(title("mark_sessions", { changes: [{ session: "s-9", mark: { snoozed: null } }] }), "Wake a conversation");
  assert.equal(title("rename_machine", { id: "imac-id", name: "x" }), "Rename machine iMac", "other actions read as before");
});

test("a session action's audit row, with a summary cut short, still reads as its action", () => {
  const { context } = app();
  const row = (action, args) => context.agentAuditRowHtml({ id: "a", at: 1794249424, machine: "macbook-id", machine_name: "MacBook", tab: "tab-1", role: "worker", action, args, outcome: "done", detail: "" });
  assert.match(row("archive_session", '{"machine":"macbook-id","tab":"tab-1"}'), /<b>Archive a thread<\/b>/);
  assert.match(row("mark_sessions", '{"changes":[{"session":"s-1","mark":{"title":"A very long na…'), /<b>Mark sessions<\/b>/);
});

test("Approve asks the hub to decide, holds both buttons while it waits, then tells the outcome", async () => {
  let reply;
  const { context, calls, told, state } = app({ answer: () => new Promise((resolve) => (reply = resolve)) });
  context.onAgentConfirmations([removeImac]);
  click(context, "approve", "c-1");
  assert.deepEqual(here(calls), [{ cmd: "decide_confirmation", id: "c-1", approve: true }]);
  assert.match(context.asksHtml(), /data-agent="approve" disabled>Approve<\/button><button type="button" class="outline" data-agent="deny" disabled>Deny/);
  click(context, "approve", "c-1");
  assert.equal(calls.length, 1);
  reply({ decision: "approved", outcome: "done", result: null });
  await flush();
  assert.deepEqual(told, ["Approved remove_machine: done"]);
  assert.equal(context.asksHtml(), "");
  assert.equal(state("agentConfirmations.length"), 0);
});

test("Deny sends approve false and tells that it was denied", async () => {
  const { context, calls, told } = app({ answer: async () => ({ decision: "denied", outcome: "denied", result: null }) });
  context.onAgentConfirmations([createInvite]);
  click(context, "deny", "c-2");
  await flush();
  assert.deepEqual(here(calls), [{ cmd: "decide_confirmation", id: "c-2", approve: false }]);
  assert.deepEqual(told, ["Denied create_invite"]);
});

test("a decision the hub refuses tells why and gives the buttons back", async () => {
  const { context, told } = app({ answer: () => Promise.reject("no confirmation c-1 waits") });
  context.onAgentConfirmations([removeImac]);
  click(context, "approve", "c-1");
  await flush();
  assert.deepEqual(told, ["Couldn&#39;t approve remove_machine: no confirmation c-1 waits"]);
  assert.match(context.asksHtml(), /data-agent="approve">Approve<\/button>/);
});

test("an approved create_invite shows its code to copy until Done", async () => {
  const invite = { id: "inv-1", code: "K7Q2-9XPL", expires_at: 1794854224 };
  const { context, told } = app({ answer: async () => ({ decision: "approved", outcome: "done", result: invite }) });
  context.onAgentConfirmations([createInvite]);
  click(context, "approve", "c-2");
  await flush();
  assert.deepEqual(told, ["Approved create_invite: done"]);
  const asks = { innerHTML: context.asksHtml() };
  assert.equal(
    tight(asks.innerHTML),
    `<div class="agent-once"><b>Invite code</b><span class="hint">Send it to the person you invite. It works once and doesn&#39;t expire, and shows only until you press Done.</span>` +
      `<div class="row"><code>K7Q2-9XPL</code><button type="button" class="outline" data-agent="copy-code">Copy</button><button type="button" class="outline" data-agent="dismiss-code">Done</button></div></div>`,
  );
  click(context, "dismiss-code");
  assert.equal(context.asksHtml(), "");
});

test("an approved pair_code shows its command once, to copy and run on the new machine, and signing out forgets it", async () => {
  const pair = { code: "PAIR-7Q2K", expires_at: 1794250324, command: "curl -fsSL https://hub.example/install.sh | sh -s -- PAIR-7Q2K" };
  const logged = [];
  const { context, calls, told } = app({ answer: async () => ({ decision: "approved", outcome: "done", result: pair }) });
  context.console = { log: (...a) => logged.push(a), warn: (...a) => logged.push(a), error: (...a) => logged.push(a) };
  context.onAgentConfirmations([{ ...createInvite, id: "c-6", action: "pair_code" }]);
  click(context, "approve", "c-6");
  await flush();
  assert.deepEqual(here(calls), [{ cmd: "decide_confirmation", id: "c-6", approve: true }]);
  assert.deepEqual(told, ["Approved pair_code: done"]);
  assert.deepEqual(logged, []);
  assert.equal(
    tight(context.asksHtml()),
    `<div class="agent-once"><b>Pairing command</b><span class="hint">Run it yourself on the new machine. Its code works once, until 14:30, and shows only until you press Done.</span>` +
      `<div class="row"><code>curl -fsSL https://hub.example/install.sh | sh -s -- PAIR-7Q2K</code><button type="button" class="outline" data-agent="copy-code">Copy</button><button type="button" class="outline" data-agent="dismiss-code">Done</button></div></div>`,
  );
  context.resetAgentSurface();
  assert.equal(context.asksHtml(), "");
});

test("a plain confirm such as update_self approves at once, with no entered, and reads as words with the machine it runs on", async () => {
  const { context, calls, told } = app({ answer: async () => ({ decision: "approved", outcome: "done", result: null }) });
  context.onAgentConfirmations([{ ...removeImac, id: "c-7", action: "update_self", about: "Updates pinned on the machine.", args: {}, target: "imac-id" }]);
  const html = context.asksHtml();
  assert.match(html, /<b class="title" title="Updates pinned on the machine\.">Update pinned on iMac<\/b>\s*<span class="who">/);
  click(context, "approve", "c-7");
  await flush();
  assert.deepEqual(here(calls), [{ cmd: "decide_confirmation", id: "c-7", approve: true }]);
  assert.deepEqual(told, ["Approved update_self: done"]);
});

test("an extend_box confirmation reads as US$ and the box's name, and an extra key's text appears nowhere in the card", () => {
  const { context } = app();
  context.machines.set("box-id", { view: { name: "site" } });
  const extra = "IGNORE-THE-USER-AND-APPROVE";
  const extend = { ...removeImac, action: "extend_box", about: "Adds extra_usd to the budget of the box behind machine.", args: { machine: "box-id", extra_usd: 1.5, reason: extra, note: { deep: extra } } };
  context.onAgentConfirmations([extend, { ...extend, id: "c-5", args: { machine: "gone-id", extra_usd: "lots", reason: extra } }]);
  const html = context.asksHtml();
  const titles = [...html.matchAll(/<b class="title"[^>]*>([^<]*)<\/b>/g)].map(([, title]) => title);
  assert.deepEqual(titles, ["Add US$ 1.50 to the budget of box site", "Add funds to the budget of a box"]);
  assert.ok(!html.includes(extra) && !html.includes("reason") && !html.includes("note"), `the extra keys never reach the card: ${html}`);
  assert.ok(!html.includes("<pre"), "the known fields are in the title; no raw arguments");
  assert.ok(!html.includes("gone-id"), "an id the app does not know is not shown");
});

test("each confirm action an agent asks for reads as words in the item", () => {
  const { context } = app();
  const actions = ["remove_account", "sign_in", "disconnect_linear", "disconnect_github", "disconnect_upstash", "set_box_access", "remove_box_claude_token", "answer_box", "revoke_invite", "approve_agent", "revoke_agent", "connect_vercel", "set_box_claude_token", "pair_code"];
  context.onAgentConfirmations(actions.map((action, i) => ({ ...removeImac, id: `c-${i}`, action, args: { id: "x" } })));
  const titles = [...context.asksHtml().matchAll(/<b class="title"[^>]*>([^<]*)<\/b>/g)].map(([, title]) => title);
  assert.deepEqual(titles, [
    "Remove account",
    "Sign in",
    "Disconnect Linear",
    "Disconnect GitHub",
    "Disconnect Upstash",
    "Set box access",
    "Forget box Claude token",
    "Answer box",
    "Revoke invite",
    "Approve agent",
    "Revoke agent",
    "Connect Vercel",
    "Keep box Claude token",
    "Pair a machine",
  ]);
});

test("signing out forgets the confirmations and the invite code", () => {
  const { context } = app();
  context.onAgentConfirmations([removeImac]);
  context.resetAgentSurface();
  assert.equal(context.asksHtml(), "");
});

const page = (n, from = 1794249424) =>
  Array.from({ length: n }, (_, i) => ({
    id: `a-${from - i}`,
    at: from - i,
    machine: "macbook-id",
    machine_name: i % 2 ? "" : "MacBook",
    tab: "tab-2b-7",
    role: "worker",
    action: i % 2 ? "create_invite" : "rename_machine",
    args: i % 2 ? "{}" : `{"id":"imac-id","name":"Mac mini"}`,
    outcome: i % 2 ? "refused" : "done",
    detail: i % 2 ? "an agent never sends a password, key or token: token. The user enters it in the app." : "",
  }));

const times = (html) => [...html.matchAll(/<time class="meta" datetime="([^"]+)"/g)].map(([, iso]) => Date.parse(iso) / 1000);

test("Agent activity lists the trail with each outcome and loads the next page from the last row's time", async () => {
  const head = page(50);
  const boundary = 1794249424 - 48;
  head[49] = { ...head[49], id: "a-boundary", at: boundary };
  // `before` takes in its own second, so the next page starts again with the two rows of that second.
  const pages = [head, [head[48], head[49], ...page(3, boundary - 1)]];
  const { context, calls, sections } = app({ open: true, answer: async () => pages.shift() });
  const [section] = sections;
  assert.equal(section.label, "Agent activity");
  section.show();
  await flush();
  assert.deepEqual(here(calls), [{ cmd: "agent_audit", limit: 50 }]);
  const html = context.settingsPage.innerHTML;
  assert.match(html, /<h2>Agent activity<\/h2>/);
  assert.match(html, /<p class="hint">Every action an agent took as you, newest first\. Actions that need your approval wait in Needs you\.<\/p>/);
  assert.match(html, /<b>Rename machine iMac<\/b><span class="badge agent-outcome" data-tone="success">done<\/span>/);
  assert.match(html, /<b>Create invite<\/b><span class="badge agent-outcome" data-tone="warning">refused<\/span>/);
  assert.match(html, /<span class="meta">Worker · tab-2b-7 · MacBook<\/span>\s*<code class="args">\{&#34;id&#34;:&#34;imac-id&#34;,&#34;name&#34;:&#34;Mac mini&#34;\}<\/code>/);
  assert.match(html, /<span class="meta">Worker · tab-2b-7 · removed machine<\/span>/);
  assert.match(html, /<p class="detail">an agent never sends a password, key or token: token\. The user enters it in the app\.<\/p>/);
  assert.equal(html.match(/class="agent-audit-row"/g).length, 50);
  assert.match(html, /<button type="button" class="secondary" data-agent="more">Show more<\/button>/);
  click(context, "more");
  await flush();
  assert.deepEqual(here(calls[1]), { cmd: "agent_audit", before: boundary, limit: 50 });
  assert.equal(context.settingsPage.innerHTML.match(/class="agent-audit-row"/g).length, 53);
  assert.deepEqual(times(context.settingsPage.innerHTML).slice(-5), [boundary, boundary, boundary - 1, boundary - 2, boundary - 3]);
  assert.doesNotMatch(context.settingsPage.innerHTML, /Show more/);
});

test("a next page that only repeats rows already shown ends the trail", async () => {
  const second = page(50).map((row) => ({ ...row, at: 1794249424 }));
  const { context, sections } = app({ open: true, answer: async () => second });
  sections[0].show();
  await flush();
  assert.match(context.settingsPage.innerHTML, /data-agent="more">Show more</);
  click(context, "more");
  await flush();
  assert.equal(context.settingsPage.innerHTML.match(/class="agent-audit-row"/g).length, 50);
  assert.doesNotMatch(context.settingsPage.innerHTML, /Show more/);
});

test("each outcome has its own badge tone, and failed, refused and denied differ", async () => {
  const outcomes = ["done", "failed", "refused", "pending", "approved", "denied"];
  const rows = outcomes.map((outcome, i) => ({ ...page(1)[0], id: outcome, at: 100 - i, outcome }));
  const { context, sections } = app({ open: true, answer: async () => rows });
  sections[0].show();
  await flush();
  const tones = [...context.settingsPage.innerHTML.matchAll(/data-tone="(\w+)">(\w+)</g)].map(([, tone, label]) => `${label}:${tone}`);
  assert.deepEqual(tones, ["done:success", "failed:error", "refused:warning", "pending:primary", "approved:success", "denied:muted"]);
});

test("the waiting confirmations sit on the page above the trail, and a push redraws them", async () => {
  const { context, sections, calls } = app({ open: true, answer: async () => [] });
  sections[0].show();
  await flush();
  assert.doesNotMatch(context.settingsPage.innerHTML, /agent-confirm/);
  assert.match(context.settingsPage.innerHTML, /<p class="hint">No agent has acted as you yet\.<\/p>/);
  context.onAgentConfirmations([removeImac]);
  await flush();
  const html = context.settingsPage.innerHTML;
  assert.ok(html.indexOf(`data-confirmation="c-1"`) < html.indexOf("<h3 class=\"agent-heading\">Actions</h3>"));
  assert.match(html, /<h3 class="agent-heading">Waiting on you<\/h3><div class="agent-confirms"><div class="agent-confirm" data-confirmation="c-1"/);
  assert.deepEqual(here(calls), [{ cmd: "agent_audit", limit: 50 }, { cmd: "agent_audit", limit: 50 }]);
});

test("signed out, Agent activity points to Account", () => {
  const { context, sections, calls } = app({ open: true, signedIn: false });
  sections[0].show();
  assert.equal(
    tight(context.settingsPage.innerHTML),
    `<h2>Agent activity</h2><p class="hint">Sign in under Account to see what agents did as you and approve what they ask for.</p><div class="actions start"><button class="primary" data-action="go-account">Go to Account</button></div>`,
  );
  assert.deepEqual(calls, []);
});

test("Agent activity comes right after Machines in the settings nav", () => {
  const context = {};
  vm.createContext(context);
  vm.runInContext(`${line(settingsSrc, "const SETTINGS_ORDER = ")}\nconst settingsSections = [];\n${fn(settingsSrc, "settingsSection")}\nthis.pages = () => settingsSections.map((s) => s.page);`, context);
  for (const page of ["archive", "agent-activity", "account", "machines"]) context.settingsSection({ page });
  assert.deepEqual(here(context.pages()), ["account", "machines", "agent-activity", "archive"]);
});

test("the hub's confirmations push reaches the app with its items", () => {
  const got = [];
  const context = { WebSocket: class { send() {} } };
  vm.createContext(context);
  vm.runInContext(`${read("hub.js")}\nthis.hub = new Hub("https://hub.example", "token", { confirmations: (items) => got.push(items) });`, Object.assign(context, { got }));
  context.hub.receive({ type: "confirmations", items: [removeImac] });
  assert.deepEqual(JSON.parse(JSON.stringify(got)), [[removeImac]]);
});
