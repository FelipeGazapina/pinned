import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

test("deferred settings.js still runs before the scripts that use it", () => {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  const tags = [...html.matchAll(/<script\b([^>]*)>/g)].map((match) => {
    const attrs = match[1];
    const src = /src="([^"]+)"/.exec(attrs)?.[1] ?? "";
    const deferred = /\bdefer\b/.test(attrs) || /type="module"/.test(attrs);
    return { src, deferred, module: /type="module"/.test(attrs) };
  });
  const graph = tags.findIndex((tag) => tag.src === "graph-load.js");
  const settings = tags.findIndex((tag) => tag.src === "settings.js");
  assert.ok(graph >= 0 && settings > graph, "graph-load.js precedes settings.js");
  assert.equal(tags[graph].module, true, "graph-load.js is a module so it can import the editor");
  assert.equal(tags[settings].deferred, true, "settings.js waits for the graph module");
  const early = tags.slice(settings + 1).filter((tag) => tag.src && !tag.deferred).map((tag) => tag.src);
  assert.deepEqual(early, [], `classic scripts run before deferred settings.js and throw: ${early.join(", ")}`);
});
