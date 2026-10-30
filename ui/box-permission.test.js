import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function element() {
  return {
    addEventListener() {},
    dataset: {},
    hidden: false,
    innerHTML: "",
    textContent: "",
    classList: { contains() { return false }, add() {}, remove() {} },
  };
}

test("the Upstash card offers a box permission for each actor kind", () => {
  const context = {
    console,
    addEventListener() {},
    account: { email: "ana@acme.dev" },
    load() { return null; },
    save() {},
    listen() {},
    document: {
      body: { dataset: {} },
      getElementById() { return element(); },
      querySelector() { return element(); },
    },
  };
  context.globalThis = context;
  const source = readFileSync(new URL("./settings.js", import.meta.url), "utf8");
  vm.runInNewContext(`${source}\nglobalThis.__card = upstashCardHtml;\nglobalThis.__settings = settings;`, context);
  context.__settings.upstash = { key_hint: "…a1b2", claude_hint: "…9f3c" };
  context.__settings.box_asks = [{ id: "ask1", connector: "c1", subject: { issue: "ENG-7" } }];
  const html = context.__card();
  assert.match(html, /id="box-workflow"/);
  assert.match(html, /<option value="fresh" selected>New box each time<\/option>/);
  assert.match(html, /id="box-session"/);
  assert.match(html, /id="box-worker"/);
  assert.match(html, /id="box-orchestrator"/);
  assert.match(html, /<option value="off" selected>Off<\/option>/);
  assert.match(html, /data-action="answer-box" data-id="ask1" data-allow="yes"/);
  assert.match(html, /Allow a box for issue ENG-7/);
  context.__settings.boxes = { workflow: { mode: "ask" }, session: { mode: "fixed", row: "row-1" } };
  context.__settings.box_asks = [];
  const chosen = context.__card();
  assert.match(chosen, /id="box-workflow"[\s\S]*<option value="ask" selected>Ask first<\/option>/);
  assert.match(chosen, /id="box-session"[\s\S]*<option value="fixed" selected>One box, reused<\/option>/);
  assert.doesNotMatch(chosen, /answer-box/);
});
