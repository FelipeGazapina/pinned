#!/usr/bin/env node
// Where a run's main thread went, from a trace run.js --trace wrote: time per top-level task kind,
// and style recalcs and layouts grouped by the script frame that forced them ("(frame)" when the
// frame's own rendering ran them).
//
//   node scripts/mobile-lag-bench/trace-top.js <trace.json>
const fs = require("fs");
const trace = JSON.parse(fs.readFileSync(process.argv[2], "utf8"));
const events = trace.traceEvents ?? trace;
const names = new Map(events.filter((e) => e.name === "thread_name").map((e) => [`${e.pid}:${e.tid}`, e.args.name]));
const main = events.filter((e) => names.get(`${e.pid}:${e.tid}`) === "CrRendererMain" && e.ph === "X");
const ms = (us) => Math.round(us / 100) / 10;
const add = (map, key, dur) => map.set(key, (map.get(key) ?? 0) + dur);
const top = (map, n = 15) => [...map].sort((a, b) => b[1] - a[1]).slice(0, n).map(([k, v]) => `${String(ms(v)).padStart(8)} ms  ${k}`);

const tasks = main.filter((e) => e.name === "RunTask" || e.name === "ThreadControllerImpl::RunTask");
const total = tasks.reduce((n, e) => n + e.dur, 0);
const byKind = new Map();
for (const e of main) if (["UpdateLayoutTree", "Layout", "Paint", "PrePaint", "Layerize", "FunctionCall", "EvaluateScript", "ParseHTML", "MajorGC", "MinorGC", "V8.GC_SCAVENGER", "HitTest", "TimerFire", "FireAnimationFrame", "EventDispatch", "UpdateLayer", "Commit"].includes(e.name)) add(byKind, e.name, e.dur);
const forced = (name) => {
  const map = new Map();
  let count = 0;
  for (const e of main.filter((e) => e.name === name)) {
    const frame = (e.args?.beginData?.stackTrace ?? e.args?.data?.stackTrace)?.[0];
    add(map, frame ? `${frame.functionName || "(anonymous)"} ${frame.url.split("/").pop()}:${frame.lineNumber + 1}` : "(frame)", e.dur);
    count++;
  }
  return { count, top: top(map) };
};
const style = forced("UpdateLayoutTree");
const layout = forced("Layout");
console.log(`main thread tasks: ${ms(total)} ms in ${tasks.length} tasks`);
console.log("by kind (nested, so they overlap):\n" + top(byKind, 20).join("\n"));
console.log(`\nstyle recalcs: ${style.count}, by the frame that forced them:\n${style.top.join("\n")}`);
console.log(`\nlayouts: ${layout.count}, by the frame that forced them:\n${layout.top.join("\n")}`);
