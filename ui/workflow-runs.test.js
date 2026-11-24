import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const line = (src, start) => {
  const at = src.indexOf(start);
  assert.ok(at >= 0, `${start} is in the source`);
  return src.slice(at, src.indexOf("\n", at));
};
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
// The page's own escape, as settings.js writes it, for the text a test expects to find.
const esc = (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);

const AT = "2026-11-20T10:00:00Z";
const LATER = "2026-11-20T10:05:00Z";
const step = (node, name, what, workflow = "c1") => ({ at: LATER, workflow, node, name, what });

const issueTrigger = { kind: "issue", id: "uuid-7", identifier: "ENG-7", title: "Export fails", description: "500 on CSV", url: "https://linear.app/x/issue/ENG-7" };
const running = {
  id: "c1:uuid-7:1", workflow: "c1", workflow_name: "Resolve", subject: { issue: "uuid-7" }, status: "running",
  started_at: AT, last_step_at: AT, trigger: issueTrigger, classified: [], session: { state: "none" },
  steps: [{ ...step("t", "Trigger", issueTrigger), at: AT }], waiting_on: { workflow: "c1", node: "sort", name: "Sort" },
};
const slackTrigger = { kind: "slack", channel: "C1", channelName: "alerts", thread: "1.2", posts: [{ user: "ana", text: "checkout is down" }] };
const waiting = {
  id: "c2:C1:1.2:1", workflow: "c2", workflow_name: "Triage Slack", subject: { channel: "C1", thread: "1.2" }, status: "waiting",
  started_at: AT, last_step_at: LATER, trigger: slackTrigger,
  classified: [{ node: "sort", name: "Sort", answers: [["category", "error"], ["urgent", "yes"]] }],
  session: { state: "live", machine: "mac-1", tab: "tab-4", claude: "ses_1" },
  steps: [step("t", "Trigger", slackTrigger, "c2"), step("sort", "Sort", { answers: { category: "error" }, port: "error" }, "c2"), step("k", "", { talk: "Look into it." }, "c3")],
  waiting_on: { workflow: "c2", node: "k", name: "Ask Claude" },
};
const observedTrigger = { kind: "observed", project: "12345", id: "0193", title: "TypeError in checkout" };
const done = {
  id: "c4:posthog:1", workflow: "c4", workflow_name: "Triage PostHog errors", subject: { issue: "posthog:12345:0193" }, status: "done",
  started_at: AT, last_step_at: LATER, trigger: observedTrigger, classified: [], session: { state: "detached", claude: "ses_9" },
  steps: [step("t", "Trigger", observedTrigger, "c4"), step("f", "File", { "file-issue": "TypeError" }, "c4")],
};
const failed = {
  id: "c5:uuid-8:1", workflow: "c5", workflow_name: "c5", subject: { issue: "uuid-8" }, status: "failed", why: "gateway down",
  trigger: null, classified: [], session: { state: "live", machine: "mini", tab: "tab-9" }, steps: [],
};

function element(id) {
  return { id, value: "", innerHTML: "", textContent: "", hidden: false, dataset: {}, addEventListener() {}, classList: { contains: () => false, add() {}, remove() {}, toggle() {} }, focus() {}, remove() {}, querySelector: () => null, querySelectorAll: () => [] };
}

/** settings.js and gui-workflow-runs.js as the page runs them, Settings open, with the hub played by
 * `answer` and the app's tabs by `tabs`. This Mac is `mac-1` on the account. */
function app({ answer = async () => [], tabs = [{ key: "local:tab-4" }], reveal = async () => {} } = {}) {
  const calls = [];
  const revealed = [];
  const agos = [];
  const dom = {};
  const byId = (id) => (dom[id] ??= element(id));
  const context = {
    visibleInterval: () => 0,
    console,
    addEventListener() {},
    load: () => null,
    save() {},
    listen() {},
    LOCAL: "local",
    account: { machine_id: "mac-1", email: "ana@acme.dev" },
    machines: new Map([["local", { view: { online: true, name: "MacBook" } }]]),
    hub: { online: true, hub: (call) => (calls.push(call), answer(call)) },
    hubUp: () => true,
    TAURI: {},
    invoke: (cmd, args) => Promise.reject(`${cmd} ${args?.call?.cmd ?? ""} should stay on the hub in this test`),
    agentApproval: () => undefined,
    picker: {},
    active: null,
    icon: () => "",
    tabs,
    ago: (unix) => (agos.push(unix), "5m"),
    machineName: (id) => ({ local: "MacBook", mini: "Mini" })[id] ?? "machine",
    revealTab: (machine, tab) => (revealed.push([machine, tab]), reveal(machine, tab)),
    document: { body: { dataset: { page: "settings" } }, getElementById: byId, querySelector: () => element(), createElement: () => element() },
  };
  context.globalThis = context;
  vm.createContext(context);
  vm.runInContext(
    [
      line(read("connection.js"), "const HUB_NEEDED = "),
      line(read("global-orchestrator.js"), "const isOlderPinned = "),
      line(read("main.js"), "const keyOf = "),
      line(read("main.js"), "const tabByKey = "),
      read("settings.js"),
      read("gui-workflow-runs.js"),
    ].join("\n"),
    context,
  );
  const state = (name) => vm.runInContext(name, context);
  const html = () => byId("settings-page").innerHTML;
  // Not awaiting the page's own load, which a test may hold open.
  const open = async () => {
    context.showPage("workflow-runs");
    await flush();
  };
  return { context, calls, revealed, agos, html, open, state };
}

