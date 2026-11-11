import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const context = {};
vm.createContext(context);
vm.runInContext(readFileSync(new URL("./global-chat.js", import.meta.url), "utf8"), context);

// Objects built inside the vm carry its prototypes; a JSON round trip makes them comparable here.
const here = (value) => (value === undefined ? value : JSON.parse(JSON.stringify(value)));
const call = (name) => (...args) => here(context[name](...args));
const reportBlocks = call("reportBlocks");
const reportFoldLabel = call("reportFoldLabel");
const quickReplyOf = call("quickReplyOf");
const speechTextOf = call("speechTextOf");
const waitingKindOf = call("waitingKindOf");
const offerOf = call("offerOf");
const askOf = call("askOf");
// A top-level const stays in the vm's script scope, so it is read back through the vm itself.
const globalSettings = vm.runInContext("globalSettings", context);

const pasted = "Relatórios de hoje:\n[pinned] Capitão site (sweep-node) [mn1 st1]: Publiquei a v2.\nE escrevi o changelog.\n[pinned] Capitão pinned (Mac mini) [mm2 st2]: Fechei o PR #12.\n[pinned] Capitão site (sweep-node) [mn1 st1]: Publiquei também a v3.";

test("a paste holds one block per report header, each running to the next header", () => {
  assert.deepEqual(reportBlocks(pasted), [
    { start: 20, project: "site", machineName: "sweep-node", machineId: "mn1", tabId: "st1", text: "Publiquei a v2.\nE escrevi o changelog." },
    { start: 105, project: "pinned", machineName: "Mac mini", machineId: "mm2", tabId: "st2", text: "Fechei o PR #12." },
    { start: 168, project: "site", machineName: "sweep-node", machineId: "mn1", tabId: "st1", text: "Publiquei também a v3." },
  ]);
});

test("the words before the first header are the sender's own, and a block needs its own line", () => {
  assert.equal(pasted.slice(0, reportBlocks(pasted)[0].start).trim(), "Relatórios de hoje:");
  assert.deepEqual(reportBlocks("vi um relatório: [pinned] Capitão site (node) [m t]: pronto"), []);
  assert.deepEqual(reportBlocks(""), []);
  assert.deepEqual(reportBlocks(null), []);
});

test("the fold line counts the reports and names each Captain once", () => {
  assert.equal(reportFoldLabel(reportBlocks(pasted)), "3 updates · Captain site, Captain pinned");
  assert.equal(reportFoldLabel(reportBlocks("[pinned] Capitão site (node) [m t]: um\n[pinned] Capitão site (node) [m t]: dois")), "2 updates · Captain site");
});

test("the English forms of the report header and the quick-reply line read like the Portuguese ones", () => {
  assert.deepEqual(reportBlocks("[pinned] Captain site (node) [m t]: done"), [{ start: 0, project: "site", machineName: "node", machineId: "m", tabId: "t", text: "done" }]);
  assert.deepEqual(quickReplyOf("Merge it?\n\nOptions: Yes | Not yet\n"), { options: ["Yes", "Not yet"], text: "Merge it?" });
  assert.equal(speechTextOf("Done.\nOptions: Yes | No"), "Done.");
  assert.equal(reportFoldLabel(reportBlocks("[pinned] Captain site (node) [m t]: one")), "1 update · Captain site");
});

test("an answer offers quick replies only when its last non-empty line reads Opções", () => {
  assert.deepEqual(quickReplyOf("Li os relatórios dos Capitães.\n\nOpções: Sim | Ainda não\n"), { options: ["Sim", "Ainda não"], text: "Li os relatórios dos Capitães." });
  assert.deepEqual(quickReplyOf("Opções: só uma"), { options: ["só uma"], text: "" });
  assert.deepEqual(quickReplyOf("Opções:  a | b "), { options: ["a", "b"], text: "" });
  assert.equal(quickReplyOf("Opções: Sim | Ainda não\nmais uma linha"), null);
  assert.equal(quickReplyOf("Li os relatórios dos Capitães."), null);
  assert.equal(quickReplyOf("Opções: "), null);
  assert.equal(quickReplyOf(""), null);
});

test("read aloud says the words without code blocks and without the Opções line", () => {
  assert.equal(speechTextOf("Fechei o PR.\n```js\nconst x = 1;\n```\nOpções: Sim | Não"), "Fechei o PR.");
  assert.equal(speechTextOf("Vou rodar os testes.\n```bash\nnpm test"), "Vou rodar os testes.");
  assert.equal(speechTextOf("Uma coisa.\n\nOutra coisa.\n"), "Uma coisa. Outra coisa.");
  assert.equal(speechTextOf("Opções: Sim"), "");
});

