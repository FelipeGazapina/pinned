#!/usr/bin/env node
// Writes an invented transcript of `turns` turns in the format claude writes, for timing how a long
// conversation opens. Nothing in it comes from a real conversation.
//
//   node scripts/history-bench/seed.js <file.jsonl> <session id> [turns=2000]
//
// Each turn: a prompt, reasoning, a reply with a Bash call, its output, and a closing reply. Turn
// 1000 is a compaction (boundary plus summary), every 250th prompt carries a small picture, and
// every 100th turn has a prompt queued while claude worked.
const fs = require("fs");

const [file, session, turns = "2000"] = process.argv.slice(2);
if (!file || !session) {
  console.error("usage: seed.js <file.jsonl> <session id> [turns]");
  process.exit(2);
}
const WORDS = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango uniform victor whiskey xray yankee zulu".split(" ");
let seed = 7;
const rand = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;
const words = (n) => Array.from({ length: n }, () => WORDS[Math.floor(rand() * WORDS.length)]).join(" ");
// A 1x1 PNG.
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

const start = Date.parse("2026-01-05T09:00:00Z");
let n = 0;
const out = [];
const line = (t, entry) => out.push(JSON.stringify({ uuid: `${session}-${++n}`, timestamp: new Date(start + t * 1000).toISOString(), sessionId: session, cwd: "/work/demo", gitBranch: "main", ...entry }));
const total = Number(turns);
for (let i = 1; i <= total; i++) {
  const t = i * 90;
  if (i === Math.floor(total / 2)) {
    line(t - 5, { type: "system", subtype: "compact_boundary", compactMetadata: { trigger: "auto", preTokens: 167000, postTokens: 12000 } });
    line(t - 4, { type: "user", isCompactSummary: true, message: { role: "user", content: `This session is being continued from a previous conversation. Summary: ${words(120)}` } });
  }
  const prompt = `Turn ${i}: please look at ${words(30)}`;
  const content = i % 250 === 0 ? [{ type: "text", text: prompt }, { type: "image", source: { type: "base64", media_type: "image/png", data: PNG } }] : prompt;
  line(t, { type: "user", message: { role: "user", content } });
  line(t + 2, { type: "assistant", message: { role: "assistant", content: [{ type: "thinking", thinking: words(60) }] } });
  const tool = `toolu_${i}`;
  line(t + 3, { type: "assistant", message: { role: "assistant", content: [{ type: "text", text: `Checking ${words(40)}` }, { type: "tool_use", id: tool, name: "Bash", input: { command: `grep -rn ${WORDS[i % 26]} src/`, description: "Search the sources" } }] } });
  const output = Array.from({ length: 30 }, (_, k) => `src/${WORDS[k % 26]}.rs:${k + 1}: ${words(12)}`).join("\n");
  line(t + 5, { type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: tool, content: output }] }, toolUseResult: { stdout: output, stderr: "" } });
  if (i % 100 === 0) line(t + 6, { type: "attachment", attachment: { type: "queued_command", origin: { kind: "human" }, prompt: `Also ${words(10)}` } });
  line(t + 8, { type: "assistant", message: { role: "assistant", content: [{ type: "text", text: `Done with turn ${i}. ${words(90)}` }], usage: { input_tokens: 4, cache_read_input_tokens: 50000 + i, output_tokens: 300 } } });
}
fs.writeFileSync(file, out.join("\n") + "\n");
console.log(`${file}: ${total} turns, ${out.length} lines, ${fs.statSync(file).size} bytes`);
