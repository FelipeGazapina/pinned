import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const context = {};
vm.createContext(context);
for (const file of ["composer-tags.js", "requests-model.js"]) {
  vm.runInContext(readFileSync(new URL(`./${file}`, import.meta.url), "utf8"), context);
}

// Objects built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => (value === undefined ? value : JSON.parse(JSON.stringify(value)));
const call = (name) => (...args) => here(context[name](...args));
const requestState = call("requestState");
const requestCounts = call("requestCounts");
const requestFilter = call("requestFilter");
const closeTo = call("closeTo");
const requestPlans = call("requestPlans");
const requestProgress = call("requestProgress");
const requestUnread = call("requestUnread");

const glance = (id, over = {}) => ({
  id,
  title: `Request ${id}`,
  project: "Pinned",
  opened_at: 1000,
  kind: "asked",
  closed_at: null,
  needs: "",
  needs_at: null,
  last_user_at: null,
  last_who: null,
  last_text: "",
  last_at: null,
  replies: 0,
  captains: [],
  tab: null,
  ...over,
});
const plan = (id, over = {}) => ({ id, title: id, state: "working", done: 1, total: 2, strips: [], ...over });

test("a request with no ask and no closing is working", () => {
  assert.equal(requestState(glance("q1")), "working");
});

test("closed_at makes a request done, even while it still has a need", () => {
  const g = glance("q1", { closed_at: 5000, needs: "Pick a database", needs_at: 4000 });
  assert.equal(requestState(g, { tabWaits: true, planGlances: [plan("p1", { state: "needs_you" })] }), "done");
});

test("a need that nothing after it answered makes the request need you", () => {
  assert.equal(requestState(glance("q1", { needs: "Pick a database", needs_at: 4000 })), "needs");
  assert.equal(requestState(glance("q1", { needs: "Pick a database", needs_at: 4000, last_user_at: 3000 })), "needs");
  assert.equal(requestState(glance("q1", { needs: "Pick a database", needs_at: 4000, last_user_at: 4000 })), "needs");
});

test("a need is stale once the user wrote after it", () => {
  assert.equal(requestState(glance("q1", { needs: "Pick a database", needs_at: 4000, last_user_at: 4001 })), "working");
});

test("a plan that needs you makes the request need you, and wins over a stale need", () => {
  const needsYou = [plan("p1", { state: "needs_you" })];
  assert.equal(requestState(glance("q1"), { planGlances: needsYou }), "needs");
  assert.equal(requestState(glance("q1", { needs: "Old ask", needs_at: 1, last_user_at: 2 }), { planGlances: needsYou }), "needs");
  assert.equal(requestState(glance("q1"), { planGlances: [plan("p1"), plan("p2", { state: "done" })] }), "working");
});

test("a request tab that waits on input makes the request need you", () => {
  assert.equal(requestState(glance("q1"), { tabWaits: true }), "needs");
  assert.equal(requestState(glance("q1"), { tabWaits: false }), "working");
});

const board = [
  glance("working-old", { opened_at: 100 }),
  glance("done-new", { opened_at: 50, closed_at: 900, last_who: "orchestrator", last_text: "Shipped", last_at: 900 }),
  glance("needs-old", { opened_at: 10, needs: "Approve the plan", needs_at: 20 }),
  glance("working-new", { opened_at: 1, last_who: "captain", last_text: "Tests green", last_at: 700 }),
  glance("needs-new", { opened_at: 5, last_who: "captain", last_text: "Waiting", last_at: 800 }),
  glance("done-old", { opened_at: 30, closed_at: 40 }),
];
const ctx = (g) => (g.id === "needs-new" ? { tabWaits: true } : {});

test("requestCounts counts every request by state", () => {
  assert.deepEqual(requestCounts(board, ctx), { all: 6, needs: 2, working: 2, done: 2 });
  assert.deepEqual(requestCounts([]), { all: 0, needs: 0, working: 0, done: 0 });
});

