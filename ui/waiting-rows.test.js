import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const sidebarSrc = read("gui-sidebar.js");
const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};

/** waitingLookOf as Needs you and the cards read it, with the page around it stubbed. `asking`
 * names the tabs whose card shows a permission menu. */
function looks(asking = []) {
  const context = { phaseOf: (tab) => (asking.includes(tab.key) ? "asking" : "working") };
  vm.createContext(context);
  vm.runInContext(read("global-chat.js").match(/function waitingKindOf[\s\S]*?\n}\n/)[0], context);
  vm.runInContext(read("global-chat.js").match(/function askOf[\s\S]*?\n}\n/)[0], context);
  vm.runInContext([line(sidebarSrc, "const menuIsQuestion = "), fn(sidebarSrc, "waitingLookOf")].join("\n"), context);
  return (tab) => context.waitingLookOf(tab, context.waitingKindOf(tab.view));
}

const tab = (key, view) => ({ key, view: { id: key, status: "needs-input", running: true, interrupted: true, ...view } });

test("a permission ask reads approval, any other ask question, and a failure failed", () => {
  const look = looks();
  assert.equal(look(tab("mini:tab-4", { message: "Claude needs your permission to use Bash" })), "approval");
  assert.equal(look(tab("mini:tab-5", { message: "Claude needs your input" })), "question");
  assert.equal(look(tab("book:tab-6", { status: "failed", message: "API Error: 500" })), "failed");
});

test("a thread whose card shows the permission menu reads approval though its status says failed", () => {
  assert.equal(looks(["book:tab-7"])(tab("book:tab-7", { status: "failed", message: "API Error: 500" })), "approval");
});

test("a question whose menu is on screen still reads question, as its card does", () => {
  assert.equal(looks(["mini:tab-5"])(tab("mini:tab-5", { message: "Claude needs your input" })), "question");
});
