import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// A deferred classic script runs after every plain one, so a plain script that follows it
// and uses its globals throws at boot and the login screen never shows.
test("no plain script follows a deferred one", () => {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  const tags = [...html.matchAll(/<script\b([^>]*)>/g)]
    .map((match) => match[1])
    .filter((attrs) => !/type="module"/.test(attrs))
    .map((attrs) => ({ src: /src="([^"]+)"/.exec(attrs)?.[1] ?? "", deferred: /\bdefer\b/.test(attrs) }));
  const first = tags.findIndex((tag) => tag.deferred);
  const early = first < 0 ? [] : tags.slice(first + 1).filter((tag) => !tag.deferred).map((tag) => tag.src);
  assert.deepEqual(early, [], `these run before ${tags[first]?.src}: ${early.join(", ")}`);
});

test("the graph editor loads as a module that settings.js reads when it renders", () => {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  assert.match(html, /<script type="module" src="graph-load.js"><\/script>/);
  const settings = readFileSync(new URL("./settings.js", import.meta.url), "utf8");
  const topLevel = settings.split("\n").filter((line) => /^\S/.test(line) && /PinnedGraph/.test(line));
  assert.deepEqual(topLevel, ["const graphApi = () => globalThis.PinnedGraph;"]);
});
