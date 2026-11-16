import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const escLine = settingsSrc.slice(settingsSrc.indexOf("const esc = "), settingsSrc.indexOf("\n", settingsSrc.indexOf("const esc = ")));

const NOW = Date.parse("2026-11-15T12:00:00Z");
const S = NOW / 1000;
const DAY = 86400;
const here = (value) => JSON.parse(JSON.stringify(value));
const INJECTION = `<img src=x onerror="window.pwned=1">Fix`;

/** An element that records how its text was set, and refuses innerHTML. */
function fakeElement(tag = "div") {
  const el = {
    tag,
    className: "",
    dataset: {},
    children: [],
    textContent: "",
    set innerHTML(_) {
      throw new Error("innerHTML was used");
    },
    append(...kids) {
      el.children.push(...kids);
    },
    replaceChildren(...kids) {
      el.children = kids;
    },
  };
  return el;
}
const walk = (el) => [el, ...(el.children ?? []).flatMap(walk)];

/** gui-reports.js as the page runs it, with the app around it stubbed. `answer` plays the hub. */
function app({ answer = async () => null, signedIn = true, confirmations = [], errors = [] } = {}) {
  const calls = [];
  const context = {
    console,
    Date,
    Intl,
    JSON,
    URL,
    esc: null,
    document: { addEventListener() {}, body: { dataset: { version: "v0.18.4" } }, createElement: fakeElement },
    navigator: { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X)" },
    crypto: { randomUUID: () => "uuid-1" },
    hub: signedIn ? { hub: async (call) => (calls.push(here(call)), answer(call)) } : null,
    SHELL_PAGES: {},
    AGENT_CLICKS: {},
    agentConfirmations: confirmations,
    tabs: [{ view: { cwd: "/Users/ana/loja-web" } }],
    threadOf: () => ({ workspace: { project: "/Users/ana/loja-web" } }),
    projectName: () => "loja-web",
    machines: new Map([["m1", { view: { name: "MacBook" } }]]),
    reportErrors: { count: () => errors.length, last: (n) => errors.slice(-n) },
  };
  vm.createContext(context);
  vm.runInContext(`${escLine}\n${read("gui-reports.js")}`, context);
  const run = (code, args = {}) => {
    Object.assign(context, { args });
    return vm.runInContext(code, context);
  };
  return { run, calls, context };
}

const similar = (extra = {}) => ({ number: 318, title: "Diff is slow on files over 5k lines", url: "https://github.com/FelipeGazapina/pinned/issues/318", comments: 4, ...extra });

/** A dialog the way typing in the form leaves it. */
function typed(a, draft = {}, extra = {}) {
  return a.run(
    `(() => { const dlg = reportDialogState(); Object.assign(dlg.draft, args.draft); Object.assign(dlg, args.extra); reportsUi.dialog = dlg; return dlg; })()`,
    { draft: { title: "The diff doesn't open", happened: "It froze for 20 seconds.", expected: "It opens.", repeat: "Open a 9k-line file.", ...draft }, extra },
  );
}

test("Review and send asks the hub to preview exactly what was typed, with the app's names for the scrubber", async () => {
  const a = app({ answer: async () => ({ title: "t", body: "### What happened\nx", labels: ["bug"], similar: [], left_today: 5, shown: "sha-1" }) });
  const dlg = typed(a);
  await a.run("reportPreview(reportsUi.dialog)");
  assert.deepEqual(a.calls, [
    {
      cmd: "reports",
      ask: {
        op: "preview",
        draft: { kind: "problem", title: "The diff doesn't open", happened: "It froze for 20 seconds.", expected: "It opens.", repeat: "Open a 9k-line file.", attach: { environment: true, errors: [] }, to_issue: null },
        context: { projects: ["loja-web"], folders: ["/Users/ana/loja-web"], files: [], names: ["MacBook"] },
      },
    },
  ]);
  assert.equal(dlg.step, "preview");
  assert.equal(dlg.requestId, "uuid-1");
});

test("an idea sends its two sections and no repeat steps", () => {
  const a = app();
  typed(a, { kind: "idea" });
  assert.deepEqual(here(a.run("reportDraftOf(reportsUi.dialog)")), { kind: "idea", title: "The diff doesn't open", happened: "It froze for 20 seconds.", expected: "It opens.", repeat: "", attach: { environment: true, errors: [] }, to_issue: null });
  assert.deepEqual(here(a.run("REPORT_TEXT.fields.idea")), { happened: "The idea", expected: "Why it helps" });
});

