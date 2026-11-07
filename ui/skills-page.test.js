import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = (file) => readFileSync(new URL(file, import.meta.url), "utf8");
const between = (src, from, to) => src.slice(src.indexOf(from), src.indexOf(to));

function page(byMachine) {
  const skills = source("./gui-skills.js");
  const sandbox = { skillsUi: { byMachine: new Map(Object.entries(byMachine)) }, LOCAL: "mac", machineName: (m) => m };
  vm.createContext(sandbox);
  vm.runInContext(`${between(source("./diff.js"), "function parseDiff", "/** What `git diff -w`")}
    ${between(skills, "const copyId", "async function fetchSkills")}
    ${between(skills, "/** One row per scope", "function machineNotesHtml")}
    ${between(skills, "/** A unified patch", "const diffHtml")}
    this.api = { parseDiff, textPatch, skillRows, whereLabel };`, sandbox);
  return sandbox.api;
}

test("a skill's edit reads as the lines it removed and added", () => {
  const { parseDiff, textPatch } = page({});
  const [file] = parseDiff(textPatch("SKILL.md", "---\nname: x\n---\nSay one.\nKeep.\n", "---\nname: x\n---\nSay two.\nKeep.\nMore.\n"));
  const rows = Array.from(file.hunks[0].lines, (l) => `${{ add: "+", del: "-", ctx: " " }[l.type]}${l.text}`);
  assert.deepEqual(rows, [" ---", " name: x", " ---", "-Say one.", "+Say two.", " Keep.", "+More."]);
  assert.deepEqual([file.added, file.removed], [2, 1]);
  const [created] = parseDiff(textPatch("SKILL.md", "", "new\n"));
  assert.deepEqual(Array.from(created.hunks[0].lines, (l) => [l.type, l.text]), [["add", "new"]]);
});

test("copies group by scope and name, across machines, and agree only with one tree", () => {
  const copy = (key, scope, tree) => ({ key, scope, tree, description: "", readers: [], place: { kind: "agents" } });
  const { skillRows } = page({
    mac: { copies: [copy("deploy", { kind: "personal" }, "t1"), copy("deploy", { kind: "project", checkout: "/src/app" }, "t2"), copy("pstack:unslop", { kind: "plugin", plugin: "pstack" }, "t3")] },
    mini: { copies: [copy("deploy", { kind: "personal" }, "t1"), copy("pstack:unslop", { kind: "plugin", plugin: "pstack" }, "t4")] },
    old: { error: "this machine can't answer that" },
  });
  const rows = Array.from(skillRows(), (r) => [r.key, r.scope.kind, r.copies.map((c) => c.machine).join(","), r.agree]);
  assert.deepEqual(rows, [
    ["deploy", "personal", "mac,mini", true],
    ["deploy", "project", "mac", true],
    ["pstack:unslop", "plugin", "mac,mini", false],
  ]);
});

test("a folder two places reach is one copy named by both", () => {
  const { whereLabel } = page({ mac: { copies: [] } });
  const readers = [{ provider: "claude", account: "claude" }, { provider: "claude", account: "work" }, { provider: "codex", account: "codex" }];
  const copy = { machine: "mac", readers, place: { kind: "claude-personal" }, also: [{ kind: "claude-personal" }, { kind: "agents" }] };
  assert.equal(whereLabel(copy), "Claude (claude, work) + ~/.agents");
});

function checklist() {
  const skills = source("./gui-skills.js");
  const fetched = [];
  const timers = [];
  const sandbox = {
    skillsUi: { byMachine: new Map() },
    LOCAL: "mac",
    machineName: (m) => m,
    document: { body: { dataset: { page: "skills" } } },
    machines: new Map(["mac", "mini"].map((m) => [m, { view: { online: true } }])),
    fetchSkills: (only) => fetched.push(only),
    setTimeout: (run) => timers.push(run),
    clearTimeout: () => {},
    renderSkillsPage: () => {},
  };
  vm.createContext(sandbox);
  vm.runInContext(`${between(skills, "const copyId", "async function fetchSkills")}
    ${between(skills, "const baseName", "function machineNotesHtml")}
    ${between(skills, "const openRow", "/** Opens `copy`")}
    ${between(skills, "/** Runs `go` now", "async function compareCopy")}
    this.api = { openChecks, refreshChecks, sawCopy, copyCheck, saveBlocker, saveTargets, skillsChanged, leaveThen };`, sandbox);
  return { ...sandbox.api, fetched, timers, ui: sandbox.skillsUi };
}

