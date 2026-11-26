import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const surfacesSrc = read("gui-surfaces.js");
const line = (src, from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
/** The panel's `type` listener that handles the weight field. */
const listener = (src, type) => {
  const head = `panelBody.addEventListener("${type}", `;
  for (let at = src.indexOf(head); at >= 0; at = src.indexOf(head, at + 1)) {
    const block = src.slice(at, src.indexOf("\n});\n", at) + 4);
    if (block.includes(".memory [data-weight]")) return block;
  }
  assert.fail(`no ${type} listener handles the weight field`);
};
const plain = (value) => JSON.parse(JSON.stringify(value));
const decode = (html) => html.replace(/&#(\d+);/g, (_, c) => String.fromCharCode(c));

/** memoryRowHtml with the real esc. */
const page = vm.createContext({ icon: (name) => `<svg data-icon="${name}"></svg>` });
vm.runInContext(line(settingsSrc, "const esc = "), page);
vm.runInContext(fn(surfacesSrc, "memoryRowHtml"), page);
const row = (memory) => page.memoryRowHtml(memory);
const field = (html) => html.match(/<input [^>]*>/)[0];
const tests = (over = {}) => ({ id: 7, title: "Tests", text: "Run cargo test.", ...over });

test("a memory at weight 1 with no uses has a quiet 1 and no use count", () => {
  const html = row(tests({ weight: 1, hits: 0 }));
  assert.match(field(html), /class="weight quiet"/);
  assert.match(field(html), /value="1"/);
  assert.doesNotMatch(html, /class="uses"|\buses?\b/);
});

test("a weighted memory shows its weight plainly and counts its uses", () => {
  const html = row(tests({ weight: 2.5, hits: 3, last_used: 1763000000 }));
  assert.match(field(html), /value="2.5"/);
  assert.doesNotMatch(field(html), /quiet/);
  assert.ok(html.includes(">3 uses<"), html);
  assert.ok(row(tests({ weight: 1, hits: 1 })).includes(">1 use<"));
});

test("the weight field names its memory, holds 0.1 to 10 and has no name the panel's text fields answer to", () => {
  const input = field(row(tests({ id: 12, weight: 1, hits: 0 })));
  for (const attribute of ['type="number"', 'data-weight="12"', 'aria-label="Weight of #12"', 'min="0.1"', 'max="10"', 'step="0.1"']) {
    assert.ok(input.includes(attribute), `${attribute} in ${input}`);
  }
  assert.doesNotMatch(input, /\sname=/);
  assert.doesNotMatch(row(tests({ weight: undefined })), /data-weight/, "a machine that sends no weight gets no weight field");
});

test("title, kind and text are escaped", () => {
  const html = row({ id: 3, title: "<b>Deploys</b>", kind: "<i>decision</i>", text: `<img src=x onerror=alert(1)> & "tags"`, weight: 1, hits: 0 });
  assert.ok(!html.includes("<b>") && !html.includes("<i>") && !html.includes("<img"), html);
  assert.ok(decode(html).includes(`<b>Deploys</b>`) && decode(html).includes(`<i>decision</i>`) && decode(html).includes(`<img src=x onerror=alert(1)> & "tags"`));
});

/** The memory panel on a stub page: one project with one weight field (id 7, stored 2.5), the
 * panel's render and the listeners asked for, and what the page drew, sent and read again. */
function panel({ ok = true, listen = ["change"] } = {}) {
  const listeners = {};
  const state = { query: "", hits: [tests({ weight: 2.5, hits: 3 })], status: { count: 1, vectors: "off" }, title: "", text: "", at: Date.now() };
  const field = { value: "2.5", dataset: { weight: "7" }, closest: (selector) => (selector === ".memory [data-weight]" ? field : null) };
  const calls = [];
  const drawn = [];
  const refreshes = [];
  const context = vm.createContext({
    active: { machine: "m1" },
    document: { activeElement: null },
    panelBody: { contains: () => true, querySelector: () => null, addEventListener: (type, handler) => (listeners[type] = handler) },
    showPanel: (html) => (drawn.push(html), true),
    hasProject: () => true,
    memoryOf: () => state,
    changeMemory: async (_tab, call) => (calls.push(call), ok),
    refreshMemory: () => refreshes.push(1),
    isGlobalOrchestratorTab: () => false,
    projectName: () => "p",
    globalOrchestratorName: () => "Orchestrator",
    icon: (name) => `<svg data-icon="${name}"></svg>`,
  });
  vm.runInContext(line(settingsSrc, "const esc = "), context);
  vm.runInContext(line(surfacesSrc, "const VECTORS = "), context);
  vm.runInContext(`${fn(surfacesSrc, "memoryRowHtml")}\n${fn(surfacesSrc, "renderMemoryPanel")}`, context);
  for (const type of listen) vm.runInContext(listener(surfacesSrc, type), context);
  return {
    listeners, field, calls, drawn, refreshes,
    focus: (on) => (context.document.activeElement = on ? field : null),
    redraw: () => vm.runInContext("renderMemoryPanel()", context),
  };
}

test("a redraw while a weight field has focus leaves the panel alone, so its partial value stays and nothing is sent", () => {
  const { field, calls, drawn, focus, redraw } = panel();
  focus(true);
  field.value = "2.";
  redraw();
  assert.deepEqual(drawn, [], "the panel's HTML is not rebuilt");
  assert.equal(field.value, "2.");
  assert.deepEqual(calls, []);
  focus(false);
  redraw();
  assert.equal(drawn.length, 1, "the panel redraws once the field lets go");
});

test("a change sends the typed weight", async () => {
  const { listeners, field, calls } = panel();
  field.value = "4";
  await listeners.change({ target: field });
  assert.deepEqual(plain(calls), [{ cmd: "memory_set_weight", id: 7, weight: 4 }]);
});

test("leaving a weight field reads the panel again, so a redraw that focus held back shows", () => {
  const { listeners, field, refreshes } = panel({ listen: ["focusout"] });
  listeners.focusout({ target: field });
  assert.equal(refreshes.length, 1);
});

test("an empty or non-numeric weight sends nothing and shows the stored weight again", async () => {
  for (const value of ["", "  ", "abc"]) {
    const { listeners, field, calls } = panel();
    field.value = value;
    await listeners.change({ target: field });
    assert.deepEqual(calls, [], JSON.stringify(value));
    assert.equal(field.value, "2.5");
  }
});

test("a weight the machine refuses shows the stored weight again", async () => {
  const { listeners, field, calls } = panel({ ok: false });
  field.value = "20";
  await listeners.change({ target: field });
  assert.deepEqual(plain(calls), [{ cmd: "memory_set_weight", id: 7, weight: 20 }]);
  assert.equal(field.value, "2.5");
});
