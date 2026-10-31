import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

function load() {
  const source = readFileSync(new URL("./main.js", import.meta.url), "utf8");
  const start = source.indexOf("function headerUpdateResult");
  const end = source.indexOf("async function installUpdate");
  if (start < 0 || end < 0) throw new Error("headerUpdateResult is missing");
  return new Function(`${source.slice(start, end)}\nreturn headerUpdateResult;`)();
}

test("a failed, refused, or current header update can be pressed again", () => {
  const result = load();
  assert.equal(result({ state: "failed", reason: "checksum mismatch" }), "error");
  assert.equal(result({ state: "cannot", reason: "update by hand" }), "error");
  assert.equal(result({ state: "up_to_date", version: "0.16.0" }), "refresh");
  assert.equal(result({ state: "restarting", to: "0.17.0" }), "wait");
  assert.equal(result({ state: "updating", to: "0.17.0" }), "wait");
});
