/** Pure graph edits for the workflow editor. No DOM. */

export const COL = 230;
export const TILE = 84;
export const TEXT_W = 150;
/** Label block under the tile, including the gap the canvas leaves above the text. */
export const TEXT_H = 56;
/** Old canvas row. One pitch is the tile plus its label, with a gap so the boxes do not touch. */
export const ROW = 150;
export const OUT = "out";
export const FALLBACK = "fallback";
export const NOWHERE = "nowhere";

const DEFAULT_ASK = {
  model: "typesafe-ai/jev",
  instructions: "Classify the request below for an engineering team. Answer from the text only.",
  questions: [
    {
      name: "kind",
      ask: "What does it ask for?",
      choices: [
        { id: "bug", label: "bug", description: "" },
        { id: "question", label: "question", description: "" },
        { id: "critique", label: "critique", description: "" },
        { id: "enhancement", label: "enhancement", description: "" },
      ],
    },
    {
      name: "agent",
      ask: "Can a coding agent handle it on its own, without a person deciding first?",
      choices: [
        { id: "yes", label: "yes", description: "" },
        { id: "no", label: "no", description: "" },
      ],
    },
  ],
};

let seq = 0;
export function resetIds(forTests = 0) {
  seq = forTests;
}

function newId(prefix) {
  seq += 1;
  return `${prefix}-${seq}`;
}

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function nodeKind(node) {
  return node.type ?? node.kind;
}

function edgeKey(e) {
  return `${e.from}\0${e.port}\0${e.to}`;
}

function findEdgeIndex(graph, edge) {
  const key = edgeKey(edge);
  return graph.edges.findIndex((e) => edgeKey(e) === key);
}

/** Output ports for a node, in draw order. */
export function ports(node) {
  const type = nodeKind(node);
  if (type === "stop") return [];
  if (type === "switch") {
    const list = (node.cases ?? []).map((item) => ({ id: String(item.id), label: item.equals ?? "" }));
    list.push({ id: OUT, label: "otherwise" });
    return list;
  }
  if (type !== "classify") return [{ id: OUT, label: "" }];

  const branchOn = node.branchOn ?? node.branch_on ?? null;
  if (!branchOn) return [{ id: OUT, label: "" }];

  const question = (node.questions ?? []).find((q) => q.name === branchOn);
  const list = [];
  for (const choice of question?.choices ?? []) {
    const id = choice.id?.id ?? choice.id ?? choice.label;
    list.push({ id: String(id), label: choice.label ?? String(id) });
  }
  const fallback = node.fallback ?? { kind: "none" };
  if (fallback.kind === "extra") list.push({ id: FALLBACK, label: "fallback" });
  const gate = node.gate ?? { kind: "direct" };
  if (gate.kind === "decide") list.push({ id: NOWHERE, label: "nowhere" });
  return list;
}

export function nextCaseId(cases, equals = "") {
  const used = new Set((cases ?? []).map((item) => String(item.id)));
  used.add(OUT);
  const text = String(equals ?? "").trim();
  const base = text && !/\s/.test(text) ? text : "";
  if (base && !used.has(base)) return base;
  const stem = base || "case";
  for (let i = 1; i < 1000; i += 1) {
    const id = `${stem}-${i}`;
    if (!used.has(id)) return id;
  }
  return `case-${used.size + 1}`;
}

function nearestClassify(graph, upstreamId) {
  const classifies = graph.nodes.filter((n) => nodeKind(n) === "classify");
  if (!classifies.length) return null;
  if (upstreamId) {
    const incoming = new Map();
    for (const edge of graph.edges) {
      const list = incoming.get(edge.to);
      if (list) list.push(edge.from);
      else incoming.set(edge.to, [edge.from]);
    }
    const seen = new Set();
    const queue = [upstreamId];
    while (queue.length) {
      const id = queue.shift();
      if (!id || seen.has(id)) continue;
      seen.add(id);
      const node = nodeById(graph, id);
      if (node && nodeKind(node) === "classify") return node;
      for (const prev of incoming.get(id) ?? []) queue.push(prev);
    }
    return null;
  }
  return classifies.reduce((best, node) => {
    if (!best) return node;
    const [bx, by] = best.position;
    const [x, y] = node.position;
    if (x !== bx) return x > bx ? node : best;
    if (y !== by) return y < by ? node : best;
    return String(node.id) < String(best.id) ? node : best;
  }, null);
}

