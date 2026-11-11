#!/usr/bin/env node
// Builds the state the RAM bench opens: saved sessions, their transcripts, and their folders.
//
//   node scripts/ram-bench/state.js synthetic <dir>   # generated transcripts, the same every run
//   node scripts/ram-bench/state.js replica <dir>     # this Mac's installed app: its open tabs and their transcripts
//
// <dir>/home is the HOME the bench's node and app run under, <dir>/sessions.json and
// <dir>/projects.json the files they start from, and <dir>/manifest.json what was built. A replica holds private
// transcripts: keep <dir> out of the repository and delete it when the run is over.
const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

const [kind, asked] = process.argv.slice(2);
if (!["synthetic", "replica"].includes(kind) || !asked) {
  console.error("usage: state.js synthetic|replica <dir>");
  process.exit(2);
}
fs.rmSync(asked, { recursive: true, force: true });
fs.mkdirSync(asked, { recursive: true });
// A project is known by its canonical path, so every path here is under the real folder.
const dir = fs.realpathSync(asked);
const home = path.join(dir, "home");
const slug = (cwd) => cwd.replace(/[^a-zA-Z0-9]/g, "-");
const projects = path.join(home, ".claude", "projects");

function project(name) {
  const cwd = path.join(home, "work", name);
  if (!fs.existsSync(cwd)) {
    fs.mkdirSync(cwd, { recursive: true });
    fs.writeFileSync(path.join(cwd, "README.md"), `# ${name}\n`);
    const git = (...args) => execFileSync("git", ["-C", cwd, "-c", "user.name=bench", "-c", "user.email=bench@example.test", ...args], { stdio: "ignore" });
    git("init", "-q");
    git("add", ".");
    git("commit", "-qm", "Start");
  }
  return cwd;
}
const saved = (cwd, session) => ({ cwd, session, env: [], bypass: false, interrupted: false, resume_at: null, held: false, account: "claude" });

// Megabytes of each open tab's transcript on the Mac this bench was written on, 2026-11-10.
const SIZES_MB = [81, 26.4, 23.7, 11.9, 11.2, 10.9, 9.6, 6.8, 6.6, 6.1, 4.9, 3.8, 3.2, 2.9, 2.6, 2.6, 2.5, 2.2, 2.1, 2.1, 1.9, 1.8, 1.7, 1.7, 1.6, 1.5, 1.5, 1.4, 1.4, 1.3, 1.3, 1.3, 1.1, 1, 0.9, 0.9, 0.8];

// mulberry32: the same bytes on every machine.
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const WORDS = "the session reads a file then edits it and runs the tests again because one case failed on the boundary so the parser keeps a cursor per line and the hub relays each frame to every client that watches the tab while the node writes the transcript to disk".split(" ");