const skillCopy = (machine, dir, tree, place = { kind: "agents" }, more = {}) => ({ machine, dir, tree, place, edit: "yes", readers: [], linked: null, ...more });
const opened = (copy, tree) => ({ copy, got: { text: "body", tree }, draft: null, compared: new Map(), checked: new Set(), since: new Map(), changed: new Set() });
const plain = (value) => JSON.parse(JSON.stringify(value));
const boxes = (api, o, row) => row.copies.map((c) => {
  const s = api.copyCheck(o, row, c);
  return [`${c.machine}:${c.dir}`, s.checked, s.disabled, s.why];
});

test("a copy that changes after the skill opened is unchecked, and a save sends the tree the page showed", () => {
  const api = checklist();
  const [a, b, c] = [skillCopy("mac", "/a", "t1"), skillCopy("mac", "/b", "t1"), skillCopy("mac", "/c", "t2")];
  const o = opened(a, "t1");
  api.openChecks(o, { copies: [a, b, c] });
  const later = { copies: [a, { ...b, tree: "t3" }, c] };
  api.refreshChecks(o, later);
  assert.deepEqual(boxes(api, o, later), [
    ["mac:/a", true, false, ""],
    ["mac:/b", false, true, "Changed since you opened it. Compare it first."],
    ["mac:/c", false, true, "Differs from the copy you opened. Compare it first."],
  ]);
  assert.deepEqual(plain([...api.saveTargets(o, later)]), [["mac", [{ dir: "/a", since: "t1" }]]]);
  api.sawCopy(o, later.copies[1], { text: "theirs", tree: "t3" });
  o.checked.add("mac\n/b");
  assert.deepEqual(plain([...api.saveTargets(o, later)]), [["mac", [{ dir: "/a", since: "t1" }, { dir: "/b", since: "t3" }]]]);
});

test("a linked copy on another machine with another tree blocks the save until its diff was opened", () => {
  const api = checklist();
  const personal = { kind: "claude-personal" };
  const open = skillCopy("mac", "/h/.claude/skills/x", "t1", personal, { linked: "user-skills/x" });
  const same = skillCopy("mini", "/m/.claude/skills/x", "t1", personal, { linked: "user-skills/x" });
  const other = skillCopy("mini", "/m/bloom/skills/x", "t2", personal, { linked: "user-skills/x" });
  const row = { copies: [open, same, other] };
  const o = opened(open, "t1");
  api.openChecks(o, row);
  assert.deepEqual(boxes(api, o, row), [
    ["mac:/h/.claude/skills/x", true, false, ""],
    ["mini:/m/.claude/skills/x", true, false, ""],
    ["mini:/m/bloom/skills/x", false, true, "Differs from the copy you opened. Compare it first."],
  ]);
  assert.equal(api.saveBlocker(o, row), other);
  api.sawCopy(o, other, { text: "older", tree: "t2" });
  assert.equal(api.saveBlocker(o, row), undefined);
  assert.deepEqual(boxes(api, o, row)[2], ["mini:/m/bloom/skills/x", false, false, "Sync replaces this copy with this save, checked or not."]);
  o.checked.delete("mini\n/m/.claude/skills/x");
  assert.deepEqual(boxes(api, o, row)[1], ["mini:/m/.claude/skills/x", false, false, "Sync replaces this copy with this save, checked or not."]);
});

test("the cache copy of a checked plugin source says the save writes it too", () => {
  const api = checklist();
  const source = skillCopy("mac", "/mods/mods/skills/tidy", "t1", { kind: "plugin-source", plugin: "mods@local-mods", home: "/h/.claude" });
  const cache = skillCopy("mac", "/h/.claude/plugins/cache/local-mods/mods/0.1.0/skills/tidy", "t1", { kind: "plugin-cache", plugin: "mods@local-mods", home: "/h/.claude", loaded: false }, { edit: "lost-on-update" });
  const row = { copies: [source, cache] };
  const o = opened(source, "t1");
  api.openChecks(o, row);
  o.checked.delete(`mac\n${cache.dir}`);
  assert.deepEqual(boxes(api, o, row)[1].slice(1), [false, false, "Saving the plugin's source copy writes this copy too."]);
});

test("skills changing on two machines at once lists both again", () => {
  const api = checklist();
  api.skillsChanged("mac");
  api.skillsChanged("mini");
  api.skillsChanged("mac");
  api.timers.at(-1)();
  assert.deepEqual(plain(api.fetched), [["mac", "mini"]]);
});

test("opening another skill over unsaved edits waits for the user's answer", () => {
  const api = checklist();
  const went = [];
  const o = opened(skillCopy("mac", "/a", "t1"), "t1");
  api.ui.open = o;
  o.draft = "edited";
  api.leaveThen(() => went.push("other"));
  assert.deepEqual([went, typeof o.leaving], [[], "function"]);
  o.draft = "body";
  api.leaveThen(() => went.push("clean"));
  assert.deepEqual(went, ["clean"]);
});
