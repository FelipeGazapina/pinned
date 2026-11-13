import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const usageSrc = read("gui-usage.js");
const between = (src, from, to) => {
  const start = src.indexOf(from);
  assert.ok(start >= 0, `${from} is in the source`);
  return src.slice(start, src.indexOf(to, start));
};
const line = (src, prefix) => src.split("\n").find((l) => l.startsWith(prefix));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};

const sectionSrc = between(usageSrc, "const BOX_LIVE", 'document.addEventListener("click"');
const NOW = 1793491210 + 2 * 3600 + 5 * 60;

/** The section's code as the page runs it, over a hub that answers `box_usage` with `answer`. */
function boxPage({ answer, stuckAnswer = async () => [], hubOn = true } = {}) {
  const renders = [];
  const context = {
    esc: (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"),
    icon: (name) => `[${name}]`,
    shellPage: () => "usage",
    renderUsagePage: () => renders.push(context.boxUsageHtml(context.usageState.box, NOW)),
    usageState: { box: { data: null, error: "", loading: false, seq: 0 }, stuck: [] },
    hub: hubOn ? { hub: async (message) => (message.cmd === "stuck_boxes" ? stuckAnswer(message) : (assert.equal(JSON.stringify(message), '{"cmd":"box_usage"}'), answer())) } : null,
    pageView: { addEventListener: (type, handler) => (context.toggled = { type, handler }) },
    renders,
    drawn: 0,
    drawNav: () => context.drawn++,
    setTimeout: () => 0,
    setInterval: () => 0,
  };
  vm.createContext(context);
  vm.runInContext(`${line(usageSrc, "const usd = ")}\n${line(read("settings.js"), "const BOX_SIZES = ")}\n${sectionSrc}
    this.boxUsageHtml = boxUsageHtml; this.fetchBoxUsage = fetchBoxUsage; this.stuckMarkTitle = stuckMarkTitle;`, context);
  return context;
}
const text = (html) => html.replace(/<[^>]*>/g, " ").replace(/&amp;/g, "&").replace(/\s+/g, " ").trim();
const show = (data, error = "") => boxPage().boxUsageHtml({ data, error, loading: false }, NOW);

const PINNED = { box_hours: 2.0, tasks: 2, since: 1793491200 };
const A = {
  connected: true,
  error: null,
  boxes: [
    { id: "u-web", name: "web", size: "large", status: "running", created: 1793491210, last_activity: 1793491230, cpu_hours: 2.0, compute_usd: 1.5, pinned: { project: "site", role: "worker", machine: "ana-mac" } },
    { id: "u-old", name: "last month", size: null, status: "deleted", created: 1793404800, last_activity: null, cpu_hours: 1.0, compute_usd: 0.5, pinned: null },
  ],
  upstash: { cpu_hours: 2.0, compute_usd: 1.5, boxes: 1 },
  pinned: PINNED,
};

test("state a: Upstash's numbers, pinned's count, the running box and the other boxes folded away", () => {
  const html = show(A);
  assert.equal(
    text(html),
    "Upstash Box This month From Upstash 2.00 CPU hours $1.50 compute cost Upstash's numbers for the 1 box it still lists. Deleted boxes may be missing. " +
      "pinned's count 2.0 box hours 2 tasks Box time pinned recorded for its delegated tasks; not a bill. " +
      "Running now Box Size Belongs to Up for CPU hours Cost web site · worker Large site · worker 2 h 5 min 2.00 $1.50 " +
      "Other boxes (1) Box Size Belongs to Status CPU hours Cost last month Not opened by pinned — Not opened by pinned Deleted 1.00 $0.50",
  );
  assert.match(html, /<details class="box-others"><summary>Other boxes \(1\)<\/summary>/);
  assert.doesNotMatch(html, /<details[^>]* open/);
  assert.doesNotMatch(text(html.split("pinned's count")[1].split("</div></div>")[0]), /\$/, "no dollar figure for pinned's count");
});

test("Other boxes stays open when the page redraws while it was open", () => {
  const page = boxPage();
  assert.match(page.boxUsageHtml({ data: A, error: "", loading: false, open: true }, NOW), /<details class="box-others" open>/);
  assert.match(page.boxUsageHtml({ data: A, error: "", loading: false, open: false }, NOW), /<details class="box-others"><summary>/);
  assert.equal(page.toggled.type, "toggle");
  page.toggled.handler({ target: { matches: (selector) => selector === ".box-others", open: true } });
  assert.equal(page.usageState.box.open, true, "opening Other boxes is remembered");
  page.toggled.handler({ target: { matches: () => false, open: false } });
  assert.equal(page.usageState.box.open, true, "another element's toggle is not");
});

test("Running now holds creating, idle and running boxes; every other status goes under Other boxes", () => {
  const box = (name, status) => ({ id: name, name, size: "small", status, created: NOW - 600, cpu_hours: 0.1, compute_usd: 0.01, pinned: null });
  const html = show({ ...A, boxes: ["creating", "idle", "running", "paused", "stopped", "deleted"].map((s) => box(`b-${s}`, s)) });
  const [running, others] = html.split('<details class="box-others">');
  for (const s of ["creating", "idle", "running"]) assert.match(running, new RegExp(`b-${s}`));
  for (const s of ["paused", "stopped", "deleted"]) assert.match(others, new RegExp(`b-${s}`));
  for (const s of ["paused", "stopped", "deleted"]) assert.doesNotMatch(running, new RegExp(`b-${s}`));
  assert.match(others, /Other boxes \(3\)/);
  assert.match(text(others), /b-paused Not opened by pinned Small Not opened by pinned Paused/);
  assert.match(text(running), /b-creating[^]*Starting…/);
  assert.match(text(running), /b-idle[^]*10 min/);
});

test("a missing number is a dash, a long cell is cut and keeps its whole text in a tooltip, and the answer's strings are escaped", () => {
  const html = show({
    ...A,
    boxes: [{ id: "x", name: "<img src=x onerror=1>", size: null, status: "running", created: NOW - 3 * 86400 - 4 * 3600, cpu_hours: null, compute_usd: null, pinned: { project: "demo-app", role: "captain & co", machine: "m" } }],
  });
  assert.doesNotMatch(html, /<img/);
  assert.match(html, /&lt;img src=x onerror=1&gt;/);
  assert.match(html, /<td class="opt" title="demo-app · captain &amp; co">demo-app · captain &amp; co<\/td>/);
  assert.match(html, /<td class="opt" title="—">—<\/td>/);
  assert.match(html, /<td title="3 d 4 h">3 d 4 h<\/td>/);
  assert.match(html, /<td class="num">—<\/td><td class="num">—<\/td>/);
  const css = read("gui-usage.css");
  assert.match(css, /\.usage \.box-usage :is\(th, td\) \{[^}]*overflow: hidden; text-overflow: ellipsis; white-space: nowrap;/);
});

test("a box pinned opened without a project or a role still says pinned opened it", () => {
  const box = (pinned) => ({ id: "x", name: "x", size: "small", status: "running", created: NOW, cpu_hours: 0, compute_usd: 0, pinned });
  assert.match(show({ ...A, boxes: [box({ project: null, role: "worker", machine: "m" })] }), /title="worker">worker<\/td>/);
  assert.match(show({ ...A, boxes: [box({ project: null, role: null, machine: null })] }), /title="Opened by pinned">Opened by pinned<\/td>/);
});

test("state b: no connection says where to connect and shows no figures or tables", () => {
  const html = show({ connected: false, error: null, boxes: [], upstash: null, pinned: PINNED });
  assert.equal(text(html), "Upstash Box This month Connect Upstash in Settings › Connectors to see box usage.");
});

test("state c: an Upstash error, as the hub words it, is a warning with pinned's count only, and no tables", () => {
  const html = show({ connected: true, error: "Upstash did not answer: Upstash Box answered 500: boom", boxes: [], upstash: { cpu_hours: 0, compute_usd: 0, boxes: 0 }, pinned: PINNED });
  assert.match(html, /<div class="limit-notices" role="alert">\[triangle-alert\]<div><p>Upstash did not answer: Upstash Box answered 500: boom<\/p><\/div><\/div>/);
  assert.equal(
    text(html),
    "Upstash Box This month [triangle-alert] Upstash did not answer: Upstash Box answered 500: boom pinned's count 2.0 box hours 2 tasks Box time pinned recorded for its delegated tasks; not a bill.",
  );
});

test("state d: no boxes says so beside both figures", () => {
  const html = show({ connected: true, error: null, boxes: [], upstash: { cpu_hours: 0, compute_usd: 0, boxes: 0 }, pinned: { box_hours: 0.25, tasks: 1, since: 1 } });
  assert.equal(
    text(html),
    "Upstash Box This month From Upstash 0.00 CPU hours $0.00 compute cost Upstash's numbers for the 0 boxes it still lists. Deleted boxes may be missing. " +
      "pinned's count 0.3 box hours 1 task Box time pinned recorded for its delegated tasks; not a bill. Running now No boxes right now.",
  );
});

test("while the hub is slow the section shows Loading…, and a failed call shows a warning", async () => {
  let answer;
  const slow = boxPage({ answer: () => new Promise((resolve) => (answer = resolve)) });
  const pending = slow.fetchBoxUsage();
  assert.match(text(slow.renders.at(-1)), /^Upstash Box This month Loading…$/);
  assert.match(slow.renders.at(-1), /aria-busy="true"/);
  answer(A);
  await pending;
  assert.match(text(slow.renders.at(-1)), /From Upstash 2\.00 CPU hours/);
  assert.match(slow.renders.at(-1), /aria-busy="false"/);

  const failing = boxPage({ answer: async () => Promise.reject(new Error("hub is down")) });
  await failing.fetchBoxUsage();
  assert.match(text(failing.renders.at(-1)), /Could not read Upstash Box usage: hub is down/);
});

test("an older answer never overwrites a newer one", async () => {
  const answers = [];
  const page = boxPage({ answer: () => new Promise((resolve) => answers.push(resolve)) });
  const first = page.fetchBoxUsage();
  const second = page.fetchBoxUsage();
  answers[1]({ ...A, upstash: { cpu_hours: 9, compute_usd: 9, boxes: 3 } });
  await second;
  answers[0](A);
  await first;
  assert.match(text(page.renders.at(-1)), /9\.00 CPU hours \$9\.00/);
});

/** renderUsagePage and the page's open and refresh paths, over stubs that record what they were asked. */
function pageWith({ metric, hubOn = true, boxPending = true }) {
  const calls = [];
  const set = [];
  const context = {
    usageState: { metric, days: 30, refreshing: false, box: { data: null, error: "", loading: true, seq: 0 } },
    hub: hubOn ? {} : null,
    machines: new Map([["m", {}]]),
    pageView: { querySelector: () => null },
    SHELL_PAGES: {},
    usageSelected: () => ["m"],
    summarize: () => ({ buckets: [] }),
    usageEnvLabel: () => "All environments",
    icon: () => "",
    envOnline: () => true,
    usageSkeleton: () => "",
    envReadable: () => true,
    limitsHtml: () => `<div class="limits-view">LIMITS</div>`,
    spendHtml: () => `<section class="overview">SPEND</section>`,
    usagePartial: () => false,
    headHtml: () => "<header></header>",
    usageRange: () => "",
    segmented: () => "",
    pickButton: () => "",
    metricOptions: () => [],
    periodOptions: () => [],
    drawChart: () => {},
    esc: String,
    machineName: String,
    setPage: (html) => set.push(html),
    shellPage: () => "usage",
    onlineMachines: () => ["m"],
    boxUsageHtml: () => `<section class="box-usage">BOX</section>`,
    checkLimits: async () => calls.push("limits"),
    fetchUsage: async () => calls.push("spend"),
    fetchBoxUsage: () => (calls.push("box"), boxPending ? new Promise(() => {}) : Promise.resolve()),
    Date,
    set,
    calls,
  };
  context.usageState.envs = new Map([["m", { loaded: true }]]);
  vm.createContext(context);
  vm.runInContext(`${fn(usageSrc, "renderUsagePage")}\n${fn(usageSrc, "refreshUsage")}\n${between(usageSrc, "SHELL_PAGES.usage = {", "\n};\n")}\n};
    this.renderUsagePage = renderUsagePage; this.refreshUsage = refreshUsage;`, context);
  return context;
}

test("the section sits below the body in both the spend and the limits view, and is left out without a hub", () => {
  for (const [metric, body] of [["cost", "SPEND"], ["limits", "LIMITS"]]) {
    const page = pageWith({ metric });
    page.renderUsagePage();
    assert.ok(page.set[0].indexOf(body) > 0 && page.set[0].indexOf(body) < page.set[0].indexOf(`<section class="box-usage">BOX</section>`), `${metric}: the box section follows the body`);
  }
  const none = pageWith({ metric: "cost", hubOn: false });
  none.renderUsagePage();
  assert.doesNotMatch(none.set[0], /box-usage/);
});

test("opening the page and the refresh button reload the box section, and a slow box_usage holds back neither", async () => {
  for (const metric of ["cost", "limits"]) {
    const page = pageWith({ metric });
    page.SHELL_PAGES.usage.show();
    assert.ok(page.set.length >= 1, "the page draws at once");
    assert.ok(page.calls.includes("box") && page.calls.includes(metric === "limits" ? "limits" : "spend"));
    page.calls.length = 0;
    await page.refreshUsage();
    assert.deepEqual([...page.calls].sort(), ["box", metric === "limits" ? "limits" : "spend"].sort());
    assert.equal(page.usageState.refreshing, false, "refresh finished while box_usage is still pending");
  }
});

test("boxes pinned could not delete show as one notice per reason, also when Upstash is not connected", () => {
  const page = boxPage({ answer: async () => ({}) });
  const html = (data) => page.boxUsageHtml({ data, error: "", loading: false, open: {} }, NOW);
  const stuck = [{ id: "r1", why: "Upstash answered 503" }, { id: "r2", why: "Upstash answered 503" }, { id: "r3", why: "Upstash answered 500" }];
  const off = html({ connected: false, stuck });
  assert.match(off, /2 boxes pinned could not delete: Upstash answered 503/);
  assert.match(off, /1 box pinned could not delete: Upstash answered 500\. It is still running, and may still be billed, on Upstash\. It can be deleted in the Upstash console\./);
  assert.match(off, /2 boxes pinned could not delete: Upstash answered 503\. They are still running, and may still be billed, on Upstash\. They can be deleted in the Upstash console\./);
  assert.match(off, /Connect Upstash in Settings/);
  assert.equal(off.match(/limit-notices/g).length, 2);
  const on = html({ connected: true, stuck: [stuck[2]], boxes: [], upstash: { cpu_hours: 0, compute_usd: 0, boxes: 0 }, pinned: PINNED });
  assert.match(on, /1 box pinned could not delete/);
  assert.doesNotMatch(html({ connected: false }), /could not delete/);
});

test("a reason that ends in a full stop gets no second one, and a reason the hub cut short closes on its ellipsis", () => {
  const page = boxPage({ answer: async () => ({}) });
  const html = (stuck) => page.boxUsageHtml({ data: { connected: false, stuck }, error: "", loading: false, open: {} }, NOW);
  const dotted = html([{ id: "r1", why: "Upstash said: no such box." }, { id: "r2", why: "Upstash gave up...", unconfirmed: true }]);
  assert.match(dotted, /1 box pinned could not delete: Upstash said: no such box\. It is still running/);
  assert.match(dotted, /1 box pinned could not end: Upstash gave up\. pinned could not confirm/);
  assert.doesNotMatch(dotted, /\.\./);
  const cut = html([{ id: "r3", why: `${"x".repeat(299)}…` }]);
  assert.match(cut, /x…\s+It is still running, and may still be billed/);
  assert.doesNotMatch(cut, /…\./);
});

test("the mark that closes a reason: a question or exclamation mark gets no full stop, and trailing full stops and white space are dropped", () => {
  const page = boxPage({ answer: async () => ({}) });
  const reasonOf = (why) => {
    const notice = text(page.boxUsageHtml({ data: { connected: false, stuck: [{ id: "r1", why }] }, error: "", loading: false, open: {} }, NOW));
    return notice.match(/1 box pinned could not delete: (.*?) It is still running/)[1];
  };
  assert.equal(reasonOf("Upstash said why?"), "Upstash said why?");
  assert.equal(reasonOf("Upstash said no!"), "Upstash said no!");
  assert.equal(reasonOf("Upstash said no."), "Upstash said no.");
  assert.equal(reasonOf("Upstash said no.\n"), "Upstash said no.");
  assert.equal(reasonOf("Upstash said no. "), "Upstash said no.");
  assert.equal(reasonOf("Upstash said no. ."), "Upstash said no.");
  assert.equal(reasonOf("Upstash said no..."), "Upstash said no.");
  assert.equal(reasonOf("Upstash said no?."), "Upstash said no?");
  assert.equal(reasonOf("Upstash said no!\n"), "Upstash said no!");
  assert.equal(reasonOf(`${"x".repeat(5)}…`), "xxxxx…");
  assert.equal(reasonOf("  Upstash said no  "), "Upstash said no.");
  assert.equal(reasonOf("Upstash answered 503"), "Upstash answered 503.");
});

test("a stuck reason is escaped", () => {
  const page = boxPage({ answer: async () => ({}) });
  const html = page.boxUsageHtml({ data: { connected: false, stuck: [{ id: "r1", why: "<b>bold</b> & more" }] }, error: "", loading: false, open: {} }, NOW);
  assert.ok(!html.includes("<b>"));
  assert.match(html, /&lt;b&gt;bold&lt;\/b&gt; &amp; more/);
});

/** The sidebar's Usage mark, drawn by the code `drawNav` runs, over a stuck list. */
function drawMark(page, stuck) {
  const mark = between(read("gui-sidebar.js"), "const stuck = typeof usageState", '  for (const name of ["needs-you"');
  const el = { hidden: true, title: "" };
  const context = { usageState: { stuck }, stuckMarkTitle: page.stuckMarkTitle, navRow: () => ({ querySelector: () => el }) };
  vm.createContext(context);
  vm.runInContext(mark, context);
  return el;
}

const KNOWN = { id: "fake-row-1", why: "Upstash answered 503" };
const UNCONFIRMED = { id: "fake-row-2", why: "Upstash is not connected", unconfirmed: true };
const CONFIRM = "pinned could not confirm whether a box exists on Upstash for it. Check the Upstash console and delete it there if it does.";

test("a row with no Upstash id says pinned could not confirm a box exists, and does not say it is still running", () => {
  const page = boxPage({ answer: async () => ({}) });
  const html = (stuck, connected = false) => page.boxUsageHtml({ data: { connected, stuck }, error: "", loading: false, open: {} }, NOW);
  const notice = text(html([UNCONFIRMED]));
  assert.ok(notice.includes(`1 box pinned could not end: Upstash is not connected. ${CONFIRM}`), notice);
  assert.doesNotMatch(notice, /still running/);
  assert.equal(html([UNCONFIRMED]).match(/limit-notices/g).length, 1);
  const many = text(html([UNCONFIRMED, { ...UNCONFIRMED, id: "fake-row-3" }]));
  assert.match(many, /2 boxes pinned could not end: .* for any of them\. Check the Upstash console and delete any you find there\./);
  assert.doesNotMatch(many, /still running/);
});

test("a row with its Upstash id keeps saying it is still running on Upstash", () => {
  const page = boxPage({ answer: async () => ({}) });
  const known = text(page.boxUsageHtml({ data: { connected: false, stuck: [KNOWN] }, error: "", loading: false, open: {} }, NOW));
  assert.match(known, /1 box pinned could not delete: Upstash answered 503\. It is still running, and may still be billed, on Upstash\. It can be deleted in the Upstash console\./);
  assert.doesNotMatch(known, /could not confirm/);
});

test("known and unconfirmed boxes with the same reason show as two notices, each with its own text", () => {
  const page = boxPage({ answer: async () => ({}) });
  const same = { ...UNCONFIRMED, why: KNOWN.why };
  const html = page.boxUsageHtml({ data: { connected: true, stuck: [same, KNOWN, { ...KNOWN, id: "fake-row-4" }], boxes: [], upstash: { cpu_hours: 0, compute_usd: 0, boxes: 0 }, pinned: PINNED }, error: "", loading: false, open: {} }, NOW);
  assert.equal(html.match(/limit-notices/g).length, 2);
  assert.match(text(html), /2 boxes pinned could not delete: Upstash answered 503\. They are still running, and may still be billed, on Upstash\. They can be deleted in the Upstash console\./);
  assert.match(text(html), new RegExp(`1 box pinned could not end: Upstash answered 503\. ${CONFIRM.replace(/\./g, "\\.")}`));
});

test("each notice lists which boxes: the Upstash id of a known row, the label to look for for an unconfirmed one, and a long list stops after a few", () => {
  const page = boxPage({ answer: async () => ({}) });
  const html = (stuck) => page.boxUsageHtml({ data: { connected: false, stuck }, error: "", loading: false, open: {} }, NOW);
  const two = html([{ ...KNOWN, upstash: "box_fake_1" }, { ...KNOWN, id: "fake-row-5", upstash: "box_fake_2" }]);
  assert.match(two, /<p class="stuck-ids muted">Upstash ids: <code>box_fake_1<\/code>, <code>box_fake_2<\/code><\/p>/);
  assert.match(html([{ ...KNOWN, upstash: "box_fake_1" }]), /Upstash id: <code>box_fake_1<\/code>/);
  const unconfirmed = html([{ ...UNCONFIRMED, label: "p:fake-row-2" }]);
  assert.match(unconfirmed, /<p class="stuck-ids muted">Look for: <code>p:fake-row-2<\/code><\/p>/);
  assert.doesNotMatch(unconfirmed, /Upstash id/);
  // A known row found through its label has no id: it is listed by the label.
  assert.match(html([{ ...KNOWN, label: "p:fake-row-1" }]), /Look for: <code>p:fake-row-1<\/code>/);
  // Each notice lists only its own boxes.
  const both = html([{ ...KNOWN, upstash: "box_fake_1" }, { ...UNCONFIRMED, why: KNOWN.why, label: "p:fake-row-2" }]);
  assert.equal(both.match(/limit-notices/g).length, 2);
  const [first, second] = both.split('class="limit-notices"').slice(1);
  assert.ok(first.includes("box_fake_1") && !first.includes("p:fake-row-2"));
  assert.ok(second.includes("p:fake-row-2") && !second.includes("box_fake_1"));
  // More than three: the first three and a count.
  const many = html([1, 2, 3, 4, 5].map((n) => ({ ...KNOWN, id: `fake-row-${n}`, upstash: `box_fake_${n}` })));
  assert.match(many, /Upstash ids: <code>box_fake_1<\/code>, <code>box_fake_2<\/code>, <code>box_fake_3<\/code> and 2 more<\/p>/);
  assert.doesNotMatch(many, /box_fake_4|box_fake_5/);
  // An older hub sends no id: nothing is listed, and the notice is as before.
  assert.doesNotMatch(html([KNOWN]), /stuck-ids/);
  // The ids are escaped.
  assert.match(html([{ ...KNOWN, upstash: "<i>x</i>" }]), /<code>&lt;i&gt;x&lt;\/i&gt;<\/code>/);
});

test("when Upstash is not connected the notice says reconnecting the Upstash key lets pinned delete or try to end it on its next pass, and no other reason says it", () => {
  const page = boxPage({ answer: async () => ({}) });
  const notices = (stuck) => text(page.boxUsageHtml({ data: { connected: false, stuck }, error: "", loading: false, open: {} }, NOW));
  const known = notices([{ ...KNOWN, why: "Upstash is not connected" }]);
  assert.match(known, /It is still running, and may still be billed, on Upstash\. It can be deleted in the Upstash console\. Reconnecting the Upstash key lets pinned delete it on its next pass\./);
  assert.match(notices([{ ...KNOWN, why: "Upstash is not connected" }, { ...KNOWN, id: "fake-row-5", why: "Upstash is not connected" }]), /Reconnecting the Upstash key lets pinned delete them on its next pass\./);
  assert.ok(notices([UNCONFIRMED]).includes(`${CONFIRM} Reconnecting the Upstash key lets pinned try to end it on its next pass.`), notices([UNCONFIRMED]));
  assert.doesNotMatch(notices([KNOWN]), /Reconnecting/);
  assert.doesNotMatch(notices([{ ...KNOWN, why: "Upstash answered 503" }]), /Reconnecting/);
});

test("the mark's title follows the notice: known, unconfirmed, and both", () => {
  const page = boxPage({ answer: async () => ({}) });
  const known = drawMark(page, [KNOWN]);
  assert.equal(known.hidden, false);
  assert.equal(known.title, "pinned could not delete 1 Upstash box. It is still running, and may still be billed, on Upstash. It can be deleted in the Upstash console. Open Usage.");
  const unconfirmed = drawMark(page, [UNCONFIRMED]);
  assert.equal(unconfirmed.hidden, false, "a row with no Upstash id is not hidden");
  assert.equal(unconfirmed.title, `pinned could not end 1 box. ${CONFIRM} Open Usage.`);
  assert.doesNotMatch(unconfirmed.title, /still running/);
  const two = drawMark(page, [UNCONFIRMED, { ...UNCONFIRMED, id: "fake-row-3" }]);
  assert.equal(two.title, "pinned could not end 2 boxes. pinned could not confirm whether a box exists on Upstash for any of them. Check the Upstash console and delete any you find there. Open Usage.");
  const mix = drawMark(page, [KNOWN, { ...KNOWN, id: "fake-row-4" }, UNCONFIRMED]);
  assert.equal(mix.hidden, false);
  assert.equal(mix.title, `pinned could not delete 2 Upstash boxes. They are still running, and may still be billed, on Upstash. They can be deleted in the Upstash console. Separately, pinned could not end 1 more box. ${CONFIRM} Open Usage.`);
  assert.equal(drawMark(page, []).hidden, true);
});

test("the sidebar's Usage entry is marked from the hub's own look, and redrawn once the answer is in", async () => {
  const calls = [];
  const page = boxPage({ answer: async () => ({ connected: false, stuck: [] }), stuckAnswer: async (m) => (calls.push(m.cmd), [{ id: "fake-box-0001", why: "Upstash answered 503" }]) });
  await page.fetchStuckBoxes();
  assert.deepEqual(calls, ["stuck_boxes"]);
  assert.equal(page.drawn, 1);
  const sidebarSrc = read("gui-sidebar.js");
  const mark = between(sidebarSrc, "const stuck = typeof usageState", '  for (const name of ["needs-you"');
  const draw = (stuck) => {
    const el = { hidden: true, title: "" };
    const context = { usageState: { stuck }, stuckMarkTitle: page.stuckMarkTitle, navRow: () => ({ querySelector: () => el }) };
    vm.createContext(context);
    vm.runInContext(mark, context);
    return el;
  };
  const marked = draw(page.usageState.stuck);
  assert.equal(marked.hidden, false);
  assert.match(marked.title, /could not delete 1 Upstash box\. It is still running, and may still be billed, on Upstash\. It can be deleted in the Upstash console\. Open Usage\./);
  assert.equal(draw([]).hidden, true);
});

test("the background look asks stuck_boxes, never box_usage, and keeps its last answer when a look fails or the hub does not know the call", async () => {
  let next = async () => [{ id: "fake-box-0001", why: "Upstash answered 503" }];
  const page = boxPage({ answer: async () => assert.fail("box_usage is not the background look"), stuckAnswer: () => next() });
  await page.fetchStuckBoxes();
  assert.equal(page.usageState.stuck.length, 1);
  next = async () => { throw new Error("unknown command: stuck_boxes"); };
  await page.fetchStuckBoxes();
  assert.equal(page.usageState.stuck.length, 1, "a failed look keeps the last answer");
  next = async () => ({ error: "not a list" });
  await page.fetchStuckBoxes();
  assert.equal(page.usageState.stuck.length, 1);
  next = async () => [];
  await page.fetchStuckBoxes();
  assert.equal(page.usageState.stuck.length, 0, "an answer with none clears the mark");
  const old = boxPage({ answer: async () => assert.fail("no fallback to box_usage"), stuckAnswer: async () => { throw new Error("unknown command: stuck_boxes"); } });
  await old.fetchStuckBoxes();
  assert.deepEqual(old.usageState.stuck, []);
  assert.equal(old.drawn, 0, "an older hub shows no mark");
  assert.equal(old.renders.length, 0, "and no error");
  const offline = boxPage({ answer: async () => assert.fail("no hub"), hubOn: false });
  await offline.fetchStuckBoxes();
  assert.equal(offline.drawn, 0);
});

test("the Usage page's own answer also feeds the mark, and a failed box_usage still shows the hub's last list", async () => {
  const stuck = [{ id: "fake-box-0002", why: "Upstash answered 500: fake outage" }];
  const page = boxPage({ answer: async () => ({ connected: true, error: "Upstash answered 500", stuck }) });
  await page.fetchBoxUsage();
  assert.deepEqual(page.usageState.stuck, stuck);
  const html = page.boxUsageHtml(page.usageState.box, NOW);
  assert.match(html, /1 box pinned could not delete: Upstash answered 500: fake outage/, "shown also when the Upstash list call failed");
  const failing = boxPage({ answer: async () => { throw new Error("hub said no"); } });
  failing.usageState.stuck = stuck;
  await failing.fetchBoxUsage();
  assert.match(failing.boxUsageHtml(failing.usageState.box, NOW), /could not delete[^]*Could not read Upstash Box usage: hub said no/);
  assert.deepEqual(failing.usageState.stuck, stuck);
});