/** Each run's row, in the order drawn. */
const rows = (html) => html.split("<details").slice(1);

test("Workflow runs sits right after Workflows in the nav, and index.html loads it after settings.js", () => {
  const { state } = app();
  const order = state("SETTINGS_ORDER");
  assert.equal(order[order.indexOf("connectors") + 1], "workflow-runs");
  const pages = state("settingsSections").map((s) => s.page);
  assert.equal(pages[pages.indexOf("connectors") + 1], "workflow-runs", pages.join(", "));
  const section = state('settingsSectionOf("workflow-runs")');
  assert.equal(section.label, "Workflow runs");
  assert.match(section.search[0].terms[0], /workflow runs history trigger classify steps failed stalled/);
  assert.match(read("index.html"), /<script src="settings.js"><\/script>[\s\S]*<script src="gui-workflow-runs.js"><\/script>/);
});

test("opening the page shows Loading, asks for the default runs, and draws the answer only while it is open", async () => {
  let answerNow;
  const { context, calls, html, open, state } = app({ answer: () => new Promise((resolve) => (answerNow = resolve)) });
  await open();
  assert.match(html(), /<h2>Workflow runs<\/h2>/);
  assert.match(html(), /Loading…/);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{ cmd: "workflow_runs" }], "no arguments: the hub's default 50");
  vm.runInContext('page = "general"', context);
  answerNow([running]);
  await flush();
  assert.match(html(), /Loading…/, "a page closed meanwhile is not drawn over");
  assert.equal(state("workflowRunsUi").runs.length, 1);
});