test("error lines are off by default and go out only while their box is ticked", async () => {
  const lines = Array.from({ length: 30 }, (_, i) => `12:00:${String(i).padStart(2, "0")} error ${i}`);
  const a = app({ errors: lines, answer: async (c) => (c.cmd === "report_issue" ? { number: 341, url: "https://github.com/FelipeGazapina/pinned/issues/341" } : { title: "t", body: "", labels: [], similar: [], left_today: 5, shown: "sha-1" }) });
  const dlg = typed(a);
  assert.equal(dlg.errorsOn, false);
  assert.deepEqual(here(a.run("reportDraftOf(reportsUi.dialog).attach")), { environment: true, errors: [] });
  const box = { dataset: { attach: "errors" } };
  a.run("REPORT_DIALOG_ACTS.attach(reportsUi.dialog, args.box)", { box });
  assert.deepEqual(here(a.run("reportDraftOf(reportsUi.dialog).attach.errors")), lines.slice(-20));
  await a.run("reportPreview(reportsUi.dialog)");
  assert.equal(a.calls[0].ask.draft.attach.errors.length, 20);
  a.run("REPORT_DIALOG_ACTS.attach(reportsUi.dialog, args.box)", { box });
  assert.deepEqual(here(a.run("reportDraftOf(reportsUi.dialog).attach.errors")), []);
  a.run("REPORT_DIALOG_ACTS.attach(reportsUi.dialog, args.box)", { box });
  await a.run("reportSend(reportsUi.dialog)");
  const sent = a.calls.at(-1);
  assert.equal(sent.cmd, "report_issue");
  assert.equal(sent.draft.attach.errors.length, 20);
});

test("Send carries the hash the preview showed, the draft id and one request id, and ends on the report's number", async () => {
  const a = app({ answer: async (c) => (c.cmd === "report_issue" ? { number: 341, url: "https://github.com/FelipeGazapina/pinned/issues/341" } : { title: "t", body: "", labels: [], similar: [], left_today: 4, shown: "sha-9" }) });
  const dlg = typed(a, {}, { draftId: "d-7" });
  await a.run("reportPreview(reportsUi.dialog)");
  await a.run("reportSend(reportsUi.dialog)");
  assert.deepEqual(a.calls.at(-1), {
    cmd: "report_issue",
    draft: { kind: "problem", title: "The diff doesn't open", happened: "It froze for 20 seconds.", expected: "It opens.", repeat: "Open a 9k-line file.", attach: { environment: true, errors: [] }, to_issue: null },
    shown: "sha-9",
    draft_id: "d-7",
    request_id: "uuid-1",
  });
  assert.equal(dlg.step, "done");
  assert.deepEqual(here(dlg.result), { kind: "sent", number: 341, url: "https://github.com/FelipeGazapina/pinned/issues/341" });
});

test("the daily limit the hub answers is shown as it is, and the form stays", async () => {
  const limit = "You sent 5 reports in the last day. You can send again at 14:05 UTC.";
  const a = app({ answer: async (c) => (c.cmd === "report_issue" ? Promise.reject(limit) : { title: "t", body: "", labels: [], similar: [], left_today: 0, shown: "s" }) });
  const dlg = typed(a);
  await a.run("reportPreview(reportsUi.dialog)");
  await a.run("reportSend(reportsUi.dialog)");
  assert.equal(dlg.error, limit);
  assert.equal(dlg.step, "preview");
  assert.equal(dlg.busy, false);
});

test("a hub that says available:false, or does not know reports, shows the sentence and no form", async () => {
  for (const answer of [async () => ({ available: false }), async () => Promise.reject("unknown command")]) {
    const a = app({ answer });
    const dlg = a.run("reportDialogState()");
    await a.run("reportCheckStatus(args.dlg)", { dlg });
    assert.equal(dlg.status, "unavailable");
    const html = a.run("reportDialogHtml(args.dlg)", { dlg });
    assert.match(html, /Reports are not available on this hub\./);
    assert.doesNotMatch(html, /<input|<textarea|data-rp="review"/);
  }
  const a = app({ answer: async () => ({ available: true }) });
  const dlg = a.run("reportDialogState()");
  await a.run("reportCheckStatus(args.dlg)", { dlg });
  assert.match(a.run("reportDialogHtml(args.dlg)", { dlg }), /<textarea/);
});

