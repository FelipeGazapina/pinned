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
  const start = src.indexOf("function enterNeedsMod");
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

function keydown(extra = {}) {
  const event = {
    key: "Enter",
    shiftKey: false,
    altKey: false,
    metaKey: false,
    ctrlKey: false,
    isComposing: false,
    defaultPrevented: false,
    ...extra,
  };
  event.preventDefault = () => {
    event.defaultPrevented = true;
  };
  return event;
}

function highlighted(pick) {
  return {
    classList: { add() {}, remove() {}, contains: (cls) => cls === "highlight" },
    dataset: { pick, url: "" },
    scrollIntoView() {},
  };
}

function loadPromptKey({ promptValue, coarse, fine, isMac = true, rows = [], menuHidden = true, shortcut = "enter" }) {
  const src = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
  const helpers = src.slice(src.indexOf("function enterNeedsMod"), src.indexOf("const SEND_MOD"));
  const trigger = src.slice(src.indexOf("function triggerKey"), src.indexOf('document.getElementById("stash").insertAdjacentHTML'));
  const listenerRaw = src.slice(src.indexOf('prompt.addEventListener("keydown", (e) => {'), src.indexOf('document.getElementById("send").onclick'));
  const listener = listenerRaw
    .replace('prompt.addEventListener("keydown", (e) => {', "function onPromptKeyDown(e) {")
    .replace(/\}\);\s*$/, "}");
  const picks = [];
  const sent = [];
  const prompt = { value: promptValue };
  const composerMenu = { hidden: menuHidden, dataset: { kind: rows.length ? "slash" : "" }, querySelectorAll: () => rows };
  const context = {
    matchMedia: mq(coarse, fine),
    prompt,
    composerMenu,
    sendShortcut: shortcut,
    IS_MAC: isMac,
    active: null,
    SHIFT_TAB: "",
    isTrigger: () => rows.length > 0 && !composerMenu.hidden,
    pickTrigger: (pick) => {
      picks.push(pick);
      composerMenu.hidden = true;
      prompt.value = `${pick} `;
    },
    deleteChip: () => false,
    promptCodeKey: () => false,
    sendAndNewThread: () => sent.push("new-thread"),
    send: () => sent.push(prompt.value),
    phaseOf: () => "idle",
    stop() {},
    recall: () => false,
    writeTo() {},
  };
  vm.createContext(context);
  vm.runInContext(
    `${helpers}\n${trigger}\n${listener}\nglobalThis.onPromptKeyDown = onPromptKeyDown;\nglobalThis.composerEnterIntent = composerEnterIntent;\nglobalThis.touchComposer = touchComposer;`,
    context,
  );
  const element = {
    listener: null,
    addEventListener(type, fn) {
      if (type === "keydown") this.listener = fn;
    },
    dispatch(event) {
      this.listener(event);
    },
  };
  element.addEventListener("keydown", context.onPromptKeyDown);
  return { ...context, prompt, composerMenu, picks, sent, element };
}

test("Cmd or Ctrl+Enter sends on a touch device with no trackpad", () => {
  const mac = loadPromptKey({ promptValue: "hello", coarse: true, fine: false, isMac: true });
  const cmd = keydown({ metaKey: true });
  mac.element.dispatch(cmd);
  assert.equal(mac.touchComposer(), true);
  assert.equal(mac.composerEnterIntent(cmd, { promptText: "hello", shortcut: "enter", touch: true, isMac: true }), "send");
  assert.equal(cmd.defaultPrevented, true);
  assert.deepEqual(mac.sent, ["hello"]);

  const win = loadPromptKey({ promptValue: "hello", coarse: true, fine: false, isMac: false });
  const ctrl = keydown({ ctrlKey: true });
  win.element.dispatch(ctrl);
  assert.equal(win.composerEnterIntent(ctrl, { promptText: "hello", shortcut: "enter", touch: true, isMac: false }), "send");
  assert.equal(ctrl.defaultPrevented, true);
  assert.deepEqual(win.sent, ["hello"]);

  const plain = loadPromptKey({ promptValue: "hello", coarse: true, fine: false, isMac: true });
  const enter = keydown();
  plain.element.dispatch(enter);
  assert.equal(plain.composerEnterIntent(enter, { promptText: "hello", shortcut: "enter", touch: true, isMac: true }), "newline");
  assert.equal(enter.defaultPrevented, false);
  assert.deepEqual(plain.sent, []);
});