test("each run is one row with its workflow, subject, status and times", async () => {
  const { html, open, agos } = app({ answer: async () => [running, waiting, done, failed] });
  await open();
  const drawn = rows(html());
  assert.equal(drawn.length, 4);
  const summary = (row) => row.slice(0, row.indexOf("</summary>"));
  assert.match(summary(drawn[0]), /<b>Resolve<\/b> <span>ENG-7 Export fails<\/span>[\s\S]*>Running<\/span>[\s\S]*Started 5m ago · last step 5m ago/);
  assert.match(summary(drawn[1]), /<b>Triage Slack<\/b> <span>#alerts<\/span>[\s\S]*>Waiting<\/span>/);
  assert.match(summary(drawn[2]), /<b>Triage PostHog errors<\/b> <span>TypeError in checkout<\/span>[\s\S]*>Done<\/span>/);
  assert.match(summary(drawn[3]), /<b>c5<\/b> <span>uuid-8<\/span>[\s\S]*>Failed<\/span>[\s\S]*Started — · last step —/, "no trigger kind falls back to the subject, and no log to dashes");
  assert.ok(agos.includes(Date.parse(AT) / 1000), "times go through ago in unix seconds");
  assert.match(drawn[3], /<p class="error">Failed: gateway down<\/p>/);
  assert.match(drawn[1], /Waiting on <b>Ask Claude<\/b> in Triage Slack · last step 5m ago/);
  assert.match(drawn[0], /Waiting on <b>Sort<\/b> in Resolve/);
  assert.doesNotMatch(drawn[2], /Waiting on|Failed:/, "a done run has neither");
});

test("a row's detail holds the trigger, the classify answers, the session and every step", async () => {
  const { html, open } = app({ answer: async () => [running, waiting, done, failed] });
  await open();
  const [first, second, third, fourth] = rows(html());
  assert.ok(second.includes(`<pre>${esc(JSON.stringify(slackTrigger, null, 2))}</pre>`), "the trigger as indented JSON");
  assert.ok(fourth.includes("<pre>null</pre>"));
  assert.match(second, /<p><b>Sort<\/b><\/p><ul><li>category: error<\/li><li>urgent: yes<\/li><\/ul>/);
  assert.match(first, /No classify step answered/);
  assert.match(second, /MacBook · tab-4 · Claude ses_1 <button type="button" class="secondary" data-action="workflow-run-open" data-machine="local" data-tab="tab-4">Open thread<\/button>/);
  assert.match(first, /<p>No session<\/p>/);
  assert.match(third, /<p>Tab closed · Claude ses_9<\/p>/);
  assert.match(fourth, /Mini · tab-9<\/p>/);
  assert.doesNotMatch(fourth, /Open thread/, "a tab the app does not list has no button");
  const steps = (row) => row.slice(row.indexOf('<ol class="workflow-run-steps">'), row.indexOf("</ol>"));
  assert.equal(steps(second).split("<li>").length - 1, 3, "one line per step");
  assert.ok(steps(second).includes(`<b>Sort</b> <code>${esc(JSON.stringify({ answers: { category: "error" }, port: "error" }))}</code>`), "what happened as compact JSON");
  assert.match(steps(second), /<span class="hint">c3<\/span> <b>k<\/b>/, "a step in another workflow names it, and a step with no name its node");
  assert.doesNotMatch(steps(second), /<span class="hint">c2<\/span>/, "the run's own workflow is not repeated");
  assert.equal(steps(fourth), '<ol class="workflow-run-steps">');
});

test("Open thread reveals the local tab, and a failure says so on its line", async () => {
  const { context, html, open, revealed } = app({ answer: async () => [waiting], reveal: async () => Promise.reject(new Error("the tab closed")) });
  await open();
  const [, machine, tab] = html().match(/data-action="workflow-run-open" data-machine="([^"]*)" data-tab="([^"]*)"/);
  const sessionLine = { textContent: "MacBook · tab-4" };
  const button = { dataset: { machine, tab }, closest: (selector) => (selector === ".workflow-run-session" ? sessionLine : null) };
  await context.runAction("workflow-run-open", button);
  assert.deepEqual(revealed, [["local", "tab-4"]]);
  assert.equal(sessionLine.textContent, "Couldn't open that session: the tab closed");
});

test("every value of the answer is escaped, as text and inside attributes", async () => {
  const title = "<img src=x onerror=alert(1)>";
  const hostile = {
    ...waiting,
    workflow_name: "Fix <i>it</i>",
    status: "failed",
    why: '"><b>x</b>',
    trigger: { kind: "issue", identifier: "ENG-9", title },
    classified: [{ node: "n<1>", name: "", answers: [["<q>", "<a>"]] }],
    session: { state: "live", machine: "mac-1", tab: '"><u>t</u>', claude: "<s>c</s>" },
    steps: [step("t", "<em>Trigger</em>", { reply: "<script>alert(1)</script>" }, "c2")],
  };
  const { html, open } = app({ answer: async () => [hostile], tabs: [{ key: 'local:"><u>t</u>' }] });
  await open();
  const drawn = html();
  for (const raw of ["<img", "<script", "<b>x", "<i>", "<u>", "<s>", "<em>", "<q>", "<a>", "n<1>"]) assert.ok(!drawn.includes(raw), `${raw} reaches the page raw`);
  for (const escaped of ["&#60;img src=x onerror=alert(1)&#62;", "&#60;script&#62;alert(1)&#60;/script&#62;", "Failed: &#34;&#62;&#60;b&#62;x&#60;/b&#62;", "Fix &#60;i&#62;it&#60;/i&#62;", "n&#60;1&#62;", "&#60;q&#62;: &#60;a&#62;", "Claude &#60;s&#62;c&#60;/s&#62;"]) {
    assert.ok(drawn.includes(escaped), `${escaped} is in the page`);
  }
  assert.ok(drawn.includes('data-tab="&#34;&#62;&#60;u&#62;t&#60;/u&#62;"'), "an attribute holds the escaped tab");
});

test("an empty answer, a hub out of reach and a hub too old each say so", async () => {
  const empty = app({ answer: async () => [] });
  await empty.open();
  assert.match(empty.html(), /<p class="hint">No workflow has run yet\.<\/p>/);

  const away = app({ answer: async () => Promise.reject("needs the hub") });
  await away.open();
  assert.match(away.html(), /<p class="error">Can&#39;t reach the hub, and this needs it\.<\/p>/);
  assert.equal(rows(away.html()).length, 0);

  const old = app({ answer: async () => Promise.reject(new Error("unknown variant `workflow_runs`, expected one of `get_settings`")) });
  await old.open();
  assert.match(old.html(), /<p class="error">Update pinned on the hub to see workflow runs\.<\/p>/);
});
