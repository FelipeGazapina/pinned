import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function loadDictation() {
  const src = readFileSync(new URL("./gui-dictation.js", import.meta.url), "utf8");
  const start = src.indexOf("// Terms Whisper/SFSpeech");
  const end = src.indexOf("/** One transcript from either engine");
  const context = { navigator: { language: "en-US" } };
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}`, context);
  return context;
}

test("english fillers drop but meaning stays", () => {
  const { dictationCleanup } = loadDictation();
  assert.equal(
    dictationCleanup("um so we should uh refactor the user service"),
    "so we should refactor the user service",
  );
});

test("portuguese keeps 'um' (it means a/one)", () => {
  const { dictationCleanup } = loadDictation();
  assert.equal(dictationCleanup("encontrei um bug no deploy", "pt-BR"), "encontrei um bug no deploy");
});

test("hallucinations on silence become nothing", () => {
  const { dictationCleanup } = loadDictation();
  assert.equal(dictationCleanup("Thank you."), "");
  assert.equal(dictationCleanup("Obrigado."), "");
});

test("case phrases become identifiers", () => {
  const { dictationCleanup } = loadDictation();
  assert.equal(dictationCleanup("rename it to snake case max retry count"), "rename it to max_retry_count");
  assert.equal(dictationCleanup("call it camel case fetch user"), "call it fetchUser");
});

test("spoken file shapes become paths", () => {
  const { dictationCleanup } = loadDictation();
  assert.equal(dictationCleanup("put it in utils dot ts"), "put it in utils.ts");
  assert.equal(dictationCleanup("push to feature slash login"), "push to feature/login");
  assert.equal(dictationCleanup("run git push dash dash force"), "run git push --force");
});

test("prose dots survive", () => {
  const { dictationCleanup } = loadDictation();
  assert.equal(dictationCleanup("I agree. Dot. Next topic"), "I agree. Dot. Next topic");
});

test("glossary fixes dev terms without rewording", () => {
  const { dictationCleanup } = loadDictation();
  assert.equal(dictationCleanup("check cube control get pods"), "check kubectl get pods");
  assert.equal(dictationCleanup("open a pool request on get hub"), "open a pull request on GitHub");
});

test("joining phrases spaces them once", () => {
  const { joinDictation } = loadDictation();
  assert.equal(joinDictation("deploy Friday", "and run tests"), "deploy Friday and run tests");
  assert.equal(joinDictation("", "hello"), "hello");
  assert.equal(joinDictation("done", ""), "done");
});
