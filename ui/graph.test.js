import assert from "node:assert/strict";
import test from "node:test";
import {
  append,
  connect,
  disconnect,
  move,
  newNode,
  ports,
  remove,
  resetIds,
  splice,
  validate,
  COL,
  OUT,
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
      { id: "talk-4", name: "talk", position: [Math.round(COL / 2), 0], type: "talk", prompt: "" },
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