export function newNode(type, graph, upstreamId) {
  const id = newId(type);
  const used = new Set(graph.nodes.map((n) => n.name));
  let name = type;
  for (let i = 2; used.has(name); i += 1) name = `${type} ${i}`;

  const base = { id, name, position: [0, 0] };
  switch (type) {
    case "trigger":
      return { ...base, type: "trigger", watch: { kind: "called" } };
    case "classify":
      return {
        ...base,
        type: "classify",
        model: DEFAULT_ASK.model,
        instructions: DEFAULT_ASK.instructions,
        questions: clone(DEFAULT_ASK.questions),
        branchOn: "kind",
        gate: { kind: "direct" },
        fallback: { kind: "none" },
      };
    case "switch": {
      const classify = nearestClassify(graph, upstreamId);
      const question = classify?.questions?.[0]?.name ?? "";
      let switchName = question ? `Stops on ${question}` : "switch";
      if (used.has(switchName)) {
        const stem = switchName;
        for (let i = 2; used.has(switchName); i += 1) switchName = `${stem} ${i}`;
      }
      return { ...base, name: switchName, type: "switch", classify: classify?.id ?? "", question, cases: [] };
    }
    case "talk":
      return { ...base, type: "talk", prompt: "" };
    case "page":
      return { ...base, type: "page", group: "" };
    case "stage-picked":
      return { ...base, type: "stage-picked" };
    case "call":
      return { ...base, type: "call", workflow: "" };
    case "stop":
      return { ...base, type: "stop" };
    default:
      throw new Error(`unknown node type ${type}`);
  }
}

function nodeById(graph, id) {
  return graph.nodes.find((n) => n.id === id);
}

export function nodeHeight(node) {
  const n = ports(node).length;
  return n > 2 ? Math.max(TILE, n * 28 + 16) : TILE;
}

function nodeBox(node) {
  const h = nodeHeight(node);
  const [x, y] = node.position;
  return { id: node.id, left: x - TEXT_W / 2, right: x + TEXT_W / 2, top: y - h / 2, bottom: y + h / 2 + TEXT_H };
}