test("picking a similar issue sends the case to it, and undoing goes back to a new issue", async () => {
  const a = app({ answer: async () => ({ title: "t", body: "", labels: [], similar: [], left_today: 5, shown: "s" }) });
  const dlg = typed(a, {}, { similar: [similar(), similar({ number: 305, title: "Large diffs freeze" })] });
  a.run("REPORT_DIALOG_ACTS.pick(reportsUi.dialog, args.b)", { b: { dataset: { index: "1" } } });
  assert.equal(dlg.draft.to_issue, 305);
  await a.run("reportPreview(reportsUi.dialog)");
  assert.equal(a.calls[0].ask.draft.to_issue, 305);
  a.run("REPORT_DIALOG_ACTS.unpick(reportsUi.dialog)");
  assert.equal(dlg.draft.to_issue, null);
});

test("typing a title looks for similar issues by the title alone, once it is long enough", async () => {
  const a = app({ answer: async () => [similar()] });
  const dlg = typed(a);
  a.run("searchSimilar(reportsUi.dialog)");
  await new Promise((r) => setImmediate(r));
  assert.deepEqual(a.calls, [{ cmd: "reports", ask: { op: "similar", title: "The diff doesn't open" } }]);
  assert.equal(dlg.similar.length, 1);
  const b = app();
  typed(b, { title: "diff" });
  b.run("searchSimilar(reportsUi.dialog)");
  assert.deepEqual(b.calls, []);
});

test("Save draft stores the form under its id and Open and send brings it back", async () => {
  const a = app({ answer: async () => ({ id: "d-1" }) });
  const dlg = typed(a);
  await a.run("reportSaveDraft(reportsUi.dialog)");
  assert.deepEqual(a.calls[0].ask.op, "save_draft");
  assert.equal(a.calls[0].ask.id, null);
  assert.equal(dlg.draftId, "d-1");
  assert.equal(dlg.result.kind, "draft");
  const back = a.run("reportDialogState({ draft: args.draft, draftId: 'd-1' })", { draft: { title: "T", happened: "H", attach: { environment: false, errors: ["e"] } } });
  assert.equal(back.errorsOn, true);
  assert.equal(back.draft.attach.environment, false);
});

const REPORTS = [
  { id: "a", kind: "problem", title: "Shortcut broken on Windows", issue: 329, url: "https://github.com/o/p/issues/329", followed: false, by_agent: false, state: { state: "fixed", at: S - DAY, version: "0.18.5" }, draft: null, created_at: S - 6 * DAY, updated_at: S - DAY },
  { id: "b", kind: "idea", title: "Light theme", issue: 322, url: "https://github.com/o/p/issues/322", followed: false, by_agent: false, state: { state: "answered", at: S - 2 * DAY }, draft: null, created_at: S - 9 * DAY, updated_at: S - 2 * DAY },
  { id: "c", kind: "problem", title: "Diff does not open", issue: 341, url: "https://github.com/o/p/issues/341", followed: false, by_agent: true, state: { state: "open" }, draft: null, created_at: S - 3600, updated_at: S - 3600 },
  { id: "d", kind: "problem", title: "Diff is slow", issue: 318, url: "https://github.com/o/p/issues/318", followed: true, by_agent: false, state: { state: "open" }, draft: null, created_at: S - 3 * DAY, updated_at: S - 3 * DAY },
  { id: "e", kind: "problem", title: "", issue: null, url: null, followed: false, by_agent: false, state: { state: "draft" }, draft: { kind: "problem", title: "Search misses archived threads" }, created_at: S - 2 * DAY, updated_at: S - 2 * DAY },
  { id: "f", kind: "problem", title: "Banner stays", issue: 290, url: "https://github.com/o/p/issues/290", followed: false, by_agent: false, state: { state: "closed", at: S - 20 * DAY, reason: "completed" }, draft: null, created_at: S - 30 * DAY, updated_at: S - 20 * DAY },
  { id: "g", kind: "problem", title: "Pairing code", issue: 280, url: "https://github.com/o/p/issues/280", followed: false, by_agent: false, state: { state: "fixed", at: S - 40 * DAY, version: "0.18.2" }, draft: null, created_at: S - 50 * DAY, updated_at: S - 40 * DAY },
  { id: "h", kind: "problem", title: "Tab titles", issue: null, url: null, followed: false, by_agent: false, state: { state: "failed", error: "GitHub was unreachable" }, draft: { kind: "problem", title: "Tab titles" }, created_at: S - 600, updated_at: S - 600 },
];

