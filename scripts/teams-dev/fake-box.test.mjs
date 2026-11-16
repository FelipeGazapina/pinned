// node --test scripts/teams-dev/fake-box.test.mjs
// The fake box must never touch anything outside its own stack directory. Each test names a sentinel
// directory outside it and checks that, whatever the request, the sentinel is exactly as it was.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const FAKE = path.join(path.dirname(fileURLToPath(import.meta.url)), "fake-box");
const WORK = fs.existsSync("/home/boxuser/work") ? "/home/boxuser/work" : os.tmpdir();
const tag = Math.random().toString(36).slice(2, 8);
const STACK = path.join(WORK, `fake-box-test-${tag}`);
const OUTSIDE = path.join(WORK, `outside-${tag}`);
const KEY = { "x-box-api-key": "box_dev-test", "content-type": "application/json" };
let child;
let base;
let boxId;

/** Every path under `dir` with its mtime and size, so any create, delete or write shows. */
function snapshot(dir) {
  const rows = [];
  const walk = (d) => {
    for (const name of fs.readdirSync(d).sort()) {
      const p = path.join(d, name);
      const st = fs.lstatSync(p);
      rows.push([p, st.isDirectory() ? "dir" : st.isSymbolicLink() ? "link" : "file", st.mtimeMs, st.size]);
      if (st.isDirectory()) walk(p);
    }
  };
  rows.push([dir, "root", fs.statSync(dir).mtimeMs]);
  walk(dir);
  return rows;
}

async function api(method, route, body) {
  const res = await fetch(base + route, { method, headers: KEY, body: body === undefined ? undefined : JSON.stringify(body) });
  return { status: res.status, body: await res.json() };
}
const write = (p, content = "x") => api("POST", `/v2/box/${boxId}/files/write`, { path: p, content });
const exec = (...command) => api("POST", `/v2/box/${boxId}/exec`, { command });

before(async () => {
  fs.mkdirSync(STACK, { recursive: true });
  fs.mkdirSync(OUTSIDE, { recursive: true });
  fs.writeFileSync(path.join(OUTSIDE, "f"), "keep");
  child = spawn("node", [FAKE, "0"], { env: { ...process.env, TEAMS_DEV_DIR: STACK }, stdio: ["ignore", "pipe", "inherit"] });
  base = await new Promise((resolve, reject) => {
    let seen = "";
    child.stdout.on("data", (chunk) => {
      seen += chunk;
      const m = seen.match(/listening on 127\.0\.0\.1:(\d+)/);
      if (m) resolve(`http://127.0.0.1:${m[1]}`);
    });
    child.on("exit", (code) => reject(new Error(`fake-box exited ${code}`)));
  });
  const made = await api("POST", "/v2/box", { name: "t", labels: [], size: "small" });
  assert.equal(made.status, 200);
  boxId = made.body.id;
});

after(() => {
  child?.kill("SIGTERM");
  fs.rmSync(STACK, { recursive: true, force: true });
  fs.rmSync(OUTSIDE, { recursive: true, force: true });
});

test("a refused request leaves a sentinel outside the stack untouched", async () => {
  const before = snapshot(OUTSIDE);
  const home = path.join(STACK, "boxes", boxId, "home");
  // A symlink inside the box that points outside: to the sentinel directory and to a file in it.
  fs.symlinkSync(OUTSIDE, path.join(home, "out"));
  fs.symlinkSync(path.join(OUTSIDE, "f"), path.join(home, "outfile"));
  fs.symlinkSync(path.join(OUTSIDE, "not-there"), path.join(home, "dangling"));
  const stackBefore = snapshot(STACK);

  const refused = [
    ["write outside the box", await write(path.join(OUTSIDE, "new"))],
    ["write with a .. out of the box", await write("/workspace/home/../../../../" + OUTSIDE.slice(1) + "/new")],
    ["write through a directory symlink", await write("/workspace/home/out/new")],
    ["write through a file symlink", await write("/workspace/home/outfile")],
    ["write through a dangling symlink", await write("/workspace/home/dangling")],
    ["write into another box-less place", await write("/etc/fake-box-test")],
    ["exec naming the sentinel", await exec("sh", "-c", `touch ${OUTSIDE}/x; rm -f ${OUTSIDE}/f`)],
    ["exec naming the sentinel as an argument", await exec("rm", "-rf", OUTSIDE)],
    ["exec through a symlink", await exec("sh", "-c", "touch /workspace/home/out/x")],
    ["exec with a relative .. out of the box", await exec("sh", "-c", "touch ../../../../" + OUTSIDE.slice(1) + "/x")],
    ["exec with a real home that escaped the rewrite", await exec("sh", "-c", "cat ./x/workspace/home/.pinned/start.sh")],
    ["exec naming ~/.pinned literally", await exec("sh", "-c", "cat ~/.pinned/start.sh")],
    ["exec naming a root path", await exec("ls", "/")],
    ["exec naming /home", await exec("ls", "/home/boxuser")],
  ];
  for (const [what, res] of refused) assert.equal(res.status, 400, `${what}: ${JSON.stringify(res.body)}`);

  assert.deepEqual(snapshot(OUTSIDE), before, "the sentinel is as it was");
  assert.deepEqual(snapshot(STACK), stackBefore, "nothing was written in the box either");
});

