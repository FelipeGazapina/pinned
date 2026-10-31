import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function mq(coarse, fine) {
  return (query) => ({
    matches: query === "(pointer: coarse)" ? coarse : query === "(any-pointer: fine)" ? fine : false,
  });
}

function loadComposerEnter() {
  const src = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
  const start = src.indexOf("function touchComposer");
  const end = src.indexOf("const SEND_MOD = IS_MAC");
  const context = { matchMedia: mq(false, true) };
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}`, context);
  return context;
}

function loadTriggerKey() {
  const src = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
  const start = src.indexOf("function triggerKey");
  const end = src.indexOf('document.getElementById("stash").insertAdjacentHTML');
  const picks = [];
  const rows = [
    { classList: { add() {}, remove() {} }, dataset: { pick: "/help", url: "" }, scrollIntoView() {} },
  ];
  rows[0].classList.contains = (c) => c === "highlight";
  const composerMenu = { hidden: false, querySelectorAll: () => rows };
  const context = {
    composerMenu,
    prompt: { value: "/he" },
    pickTrigger: (pick) => picks.push(pick),
  };
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}\nglobalThis.triggerKey = triggerKey;`, context);
  return { triggerKey: context.triggerKey, picks, composerMenu };
}

test("touch without a fine pointer treats Enter as a newline, not send", () => {
  const { touchComposer, composerEnterIntent } = loadComposerEnter();
  assert.equal(touchComposer(mq(true, false)), true);
  assert.equal(touchComposer(mq(true, true)), false);
  const enter = { key: "Enter", shiftKey: false, isComposing: false, metaKey: false, ctrlKey: false };
  assert.equal(composerEnterIntent(enter, { promptText: "hi", shortcut: "enter", touch: true, isMac: true }), "newline");
  assert.equal(composerEnterIntent(enter, { promptText: "hi", shortcut: "enter", touch: false, isMac: true }), "send");
});

test("desktop send shortcut still sends on Enter when configured", () => {
  const { composerEnterIntent } = loadComposerEnter();
  const enter = { key: "Enter", shiftKey: false, isComposing: false, metaKey: false, ctrlKey: false };
  assert.equal(composerEnterIntent(enter, { promptText: "one line", shortcut: "enter", touch: false, isMac: false }), "send");
  assert.equal(
    composerEnterIntent(enter, { promptText: "line one\nline two", shortcut: "mod-enter-multiline", touch: false, isMac: false }),
    "newline",
  );
  assert.equal(
    composerEnterIntent(
      { ...enter, ctrlKey: true },
      { promptText: "line one\nline two", shortcut: "mod-enter-multiline", touch: false, isMac: false },
    ),
    "send",
  );
});

test("slash menu Enter still picks while touch would newline in the bare prompt", () => {
  const { triggerKey, picks, composerMenu } = loadTriggerKey();
  const handled = triggerKey({ key: "Enter", shiftKey: false });
  assert.equal(handled, true);
  assert.deepEqual(picks, ["/help"]);
  assert.equal(composerMenu.hidden, false);
});