test("requestFilter all lists needs first, then working, then done, newest activity first in each", () => {
  assert.deepEqual(requestFilter(board, "all", ctx).map((g) => g.id), [
    "needs-new",
    "needs-old",
    "working-new",
    "working-old",
    "done-new",
    "done-old",
  ]);
});

test("requestFilter narrows to one state and keeps the order", () => {
  assert.deepEqual(requestFilter(board, "needs", ctx).map((g) => g.id), ["needs-new", "needs-old"]);
  assert.deepEqual(requestFilter(board, "working", ctx).map((g) => g.id), ["working-new", "working-old"]);
  assert.deepEqual(requestFilter(board, "done", ctx).map((g) => g.id), ["done-new", "done-old"]);
});

test("requestFilter does not reorder the list it was given", () => {
  const before = board.map((g) => g.id);
  requestFilter(board, "all", ctx);
  assert.deepEqual(board.map((g) => g.id), before);
});

test("CLOSE_TO is 0.4", () => {
  assert.equal(vm.runInContext("CLOSE_TO", context), 0.4);
});

test("closeTo matches a paragraph to an open request of the same project", () => {
  const open = [glance("q1", { title: "Migracao do banco de dados" }), glance("q2", { title: "Redesenhar a pagina inicial" })];
  assert.equal(closeTo("#pinned Preciso da migracao do banco de dados rodando", open).id, "q1");
});

test("closeTo folds accents: migração matches migracao", () => {
  const open = [glance("q1", { title: "Migracao banco" })];
  assert.equal(closeTo("#pinned migração banco", open).id, "q1");
  assert.equal(closeTo("#pinned MIGRAÇÃO BANCO", open).id, "q1");
});

test("closeTo reads the last line of the thread as well as its title", () => {
  const open = [glance("q1", { title: "Ajustes", last_who: "captain", last_text: "Provedor autenticacao concluido", last_at: 5 })];
  assert.equal(closeTo("#pinned provedor autenticacao", open).id, "q1");
});

test("closeTo ignores a closed request", () => {
  const open = [glance("q1", { title: "Migracao banco", closed_at: 9 })];
  assert.equal(closeTo("#pinned migracao banco", open), null);
});

test("closeTo ignores a request of another project and a paragraph with no project", () => {
  const open = [glance("q1", { title: "Migracao banco", project: "Site" })];
  assert.equal(closeTo("#pinned migracao banco", open), null);
  assert.equal(closeTo("migracao banco", [glance("q2", { title: "Migracao banco" })]), null);
  assert.equal(closeTo("#pinned migracao banco", [glance("q3", { title: "Migracao banco", project: null })]), null);
});

test("closeTo takes the project from the tag by slug: Pinned and #pinned are one project", () => {
  assert.equal(closeTo("#my-app migracao banco", [glance("q1", { title: "Migracao banco", project: "My App" })]).id, "q1");
});

test("closeTo wants a Jaccard score of 0.4: 2 of 5 words is a match, 1 of 7 is not", () => {
  const exact = [glance("q1", { title: "aaaa bbbb dddd eeee" })];
  assert.equal(closeTo("#pinned aaaa bbbb cccc", exact).id, "q1");
  const far = [glance("q2", { title: "aaaa delta echoes foxtrot golfs" })];
  assert.equal(closeTo("#pinned aaaa bbbb cccc", far), null);
});

test("closeTo keeps words of 4 letters or more only", () => {
  const open = [glance("q1", { title: "fix the bug now" })];
  assert.equal(closeTo("#pinned fix the bug now", open), null);
});

test("closeTo returns the best match, and the first of equal scores", () => {
  const open = [
    glance("q1", { title: "migracao banco usuarios" }),
    glance("q2", { title: "migracao banco" }),
    glance("q3", { title: "migracao banco" }),
  ];
  assert.equal(closeTo("#pinned migracao banco", open).id, "q2");
  assert.equal(closeTo("#pinned migracao banco", []), null);
});

