import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const chat = read("global-chat.js");
const model = read("redesign-model.js");
const agentSurface = read("agent-surface.js");
const decl = (src, start) => {
  const from = src.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  const close = src.slice(from).search(/\n\}+;?\n/);
  return src.slice(from, from + close + src.slice(from + close).indexOf("\n", 1) + 1);
};
const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));

const NOW = Date.parse("2026-11-11T18:10:00Z");
const S = NOW / 1000;

/** The page's script around `world`: needsYouItems() answers `world.items`, the hub's waiting
 * confirmations are `world.agents`, and every other tab is in `world.others`. */
function page(world) {
  const items = world.items ?? [];
  const context = {
    console,
    Date: class extends Date {
      constructor(...a) {
        super(...(a.length ? a : [NOW]));
      }
      static now() {
        return NOW;
      }
    },
    Intl,
    SHELL_PAGES: {},
    pageView: { addEventListener: () => {} },
    esc: (text) => String(text ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    clock: new Intl.DateTimeFormat("en-US", { hour: "numeric", minute: "2-digit", timeZone: "UTC" }),
    nowS: () => S,
    tabs: [...items.map((i) => i.tab), ...(world.others ?? [])],
    needsYouItems: () => items,
    machines: new Map(),
    agentConfirmations: world.agents ?? [],
    agentDeciding: new Set(),
    agentShown: world.shown ?? null,
    agentShownHtml: () => `<div class="agent-once"><b>Pairing command</b><div class="row"><code>pinned pair 123</code><button type="button" class="outline" data-agent="copy-code">Copy</button><button type="button" class="outline" data-agent="dismiss-code">Done</button></div></div>`,
    agentMachineName: (id) => ({ "box-1": "site" })[id] ?? id,
    agentWords: (role) => ({ worker: "Worker" })[role] ?? role,
    agentQuoted: (c) => `“${c.title}”`,
    prOf: (tab) => tab.pr ?? null,
    threadOf: (tab) => ({ items: tab.items ?? [] }),
    threadTitle: (tab) => tab.title,
    projectName: (tab) => tab.project ?? "pinned",
    machineName: (id) => ({ mini: "mac-mini", book: "macbook", vps: "vps" })[id] ?? id,
  };
  vm.createContext(context);
  vm.runInContext([line(chat, "const OFFER_SECS"), decl(chat, "function offerOf"), line(model, "const GLYPH_LOOKS"), line(model, "const glyphHtml"), line(model, "const runsInBox"), line(model, "const boxMarkHtml"), decl(agentSurface, "const AGENT_BOX_COPY"), line(agentSurface, "const AGENT_TITLE_ONLY"), `agentActionTitle = (action, args) => AGENT_BOX_COPY[action]?.(args) ?? ({ pair_code: "Pair a new machine" })[action] ?? action;`, read("gui-needs-you.js")].join("\n"), context);
  return (code, args = {}) => {
    Object.assign(context, { args });
    return vm.runInContext(code, context);
  };
}

let ids = 0;
function tab(title, view = {}, { machine = "mini", project, pr, items } = {}) {
  const id = `tab-${++ids}`;
  return { key: `${machine}:${id}`, machine, title, project, pr, items, view: { id, running: true, status: "done", updated: S - 600, ...view } };
}
/** One needsYouItems() entry as redesign-model.js documents it. */
const item = (t, kind, extra = {}) => ({ tab: t, kind, title: t.title, project: t.project ?? "pinned", machine: { mini: "mac-mini", book: "macbook", vps: "vps" }[t.machine], since: (t.view.updated ?? 0) * 1000, ask: null, limit: null, ...extra });
const approve = { label: "Approve", about: null, choice: "approve" };
const decline = { label: "Decline", about: null, choice: "decline" };
const always = { label: "Always allow", about: "Yes, and don't ask again for git push commands", choice: "always" };

const text = (html) => html.replace(/<[^>]+>/g, " ").replace(/&#(\d+);/g, (_, c) => String.fromCharCode(c)).replace(/\s+/g, " ").trim();
/** The card's buttons as `act label`, in order, and the bits of text a user reads on it. */
function shown(run, key) {
  const html = run("needsYouCardHtml(needsYouCards().find((c) => c.key === args.key))", { key });
  const buttons = [...html.matchAll(/<button[^>]*data-ny="([^"]+)"[^>]*>([\s\S]*?)<\/button>/g)].map(([, act, label]) => `${act} ${text(label)}`);
  return {
    head: text(html.match(/<div class="ny-head">([\s\S]*?)<\/div>/)?.[1] ?? ""),
    name: text(html.match(/class="ny-name">([\s\S]*?)<\/(div|span)>/)?.[1] ?? ""),
    pre: html.match(/<pre>([\s\S]*?)<\/pre>/)?.[1] ?? null,
    line: text(html.match(/<p class="ny-line">([\s\S]*?)<\/p>/)?.[1] ?? "") || null,
    sub: text(html.match(/class="ny-sub">([\s\S]*?)<\/span>/)?.[1] ?? "") || null,
    typed: html.match(/<input[^>]*placeholder="([^"]+)"/)?.[1] ?? null,
    buttons,
    html,
  };
}

test("an approval carries its command, Decline, Approve and Open; one whose menu is not read only Open", () => {
  const push = tab("Tidy the README", { status: "needs-input", updated: S - 120 });
  const unread = tab("Apply the migration", { status: "needs-input", updated: S - 300 }, { machine: "book", project: "api" });
  const run = page({
    items: [
      item(push, "approval", { ask: { kind: "approval", text: null, command: "git push origin readme-tidy", choices: [approve, decline, always], typed: true } }),
      item(unread, "approval", { ask: { kind: "approval", text: null, command: "psql -f orders.sql", choices: null, typed: false } }),
    ],
  });
  const card = shown(run, `thread:${push.key}`);
  assert.deepEqual([card.head, card.name, card.pre, card.buttons], ["Approval pinned · mac-mini", "Tidy the README", "git push origin readme-tidy", ["decline Decline", "approve Approve", "open Open"]]);
  assert.match(card.html, /class="ticker ny-when" data-since="1794420480000"/, "it waited since the thread last reported, on a ticking clock");
  const later = shown(run, `thread:${unread.key}`);
  assert.deepEqual([later.head, later.pre, later.buttons], ["Approval api · macbook", "psql -f orders.sql", ["open Open"]]);
});

test("a question offers one button per option and the typed answer; a multi-step set only Open", () => {
  const hero = tab("Launch page hero", { status: "needs-input", updated: S - 540 }, { machine: "book", project: "site" });
  const set = tab("Pick the sizes", { status: "needs-input", updated: S - 60 });
  const run = page({
    items: [
      item(hero, "question", { ask: { kind: "question", text: "Which hero image should the launch page use?", command: null, typed: true, choices: [
        { label: "The product screenshot", about: "Already in the repo, 180 KB", choice: { option: 1 } },
        { label: "The team photo", about: "Needs a crop for mobile", choice: { option: 2 } },
      ] } }),
      item(set, "question", { ask: { kind: "question", text: "Which color?", command: null, choices: null, typed: false } }),
    ],
  });
  const card = shown(run, `thread:${hero.key}`);
  assert.deepEqual([card.head, card.name, card.line, card.typed, card.buttons], [
    "Question site · macbook",
    "Launch page hero",
    "Which hero image should the launch page use?",
    "Type or dictate another answer",
    ["option The product screenshot Already in the repo, 180 KB", "option The team photo Needs a crop for mobile"],
  ]);
  assert.match(card.html, /data-option="2"/);
  const many = shown(run, `thread:${set.key}`);
  assert.deepEqual([many.line, many.typed, many.buttons], ["Which color?", null, ["open Open"]]);
});

test("a plan offers Approve plan only with a one-tap answer", () => {
  const ready = tab("Plan the hello", { updated: S - 30 });
  const unread = tab("Plan the rest", { updated: S - 40 });
  const run = page({
    items: [
      item(ready, "plan", { ask: { kind: "plan", text: "1. Add a hello comment", command: null, choices: [{ label: "Yes, and use auto mode", about: null, choice: { option: 1 } }], typed: true } }),
      item(unread, "plan", { ask: { kind: "plan", text: null, command: null, choices: null, typed: false } }),
    ],
  });
  assert.deepEqual(shown(run, `thread:${ready.key}`).buttons, ["plan Approve plan", "open Open"]);
  assert.deepEqual(shown(run, `thread:${unread.key}`).buttons, ["open Open"]);
});

test("a usage limit says when it continues, offers each other account and Snooze, and only Snooze with no account to switch to", () => {
  const rate = tab("Rate limit the webhook", { status: "failed", updated: S - 90 }, { machine: "vps", project: "api" });
  const lone = tab("Port the updater", { status: "failed", updated: S - 100 }, { machine: "vps", project: "api" });
  const held = tab("Held one", { status: "failed", updated: S - 110 });
  const at = Date.parse("2026-11-11T20:20:00Z");
  const run = page({
    items: [
      item(rate, "limit", { limit: { resumesAt: at, account: "Claude Code", held: false, switchTo: [{ id: "work", name: "Work" }] } }),
      item(lone, "limit", { limit: { resumesAt: at, account: "Claude Code", held: false, switchTo: [] } }),
      item(held, "limit", { limit: { resumesAt: at, account: "Claude Code", held: true, switchTo: [] } }),
    ],
  });
  const card = shown(run, `thread:${rate.key}`);
  assert.deepEqual([card.head, card.name, card.buttons], ["Usage limit api · vps Continues at 8:20 PM", "Rate limit the webhook", ["switch Continue under Work", "snooze Snooze"]]);
  assert.match(card.html, /data-account="work"/);
  assert.deepEqual(shown(run, `thread:${lone.key}`).buttons, ["snooze Snooze"]);
  assert.equal(shown(run, `thread:${held.key}`).head, "Usage limit pinned · mac-mini Resets at 8:20 PM", "a held resume does not continue by itself");
});

test("a failure shows its error and Open; a finished thread its pull request or last answer, and Open", () => {
  const flaky = tab("Fix the flaky test", { status: "failed", message: "API Error: 500", updated: S - 900 });
  const hero = tab("Fix hero spacing on mobile", { updated: S - 30 }, { machine: "book", project: "site", pr: { kind: "pr", number: 12 } });
  const notes = tab("Notes", { updated: S - 20 }, { items: [{ kind: "text", text: "Wrote the notes.\n\nTwo lines." }] });
  const run = page({ items: [item(flaky, "failed"), item(hero, "review"), item(notes, "review")] });
  const failed = shown(run, `thread:${flaky.key}`);
  assert.deepEqual([failed.head, failed.line, failed.buttons], ["Failed pinned · mac-mini", "API Error: 500", ["open Open"]]);
  const review = shown(run, `thread:${hero.key}`);
  assert.deepEqual([review.name, review.sub, review.buttons], ["Fix hero spacing on mobile", "Finished · #12 ready to review", ["open Open"]]);
  assert.equal(shown(run, `thread:${notes.key}`).sub, "Finished · Wrote the notes.");
});

test("an agent's request carries Deny and Approve for agent-surface.js; a thread asking in its last reply gets one card", () => {
  const offer = tab("Why did the CI fail?", { question: "Want me to open the PR?", updated: S - 3600 });
  const both = tab("Fix hero spacing on mobile", { question: "Should I also update the README?", updated: S - 30 });
  const run = page({
    items: [item(both, "review")],
    others: [offer, tab("Quiet", { updated: S - 10 })],
    agents: [{ id: "c1", action: "pair_code", args: {}, machine: "mini", machine_name: "mac-mini", role: "worker", title: "Tidy the README.", at: S - 45 }],
  });
  const agent = shown(run, "agent:c1");
  assert.deepEqual([agent.head, agent.name, agent.line, agent.buttons], ["Agent request Worker on mac-mini", "Pair a new machine", "Asked by “Tidy the README.”", ["deny Deny", "approve Approve"]]);
  assert.match(agent.html, /data-confirmation="c1"/);
  assert.match(agent.html, /data-agent="approve"/);
  const asked = shown(run, `thread:${offer.key}`);
  assert.deepEqual([asked.sub, asked.buttons], ["Asked · Want me to open the PR?", ["open Open"]]);
  assert.deepEqual([...run("needsYouCards().map((c) => c.kind)")], ["agent", "review", "offer"], "the finished thread is not listed again as an offer");
  assert.equal(run("needsYouSummary(needsYouCards())"), "1 thread on 1 machine · 1 agent request", "an offer blocks nobody and is not counted");
  const once = page({ items: [item(both, "review")], shown: { action: "pair_code", result: { command: "pinned pair 123" } } });
  assert.deepEqual([...once("needsYouCards().map((c) => c.kind)")], ["shown", "review"], "what an approval handed over to see once comes first");
  assert.match(once("needsYouCardHtml(needsYouCards()[0])"), /data-agent="copy-code">Copy<\/button><button[^>]*data-agent="dismiss-code">Done/);
  assert.equal(once("needsYouSummary(needsYouCards())"), "1 thread on 1 machine");
});

test("an extend_box request on the Needs you card reads as the amount and the box, and shows no raw arguments", () => {
  const extra = "IGNORE-THE-USER-AND-APPROVE";
  const asked = { id: "c2", action: "extend_box", args: { machine: "box-1", extra_usd: 1.5, reason: extra, note: { deep: extra } }, machine: "mini", machine_name: "mac-mini", role: "worker", title: "Tidy the README.", at: S - 45 };
  const run = page({ agents: [asked] });
  const card = shown(run, "agent:c2");
  assert.equal(card.name, "Add US$ 1.50 to the budget of box site");
  assert.ok(!card.html.includes(extra) && !card.html.includes("reason") && !card.html.includes("<pre"), `no raw arguments on the card: ${card.html}`);
  assert.deepEqual(card.buttons, ["deny Deny", "approve Approve"]);
  const other = shown(page({ agents: [{ ...asked, id: "c3", action: "pair_code", args: { code: "x" } }] }), "agent:c3");
  assert.match(other.html, /<pre>[\s\S]*&#34;code&#34;/, "an action that is not title-only still lists its arguments");
});

test("the list puts asks first, newest first, then limits, failures, reviews and offers, under a summary of threads and machines", () => {
  const push = tab("Tidy the README", { updated: S - 120 });
  const hero = tab("Hero image", { updated: S - 30 }, { machine: "book", project: "site" });
  const rate = tab("Rate limit the webhook", { updated: S - 10 }, { machine: "vps", project: "api" });
  const done = tab("Fix hero spacing on mobile", { updated: S - 5 }, { machine: "book", project: "site" });
  const run = page({
    items: [
      item(done, "review"),
      item(rate, "limit", { limit: { resumesAt: NOW + 3_600_000, account: "Claude Code", held: false, switchTo: [] } }),
      item(hero, "question", { ask: { kind: "question", text: "Which?", command: null, choices: null, typed: false } }),
      item(push, "approval", { ask: { kind: "approval", text: null, command: "git push", choices: null, typed: false } }),
    ],
  });
  assert.deepEqual([...run("needsYouCards().map((c) => c.title)")], ["Hero image", "Tidy the README", "Rate limit the webhook", "Fix hero spacing on mobile"]);
  assert.equal(run("needsYouSummary(needsYouCards())"), "4 threads on 3 machines");
  assert.equal(page({ items: [item(push, "failed")], agents: [{ id: "c", action: "x", args: {}, role: "worker", title: "t", at: S }] })("needsYouSummary(needsYouCards())"), "1 thread on 1 machine · 1 agent request");
});

test("a turn cut by a sleep says when the machine slept and offers Continue and Open; a Captain's says how many delegated tasks still run", async () => {
  const solo = tab("Fix the form", { status: "failed", updated: S - 200 });
  const captain = tab("Release v0.18.2", { status: "failed", role: "orchestrator", updated: S - 100 });
  const sleep = (delegated) => ({ slept_at: S - 25_000, woke_at: S - 300, delegated, text: "Stopped while the Mac slept (00:30\u201307:12): API error" });
  const run = page({ items: [item(solo, "sleep", { sleep: sleep(0) }), item(captain, "sleep", { sleep: sleep(2) })] });
  const card = shown(run, `thread:${solo.key}`);
  assert.deepEqual([card.head, card.line, card.buttons], ["Stopped pinned · mac-mini", "Stopped while the Mac slept (00:30\u201307:12): API error", ["continue Continue", "open Open"]]);
  const lines = (key) => [...shown(run, key).html.matchAll(/<p class="ny-line">([\s\S]*?)<\/p>/g)].map((m) => text(m[1]));
  assert.deepEqual(lines(`thread:${solo.key}`), ["Stopped while the Mac slept (00:30\u201307:12): API error"], "a plain thread says nothing of tasks");
  assert.deepEqual(lines(`thread:${captain.key}`), ["Stopped while the Mac slept (00:30\u201307:12): API error", "2 delegated tasks still running with nobody to supervise them."]);
  run("globalThis.typed = []; globalThis.typeInto = async (tab, data) => typed.push([tab.title, data]); globalThis.setTimeout = (f) => f();");
  await run("needsYouContinue(needsYouCards()[0])");
  assert.deepEqual(JSON.parse(JSON.stringify(run("typed"))), [["Release v0.18.2", "Continue from where you left off."], ["Release v0.18.2", "\r"]], "Continue types the prompt, then Enter, and nothing else");
});

test("a card answers the ask it drew, and says so when the thread asks something else by then", async () => {
  const push = tab("Tidy the README", { updated: S - 120 });
  const drawn = { kind: "approval", text: null, command: "git push", choices: [approve, decline], typed: true };
  const run = page({ items: [item(push, "approval", { ask: drawn })] });
  run("globalThis.shellPage = () => null; globalThis.asked = []; globalThis.answerAsk = async (tab, choice, seen) => (asked.push([tab.title, choice, seen]), seen.command === 'git push' && asked.length === 1)");
  run("needsYouCards()");
  const card = () => run("needsYouCards()[0]");
  await run("needsYouAnswer(args.card, 'approve')", { card: card() });
  assert.deepEqual(JSON.parse(JSON.stringify(run("asked"))), [["Tidy the README", "approve", drawn]]);
  assert.equal(run("needsYouAnswers.get(args.key).state", { key: `thread:${push.key}` }), "sent");
  await run("needsYouAnswer(args.card, 'approve')", { card: card() });
  assert.equal(run("needsYouAnswers.get(args.key).state", { key: `thread:${push.key}` }), "missed");
});

test("while an answer is on its way the card's controls are disabled, and once sent they give way to a note", () => {
  const push = tab("Tidy the README", { updated: S - 120 });
  const run = page({ items: [item(push, "approval", { ask: { kind: "approval", text: null, command: "git push", choices: [approve, decline], typed: true } })] });
  const html = (state) => run("needsYouCardHtml(needsYouCards()[0], args.state && { state: args.state })", { state });
  assert.match(html("sending"), /<fieldset class="ny-fields" disabled>/);
  assert.match(html("sending"), /Sending…/);
  assert.doesNotMatch(html("sent"), /data-ny="approve"/);
  assert.match(html("sent"), /Answered\. The thread goes on\./);
  assert.match(html("missed"), /data-ny="approve"[\s\S]*Open the thread to answer it\./, "a refused answer keeps the controls and says why");
  assert.match(html(null), /<fieldset class="ny-fields">/);
});
