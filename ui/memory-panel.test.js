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
const row = (memory, typed) => page.memoryRowHtml(memory, typed);
const field = (html) => html.match(/<input [^>]*>/)[0];
const tests = (over = {}) => ({ id: 7, title: "Tests", text: "Run cargo test.", ...over });

test("a memory at weight 1 with no uses, or from a machine that sends neither, has a quiet 1 and no use count", () => {
  for (const memory of [tests({ weight: 1, hits: 0 }), tests()]) {
    const html = row(memory);
    assert.match(field(html), /class="weight quiet"/);
    assert.match(field(html), /value="1"/);
    assert.doesNotMatch(html, /class="uses"|\buses?\b/);
  }
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
  assert.match(field(row(tests({ weight: 2.5 }), "3.")), /value="3\."/, "an unsent weight is drawn in place of the stored one");
});

test("title, kind and text are escaped", () => {
  const html = row({ id: 3, title: "<b>Deploys</b>", kind: "<i>decision</i>", text: `<img src=x onerror=alert(1)> & "tags"`, weight: 1, hits: 0 });
  assert.ok(!html.includes("<b>") && !html.includes("<i>") && !html.includes("<img"), html);
  assert.ok(decode(html).includes(`<b>Deploys</b>`) && decode(html).includes(`<i>decision</i>`) && decode(html).includes(`<img src=x onerror=alert(1)> & "tags"`));
});

/** The panel's input and change listeners on a stub panel: one project's state, and a
 * changeMemory that records each call and answers `ok`. */
function panel(ok = true) {
  const listeners = {};
  const state = { query: "", hits: [tests({ weight: 2.5, hits: 3 })], weights: {} };
  const calls = [];
  const context = vm.createContext({
    active: { machine: "m1" },
    panelBody: { addEventListener: (type, handler) => (listeners[type] = handler) },
    memoryOf: () => state,
    changeMemory: async (_tab, call) => (calls.push(call), ok),
    refreshMemory: () => {},
    setTimeout,
    clearTimeout,
  });
  vm.runInContext(`let memorySearchTimer;\n${listener(surfacesSrc, "input")}\n${listener(surfacesSrc, "change")}`, context);
  const weightField = (value) => {
    const input = { value, dataset: { weight: "7" } };
    input.closest = (selector) => (selector === ".memory [data-weight]" ? input : null);
    return input;
  };
  return { listeners, state, calls, weightField };
}

test("typing a weight keeps it unsent in the panel's state, and a change sends it", async () => {
  const { listeners, state, calls, weightField } = panel();
  const input = weightField("4");
  listeners.input({ target: input });
  assert.deepEqual(plain(state.weights), { 7: "4" });
  assert.deepEqual(calls, [], "typing sends nothing");
  await listeners.change({ target: input });
  assert.deepEqual(plain(calls), [{ cmd: "memory_set_weight", id: 7, weight: 4 }]);
  assert.deepEqual(plain(state.weights), {}, "the sent weight is no longer unsent");
});

test("an empty or non-numeric weight sends nothing and shows the stored weight again", async () => {
  for (const value of ["", "  ", "abc"]) {
    const { listeners, state, calls, weightField } = panel();
    const input = weightField(value);
    listeners.input({ target: input });
    await listeners.change({ target: input });
    assert.deepEqual(calls, [], JSON.stringify(value));
    assert.equal(input.value, "2.5");
    assert.deepEqual(plain(state.weights), {});
  }
});

test("a weight the machine refuses shows the stored weight again", async () => {
  const { listeners, calls, weightField } = panel(false);
  const input = weightField("20");
  await listeners.change({ target: input });
  assert.deepEqual(plain(calls), [{ cmd: "memory_set_weight", id: 7, weight: 20 }]);
  assert.equal(input.value, "2.5");
});