function intersects(a, b) {
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

/** Boxes in graph coordinates. `bottom` includes the label under the tile. */
export function layout(graph) {
  return graph.nodes.map(nodeBox);
}

/** Pairs of node ids whose boxes intersect, labels included. */
export function collisions(boxes) {
  const hits = [];
  for (let i = 0; i < boxes.length; i += 1) {
    for (let j = i + 1; j < boxes.length; j += 1) {
      if (intersects(boxes[i], boxes[j])) hits.push([boxes[i].id, boxes[j].id]);
    }
  }
  return hits;
}

function fanY(node, portId) {
  const list = ports(node);
  const n = Math.max(1, list.length);
  const idx = Math.max(0, list.findIndex((p) => p.id === portId));
  return node.position[1] + (idx - (n - 1) / 2) * ROW;
}

function clearY(graph, node) {
  const [x, prefer] = node.position;
  const blocked = graph.nodes.filter((n) => Math.abs(n.position[0] - x) < TEXT_W).map(nodeBox);
  for (let i = 0; i < 48; i += 1) {
    const y = prefer + (i === 0 ? 0 : (i % 2 ? 1 : -1) * Math.ceil(i / 2) * ROW);
    const box = nodeBox({ ...node, position: [x, y] });
    if (!blocked.some((b) => intersects(box, b))) return y;
  }
  return prefer + 48 * ROW;
}

/** Insert `newNode` on `edge`. The new node takes the target's column. Every node at or right of that column moves one column right. */
export function splice(graph, edge, newNode) {
  const idx = findEdgeIndex(graph, edge);
  if (idx < 0) throw new Error("edge not found");
  const from = nodeById(graph, edge.from);
  const to = nodeById(graph, edge.to);
  if (!from || !to) throw new Error("edge endpoints missing");

  const insertX = to.position[0];
  const shifted = graph.nodes.map((n) => (n.position[0] >= insertX ? { ...n, position: [n.position[0] + COL, n.position[1]] } : n));
  const placed = { ...newNode, position: [insertX, from.position[1]] };
  placed.position = [insertX, clearY({ nodes: shifted }, placed)];
  const nodes = [...shifted, placed].sort((a, b) => a.position[0] - b.position[0] || String(a.id).localeCompare(String(b.id)));
  const outPort = ports(placed)[0]?.id ?? OUT;
  const edges = graph.edges.slice();
  edges.splice(idx, 1, { from: edge.from, port: edge.port, to: placed.id }, { from: placed.id, port: outPort, to: edge.to });
  return { nodes, edges };
}

/** Connect a dangling port to a new node one column to the right, on a row that does not cover a neighbor. */
export function append(graph, fromNode, port, newNode) {
  const from = nodeById(graph, fromNode);
  if (!from) throw new Error("from node missing");
  const x = from.position[0] + COL;
  const placed = { ...newNode, position: [x, fanY(from, port)] };
  placed.position = [x, clearY(graph, placed)];
  return {
    nodes: [...graph.nodes, placed],
    edges: [...graph.edges, { from: fromNode, port, to: placed.id }],
  };
}

export function connect(graph, from, port, to) {
  if (!nodeById(graph, from) || !nodeById(graph, to)) throw new Error("unknown node");
  if (!ports(nodeById(graph, from)).some((p) => p.id === port)) throw new Error("invalid port");
  return { ...graph, edges: [...graph.edges, { from, port, to }] };
}

export function disconnect(graph, edge) {
  const idx = findEdgeIndex(graph, edge);
  if (idx < 0) return graph;
  const edges = graph.edges.slice();
  edges.splice(idx, 1);
  return { ...graph, edges };
}

export function move(graph, nodeId, position) {
  return {
    ...graph,
    nodes: graph.nodes.map((n) => (n.id === nodeId ? { ...n, position: [position[0], position[1]] } : n)),
  };
}

export function remove(graph, nodeId) {
  const node = nodeById(graph, nodeId);
  if (!node) return graph;
  const outPorts = ports(node);
  const incoming = graph.edges.filter((e) => e.to === nodeId);
  const outgoing = graph.edges.filter((e) => e.from === nodeId);
  let edges = graph.edges.filter((e) => e.from !== nodeId && e.to !== nodeId);

  if (outPorts.length === 1 && outPorts[0].id === OUT && outgoing.length) {
    const outTargets = outgoing.filter((e) => e.port === OUT);
    for (const inn of incoming) {
      for (const out of outTargets) edges.push({ from: inn.from, port: inn.port, to: out.to });
    }
  }

  return { nodes: graph.nodes.filter((n) => n.id !== nodeId), edges };
}

function triggers(graph) {
  return graph.nodes.filter((n) => nodeKind(n) === "trigger");
}

/** @param { (workflowId: string) => boolean } called */
export function validate(graph, called) {
  const ids = new Set();
  for (const n of graph.nodes) {
    if (ids.has(n.id)) return `Duplicate node id “${n.id}”.`;
    ids.add(n.id);
  }

  const triggers = graph.nodes.filter((n) => nodeKind(n) === "trigger");
  if (triggers.length !== 1) return triggers.length ? "Only one trigger node is allowed." : "Add a trigger node.";
  const trigger = triggers[0];

  for (const e of graph.edges) {
    if (e.to === trigger.id) return "Nothing may connect into the trigger.";
    const from = nodeById(graph, e.from);
    const to = nodeById(graph, e.to);
    if (!from) return `Edge from unknown node “${e.from}”.`;
    if (!to) return `Edge to unknown node “${e.to}”.`;
    if (!ports(from).some((p) => p.id === e.port)) return `Invalid port “${e.port}” on “${from.name}”.`;
  }

  const visiting = new Set();
  const stack = new Set();
  const callTarget = (id) => {
    const n = nodeById(graph, id);
    return n && nodeKind(n) === "call" ? n.workflow : null;
  };

  function walk(nodeId) {
    if (stack.has(nodeId)) return "Call workflows must not form a cycle.";
    if (visiting.has(nodeId)) return null;
    visiting.add(nodeId);
    stack.add(nodeId);
    for (const e of graph.edges.filter((x) => x.from === nodeId)) {
      const wf = callTarget(e.to);
      if (wf) {
        if (!called(wf)) return `“${nodeById(graph, nodeId).name}” calls a missing workflow.`;
        const err = walkCall(wf);
        if (err) return err;
      } else {
        const err = walk(e.to);
        if (err) return err;
      }
    }
    stack.delete(nodeId);
    return null;
  }

  function walkCall(workflowId) {
    if (stack.has(`wf:${workflowId}`)) return "Call workflows must not form a cycle.";
    stack.add(`wf:${workflowId}`);
    stack.delete(`wf:${workflowId}`);
    return null;
  }

  for (const n of graph.nodes) {
    if (nodeKind(n) === "call" && n.workflow) {
      if (!called(n.workflow)) return `“${n.name}” calls a missing workflow.`;
    }
    const problem = switchProblem(graph, n);
    if (problem) return problem;
  }

  return walk(trigger.id) ?? null;
}

export function switchProblem(graph, node) {
  if (nodeKind(node) !== "switch") return null;
  const target = nodeById(graph, node.classify);
  if (!target || nodeKind(target) !== "classify") return `“${node.name}” must follow a classify node.`;
  const asked = (target.questions ?? []).some((q) => q.name && q.name === node.question);
  if (!asked) return `“${node.name}” must use a question on “${target.name}”.`;
  const seen = new Set();
  for (const item of node.cases ?? []) {
    const id = String(item?.id ?? "");
    if (!id || id === OUT) return `“${node.name}” needs a port name other than otherwise.`;
    if (seen.has(id)) return `“${node.name}” repeats the port “${id}”.`;
    seen.add(id);
  }
  return null;
}

function watchOf(trigger) {
  const kind = trigger?.kind ?? "called";
  if (kind === "linear-label") return { kind, label: trigger.label ?? "" };
  if (kind === "slack-mention" || kind === "slack-message") return { kind, channel: trigger.channel ?? "" };
  return { kind: "called" };
}

/** One trigger of the chosen kind and one talk. A connector without `graph` is not guessed into this. */
export function startingGraph(connector) {
  resetIds(0);
  const tid = newId("trigger");
  const talkId = newId("talk");
  return {
    nodes: [
      { id: tid, name: "Trigger", position: [0, 0], type: "trigger", watch: watchOf(connector?.trigger) },
      { id: talkId, name: "Run session", position: [COL, 0], type: "talk", prompt: "" },
    ],
    edges: [{ from: tid, port: OUT, to: talkId }],
  };
}

export const PICKER_TYPES = ["classify", "switch", "talk", "page", "stage-picked", "call", "stop"];
