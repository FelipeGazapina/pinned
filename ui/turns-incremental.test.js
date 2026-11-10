import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("./gui-timeline.js", import.meta.url), "utf8");
const between = (from, to) => {
  const start = source.indexOf(from);
  const end = source.indexOf(to, start);
  if (start < 0 || end < 0) throw new Error(`missing slice ${from}`);
  return source.slice(start, end);
};

/** `turnsOf` as it was before the turns were extended item by item: the whole transcript, every time. */
const ORACLE = `
const PLAN_TOOL = "ExitPlanMode";
const QUESTION_TOOL = "AskUserQuestion";
/** A page the Artifact tool published to claude.ai: its local file and its link. */
const PUBLISHED = /^Published (.+?) at (https:\\/\\/claude\\.ai\\/(?:code\\/)?artifact\\/[\\w-]+)/;
const published = (step) => (step.kind === "tool" && step.name === "Artifact" && !step.result?.error && step.result?.text.match(PUBLISHED)) || null;
const interruption = (step) => /^\\[Request interrupted/.test(step.result?.text ?? "");

/** Claude writes a manual compaction just before the \`/compact\` prompt that asked for it, which T3 Code
 * shows after that prompt. */
function referenceCompactions(items) {
  const out = [];
  let held = null;
  for (const item of items) {
    if (item.kind === "compaction") {
      if (held) out.push(held);
      held = item;
      continue;
    }
    if (held && (item.kind === "user" || item.kind === "text" || item.kind === "tool")) {
      out.push(...(item.kind === "user" && /^\\/compact\\b/.test(item.text) ? [item, held] : [held, item]));
      held = null;
      continue;
    }
    out.push(item);
  }
  return held ? [...out, held] : out;
}

/** Rules across the lane that mark the conversation itself: a compaction, or the fork it came from. */
const DIVIDERS = new Set(["compaction", "fork"]);

function referenceTurnsOf(items) {
  const turns = [];
  const results = new Map(items.filter((i) => i.kind === "result").map((r) => [r.tool, r]));
  let turn = null;
  for (const item of referenceCompactions(items)) {
    if (item.kind === "result" || item.kind === "model") continue;
    if (item.kind === "user" || item.kind === "steer" || !turn) {
      const user = item.kind === "user" ? item : item.kind === "steer" ? { ...item, kind: "user", images: 0, steer: true } : null;
      turn = { id: item.id ?? \`t\${turns.length}\`, user, steps: [], prs: [], notes: [], dividers: [] };
      turns.push(turn);
      if (user) continue;
    }
    if (item.kind === "pr") turn.prs.some((p) => p.url === item.url) || turn.prs.push(item);
    else if (item.kind === "note") turn.notes.push(item);
    else if (DIVIDERS.has(item.kind)) turn.dividers.push(item);
    else turn.steps.push(item.kind === "tool" ? { ...item, result: results.get(item.id) } : item);
  }
  for (const t of turns) {
    // The output of /compact repeats what its divider says.
    if (t.dividers.some((d) => d.kind === "compaction")) t.notes = t.notes.filter((n) => !/^Compacted\\b/.test(n.text));
    const texts = t.steps.filter((s) => s.kind === "text");
    t.answer = t.steps.at(-1)?.kind === "text" ? t.steps.at(-1) : null;
    t.lead = texts[0] && texts[0] !== t.answer ? texts[0] : null;
    const lastText = t.steps.lastIndexOf(texts.at(-1));
    // Errors, answered questions and plans stay in sight, and once the turn settles so does a tool that
    // failed after the last thing claude said.
    const shown = (s) => s === t.lead || s === t.answer || s.kind === "error" || (s.kind === "tool" && (s.name === PLAN_TOOL || (s.name === QUESTION_TOOL && s.result) || !!published(s)));
    t.failed = new Set(t.steps.filter((s, i) => i > lastText && s.kind === "tool" && s.result?.error && !shown(s) && !interruption(s)));
    t.shown = t.steps.filter(shown);
    t.work = t.steps.filter((s) => !shown(s));
    t.interrupted = t.notes.some((n) => n.text === "Interrupted.");
    const times = [t.user?.at, ...t.steps.flatMap((s) => [s.at, s.result?.at])].filter(Boolean).map(Date.parse).filter(Number.isFinite);
    t.start = times.length ? Math.min(...times) : 0;
    t.end = times.length ? Math.max(...times) : 0;
  }
  // A turn's own fold counts the work its steers continued, as T3 Code's top row does.
  let runEnd = 0;
  for (const t of [...turns].reverse()) {
    runEnd = Math.max(runEnd, t.end);
    t.runEnd = runEnd;
    if (!t.user?.steer) runEnd = 0;
  }
  return turns;
}

`;

// Each source runs in its own function scope of this realm, so the two builds share Array, Set and Object.
const { referenceTurnsOf } = new Function(`${ORACLE}; return { referenceTurnsOf };`)();
const { turnsOf, turnsFor } = new Function(`${between("const PLAN_TOOL", "function duration(ms)")}; return { turnsOf, turnsFor };`)();