test("a fully typed slash command sends on desktop and is picked on touch", () => {
  const row = highlighted("/help");
  const desk = loadPromptKey({ promptValue: "/help", coarse: false, fine: true, rows: [row], menuHidden: false });
  const deskEnter = keydown();
  desk.element.dispatch(deskEnter);
  assert.equal(desk.composerMenu.hidden, true);
  assert.deepEqual(desk.picks, []);
  assert.deepEqual(desk.sent, ["/help"]);
  assert.equal(deskEnter.defaultPrevented, true);
  assert.equal(desk.prompt.value, "/help");

  const phone = loadPromptKey({ promptValue: "/help", coarse: true, fine: false, rows: [highlighted("/help")], menuHidden: false });
  const phoneEnter = keydown();
  phone.element.dispatch(phoneEnter);
  assert.deepEqual(phone.picks, ["/help"]);
  assert.deepEqual(phone.sent, []);
  assert.equal(phoneEnter.defaultPrevented, true);
  assert.equal(phone.prompt.value, "/help ");
});

function loadPlaceholder(coarse, fine) {
  const src = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
  const touch = src.slice(src.indexOf("function touchComposer"), src.indexOf("function composerEnterIntent"));
  const body = src.slice(src.indexOf("function placeholderFor"), src.indexOf("function renderComposer"));
  const context = {
    matchMedia: mq(coarse, fine),
    composer: { dataset: {} },
    machines: { get: () => ({ view: { online: true } }) },
  };
  vm.createContext(context);
  vm.runInContext(`${touch}\n${body}\nglobalThis.placeholderFor = placeholderFor;`, context);
  return context.placeholderFor;
}

test("a stopped claude names the control that starts it", () => {
  const tab = { menu: null, machine: "local", view: { running: false } };
  assert.equal(loadPlaceholder(true, false)(tab), "claude isn't running. Use the send button to start it.");
  assert.equal(loadPlaceholder(false, true)(tab), "claude isn't running. Press Enter to start it.");
});

test("needsMod and Enter share enterNeedsMod", () => {
  const src = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
  const start = src.indexOf("function enterNeedsMod");
  const end = src.indexOf("const SEND_MOD");
  assert.equal(start > 0 && end > start, true);
  const context = { sendShortcut: "mod-enter-multiline", matchMedia: mq(false, true) };
  vm.createContext(context);
  vm.runInContext(`${src.slice(start, end)}\nglobalThis.enterNeedsMod = enterNeedsMod;\nglobalThis.needsMod = needsMod;\nglobalThis.composerEnterIntent = composerEnterIntent;`, context);
  assert.equal(context.enterNeedsMod("enter", "hi"), false);
  assert.equal(context.enterNeedsMod("mod-enter", "hi"), true);
  assert.equal(context.enterNeedsMod("mod-enter-multiline", "hi"), false);
  assert.equal(context.enterNeedsMod("mod-enter-multiline", "one\ntwo"), true);
  assert.equal(context.needsMod("one\ntwo"), true);
  assert.equal(context.needsMod("hi"), false);
  const plain = { key: "Enter", shiftKey: false, isComposing: false, metaKey: false, ctrlKey: false };
  assert.equal(
    context.composerEnterIntent(plain, { promptText: "one\ntwo", shortcut: "mod-enter-multiline", touch: false, isMac: false }),
    "newline",
  );
  assert.equal(src.slice(src.indexOf("const needsMod"), src.indexOf("function touchComposer")).includes("enterNeedsMod(sendShortcut, text)"), true);
  assert.equal(src.slice(src.indexOf("function composerEnterIntent"), end).includes("enterNeedsMod(shortcut, promptText)"), true);
});