const orqTab = { id: "t-orq", cwd: "/orq", request: "q1", plans: [plan("p2"), plan("p1", { requests: ["q1"] })] };
const macCaptain = { id: "t-cap", cwd: "/repo/pinned", plans: [plan("pc", { requests: ["q1"] }), plan("px", { requests: ["q9"] }), plan("py")] };
const miniCaptain = { id: "t-cap2", cwd: "/srv/pinned", plans: [plan("pm", { requests: ["q1", "q1"] })] };
const stray = { id: "t-stray", cwd: "/elsewhere", plans: [plan("ps", { requests: ["q1"] })] };
const flat = (rows) => rows.map((r) => `${r.machine}/${r.tab.id}/${r.plan.id}`);

test("requestPlans lists the request's own tab first, then each Captain in captains order, then by plan id", () => {
  const g = glance("q1", {
    captains: [
      { machine: "mini", path: "/srv/pinned", project: "Pinned", machine_name: "Mac mini" },
      { machine: "mac", path: "/repo/pinned", project: "Pinned", machine_name: "MacBook" },
    ],
  });
  const tabs = { mac: [macCaptain, orqTab, stray], mini: [miniCaptain] };
  assert.deepEqual(flat(requestPlans(g, tabs)), [
    "mac/t-orq/p1",
    "mac/t-orq/p2",
    "mini/t-cap2/pm",
    "mac/t-cap/pc",
    "mac/t-stray/ps",
  ]);
});

test("requestPlans counts a plan once, however many ways it names the request", () => {
  const g = glance("q1");
  const rows = requestPlans(g, { mac: [orqTab, miniCaptain] });
  assert.deepEqual(flat(rows), ["mac/t-orq/p1", "mac/t-orq/p2", "mac/t-cap2/pm"]);
});

test("requestPlans gives two machines for two Captains and keeps the entry's tab and plan", () => {
  const g = glance("q1", {
    captains: [
      { machine: "mac", path: "/repo/pinned", project: "Pinned", machine_name: "MacBook" },
      { machine: "mini", path: "/srv/pinned", project: "Pinned", machine_name: "Mac mini" },
    ],
  });
  const rows = requestPlans(g, { mac: [macCaptain], mini: [miniCaptain] });
  assert.deepEqual(rows.map((r) => r.machine), ["mac", "mini"]);
  assert.equal(rows[0].tab.id, "t-cap");
  assert.equal(rows[0].plan.id, "pc");
  assert.equal(rows[1].plan.state, "working");
});

test("requestPlans is empty when nothing names the request", () => {
  assert.deepEqual(requestPlans(glance("q7"), { mac: [macCaptain] }), []);
  assert.deepEqual(requestPlans(glance("q7"), {}), []);
  assert.deepEqual(requestPlans(glance("q7")), []);
});

test("requestProgress sums done and total over the plans", () => {
  assert.deepEqual(requestProgress([{ plan: plan("a", { done: 3, total: 4 }) }, { plan: plan("b", { done: 4, total: 4 }) }]), { done: 7, total: 8 });
  assert.deepEqual(requestProgress([plan("a", { done: 1, total: 5 })]), { done: 1, total: 5 });
  assert.deepEqual(requestProgress([]), { done: 0, total: 0 });
  assert.deepEqual(requestProgress(), { done: 0, total: 0 });
});

test("requestUnread is the replies the user has not seen, never below 0", () => {
  assert.equal(requestUnread({ replies: 5 }, 2), 3);
  assert.equal(requestUnread({ replies: 2 }, 5), 0);
  assert.equal(requestUnread({ replies: 3 }, null), 3);
  assert.equal(requestUnread({ replies: 3 }, undefined), 3);
  assert.equal(requestUnread({ replies: 0 }, 0), 0);
});
