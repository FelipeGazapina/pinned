import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const settingsSrc = read("settings.js");
const surfacesSrc = read("gui-surfaces.js");
const panelSrc = read("gui-panel.js");
const line = (src, from) => src.slice(src.indexOf(from), src.indexOf("\n", src.indexOf(from)));
const fn = (src, name) => {
  const at = src.search(new RegExp(`^function ${name}\\(`, "m"));
  assert.ok(at >= 0, `${name} is in the source`);
  return src.slice(at, src.indexOf("\n}\n", at) + 2);
};
/** The Memory panel's code: from its state to the context menu's listener, which is not memory's. */
const memorySection = (src) => {
  const start = src.indexOf("/** Each project's Memory panel");
  return src.slice(start, src.indexOf('panelBody.addEventListener("contextmenu"', start));
};
const plain = (value) => JSON.parse(JSON.stringify(value));
const decode = (html) => html.replace(/&#(\d+);/g, (_, c) => String.fromCharCode(c));
const flush = () => new Promise((done) => setImmediate(done));
const ALPHA = { machine: "m1", path: "/w/alpha" };
const BETA = { machine: "m1", path: "/w/beta" };

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

const attr = (html, name) => html.match(new RegExp(` ${name}="([^"]*)"`))?.[1];
/** A control of the panel drawn as `html`, as a listener sees it: `closest` finds the control for
 * its selector and the panel it was drawn in for `.memory`, whatever the thread active now. */
function control(html, selector, dataset, value = "") {
  const panel = { dataset: { machine: attr(html, "data-machine"), path: attr(html, "data-path") } };
  const el = { dataset, value, closest: (s) => (s === selector ? el : s === ".memory" ? panel : null) };
  return el;
}
/** Memory #7's weight field in the panel drawn as `html`, holding the value the panel drew. Focusing
 * it and replacing the panel does not blur it; typedField is the field that does. */
const weightField = (html) => control(html, ".memory [data-weight]", { weight: "7" }, attr(html.match(/<input [^>]*data-weight="7"[^>]*>/)[0], "value"));
/** Memory #7's weight field as a browser has it once typed in: `blur()` leaves the field the way a
 * real blur does, change then focusout, both to the panel's listeners, while the field is still in
 * the page. Whatever panel replaces it later finds it under the selector showPanel blurs by. */
function typedField(p, html, value) {
  const panel = { dataset: { machine: attr(html, "data-machine"), path: attr(html, "data-path") } };
  const el = { dataset: { weight: "7" }, value, closest: (s) => (s.endsWith("[data-weight]") ? el : s === ".memory" ? panel : null) };
  el.blur = () => {
    p.listeners.change({ target: el });
    p.listeners.focusout({ target: el });
  };
  return el;
}

/** The Memory panel on a stub page, running its real code. Two projects, alpha and beta, each with
 * memory #7 (alpha's weight stored 2.5, beta's 1). The thread and the hub are stubs: `show` makes a
 * thread active and draws its panel, and what the page drew, sent to the hub and read back is kept. */
function panel({ ok = true } = {}) {
  const listeners = {};
  const drawn = [];
  const sent = [];
  const reads = [];
  const flashes = [];
  const stored = { "/w/alpha": 2.5, "/w/beta": 1 };
  const context = vm.createContext({
    active: null,
    document: { activeElement: null },
    panelBody: {
      isConnected: true,
      contains: () => true,
      querySelector: () => null,
      addEventListener: (type, handler) => (listeners[type] = handler),
      set innerHTML(html) {
        drawn.push(html);
        context.document.activeElement = null; // a redraw takes the focused element out of the page
      },
    },
    threadOf: () => ({}),
    projectPathOf: (tab) => tab.path,
    projectName: (tab) => tab.path.slice(3),
    isGlobalOrchestratorTab: () => false,
    globalOrchestratorName: () => "Orchestrator",
    shownSurface: () => ({ kind: "memory" }),
    renderPanel: () => vm.runInContext("renderMemoryPanel()", context),
    callOn: async (machine, call) => {
      if (call.cmd === "memory_search") {
        reads.push(call.path);
        return [{ id: 7, title: `seven of ${call.path}`, text: "Run cargo test.", weight: stored[call.path], hits: 3 }];
      }
      if (call.cmd === "memory_status") return { count: 1, vectors: "off" };
      sent.push({ machine, call: plain(call) });
      if (!ok) throw "refused";
      return {};
    },
    flash: (message) => flashes.push(message),
    icon: (name) => `<svg data-icon="${name}"></svg>`,
  });
  vm.runInContext(line(settingsSrc, "const esc = "), context);
  vm.runInContext(memorySection(surfacesSrc), context);
  vm.runInContext(line(panelSrc, "let shownPanel"), context);
  vm.runInContext(fn(panelSrc, "showPanel"), context);
  const redraw = () => vm.runInContext("renderMemoryPanel()", context);
  return {
    listeners, drawn, sent, reads, flashes,
    redraw,
    show: async (tab) => {
      context.active = tab;
      redraw();
      await flush();
    },
    switchTo: (tab) => (context.active = tab),
    /** Another surface's render of the panel, as the Diff or a file list draws it. */
    showHtml: (html) => vm.runInContext("showPanel", context)(html),
    focus: (el) => (context.document.activeElement = el),
  };
}

test("a redraw while a weight field has focus leaves the panel alone, so its partial value stays and nothing is sent", async () => {
  const p = panel();
  await p.show(ALPHA);
  const input = weightField(p.drawn.at(-1));
  const drawn = p.drawn.length;
  p.focus(input);
  input.value = "2.";
  p.redraw();
  assert.equal(p.drawn.length, drawn, "the panel's HTML is not rebuilt");
  assert.equal(input.value, "2.");
  assert.deepEqual(p.sent, []);
  p.focus(null);
  p.redraw();
  assert.equal(p.drawn.length, drawn, "the same panel is not written again once the field lets go");
});

test("a change sends the typed weight to the project its field was drawn for, after the thread became another project", async () => {
  const p = panel();
  await p.show(BETA);
  const input = weightField(p.drawn.at(-1));
  input.value = "4";
  await p.show(ALPHA);
  await p.listeners.change({ target: input });
  assert.deepEqual(p.sent, [{ machine: "m1", call: { cmd: "memory_set_weight", id: 7, weight: 4, path: "/w/beta" } }]);
});

test("a weight typed in alpha's panel is sent for alpha by the panel's own blur when beta's Memory panel replaces it", async () => {
  const p = panel();
  await p.show(ALPHA);
  const input = typedField(p, p.drawn.at(-1), "9");
  p.focus(input);
  await p.show(BETA);
  assert.deepEqual(p.sent, [{ machine: "m1", call: { cmd: "memory_set_weight", id: 7, weight: 9, path: "/w/alpha" } }]);
  assert.match(p.drawn.at(-1), /seven of \/w\/beta/);
});

test("a weight typed in alpha's panel is sent for alpha by the blur when beta's thread shows another surface", async () => {
  const p = panel();
  await p.show(ALPHA);
  const input = typedField(p, p.drawn.at(-1), "9");
  p.focus(input);
  p.switchTo(BETA);
  p.showHtml(`<div class="diff">beta's diff</div>`);
  await flush();
  assert.deepEqual(p.sent, [{ machine: "m1", call: { cmd: "memory_set_weight", id: 7, weight: 9, path: "/w/alpha" } }]);
  assert.equal(p.drawn.at(-1), `<div class="diff">beta's diff</div>`);
});

test("a weight typed in one machine's panel goes to that machine when another machine's thread has the same path", async () => {
  const p = panel();
  const ALPHA_ON_M2 = { machine: "m2", path: "/w/alpha" };
  await p.show(ALPHA);
  const input = typedField(p, p.drawn.at(-1), "9");
  p.focus(input);
  await p.show(ALPHA_ON_M2);
  assert.deepEqual(p.sent, [{ machine: "m1", call: { cmd: "memory_set_weight", id: 7, weight: 9, path: "/w/alpha" } }]);
});

test("a forget tapped on alpha's row is sent for alpha when the tap lands after the active thread became beta", async () => {
  const p = panel();
  await p.show(ALPHA);
  const forget = control(p.drawn.at(-1), ".memory [data-forget]", { forget: "7" });
  await p.show(BETA);
  await p.listeners.click({ target: forget });
  assert.deepEqual(p.sent, [{ machine: "m1", call: { cmd: "memory_forget", id: 7, path: "/w/alpha" } }]);
});

test("a redraw for beta while alpha's weight field has focus replaces the panel with beta's rows", async () => {
  const p = panel();
  await p.show(ALPHA);
  p.focus(weightField(p.drawn.at(-1)));
  await p.show(BETA);
  assert.match(p.drawn.at(-1), /seven of \/w\/beta/);
  assert.doesNotMatch(p.drawn.at(-1), /seven of \/w\/alpha/);
});

test("leaving alpha's weight field after the active thread became beta reads alpha's memory again", async () => {
  const p = panel();
  await p.show(ALPHA);
  const input = weightField(p.drawn.at(-1));
  p.focus(input);
  await p.show(BETA);
  p.reads.length = 0;
  p.listeners.focusout({ target: input });
  await flush();
  assert.deepEqual(p.reads, ["/w/alpha"]);
});

test("an add submitted in alpha's panel is saved to alpha when the submit lands after the active thread became beta", async () => {
  const p = panel();
  await p.show(ALPHA);
  const html = p.drawn.at(-1);
  await p.listeners.input({ target: Object.assign(control(html, ".memory [name]", {}, "Deploys"), { name: "title" }) });
  await p.listeners.input({ target: Object.assign(control(html, ".memory [name]", {}, "Run it."), { name: "text" }) });
  await p.show(BETA);
  await p.listeners.submit({ target: control(html, ".memory-add", {}), preventDefault() {} });
  assert.deepEqual(p.sent, [{ machine: "m1", call: { cmd: "memory_save", title: "Deploys", text: "Run it.", path: "/w/alpha" } }]);
});

test("an empty or non-numeric weight sends nothing and shows the stored weight again", async () => {
  for (const value of ["", "  ", "abc"]) {
    const p = panel();
    await p.show(ALPHA);
    const input = weightField(p.drawn.at(-1));
    input.value = value;
    await p.listeners.change({ target: input });
    assert.deepEqual(p.sent, [], JSON.stringify(value));
    assert.equal(input.value, "2.5");
  }
});

test("a weight the machine refuses shows the stored weight again", async () => {
  const p = panel({ ok: false });
  await p.show(ALPHA);
  const input = weightField(p.drawn.at(-1));
  input.value = "20";
  await p.listeners.change({ target: input });
  assert.deepEqual(p.sent, [{ machine: "m1", call: { cmd: "memory_set_weight", id: 7, weight: 20, path: "/w/alpha" } }]);
  assert.equal(input.value, "2.5");
});
