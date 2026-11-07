import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function read(name) {
  return readFileSync(new URL(name, import.meta.url), "utf8");
}

function element() {
  return {
    hidden: true,
    dataset: {},
    innerHTML: "",
    style: {},
    append() {},
    querySelector() {
      return null;
    },
    querySelectorAll() {
      return [];
    },
    addEventListener() {},
  };
}

const timeline = read("./gui-timeline.js");
const sandbox = {
  threadView: element(),
  composer: { dataset: {} },
  prompt: { value: "" },
  active: null,
  ui: {},
  document: {
    body: { dataset: {} },
    addEventListener() {},
    getElementById() {
      return element();
    },
    querySelector() {
      return element();
    },
  },
  MutationObserver: class {
    observe() {}
  },
  ResizeObserver: class {
    observe() {}
  },
  requestAnimationFrame() {
    return 0;
  },
  setTimeout() {
    return 0;
  },
  callOn: async () => null,
};
vm.createContext(sandbox);
vm.runInContext(read("./vendor/marked.js"), sandbox);
vm.runInContext(
  `
function esc(text) { return String(text ?? "").replace(/[&<>"']/g, (c) => "&#" + c.charCodeAt(0) + ";"); }
function icon(name) { return '<svg data-icon="' + name + '"></svg>'; }
function fileChip(raw) {
  const path = raw.trim();
  if (/\\s|^[@~]/.test(path) || !path.includes("/")) return null;
  const base = path.split("/").pop();
  return '<span class="file-chip" data-file-path="' + esc(path) + '">' + esc(base) + "</span>";
}
const WEB_URL = /^https?:\\/\\//i;
const PR_URL = /$a/;
function linkIcon() { return ""; }
function highlightLines(lines) { return lines; }
function wraps() { return false; }
function codeKey(text) { return text; }
const SHELL_LANGS = /$a/;
${timeline.slice(timeline.indexOf("/** T3 Code's markdown images"), timeline.indexOf("/** `@src/app.ts`"))}
Object.assign(globalThis, { marked, RENDERER, placeInlineImages });
`,
  sandbox,
);

function render(markdown) {
  return sandbox.marked.parse(markdown);
}

test("a codespan naming a picture keeps its chip and adds an inline image", () => {
  const html = render("see `/tmp/a/shot.png` here");
  assert.match(html, /class="file-chip"[^>]*data-file-path="\/tmp\/a\/shot\.png"/);
  assert.match(html, /<img class="md-image" data-inline alt="shot\.png" data-name="shot\.png"[^>]*data-local="\/tmp\/a\/shot\.png">/);
});

test("a tilde picture codespan draws the image beside the codespan", () => {
  const html = render("see `~/x.png` here");
  assert.match(html, /<code>~\/x\.png<\/code>/);
  assert.match(html, /data-inline[^>]*data-local="~\/x\.png"/);
});

test("a markdown link to a picture keeps its chip and adds an inline image", () => {
  const html = render("[menu](/Users/felipe/scratchpad/menu.png)");
  assert.match(html, /class="file-chip"[^>]*data-file-path="\/Users\/felipe\/scratchpad\/menu\.png"/);
  assert.match(html, /data-inline[^>]*data-local="\/Users\/felipe\/scratchpad\/menu\.png"/);
});

test("a chip-derived image moves to the end of its block, once", () => {
  const html = render("- `/tmp/a/shot.png`: the project filter, and again `/tmp/a/shot.png`\n  - nested");
  const box = {
    tag: "li",
    children: [],
    matches(sel) {
      return sel === "li";
    },
    closest() {
      return this;
    },
    querySelector() {
      return this.children.find((child) => child.tag === "ul") ?? null;
    },
    insertBefore(img, before) {
      const at = this.children.indexOf(img);
      if (at >= 0) this.children.splice(at, 1);
      this.children.splice(before ? this.children.indexOf(before) : this.children.length, 0, img);
    },
  };
  const images = [...html.matchAll(/data-local="([^"]+)"/g)].map(([, path]) => ({
    dataset: { local: path },
    closest() {
      return box;
    },
    remove() {
      const at = box.children.indexOf(this);
      if (at >= 0) box.children.splice(at, 1);
    },
  }));
  const nested = { tag: "ul" };
  box.children = [...images, nested];
  sandbox.placeInlineImages({ querySelectorAll: () => images });
  assert.deepEqual(box.children.map((child) => child.dataset?.local ?? child.tag), ["/tmp/a/shot.png", "ul"]);
});

test("a codespan that is not an image reference stays a chip or plain code", () => {
  assert.match(render("`src/app.ts:12`"), /class="file-chip"/);
  assert.doesNotMatch(render("`src/app.ts:12`"), /md-image/);
  assert.doesNotMatch(render("`shot.png`"), /md-image/);
  assert.match(render("`shot.png`"), /<code>shot\.png<\/code>/);
});

test("the same picture named in two list items shows in each", () => {
  const box = () => ({
    children: [],
    matches: () => true,
    querySelector: () => null,
    insertBefore(img) {
      if (!this.children.includes(img)) this.children.push(img);
    },
  });
  const first = box();
  const second = box();
  const image = (block) => ({ dataset: { local: "/tmp/a/shot.png" }, closest: () => block, remove() {} });
  const images = [image(first), image(second)];
  sandbox.placeInlineImages({ querySelectorAll: () => images });
  assert.deepEqual([first.children.length, second.children.length], [1, 1]);
});

test("plain code renders exactly as marked renders it", () => {
  assert.equal(render("`a &amp; <b>`"), "<p><code>a &amp;amp; &lt;b&gt;</code></p>\n");
});
