import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const src = readFileSync(new URL("./gui.js", import.meta.url), "utf8");
const start = src.indexOf("const firstPrompt");
const end = src.indexOf("/** `text` as a head", start);

const threadTitleContext = ({ titles = {}, selfTitles = {} }) => {
  const sandbox = { sideMarks: { titles, selfTitles }, threadKey: (tab) => `session:${tab.view.session}`, threadOf: () => ({ items: [] }) };
  vm.createContext(sandbox);
  vm.runInContext(src.slice(start, end), sandbox);
  return vm.runInContext("threadTitle", sandbox);
};

test("the user's own name beats the session's own name, which beats the generated title", () => {
  const view = { session: "s1", title: "Pinned 0.18.7 release orchestration" };
  const userMarked = threadTitleContext({ titles: { "session:s1": "Nome que eu dei" }, selfTitles: { "session:s1": { text: "Libera o formulário", author: "captain", at: 1 } } });
  assert.equal(userMarked({ view }), "Nome que eu dei");
  const captainNamed = threadTitleContext({ selfTitles: { "session:s1": { text: "Libera o formulário", author: "captain", at: 1 } } });
  assert.equal(captainNamed({ view }), "Libera o formulário");
  const plain = threadTitleContext({});
  assert.equal(plain({ view }), "Pinned 0.18.7 release orchestration");
});

test("the user clearing their own title falls back to the session's, then the generated one", () => {
  const view = { session: "s1", title: "Pinned 0.18.7 release orchestration" };
  const cleared = threadTitleContext({ selfTitles: { "session:s1": { text: "Libera o formulário", author: "captain", at: 1 } } });
  assert.equal(cleared({ view }), "Libera o formulário");
  const neither = threadTitleContext({});
  assert.equal(neither({ view }), "Pinned 0.18.7 release orchestration");
});