test("a write and an exec inside the box work, and /workspace/home is the box's own home", async () => {
  const home = path.join(STACK, "boxes", boxId, "home");
  const w = await write("/workspace/home/.pinned/start.sh", "echo DIR=/workspace/home/.pinned\n");
  assert.equal(w.status, 200, JSON.stringify(w.body));
  assert.equal(fs.readFileSync(path.join(home, ".pinned/start.sh"), "utf8"), `echo DIR=${home}/.pinned\n`, "the content is rewritten too");
  assert.equal((await write("/tmp/note.txt", "hi")).status, 200);

  const e = await exec("sh", "/workspace/home/.pinned/start.sh");
  assert.equal(e.status, 200, JSON.stringify(e.body));
  assert.equal(e.body.exit_code, 0);
  assert.equal(e.body.output.trim(), `DIR=${home}/.pinned`);

  const inBox = await exec("sh", "-c", 'cd /workspace/home && pwd && echo "$HOME $TMPDIR" && cat /tmp/note.txt && curl --version >/dev/null; echo done > ./inside.txt; cat /dev/null');
  assert.equal(inBox.status, 200, JSON.stringify(inBox.body));
  assert.ok(inBox.body.output.includes(home), inBox.body.output);
  assert.equal(fs.readFileSync(path.join(home, "inside.txt"), "utf8"), "done\n");
  // What the hub's real bootstrap does: read /proc/<pid>/cmdline of the pid a file names, curl a URL, untar into /tmp.
  const boot = await exec("sh", "-c", 'DIR=/workspace/home/.pinned; mkdir -p "$DIR/bin"; [ -f "$DIR/node.pid" ] && { grep -qs pinned-server "/proc/$(cat "$DIR/node.pid")/cmdline" && kill "$(cat "$DIR/node.pid")" 2>/dev/null || true; }; curl -fsSL http://127.0.0.1:1/download/x -o "$DIR/x.part" 2>/dev/null; ls /tmp >/dev/null; tar -tzf /dev/null -C /tmp 2>/dev/null; true');
  assert.equal(boot.status, 200, JSON.stringify(boot.body));
  // URLs and system paths are not paths of the box.
  assert.equal((await exec("sh", "-c", "echo https://github.com/example/alpha http://127.0.0.1:1/download/x >/dev/null; ls /usr/bin >/dev/null; ls /bin/sh /etc/hostname")).status, 200);
});

test("deleting a box kills what its commands left running and nothing else", async () => {
  const made = await api("POST", "/v2/box", { name: "d", labels: [], size: "small" });
  const id = made.body.id;
  const res = await api("POST", `/v2/box/${id}/exec`, { command: ["sh", "-c", "setsid nohup sleep 300 >/dev/null 2>&1 </dev/null & echo started"] });
  assert.equal(res.status, 200, JSON.stringify(res.body));
  const mine = (pid) => {
    try {
      return fs.readFileSync(`/proc/${pid}/environ`, "latin1").includes(`PINNED_FAKE_BOX_ID=${id}\0`);
    } catch {
      return false;
    }
  };
  const sleeper = fs.readdirSync("/proc").filter((n) => /^\d+$/.test(n) && mine(n));
  assert.ok(sleeper.length >= 1, "the box has a running process");
  assert.equal((await api("DELETE", `/v2/box/${id}`)).status, 200);
  assert.equal(fs.existsSync(path.join(STACK, "boxes", id)), false);
  assert.deepEqual(fs.readdirSync("/proc").filter((n) => /^\d+$/.test(n) && mine(n)).filter((n) => fs.existsSync(`/proc/${n}/cmdline`) && !fs.readFileSync(`/proc/${n}/stat`, "utf8").includes(") Z ")), []);
  assert.ok(fs.existsSync(path.join(STACK, "boxes", boxId)), "another box is left alone");
});
