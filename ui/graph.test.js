import assert from "node:assert/strict";
import test from "node:test";
import {
  append,
  collisions,
  insertCreateTicket,
  connect,
  disconnect,
  layout,
  move,
  newNode,
  nextCaseId,
  PICKER_TYPES,
  ports,
  remove,
  resetIds,
  splice,
  startingGraph,
  validate,
  COL,
  OUT,
  ROW,
} from "./graph.js";

test("ports on classify branches", () => {
  const node = {
    type: "classify",
    branchOn: "kind",
    questions: [{ name: "kind", ask: "?", choices: [{ id: "a", label: "A" }, { id: "b", label: "B" }] }],
    fallback: { kind: "extra" },
    gate: { kind: "decide" },
  };
  assert.deepEqual(
    ports(node).map((p) => p.id),
    ["a", "b", "fallback", "nowhere"],
  );
});

test("splice inserts node and shifts right column", () => {
  resetIds(0);
  const graph = {
    nodes: [
      { id: "trigger-1", name: "T", position: [0, 0], type: "trigger", watch: { kind: "called" } },
      { id: "talk-2", name: "A", position: [COL, 0], type: "talk", prompt: "" },
      { id: "talk-3", name: "B", position: [COL * 2, 0], type: "talk", prompt: "" },
    ],
    edges: [
      { from: "trigger-1", port: OUT, to: "talk-2" },
      { from: "talk-2", port: OUT, to: "talk-3" },
    ],
  };
  const inserted = newNode("talk", graph);
  inserted.id = "talk-4";
  const after = splice(graph, { from: "trigger-1", port: OUT, to: "talk-2" }, inserted);
  assert.deepEqual(after, {
    nodes: [
      { id: "trigger-1", name: "T", position: [0, 0], type: "trigger", watch: { kind: "called" } },
      { id: "talk-4", name: "talk", position: [COL, 0], type: "talk", prompt: "" },
      { id: "talk-2", name: "A", position: [COL * 2, 0], type: "talk", prompt: "" },
      { id: "talk-3", name: "B", position: [COL * 3, 0], type: "talk", prompt: "" },
    ],
    edges: [
      { from: "trigger-1", port: OUT, to: "talk-4" },
      { from: "talk-4", port: OUT, to: "talk-2" },
      { from: "talk-2", port: OUT, to: "talk-3" },
    ],
  });
});

test("append and connect", () => {
  resetIds(10);
  let graph = {
    nodes: [{ id: "trigger-11", name: "T", position: [0, 0], type: "trigger", watch: { kind: "called" } }],
    edges: [],
  };
  const stop = newNode("stop", graph);
  graph = append(graph, "trigger-11", OUT, stop);
  assert.equal(graph.edges.length, 1);
  graph = connect(graph, "trigger-11", OUT, stop.id);
  assert.equal(graph.edges.length, 2);
  graph = disconnect(graph, graph.edges[1]);
  assert.equal(graph.edges.length, 1);
});

test("remove bridges single-out node", () => {
  const graph = {
    nodes: [
      { id: "a", name: "a", position: [0, 0], type: "talk", prompt: "" },
      { id: "b", name: "b", position: [COL, 0], type: "talk", prompt: "" },
      { id: "c", name: "c", position: [COL * 2, 0], type: "talk", prompt: "" },
    ],
    edges: [
      { from: "a", port: OUT, to: "b" },
      { from: "b", port: OUT, to: "c" },
    ],
  };
  const after = remove(graph, "b");
  assert.deepEqual(after.edges, [{ from: "a", port: OUT, to: "c" }]);
  assert.deepEqual(
    after.nodes.map((n) => n.id),
    ["a", "c"],
  );
});

test("move and validate", () => {
  const graph = {
    nodes: [
      { id: "t", name: "T", position: [0, 0], type: "trigger", watch: { kind: "called" } },
      { id: "x", name: "X", position: [COL, 0], type: "stop" },
    ],
    edges: [{ from: "t", port: OUT, to: "x" }],
  };
  const moved = move(graph, "x", [400, 50]);
  assert.deepEqual(moved.nodes[1].position, [400, 50]);
  assert.equal(validate(moved, () => true), null);
  assert.match(validate({ nodes: [], edges: [] }, () => true), /trigger/);
});