test("My reports groups sent, answered, fixed, followed and draft reports and says each state in words", () => {
  const a = app();
  const groups = here(a.run("reportGroups(args.list, '0.18.4', args.now)", { list: REPORTS, now: NOW }));
  const ids = (g) => groups[g].map((row) => row.report.id);
  assert.deepEqual(ids("news"), ["a", "b"]);
  assert.deepEqual(ids("open"), ["h", "c", "d"]);
  assert.deepEqual(ids("drafts"), ["e"]);
  assert.deepEqual(ids("closed"), ["f", "g"]);
  const line = (g, id) => groups[g].find((row) => row.report.id === id).line;
  assert.deepEqual(line("news", "a"), { text: "Fixed in 0.18.5. You are on 0.18.4.", tone: "good", update: true });
  assert.equal(line("news", "b").text, "The pinned team replied 2 days ago.");
  assert.equal(line("open", "c").text, "opened today, by the agent");
  assert.equal(line("open", "d").text, "someone else's report, you follow it");
  assert.equal(line("open", "h").text, "Could not send: GitHub was unreachable");
  assert.equal(line("closed", "f").text, "Closed: completed");
  assert.equal(line("closed", "g").text, "Fixed in 0.18.2.");
});

test("a fix shipped in the version the app already runs is not news, and a newer app version compares by number", () => {
  const a = app();
  assert.equal(a.run("reportNewer('0.18.10', '0.18.9')"), true);
  assert.equal(a.run("reportNewer('0.18.5', '0.18.5')"), false);
  assert.equal(a.run("reportNewer('0.19.0', '0.18.9')"), true);
  const groups = here(a.run("reportGroups(args.list, '0.18.5', args.now)", { list: REPORTS, now: NOW }));
  assert.deepEqual(groups.news.map((row) => row.report.id), ["b"]);
  assert.ok(groups.closed.some((row) => row.report.id === "a"));
});

test("text from GitHub is never HTML: My reports holds it in slots that textContent fills", () => {
  const a = app();
  const list = [{ ...REPORTS[2], title: INJECTION, state: { state: "closed", reason: "<script>alert(1)</script>" } }];
  const html = a.run("(() => { reportsUi.list = args.list; reportsUi.listState = 'ready'; const s = reportSlots(); const html = reportsPageHtml(s); globalThis.slots = s; return html; })()", { list });
  assert.doesNotMatch(html, /<img|<script|onerror/);
  const slots = a.run("slots.list");
  assert.ok(slots.includes(INJECTION));
  assert.ok(slots.includes("Closed: <script>alert(1)</script>"));
  const slotEls = [...html.matchAll(/data-gh="(\d+)"/g)].map(([, i]) => ({ dataset: { gh: i }, textContent: "" }));
  a.run("paintSlots({ querySelectorAll: () => args.els }, slots)", { els: slotEls });
  assert.ok(slotEls.some((el) => el.textContent === INJECTION));
});

test("similar titles, the preview and its labels are slots too", () => {
  const a = app();
  const dlg = a.run("reportDialogState()");
  dlg.similar = [similar({ title: INJECTION })];
  dlg.preview = { title: INJECTION, body: `### <b>Heading</b>\n${INJECTION}\n\n### Error lines\n\`\`\`\n${INJECTION}\n\`\`\``, labels: ["bug", "<i>x</i>"], similar: [similar({ title: INJECTION })], left_today: 3, shown: "s" };
  dlg.step = "preview";
  const html = a.run("reportDialogHtml(args.dlg)", { dlg: Object.assign(dlg, { status: "ready" }) });
  assert.doesNotMatch(html, /<img|<b>|<i>x|onerror/);
  assert.match(html, /3 reports left today\./);
  assert.ok(a.run("args.dlg.gh.list", { dlg }).includes(INJECTION));
  const form = a.run("reportSimilarHtml(args.dlg, reportSlots())", { dlg: Object.assign(dlg, { picked: null, step: "form" }) });
  assert.doesNotMatch(form, /<img|onerror/);
});

