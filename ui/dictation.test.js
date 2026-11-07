import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function memoryStorage(entries = {}) {
  const items = new Map(Object.entries(entries));
  return { getItem: (key) => items.get(key) ?? null, setItem: (key, value) => items.set(key, String(value)), items };
}

function loadDictation({ language = "en-US", localStorage = memoryStorage() } = {}) {
  const src = readFileSync(new URL("./gui-dictation.js", import.meta.url), "utf8");
  const start = src.indexOf("const DICTATION_LANGUAGES");
  const end = src.indexOf("/** One transcript from either engine");
  const context = { navigator: { language }, localStorage };
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}`, context);
  return context;
}

test("with nothing chosen, the Mac's language picks the dictation locale", () => {
  assert.equal(loadDictation({ language: "pt-PT" }).dictationLocale(), "pt-BR");
  assert.equal(loadDictation({ language: "en-GB" }).dictationLocale(), "en-US");
  assert.equal(loadDictation({ language: "fr-FR" }).dictationLocale(), "en-US");
});

test("a remembered choice wins over the Mac's language", () => {
  const storage = memoryStorage({ "dictation-locale": "pt-BR" });
  assert.equal(loadDictation({ language: "en-US", localStorage: storage }).dictationLocale(), "pt-BR");
  const unknown = memoryStorage({ "dictation-locale": "fr-FR" });
  assert.equal(loadDictation({ language: "en-US", localStorage: unknown }).dictationLocale(), "en-US");
});

test("the language button cycles English, Portuguese, English and remembers it", () => {
  const storage = memoryStorage();
  const dictation = loadDictation({ language: "en-US", localStorage: storage });
  assert.equal(dictation.cycleDictationLocale(), "pt-BR");
  assert.equal(storage.items.get("dictation-locale"), "pt-BR");
  assert.equal(loadDictation({ language: "en-US", localStorage: storage }).dictationLocale(), "pt-BR");
  assert.equal(dictation.cycleDictationLocale(), "en-US");
  assert.equal(dictation.dictationLocale(), "en-US");
  assert.equal(storage.items.get("dictation-locale"), "en-US");
});

test("cycling still works when storage throws", () => {
  const broken = { getItem() { throw new Error("denied"); }, setItem() { throw new Error("denied"); } };
  const dictation = loadDictation({ language: "en-US", localStorage: broken });
  assert.equal(dictation.dictationLocale(), "en-US");
  assert.equal(dictation.cycleDictationLocale(), "pt-BR");
  assert.equal(dictation.dictationLocale(), "pt-BR");
});

test("the button shows the chosen language", () => {
  const dictation = loadDictation({ language: "pt-BR" });
  assert.equal(dictation.dictationLanguage().label, "PT");
  assert.equal(dictation.dictationLanguage().name, "Português");
});

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

test("a phrase dictated mid-text lands at the caret, spaced on both sides", () => {
  const { placeDictation } = loadDictation();
  const placed = placeDictation("fix the bug today", null, [8, 8], "login");
  assert.equal(placed.value, "fix the login bug today");
  assert.deepEqual([...placed.selection], [13, 13]);
});

test("a phrase dictated at the start or the end spaces only the side that has text", () => {
  const { placeDictation } = loadDictation();
  const start = placeDictation("run the tests", null, [0, 0], "please");
  assert.equal(start.value, "please run the tests");
  assert.deepEqual([...start.selection], [6, 6]);
  const end = placeDictation("run the tests", null, [13, 13], "and deploy");
  assert.equal(end.value, "run the tests and deploy");
  assert.deepEqual([...end.selection], [24, 24]);
});

test("no space goes before punctuation or where one already is", () => {
  const { placeDictation } = loadDictation();
  assert.equal(placeDictation("ship it. ", null, [9, 9], "Then rest").value, "ship it. Then rest");
  assert.equal(placeDictation("ship it", null, [7, 7], ", then rest").value, "ship it, then rest");
  assert.equal(placeDictation("ship .", null, [4, 4], "now").value, "ship now .");
  assert.equal(placeDictation("a b", null, [1, 1], "x").value, "a x b");
  assert.equal(placeDictation("ab", null, [1, 1], "x").value, "a x b");
});

test("dictating over a selection replaces it", () => {
  const { placeDictation } = loadDictation();
  const placed = placeDictation("use the old name here", null, [8, 16], "new name");
  assert.equal(placed.value, "use the new name here");
  assert.deepEqual([...placed.selection], [16, 16]);
});

test("each interim result replaces the last one in place, then the final one stays", () => {
  const { placeDictation } = loadDictation();
  let placed = placeDictation("fix the bug", null, [7, 7], "log");
  assert.equal(placed.value, "fix the log bug");
  placed = placeDictation(placed.value, placed.span, placed.selection, "login page");
  assert.equal(placed.value, "fix the login page bug");
  assert.deepEqual([...placed.selection], [18, 18]);
  const next = placeDictation(placed.value, null, placed.selection, "and");
  assert.equal(next.value, "fix the login page and bug");
});

test("a caret the user moved away keeps its place while the phrase grows", () => {
  const { placeDictation } = loadDictation();
  const first = placeDictation("one two", null, [3, 3], "and a");
  assert.equal(first.value, "one and a two");
  const before = placeDictation(first.value, first.span, [0, 0], "and a half");
  assert.equal(before.value, "one and a half two");
  assert.deepEqual([...before.selection], [0, 0]);
  const after = placeDictation(first.value, first.span, [13, 13], "and a half");
  assert.deepEqual([...after.selection], [18, 18]);
});