function branchFixture() {
  const ids = ["a", "b", "c", "d"];
  const ys = [-1.5, -0.5, 0.5, 1.5].map((row) => row * ROW);
  return {
    nodes: [
      { id: "trigger-1", name: "Trigger", position: [0, 0], type: "trigger", watch: { kind: "slack-mention", channel: "ops" } },
      {
        id: "classify-1",
        name: "Sort",
        position: [COL, 0],
        type: "classify",
        branchOn: "kind",
        questions: [{ name: "kind", ask: "?", choices: ids.map((id) => ({ id, label: id.toUpperCase() })) }],
        gate: { kind: "direct" },
        fallback: { kind: "none" },
      },
      ...ids.map((id, i) => ({ id: `talk-${id}`, name: id.toUpperCase(), position: [COL * 2, ys[i]], type: "talk", prompt: id })),
    ],
    edges: [
      { from: "trigger-1", port: OUT, to: "classify-1" },
      ...ids.map((id) => ({ from: "classify-1", port: id, to: `talk-${id}` })),
    ],
  };
}

test("layout boxes do not intersect on the branch fixture, after splice, or after append", () => {
  resetIds(0);
  let graph = branchFixture();
  assert.deepEqual(collisions(layout(graph)), []);
  const inserted = newNode("talk", graph);
  graph = splice(graph, { from: "trigger-1", port: OUT, to: "classify-1" }, inserted);
  assert.deepEqual(
    graph.nodes.map((n) => [n.id, n.position]),
    [
      ["trigger-1", [0, 0]],
      [inserted.id, [COL, 0]],
      ["classify-1", [COL * 2, 0]],
      ["talk-a", [COL * 3, -225]],
      ["talk-b", [COL * 3, -75]],
      ["talk-c", [COL * 3, 75]],
      ["talk-d", [COL * 3, 225]],
    ],
  );
  assert.deepEqual(collisions(layout(graph)), []);
  graph = disconnect(graph, graph.edges.find((e) => e.to === "talk-d"));
  const stop = newNode("stop", graph);
  graph = append(graph, "classify-1", "d", stop);
  assert.equal(graph.edges.find((e) => e.port === "d").to, stop.id);
  assert.deepEqual(collisions(layout(graph)), []);
});

test("ports on a switch are its cases, then otherwise", () => {
  const node = {
    type: "switch",
    cases: [
      { id: "no", equals: "no" },
      { id: "later", equals: "later" },
    ],
  };
  assert.deepEqual(ports(node), [
    { id: "no", label: "no" },
    { id: "later", label: "later" },
    { id: OUT, label: "otherwise" },
  ]);
});

test("picker offers switch after classify", () => {
  assert.equal(PICKER_TYPES[PICKER_TYPES.indexOf("classify") + 1], "switch");
});

test("a typed case equals becomes its port id", () => {
  assert.equal(nextCaseId([], "no"), "no");
  assert.equal(nextCaseId([{ id: "no", equals: "no" }], "no"), "no-1");
  assert.equal(nextCaseId([], ""), "case-1");
  assert.equal(nextCaseId([], "out"), "out-1");
  assert.equal(nextCaseId([], "not sure"), "case-1");
});

test("a new switch follows the nearest upstream classify and its first question", () => {
  resetIds(0);
  const graph = {
    nodes: [
      { id: "trigger-1", name: "Trigger", position: [0, 0], type: "trigger", watch: { kind: "called" } },
      { id: "early", name: "Early", position: [COL, 0], type: "classify", questions: [{ name: "kind", ask: "?", choices: [] }] },
      {
        id: "late",
        name: "Late",
        position: [COL * 2, 40],
        type: "classify",
        questions: [
          { name: "agent", ask: "?", choices: [{ id: "no", label: "no" }] },
          { name: "kind", ask: "?", choices: [] },
        ],
      },
      { id: "talk-1", name: "Talk", position: [COL * 3, 40], type: "talk", prompt: "" },
    ],
    edges: [
      { from: "trigger-1", port: OUT, to: "early" },
      { from: "early", port: OUT, to: "late" },
      { from: "late", port: OUT, to: "talk-1" },
    ],
  };
  const fromTalk = newNode("switch", graph, "talk-1");
  assert.equal(fromTalk.classify, "late");
  assert.equal(fromTalk.question, "agent");
  assert.deepEqual(fromTalk.cases, []);
  assert.equal(fromTalk.name, "Stops on agent");
  assert.deepEqual(ports(fromTalk), [{ id: OUT, label: "otherwise" }]);

  const fromEarly = newNode("switch", graph, "early");
  assert.equal(fromEarly.classify, "early");
  assert.equal(fromEarly.question, "kind");
  assert.equal(fromEarly.name, "Stops on kind");

  const rightmost = newNode("switch", graph);
  assert.equal(rightmost.classify, "late");
  assert.equal(rightmost.question, "agent");

  const blank = newNode("switch", { nodes: [{ id: "t", name: "T", position: [0, 0], type: "trigger", watch: { kind: "called" } }], edges: [] }, "t");
  assert.equal(blank.classify, "");
  assert.equal(blank.question, "");
  assert.equal(blank.name, "switch");
  assert.deepEqual(blank.cases, []);
});