test("the agent's card draws the draft with DOM text, never innerHTML", () => {
  const draft = { kind: "problem", title: INJECTION, happened: INJECTION, expected: "It opens.", repeat: "", attach: { environment: true, errors: [] }, to_issue: null };
  const c = { id: "c-1", action: "report_issue", args: { draft }, machine_name: "MacBook", tab: "t", title: "T", role: "worker", at: S };
  const a = app({ confirmations: [c], signedIn: false });
  a.run("reportsUi.card.set('c-1', { state: 'ready', preview: { title: args.t, body: '### What happened\\n' + args.t, labels: ['bug'], similar: [{ number: 318, title: args.t }] } })", { t: INJECTION });
  const el = fakeElement();
  el.dataset.rpCard = "c-1";
  a.run("paintReportCard(args.el)", { el });
  const texts = walk(el).map((n) => n.textContent);
  assert.ok(texts.includes(INJECTION));
  assert.ok(texts.some((t) => t.startsWith("#318 looks like the same problem.")));
  assert.deepEqual(walk(el).filter((n) => n.dataset?.agent).map((n) => n.dataset.agent), ["report-follow"]);
});

test("the card for report_issue offers Send, Edit first and Don't send, in the Needs you and the activity styles", () => {
  const a = app({ signedIn: false });
  const card = a.run("globalThis.agentCards.report_issue");
  const c = { id: "c-1", action: "report_issue", args: { draft: {} } };
  const plain = card.actions(c, "", false);
  assert.deepEqual([...plain.matchAll(/data-agent="([^"]+)"[^>]*>([^<]+)</g)].map(([, act, label]) => `${act} ${label}`), ["approve Send to pinned", "report-edit Edit first", "deny Don&#39;t send"]);
  const ny = card.actions(c, "", true);
  assert.match(ny, /data-ny="approve" data-agent="approve"/);
  assert.match(card.body(c), /data-rp-card="c-1"/);
  assert.equal(typeof a.run("AGENT_CLICKS['report-edit']"), "function");
  assert.equal(typeof a.run("AGENT_CLICKS['report-follow']"), "function");
});

test("only github.com links open from a report", () => {
  const a = app();
  assert.equal(a.run("reportUrl('https://github.com/o/p/issues/1')"), "https://github.com/o/p/issues/1");
  for (const bad of ["javascript:alert(1)", "http://github.com/o/p", "https://github.com.evil.test/x", "https://evil.test/github.com", "not a url", null]) assert.equal(a.run("reportUrl(args.u)", { u: bad }), null, String(bad));
  assert.equal(a.run("reportLinkHtml('https://evil.test/x')"), "");
  assert.match(a.run("reportLinkHtml('https://github.com/o/p/issues/1')"), /data-rp="github"/);
});

test("the issue body is read as sections, with fenced lines apart", () => {
  const a = app();
  const got = here(a.run("parseIssueBody(args.b)", { b: "### What happened\nIt froze.\n\n### Error lines\n```\n12:00 a\n12:01 # not a heading\n```\n" }));
  assert.deepEqual(got, [
    { heading: "What happened", text: "It froze.", code: [] },
    { heading: "Error lines", text: "", code: ["12:00 a", "12:01 # not a heading"] },
  ]);
});

test("without a login the dialog says to sign in and sends nothing", () => {
  const a = app({ signedIn: false });
  assert.match(a.run("reportDialogHtml(reportDialogState())"), /Sign in under Account/);
  assert.deepEqual(a.calls, []);
});

test("the Account and pages menu holds the two items in one block, and the page and styles are loaded", () => {
  const html = read("index.html");
  const block = html.slice(html.indexOf("<!-- reports: start -->"), html.indexOf("<!-- reports: end -->"));
  assert.match(block, /data-open-page="reports"/);
  assert.match(block, /data-report-new/);
  assert.match(html, /<script src="report-errors\.js"><\/script>/);
  assert.match(html, /<script src="gui-reports\.js"><\/script>/);
  assert.match(html, /<link rel="stylesheet" href="gui-reports\.css">/);
  assert.ok(html.indexOf("report-errors.js") < html.indexOf("hub.js"));
  const a = app();
  assert.equal(typeof a.run("SHELL_PAGES.reports.show"), "function");
});
