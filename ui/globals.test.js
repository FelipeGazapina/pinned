import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// index.html loads these as classic scripts, so every top-level declaration is one page-wide
// global. A second `const`/`let`/`class` of a name throws and its file never runs. A second
// `function` or `var` of a name silently replaces the first, and the earlier file's callers
// get the wrong one with no error (editorHtml in settings.js vs gui-panel.js).
const DECLARATION = /^(?:async\s+function\s*\*?|function\s*\*?|class|const|let|var)\s+([A-Za-z_$][\w$]*)/;

function classicScripts() {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  return [...html.matchAll(/<script\b([^>]*)>/g)]
    .map((match) => match[1])
    .filter((attrs) => !/type="module"/.test(attrs))
    .map((attrs) => /src="([^"]+)"/.exec(attrs)?.[1])
    .filter((src) => src && !/^(https?:)?\/\//.test(src));
}

export function topLevelDeclarations(source) {
  const found = [];
  source.split("\n").forEach((line, index) => {
    const name = DECLARATION.exec(line)?.[1];
    if (name) found.push({ name, line: index + 1 });
  });
  return found;
}

export function duplicateGlobals(files) {
  const sites = new Map();
  for (const [file, source] of files) {
    for (const { name, line } of topLevelDeclarations(source)) {
      sites.set(name, [...(sites.get(name) ?? []), { file, line }]);
    }
  }
  return [...sites]
    .filter(([, where]) => new Set(where.map((site) => site.file)).size > 1)
    .map(([name, where]) => `${name}: ${where.map((site) => `${site.file}:${site.line}`).join(" and ")}`);
}

test("no two classic scripts declare the same top-level name", () => {
  const files = classicScripts().map((file) => [`ui/${file}`, readFileSync(new URL(`./${file}`, import.meta.url), "utf8")]);
  assert.ok(files.length > 30, `expected the page's scripts, found ${files.length}`);
  assert.deepEqual(duplicateGlobals(files), []);
});

test("the duplicate finder reports a function in two files with both locations", () => {
  const files = [
    ["ui/a.js", "const x = 1;\nfunction editorHtml() {}\n"],
    ["ui/b.js", "async function other() {}\n\nfunction editorHtml(path) {}\nclass K {}\n"],
    ["ui/c.js", "  function indented() {}\nvar x = 2;\n"],
  ];
  assert.deepEqual(duplicateGlobals(files), ["x: ui/a.js:1 and ui/c.js:2", "editorHtml: ui/a.js:2 and ui/b.js:3"]);
});

test("the finder sees the declaration forms the page's scripts use", () => {
  const source = ["function a() {}", "async function b() {}", "class C {}", "const d = 1;", "let e;", "var f;", "  const nested = 1;", "// const note"].join("\n");
  assert.deepEqual(topLevelDeclarations(source).map((d) => d.name), ["a", "b", "C", "d", "e", "f"]);
});