const waitingCases = JSON.parse(readFileSync(new URL("../crates/core/src/waiting_cases.json", import.meta.url), "utf8"));

test("a thread waits on the user, or holds an offer, in exactly the cases the orchestrator's waiting() and sessions use", () => {
  const view = (c) => ({ ...waitingCases.base, ...c.view });
  assert.deepEqual(
    waitingCases.cases.map((c) => [c.case, waitingKindOf(view(c))]),
    waitingCases.cases.map((c) => [c.case, c.waits]),
  );
  assert.deepEqual(
    waitingCases.cases.map((c) => [c.case, offerOf(view(c), waitingCases.now) !== null]),
    waitingCases.cases.map((c) => [c.case, c.offer === true]),
  );
  assert.deepEqual(
    waitingCases.cases.filter((c) => c.waits === "needs_input").map((c) => [c.case, askOf(view(c)).kind]),
    waitingCases.cases.filter((c) => c.waits === "needs_input").map((c) => [c.case, c.ask]),
    "an approval and a question tell apart as the orchestrator's waiting() tells them",
  );
  assert.equal(waitingKindOf(null), null);
  assert.equal(offerOf(null, waitingCases.now), null);
});

test("the list splits blocked threads from offers: a finished turn ending in a question is never blocked", () => {
  const now = waitingCases.now;
  const threads = {
    approval: { status: "needs-input", running: true, interrupted: true },
    failure: { status: "failed", running: true },
    asked: { status: "done", running: true, question: "Want me to cut the release?", updated: now - 600 },
    "asked a day ago": { status: "done", running: true, question: "Want me to cut the release?", updated: now - 25 * 3600 },
    "asked by a worker": { role: "worker", status: "done", running: true, question: "Update the README?" },
    "asked, then stopped": { status: "done", running: false, question: "Update the README?" },
  };
  const split = Object.fromEntries(Object.entries(threads).map(([name, view]) => [name, [waitingKindOf(view), offerOf(view, now)]]));
  assert.deepEqual(split, {
    approval: ["needs_input", null],
    failure: ["failed", null],
    asked: [null, "Want me to cut the release?"],
    "asked a day ago": [null, null],
    "asked by a worker": [null, null],
    "asked, then stopped": [null, null],
  });
});

test("the settings cache starts as the contracts default them", () => {
  assert.deepEqual(here(globalSettings), { name: "Orchestrator", host: false, busy: "queue", reports: "auto", group_reports: true, read_aloud: false });
});

test("an ordinary prompt and an ordinary answer hold nothing of the global orchestrator's", () => {
  assert.deepEqual(reportBlocks("no #site, corrige o login.\n\nno #pinned, sobe a versão."), []);
  assert.deepEqual(reportBlocks("olha os logs de ontem"), []);
  assert.equal(quickReplyOf("Publiquei a v2 e escrevi o changelog."), null);
  assert.equal(speechTextOf("Publiquei a v2 e escrevi o changelog."), "Publiquei a v2 e escrevi o changelog.");
});

test("the global orchestrator's tab shows its name where an ordinary tab shows its folder", () => {
  const source = readFileSync(new URL("./gui.js", import.meta.url), "utf8");
  const line = source.slice(source.indexOf("const projectName ="), source.indexOf("\n", source.indexOf("const projectName =")));
  context.threadOf = () => ({});
  context.isGlobalOrchestratorTab = (tab) => tab?.view?.orchestrator_global === true;
  const projectName = vm.runInContext(`${line}; projectName`, context);
  assert.equal(projectName({ view: { cwd: "/home/felipe/.pinned/orquestrador", orchestrator_global: true } }), "Orchestrator");
  assert.equal(projectName({ view: { cwd: "/home/felipe/work/site" } }), "site");
});

test("an approval names its tool and a question names none", () => {
  assert.deepEqual(askOf({ message: "Claude needs your permission to use Bash" }), { kind: "approval", tool: "Bash" });
  assert.deepEqual(askOf({ message: "Claude needs your permission to use mcp__pinned__act" }), { kind: "approval", tool: "mcp__pinned__act" });
  assert.deepEqual(askOf({ message: "Claude needs your permission" }), { kind: "approval", tool: null });
  assert.deepEqual(askOf({ message: "Claude needs your input" }), { kind: "question", tool: null });
  assert.deepEqual(askOf(null), { kind: "question", tool: null });
});
