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