function synthetic() {
  const sessions = [];
  let bytes = 0;
  let lines = 0;
  SIZES_MB.forEach((mb, index) => {
    const rand = rng(index + 1);
    const pick = (list) => list[Math.floor(rand() * list.length)];
    const words = (n) => Array.from({ length: n }, () => pick(WORDS)).join(" ");
    const code = (n) => Array.from({ length: n }, (_, i) => `${"  ".repeat(i % 4)}const ${pick(WORDS)}${i} = ${pick(WORDS)}(${Math.floor(rand() * 1000)}); // ${words(4)}`).join("\n");
    const cwd = project(`project-${index % 9}`);
    const session = `bench-${String(index).padStart(2, "0")}-0000-4000-8000-000000000000`;
    const file = path.join(projects, slug(cwd), `${session}.jsonl`);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const out = fs.openSync(file, "w");
    let n = 0;
    let written = 0;
    let parent = null;
    const at = Date.parse("2026-11-01T12:00:00Z") + index * 3600_000;
    const record = (entry) => {
      const uuid = `${session}-${++n}`;
      const line = JSON.stringify({ parentUuid: parent, isSidechain: false, userType: "external", cwd, sessionId: session, version: "2.1.321", gitBranch: "main", uuid, timestamp: new Date(at + n * 1500).toISOString(), ...entry }) + "\n";
      parent = uuid;
      written += fs.writeSync(out, line);
    };
    const assistant = (content, stop_reason) =>
      record({ type: "assistant", requestId: `req_${n}`, message: { id: `msg_${n}`, type: "message", role: "assistant", model: "claude-opus-5-5", content, stop_reason, usage: { input_tokens: 12, cache_read_input_tokens: 80000 + n * 40, cache_creation_input_tokens: 900, output_tokens: 400 } } });
    const tool = (name, input, result, extra) => {
      const id = `toolu_${session.slice(0, 8)}_${n}`;
      assistant([{ type: "tool_use", id, name, input }], "tool_use");
      record({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: id, content: result }] }, toolUseResult: extra });
    };
    const target = mb * 1024 * 1024;
    while (written < target) {
      record({ type: "user", message: { role: "user", content: `${words(12 + Math.floor(rand() * 40))}?` } });
      const steps = 3 + Math.floor(rand() * 14);
      for (let s = 0; s < steps && written < target; s++) {
        if (rand() < 0.35) assistant([{ type: "thinking", thinking: words(60 + Math.floor(rand() * 200)), signature: "sig" }, { type: "text", text: words(20 + Math.floor(rand() * 60)) }], "tool_use");
        const roll = rand();
        const filePath = path.join(cwd, "src", `${pick(WORDS)}-${Math.floor(rand() * 40)}.js`);
        if (roll < 0.35) {
          const content = code(40 + Math.floor(rand() * 400));
          tool("Read", { file_path: filePath }, content, { type: "text", file: { filePath, content, numLines: content.split("\n").length, startLine: 1, totalLines: content.split("\n").length } });
        } else if (roll < 0.6) {
          const before = code(6 + Math.floor(rand() * 20));
          const after = code(6 + Math.floor(rand() * 20));
          const original = code(200 + Math.floor(rand() * 600));
          tool("Edit", { file_path: filePath, old_string: before, new_string: after }, `The file ${filePath} has been updated.`, {
            filePath,
            oldString: before,
            newString: after,
            originalFile: original,
            structuredPatch: [{ oldStart: 10, oldLines: before.split("\n").length, newStart: 10, newLines: after.split("\n").length, lines: [...before.split("\n").map((l) => `-${l}`), ...after.split("\n").map((l) => `+${l}`)] }],
            userModified: false,
            replaceAll: false,
          });
        } else if (roll < 0.9) {
          const stdout = Array.from({ length: 5 + Math.floor(rand() * 120) }, (_, i) => `test ${pick(WORDS)}_${i} ... ok (${Math.floor(rand() * 90)}ms)`).join("\n");
          tool("Bash", { command: `cargo test ${pick(WORDS)}`, description: `Run the ${pick(WORDS)} tests` }, stdout, { stdout, stderr: "", interrupted: false, isImage: false });
        } else {
          const found = Array.from({ length: 4 + Math.floor(rand() * 60) }, (_, i) => `${cwd}/src/${pick(WORDS)}-${i}.js:${Math.floor(rand() * 900)}: ${words(8)}`).join("\n");
          tool("Grep", { pattern: pick(WORDS), path: cwd }, found, { mode: "content", numFiles: 12, filenames: [], content: found, numLines: found.split("\n").length });
        }
      }
      assistant([{ type: "text", text: `## ${words(5)}\n\n${words(60 + Math.floor(rand() * 160))}\n\n\`\`\`js\n${code(8 + Math.floor(rand() * 24))}\n\`\`\`\n\n- ${words(12)}\n- ${words(14)}\n- ${words(9)}\n\n${words(40)}` }], "end_turn");
      record({ type: "system", subtype: "turn_duration", durationMs: 20000 + Math.floor(rand() * 400000), isMeta: false });
    }
    fs.closeSync(out);
    bytes += written;
    lines += n;
    sessions.push(saved(cwd, session));
  });
  return { sessions, bytes, lines };
}

function replica() {
  const real = path.join(os.homedir(), "Library", "Application Support", "dev.personal-harness.pinned");
  const open = JSON.parse(fs.readFileSync(path.join(real, "sessions.json"), "utf8"));
  const roots = [path.join(os.homedir(), ".claude", "projects")];
  for (const accounts of [path.join(real, "accounts"), path.join(os.homedir(), ".claude", "accounts")]) {
    for (const account of fs.existsSync(accounts) ? fs.readdirSync(accounts) : []) roots.push(path.join(accounts, account, "projects"));
  }
  const transcripts = new Map();
  for (const root of roots) {
    for (const folder of fs.existsSync(root) ? fs.readdirSync(root) : []) {
      let files = [];
      try {
        files = fs.readdirSync(path.join(root, folder));
      } catch {}
      for (const f of files) if (f.endsWith(".jsonl")) transcripts.set(f.slice(0, -6), path.join(root, folder, f));
    }
  }
  const folders = new Map();
  const sessions = [];
  let bytes = 0;
  let lines = 0;
  for (const s of open) {
    const from = transcripts.get(s.session);
    if (!from) continue;
    if (!folders.has(s.cwd)) folders.set(s.cwd, project(`${path.basename(s.cwd).replace(/[^a-zA-Z0-9._-]/g, "-")}-${folders.size}`));
    const cwd = folders.get(s.cwd);
    const to = path.join(projects, slug(cwd), `${s.session}.jsonl`);
    fs.mkdirSync(path.dirname(to), { recursive: true });
    fs.copyFileSync(from, to);
    const agents = from.slice(0, -6);
    if (fs.existsSync(agents)) fs.cpSync(agents, to.slice(0, -6), { recursive: true });
    const body = fs.readFileSync(to);
    bytes += body.length;
    for (const byte of body) if (byte === 10) lines++;
    sessions.push(saved(cwd, s.session));
  }
  return { sessions, bytes, lines };
}

fs.mkdirSync(projects, { recursive: true });
const built = kind === "replica" ? replica() : synthetic();
fs.writeFileSync(path.join(dir, "sessions.json"), JSON.stringify(built.sessions, null, 2));
fs.writeFileSync(path.join(dir, "projects.json"), JSON.stringify([...new Set(built.sessions.map((s) => s.cwd))].map((cwd) => ({ path: cwd, last_opened: 1793620800 })), null, 2));
const manifest = { kind, sessions: built.sessions.length, transcript_bytes: built.bytes, transcript_lines: built.lines };
fs.writeFileSync(path.join(dir, "manifest.json"), JSON.stringify(manifest, null, 2));
console.log(JSON.stringify(manifest));
