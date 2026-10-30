import assert from "node:assert/strict";
import test from "node:test";
import {
  append,
  collisions,
  connect,
  disconnect,
  layout,
  move,
  newNode,
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
