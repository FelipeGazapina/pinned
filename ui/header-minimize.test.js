import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const style = read("style.css");
const gui = read("gui.css");
const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
const main = read("main.js");
const guiJs = read("gui.js");

// A narrow window must keep the shrink-to-pill buttons reachable instead of clipping them:
// the titles yield while the action buttons hold their size, and past a point the header
// sheds secondary actions rather than pushing minimize out.
test("the terminal header title yields while its buttons hold their size", () => {
  const title = /#title \{([^}]*)\}/.exec(style)?.[1] ?? "";
  assert.match(title, /flex:\s*1/, "the title takes the free space");
  assert.match(title, /min-width:\s*0/, "the title shrinks below its text instead of pushing buttons out");
  const held = /header > :is\(([^)]*)\) \{([^}]*)\}/.exec(style);
  assert.ok(held, "a rule holds the header buttons at their size");
  assert.match(held[1], /button/, "it covers the buttons");
  assert.match(held[2], /flex:\s*none/, "they never shrink");
  for (const id of ["#minimize", "#settings-toggle", "#mode-gui", "#folder", "#attach"]) {
    assert.ok(html.includes(`id="${id.slice(1)}"`), `${id} is in the header`);
  }
  assert.ok(held[1].includes("button"), "the rule reaches the minimize button through its kind");
  const toggle = /#settings-toggle \{([^}]*)\}/.exec(style)?.[1] ?? "";
  assert.match(toggle, /width:\s*22px/, "settings keeps a fixed width like the other buttons");
});

test("a very narrow window sheds actions instead of clipping minimize", () => {
  const block = (px) => {
    const at = style.indexOf(`@media (max-width: ${px}px)`);
    assert.ok(at >= 0, `no breakpoint at ${px}px`);
    const end = style.indexOf("\n}", at);
    return style.slice(at, end < 0 ? at + 400 : end);
  };
  assert.ok(block(400).includes("header") && block(400).includes("gap:"), "tightens the header under 400px");
  for (const id of ["#status-text", "#usage"]) assert.ok(block(280).includes(id), `${id} hides under 280px`);
  assert.ok(block(280).includes("display: none"), "hiding uses display none");
  for (const id of ["#attach", "#folder"]) assert.ok(block(230).includes(id), `${id} hides under 230px`);
  for (const keep of ["#minimize", "#settings-toggle", "#mode-gui"]) {
    assert.ok(!block(280).includes(keep) && !block(230).includes(keep), `${keep} is never hidden`);
  }
});

test("the app header keeps its control cluster from shrinking", () => {
  const controls = /#chat-head \.controls \{([^}]*)\}/.exec(gui)?.[1] ?? "";
  assert.match(controls, /flex:\s*none/, "the cluster holding the minimize button never shrinks");
  const crumb = /#chat-head \.crumb \{([^}]*)\}/.exec(gui)?.[1] ?? "";
  assert.match(crumb, /min-width:\s*0/, "the crumb yields instead");
  assert.ok(html.includes('id="gui-minimize"'), "gui-minimize is in the chat head");
});

test("both minimize buttons still shrink to the pill", () => {
  assert.match(main, /getElementById\("minimize"\)\.onclick = \(\) => setView\("pill"\)/);
  assert.match(guiJs, /getElementById\("gui-minimize"\)\.onclick = \(\) => setView\("pill"\)/);
});

// The diff toolbar at a panel width: its controls need `need` pixels with labels and `icons`
// without, and the bar is `width` wide. A wrapped bar never overflows.
test("the diff toolbar turns its labels to icons first, and wraps only when icons are not enough", () => {
  const src = read("gui-diff.js");
  const context = {};
  vm.createContext(context);
  vm.runInContext(src.slice(src.indexOf("function fitDiffBar"), src.indexOf("const fitDiffBars")), context);
  const fit = (width, need, icons) => {
    const bar = { dataset: {}, clientWidth: width };
    Object.defineProperty(bar, "scrollWidth", { get: () => ({ none: need, icons, wrap: width })[bar.dataset.fold] });
    context.fitDiffBar(bar);
    return bar.dataset.fold;
  };
  assert.equal(fit(539, 470, 370), "none", "at the default panel width the labels fit");
  assert.equal(fit(419, 470, 370), "icons");
  assert.equal(fit(359, 470, 370), "wrap");
  assert.equal(fit(760, 470, 370), "none", "growing the panel back brings the labels back");
});
