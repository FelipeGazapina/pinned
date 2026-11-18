#!/usr/bin/env node
// Compares the runs of two or more sides, each a list of run.js output folders, as a Markdown table
// of the median and range of each metric per load and throttle.
//
//   node scripts/mobile-lag-bench/compare.js before=<dir>,<dir> after=<dir>,<dir>
const fs = require("fs");
const path = require("path");

const METRICS = [
  ["main thread busy %", (r) => r.mainThread.busyPct],
  ["style ms", (r) => r.mainThread.styleMs],
  ["long tasks", (r) => r.longTasks.count],
  ["long task ms", (r) => r.longTasks.totalMs],
  ["key p95 ms", (r) => r.keystrokes.p95Ms],
  ["key max ms", (r) => r.keystrokes.maxMs],
  ["INP ms", (r) => r.inpMs],
  ["scroll frames dropped %", (r) => r.scroll.droppedShare],
  ["switch thread ms", (r) => r.steps["switch thread"]],
];

const sides = process.argv.slice(2).map((arg) => {
  const [name, dirs] = arg.split("=");
  const runs = dirs.split(",").flatMap((dir) => fs.readdirSync(dir).filter((f) => /-x\d+-run\d+\.json$/.test(f)).map((f) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"))));
  return { name, runs };
});
const median = (v) => [...v].sort((a, b) => a - b)[Math.floor((v.length - 1) / 2)];
const cell = (v) => (v.length ? `${median(v)} (${Math.min(...v)}–${Math.max(...v)})` : "");
const groups = [...new Set(sides.flatMap((s) => s.runs.map((r) => `${r.load} x${r.throttle}`)))].sort();
for (const group of groups) {
  const of = (side) => side.runs.filter((r) => `${r.load} x${r.throttle}` === group);
  console.log(`\n${group}, runs: ${sides.map((s) => `${s.name} ${of(s).length}`).join(", ")}\n`);
  console.log(`| metric | ${sides.map((s) => s.name).join(" | ")} |`);
  console.log(`|---|${sides.map(() => "---").join("|")}|`);
  for (const [label, get] of METRICS) console.log(`| ${label} | ${sides.map((s) => cell(of(s).map(get))).join(" | ")} |`);
}
