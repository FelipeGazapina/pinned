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
function boxPage({ answer, hubOn = true } = {}) {
  const renders = [];
  const context = {
    esc: (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"),
    icon: (name) => `[${name}]`,
    shellPage: () => "usage",
    renderUsagePage: () => renders.push(context.boxUsageHtml(context.usageState.box, NOW)),
    usageState: { box: { data: null, error: "", loading: false, seq: 0 } },
    hub: hubOn ? { hub: async (message) => (assert.equal(JSON.stringify(message), '{"cmd":"box_usage"}'), answer()) } : null,
    pageView: { addEventListener: (type, handler) => (context.toggled = { type, handler }) },
    renders,
  };
  vm.createContext(context);
  vm.runInContext(`${line(usageSrc, "const usd = ")}\n${line(read("settings.js"), "const BOX_SIZES = ")}\n${sectionSrc}
    this.boxUsageHtml = boxUsageHtml; this.fetchBoxUsage = fetchBoxUsage;`, context);
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
  const stuck = [{ id: "r1", why: "Upstash answered 503" }, { id: "r2", why: "Upstash answered 503" }, { id: "r3", why: "still being deleted" }];
  const off = html({ connected: false, stuck });
  assert.match(off, /2 boxes pinned could not delete: Upstash answered 503/);
  assert.match(off, /1 box pinned could not delete: still being deleted/);
  assert.match(off, /Connect Upstash in Settings/);
  assert.equal(off.match(/limit-notices/g).length, 2);
  const on = html({ connected: true, stuck: [stuck[2]], boxes: [], upstash: { cpu_hours: 0, compute_usd: 0, boxes: 0 }, pinned: PINNED });
  assert.match(on, /1 box pinned could not delete/);
  assert.doesNotMatch(html({ connected: false }), /could not delete/);
});

test("a stuck reason is escaped", () => {
  const page = boxPage({ answer: async () => ({}) });
  const html = page.boxUsageHtml({ data: { connected: false, stuck: [{ id: "r1", why: "<b>bold</b> & more" }] }, error: "", loading: false, open: {} }, NOW);
  assert.ok(!html.includes("<b>"));
  assert.match(html, /&lt;b&gt;bold&lt;\/b&gt; &amp; more/);
});