/** mulberry32: a fixed seed gives the same sequences on every run. */
function random(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function generator(seed) {
  const rnd = random(seed);
  const pick = (list) => list[Math.floor(rnd() * list.length)];
  const toolIds = Array.from({ length: 12 }, (_, i) => `tool${i}`);
  let n = 0;
  const at = () => (rnd() < 0.1 ? undefined : rnd() < 0.05 ? "not a date" : new Date(1_700_000_000_000 + Math.floor(rnd() * 1e7)).toISOString());
  return function item() {
    n++;
    const base = { uuid: `u${n}`, at: at() };
    const withId = (id) => (rnd() < 0.85 ? { ...base, id } : base);
    switch (pick(["user", "user", "steer", "text", "text", "tool", "tool", "tool", "result", "result", "model", "pr", "note", "compaction", "fork", "error", "result-late"])) {
      case "user":
        return { ...withId(`m${n}`), kind: "user", text: rnd() < 0.15 ? "/compact now" : `prompt ${n}`, images: 0 };
      case "steer":
        return { ...withId(`m${n}`), kind: "steer", text: `steer ${n}` };
      case "text":
        return { ...withId(`m${n}`), kind: "text", text: `text ${n}` };
      case "tool":
        return { ...withId(pick(toolIds)), kind: "tool", name: pick(["Bash", "Read", "ExitPlanMode", "AskUserQuestion", "Artifact"]), detail: `d${n}` };
      case "result":
      case "result-late":
        return { ...base, kind: "result", tool: rnd() < 0.05 ? undefined : pick(toolIds), error: rnd() < 0.4, text: rnd() < 0.2 ? "[Request interrupted by user]" : rnd() < 0.3 ? "Published a.html at https://claude.ai/artifact/abc-1" : `out ${n}` };
      case "model":
        return { ...base, kind: "model", name: `model${n}` };
      case "pr":
        return { ...base, kind: "pr", url: pick(["https://x/pr/1", "https://x/pr/2", "https://x/pr/3"]) };
      case "note":
        return { ...base, kind: "note", text: pick(["Interrupted.", "Compacted 12 messages", "ran a hook", "Compacted"]) };
      case "compaction":
        return { ...base, kind: "compaction", text: `summary ${n}` };
      case "fork":
        return { ...base, kind: "fork", text: `fork ${n}` };
      default:
        return { ...base, kind: "error", text: `error ${n}` };
    }
  };
}

test("turns extended item by item equal a full rebuild over random appends, resets and edits", () => {
  let checks = 0;
  for (let seed = 1; seed <= 300; seed++) {
    const rnd = random(seed * 7919);
    const next = generator(seed);
    const t = { items: [] };
    for (let step = 0; step < 40; step++) {
      const roll = rnd();
      if (roll < 0.72) {
        for (let k = Math.floor(rnd() * 5) + 1; k > 0; k--) t.items.push(next());
      } else if (roll < 0.8) {
        t.items = []; // a reset, or a new session
      } else if (roll < 0.86 && t.items.length) {
        t.items = t.items.slice(0, Math.floor(rnd() * t.items.length)); // the transcript shrank
      } else if (roll < 0.93 && t.items.length) {
        if (rnd() < 0.5) {
          t.items[rnd() < 0.5 ? 0 : t.items.length - 1] = next(); // the first or last item changed in place: the cache sees it
        } else {
          t.items[Math.floor(rnd() * t.items.length)] = next(); // any other earlier item changed in place: the caller tells the cache
          t.turnCache = null;
        }
      } else if (t.items.length) {
        t.items = t.items.map((x, i) => (i === Math.floor(t.items.length / 2) ? next() : x)); // the items came again as a new array
      }
      assert.deepStrictEqual(turnsFor(t), referenceTurnsOf(t.items), `seed ${seed} step ${step}`);
      checks++;
    }
    assert.deepStrictEqual(turnsOf(t.items), referenceTurnsOf(t.items), `seed ${seed}: full build`);
  }
  assert.equal(checks, 300 * 40);
});

test("a thread that grows keeps its earlier turns and reads only the new items", () => {
  const next = generator(42);
  const items = [];
  for (let i = 0; i < 3000; i++) items.push(next());
  let reads = 0;
  const watched = new Proxy(items, {
    get(target, key, receiver) {
      if (typeof key === "string" && /^\d+$/.test(key)) reads++;
      return Reflect.get(target, key, receiver);
    },
  });
  const t = { items: watched };
  const before = turnsFor(t).slice();
  assert.ok(reads >= 3000);
  const kept = before.length - 1;
  reads = 0;
  watched.push({ kind: "user", id: "m-new", text: "next", at: "2023-11-14T22:13:20.000Z" }, { kind: "text", id: "m-new-text", text: "answer", at: "2023-11-14T22:13:21.000Z" });
  const after = turnsFor(t);
  assert.ok(reads < 20, `${reads} item reads for 2 new items`);
  assert.equal(after.length, before.length + 1);
  for (let i = 0; i < kept; i++) assert.equal(after[i], before[i], `turn ${i} is the same object`);
  assert.deepStrictEqual(after, referenceTurnsOf(items));
});

test("a result that arrives after its tool reaches the turn that tool is in", () => {
  const t = { items: [{ kind: "user", id: "m1", text: "go", at: "2023-11-14T22:13:20.000Z" }, { kind: "tool", id: "tool1", name: "Bash", at: "2023-11-14T22:13:21.000Z" }] };
  assert.equal(turnsFor(t)[0].steps[0].result, undefined);
  t.items.push({ kind: "user", id: "m2", text: "more", at: "2023-11-14T22:13:22.000Z" }, { kind: "result", tool: "tool1", error: true, text: "boom", at: "2023-11-14T22:14:00.000Z" });
  const turns = turnsFor(t);
  assert.equal(turns[0].steps[0].result.text, "boom");
  assert.equal(turns[0].end, Date.parse("2023-11-14T22:14:00.000Z"));
  assert.deepStrictEqual(turns, referenceTurnsOf(t.items));
});
