import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const sidebarSrc = read("gui-sidebar.js");
const settingsSrc = read("settings.js");
const line = (src, start) => src.slice(src.indexOf(start), src.indexOf("\n", src.indexOf(start)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};

/** The sidebar's waiting rows as the page draws them, with the page around them stubbed. `asking`
 * names the tabs whose card shows a permission menu. */
function sidebar(tabs, asking = []) {
  const context = {
    tabs,
    nowS: () => 1794240000,
    ago: (at) => `${Math.round((1794240000 - at) / 60)}m`,
    threadTitle: (tab) => tab.title,
    projectName: () => "alpha",
    machineName: (id) => ({ mini: "Mac mini", book: "MacBook" })[id],
    phaseOf: (tab) => (asking.includes(tab.key) ? "asking" : "working"),
    document: { createElement: () => ({ dataset: {}, attributes: {}, setAttribute(name, value) { this.attributes[name] = value; } }) },
  };
  vm.createContext(context);
  vm.runInContext(read("global-chat.js").match(/function waitingKindOf[\s\S]*?\n}\n/)[0], context);
  vm.runInContext(read("global-chat.js").match(/function askOf[\s\S]*?\n}\n/)[0], context);
  vm.runInContext(
    [line(settingsSrc, "const esc = "), line(sidebarSrc, "const WAITING_LOOKS = "), fn(sidebarSrc, "waitingLookOf"), fn(sidebarSrc, "askedOf"), fn(sidebarSrc, "waitingRow"), fn(sidebarSrc, "globalWaitingRows")].join("\n"),
    context,
  );
  return context;
}

const tab = (key, machine, title, view) => ({ key, machine, title, view: { id: key, status: "needs-input", running: true, interrupted: true, updated: 1794239940, ...view } });
const words = (row) => row.innerHTML.replace(/<[^>]*>/g, " ").replace(/&#34;/g, '"').replace(/\s+/g, " ").trim();

test("an approval from another machine reads Approval with its machine, its tool and the command it asks", () => {
  const worker = tab("mini:tab-4", "mini", "Apply the orders migration", { message: "Claude needs your permission to use Bash", ask_detail: 'psql -c "alter table orders add column paid_at timestamptz"' });
  const [row, ...rest] = sidebar([worker]).globalWaitingRows();
  assert.equal(rest.length, 0);
  assert.equal(words(row), 'Apply the orders migration Approval · alpha · Mac mini · 1m Bash psql -c "alter table orders add column paid_at timestamptz"');
  assert.equal(row.attributes["aria-label"], "Approval: Apply the orders migration on Mac mini");
  assert.match(row.innerHTML, /<span class="dot approval">/);
  assert.equal(row.dataset.waiting, "mini:tab-4", "clicking it opens that thread, which lives on its machine");
});

test("a question reads Question with what it asks, and a failure reads Failed with its error", () => {
  const question = tab("mini:tab-5", "mini", "Pick who opens the PR", { message: "Claude needs your input", ask_detail: "Which one should open the PR?", updated: 1794239700 });
  const failed = tab("book:tab-6", "book", "Fix the flaky test", { status: "failed", message: "API Error: 500", updated: 1794239280 });
  const rows = sidebar([failed, question]).globalWaitingRows();
  assert.deepEqual(rows.map(words), ["Pick who opens the PR Question · alpha · Mac mini · 5m Which one should open the PR?", "Fix the flaky test Failed · alpha · MacBook · 12m API Error: 500"], "newest first");
});

test("a question the machine reported only as the generic notice shows no third line", () => {
  const question = tab("mini:tab-5", "mini", "Pick who opens the PR", { message: "Claude needs your input" });
  const [row] = sidebar([question]).globalWaitingRows();
  assert.equal(words(row), "Pick who opens the PR Question · alpha · Mac mini · 1m");
});

test("a thread whose card shows the permission menu reads Approval in the list too, though its status says failed", () => {
  const stuck = tab("book:tab-7", "book", "New thread", { status: "failed", message: "API Error: 500" });
  const [row] = sidebar([stuck], ["book:tab-7"]).globalWaitingRows();
  assert.equal(words(row), "New thread Approval · alpha · MacBook · 1m");
});

test("threads that wait on nobody are left out", () => {
  const working = tab("mini:tab-8", "mini", "Busy", { status: "working" });
  const idle = tab("mini:tab-9", "mini", "Idle notice", { interrupted: false, message: "Claude is waiting for your input" });
  assert.deepEqual(sidebar([working, idle]).globalWaitingRows(), []);
});
