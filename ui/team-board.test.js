import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const flush = async () => {
  for (let i = 0; i < 6; i++) await new Promise((resolve) => setImmediate(resolve));
};

const TEAM = { id: "t1", name: "Acme", role: "owner", credential: {} };
const profile = (name, model, effort, access) => ({ name, model: { model }, effort, access, skills: [], tools: [], delivers: "", by: "user", prompt: "" });
const SETUP = {
  profiles: [profile("Implementer", "claude-opus-5-5", "high", "edit"), profile("Tester", "claude-sonnet-5-5", "high", "read_only"), profile("Judge", "claude-opus-5-5", "xhigh", "read_only")],
  builder: { profile: "Implementer", bot: "w-build", soul: "" },
  reviewer: { profile: "Tester", bot: "w-review", soul: "" },
  approver: { profile: "Judge", soul: "" },
  merge_bar: 90,
  max_rounds: 3,
};
const card = (over) => ({ id: "k1", title: "Fix Safari export", brief: "", project: "github.com/acme/site", column: "backlog", turn: null, run: null, person: null, round: 0, round_limit: 3, pr: null, requests: [], report: "", verdict: null, log: [], created_by: "a1", created_at: 1, ...over });

/** The board page over a fake hub: `cards` is what `board` answers, and every op is kept in `sent`. */
function board(cards, setup = SETUP) {
  const sent = [];
  const listeners = {};
  const docListeners = {};
  const view = { dataset: {}, innerHTML: "", contains: () => false };
  const context = {
    console,
    structuredClone,
    document: { body: { dataset: { page: "team-board" } }, activeElement: null, addEventListener: (type, fn) => (docListeners[type] = fn) },
    SHELL_PAGES: {},
    pageView: { addEventListener: (type, fn) => (listeners[type] = fn), querySelector: () => view },
    shellPage: () => "team-board",
    setPage() {},
    headHtml: (title) => `<h1>${title}</h1>`,
    esc: (v) => String(v ?? "").replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`),
    account: { email: "ana@acme.dev" },
    teamsCurrent: () => TEAM,
    teamCall: async (op) => {
      sent.push(op);
      if (op.op === "board") return { setup, cards };
      if (op.op === "bots") return [{ workflow: "w-build", name: "Builder bot" }, { workflow: "w-review", name: "Reviewer bot" }];
      if (op.op === "members") return [{ account: "a1", email: "ana@acme.dev", role: "owner", groups: [] }];
      if (op.op === "card_act") return { ...cards.find((c) => c.id === op.card), column: "ready_to_dev", turn: "builder" };
      return [];
    },
  };
  vm.createContext(context);
  for (const name of ["gui-team-store.js", "gui-team-board.js"]) vm.runInContext(read(name), context);
  const show = async () => {
    context.SHELL_PAGES["team-board"].show();
    await flush();
  };
  const click = async (act, id) => {
    const button = { dataset: { wbAct: act }, disabled: false, closest: () => ({ dataset: { card: id } }) };
    listeners.click({ target: { closest: (sel) => (sel === "[data-wb-act]" ? button : null) } });
    await flush();
  };
  const push = async (event) => {
    docListeners["teams:event"]({ detail: { team: "t1", event } });
    await flush();
  };
  return { html: () => view.innerHTML, show, click, push, sent };
}

const column = (html, name) => html.split(`data-column="${name}"`)[1].split("</section>")[0];

test("each queue names the agent, worker profile and model that picks it up, and a person for deploy", async () => {
  const b = board([card({})]);
  await b.show();
  const html = b.html();
  assert.match(column(html, "ready_to_dev"), /Picked up by <b>Builder<\/b><span class="wb-profile" title="Worker profile">Implementer · claude-opus-5-5, high<\/span><span class="wb-bot">bot Builder bot<\/span>/);
  assert.match(column(html, "ready_to_review"), /Picked up by <b>Reviewer<\/b>.*Tester · claude-sonnet-5-5, high/);
  assert.match(column(html, "in_review"), /Worked by <b>Reviewer<\/b>.*Tester · claude-sonnet-5-5, high/);
  assert.match(column(html, "ready_to_deploy"), /Picked up by <b>a person<\/b>, who merges the PR/, "no Approver sits");
  assert.match(column(html, "approving"), /Worked by <b>Approver<\/b><span class="wb-rule">Scores the merge: at 90 or more pinned merges the PR, below it a person decides<\/span>.*Judge · claude-opus-5-5, xhigh.*no bot seated/);
  assert.match(column(html, "backlog"), /Fix Safari export/);
});

test("with an Approver seated, Ready to deploy is the Approver's queue", async () => {
  const b = board([], { ...SETUP, approver: { ...SETUP.approver, bot: "w-review" } });
  await b.show();
  assert.match(column(b.html(), "ready_to_deploy"), /Picked up by <b>Approver<\/b><span class="wb-rule">Scores the merge.*Judge · claude-opus-5-5, xhigh.*bot Reviewer bot/);
});

test("queueing a backlog card sends the act and moves it to Ready to dev", async () => {
  const b = board([card({})]);
  await b.show();
  await b.click("queue", "k1");
  assert.deepEqual(JSON.parse(JSON.stringify(b.sent.find((op) => op.op === "card_act"))), { op: "card_act", card: "k1", act: { act: "queue" } });
  assert.match(column(b.html(), "ready_to_dev"), /Fix Safari export/);
  assert.doesNotMatch(column(b.html(), "backlog"), /Fix Safari export/);
});

test("a card the Approver did not approve shows its PR, round, score and why it waits on a person in Ready to deploy", async () => {
  const b = board([]);
  await b.show();
  const pr = { number: 12, url: "https://github.com/acme/site/pull/12", branch: "card/k1" };
  await b.push({ kind: "card_changed", card: card({ column: "ready_to_deploy", turn: "approver", round: 2, pr, verdict: { score: 80, bar: 90, reason: "touches sign-in", hints: [], at: 2 }, person: "The Approver scored 80/90 and did not approve the merge: touches sign-in" }) });
  const html = column(b.html(), "ready_to_deploy");
  assert.match(html, /PR #12/);
  assert.match(html, /round 2 of 3/);
  assert.match(html, /80\/90/);
  assert.match(html, /did not approve the merge/);
  assert.match(html, /data-wb-act="merged"[^>]*>Merge!</);
  assert.match(html, /data-wb-act="send_back"/);
});

test("index.html loads gui-team-board.js and its styles", () => {
  const html = read("index.html");
  assert.ok(html.includes('<script src="gui-team-board.js"></script>'));
  assert.ok(html.includes('<link rel="stylesheet" href="gui-team-board.css">'));
});