test("validate requires a switch to name a classify question", () => {
  const graph = {
    nodes: [
      { id: "t", name: "T", position: [0, 0], type: "trigger", watch: { kind: "called" } },
      { id: "c", name: "Sort", position: [COL, 0], type: "classify", questions: [{ name: "agent", ask: "?", choices: [] }] },
      { id: "s", name: "Stops when agent is no", position: [COL * 2, 0], type: "switch", classify: "missing", question: "agent", cases: [{ id: "no", equals: "no" }] },
    ],
    edges: [
      { from: "t", port: OUT, to: "c" },
      { from: "c", port: OUT, to: "s" },
    ],
  };
  assert.match(validate(graph, () => true), /classify/);
  graph.nodes[2].classify = "talk";
  graph.nodes.push({ id: "talk", name: "Talk", position: [COL * 3, 0], type: "talk", prompt: "" });
  assert.match(validate(graph, () => true), /classify/);
  graph.nodes[2].classify = "c";
  graph.nodes[2].question = "kind";
  assert.match(validate(graph, () => true), /question/);
  graph.nodes[2].question = "agent";
  assert.equal(validate(graph, () => true), null);
  graph.nodes[2].cases = [{ id: "out", equals: "no" }];
  assert.match(validate(graph, () => true), /otherwise/);
  graph.nodes[2].cases = [{ id: "no", equals: "no" }, { id: "no", equals: "NO" }];
  assert.match(validate(graph, () => true), /repeats the port “no”/);
});

test("a switch with three cases stays clear after splice", () => {
  resetIds(0);
  const cases = [
    { id: "no", equals: "no" },
    { id: "later", equals: "later" },
    { id: "never", equals: "never" },
  ];
  const portIds = [...cases.map((item) => item.id), OUT];
  const ys = portIds.map((_, idx) => (idx - (portIds.length - 1) / 2) * ROW);
  let graph = {
    nodes: [
      { id: "trigger-1", name: "Trigger", position: [0, 0], type: "trigger", watch: { kind: "called" } },
      {
        id: "ask",
        name: "Ask",
        position: [COL, 0],
        type: "classify",
        questions: [{ name: "agent", ask: "?", choices: cases.map((item) => ({ id: item.id, label: item.equals })) }],
      },
      {
        id: "s1",
        name: "Stops when agent is no",
        position: [COL * 2, 0],
        type: "switch",
        classify: "ask",
        question: "agent",
        cases,
      },
      ...portIds.map((id, i) => ({
        id: `tail-${id}`,
        name: id,
        position: [COL * 3, ys[i]],
        type: id === OUT ? "talk" : "stop",
        ...(id === OUT ? { prompt: "" } : {}),
      })),
    ],
    edges: [
      { from: "trigger-1", port: OUT, to: "ask" },
      { from: "ask", port: OUT, to: "s1" },
      ...portIds.map((id) => ({ from: "s1", port: id, to: `tail-${id}` })),
    ],
  };
  assert.deepEqual(
    ports(graph.nodes.find((n) => n.id === "s1")).map((p) => p.label),
    ["no", "later", "never", "otherwise"],
  );
  assert.deepEqual(collisions(layout(graph)), []);
  const inserted = newNode("talk", graph);
  graph = splice(graph, { from: "ask", port: OUT, to: "s1" }, inserted);
  assert.deepEqual(graph.nodes.find((n) => n.id === "s1").position, [COL * 3, 0]);
  assert.deepEqual(graph.nodes.find((n) => n.id === "tail-no").position, [COL * 4, -225]);
  assert.deepEqual(graph.nodes.find((n) => n.id === "tail-later").position, [COL * 4, -75]);
  assert.deepEqual(graph.nodes.find((n) => n.id === "tail-never").position, [COL * 4, 75]);
  assert.deepEqual(graph.nodes.find((n) => n.id === "tail-out").position, [COL * 4, 225]);
  assert.deepEqual(collisions(layout(graph)), []);
});

test("a migrated classify then a switch per stopping question does not overlap", () => {
  resetIds(0);
  let graph = {
    nodes: [{ id: "trigger", name: "Trigger", position: [0, 0], type: "trigger", watch: { kind: "called" } }],
    edges: [],
  };
  const classify = {
    id: "ask",
    name: "Ask",
    position: [0, 0],
    type: "classify",
    model: "typesafe-ai/jev",
    instructions: "Sort",
    questions: [
      { name: "kind", ask: "What does it ask for?", choices: [{ id: "bug", label: "bug" }, { id: "critique", label: "critique" }] },
      { name: "agent", ask: "Can a coding agent handle it?", choices: [{ id: "yes", label: "yes" }, { id: "no", label: "no" }] },
    ],
    branchOn: null,
    gate: { kind: "direct" },
    fallback: { kind: "none" },
  };
  graph = append(graph, "trigger", OUT, classify);
  graph = append(graph, "ask", OUT, {
    id: "sw-kind",
    name: "Stops on kind",
    position: [0, 0],
    type: "switch",
    classify: "ask",
    question: "kind",
    cases: [{ id: "critique", equals: "critique" }],
  });
  graph = append(graph, "sw-kind", OUT, {
    id: "sw-agent",
    name: "Stops when agent is no",
    position: [0, 0],
    type: "switch",
    classify: "ask",
    question: "agent",
    cases: [{ id: "no", equals: "no" }],
  });
  graph = append(graph, "sw-kind", "critique", { id: "stop-critique", name: "critique", position: [0, 0], type: "stop" });
  graph = append(graph, "sw-agent", "no", { id: "stop-no", name: "no", position: [0, 0], type: "stop" });
  graph = append(graph, "sw-agent", OUT, { id: "talk", name: "Talk", position: [0, 0], type: "talk", prompt: "" });
  assert.deepEqual(ports(graph.nodes.find((n) => n.id === "ask")), [{ id: OUT, label: "" }]);
  assert.equal(validate(graph, () => true), null);
  assert.deepEqual(collisions(layout(graph)), []);
  assert.deepEqual(
    graph.nodes.map((n) => [n.id, n.position]),
    [
      ["trigger", [0, 0]],
      ["ask", [COL, 0]],
      ["sw-kind", [COL * 2, 0]],
      ["sw-agent", [COL * 3, 75]],
      ["stop-critique", [COL * 3, -75]],
      ["stop-no", [COL * 4, 0]],
      ["talk", [COL * 4, 150]],
    ],
  );
});

test("insertCreateTicket continues, and the backlog preset files onto Backlog then stops", () => {
  const graph = {
    nodes: [{ id: "trigger-1", name: "T", position: [0, 0], type: "trigger", watch: { kind: "called" } }],
    edges: [],
  };
  const open = insertCreateTicket(graph, "trigger-1", OUT, { backlog: false });
  const ticket = open.nodes.find((node) => node.type === "create-ticket");
  assert.equal(ticket.team, "");
  assert.equal(ticket.label, "");
  assert.equal(ticket.column, "");
  assert.equal(ticket.instructions, "");
  assert.equal(open.nodes.some((node) => node.type === "stop"), false);
  assert.deepEqual(open.edges, [{ from: "trigger-1", port: OUT, to: ticket.id }]);

  const backlog = insertCreateTicket(graph, "trigger-1", OUT, { backlog: true });
  const filed = backlog.nodes.find((node) => node.type === "create-ticket");
  const stop = backlog.nodes.find((node) => node.type === "stop");
  assert.equal(filed.column, "Backlog");
  assert.equal(filed.label, "");
  assert.equal(stop.type, "stop");
  assert.deepEqual(backlog.edges, [
    { from: "trigger-1", port: OUT, to: filed.id },
    { from: filed.id, port: OUT, to: stop.id },
  ]);
});

test("newNode skips an id the graph already uses", () => {
  resetIds(0);
  const graph = {
    nodes: [{ id: "talk-1", name: "talk", position: [0, 0], type: "talk", prompt: "" }],
    edges: [],
  };
  assert.equal(newNode("talk", graph).id, "talk-2");
});

test("starting graph is a trigger and a talk, with no migrated classify", () => {
  const graph = startingGraph({
    trigger: {
      kind: "slack-message",
      channel: "ops",
      decide: { model: "typesafe-ai/jev", instructions: "Sort", categories: [{ name: "bug" }, { name: "question" }], fallback: null },
    },
  });
  assert.deepEqual(
    graph.nodes.map((n) => n.type),
    ["trigger", "talk"],
  );
  assert.deepEqual(graph.nodes[0].watch, { kind: "slack-message", channel: "ops" });
  assert.deepEqual(graph.edges, [{ from: "trigger-1", port: OUT, to: "talk-2" }]);
});
