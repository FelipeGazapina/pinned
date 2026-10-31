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

class KeyDown extends Event {
  constructor(init = {}) {
    super("keydown", { cancelable: true });
    Object.assign(this, {
      key: "Enter",
      shiftKey: false,
      altKey: false,
      metaKey: false,
      ctrlKey: false,
      isComposing: false,
      keyCode: 0,
      ...init,
    });
  }
}

function keydown(extra = {}) {
  return new KeyDown(extra);
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
    sendAndNewThread: () => sent.push(`new-thread ${prompt.value}`),
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
  const element = new EventTarget();
  element.addEventListener("keydown", (event) => context.onPromptKeyDown(event));
  return {
    ...context,
    prompt,
    composerMenu,
    picks,
    sent,
    element: { dispatch: (event) => element.dispatchEvent(event) },
  };
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
  const chord = keydown({ ctrlKey: true });
  assert.equal(
    context.composerEnterIntent(chord, { promptText: "one\ntwo", shortcut: "mod-enter-multiline", touch: false, isMac: false }),
    "send",
  );
});

function openSlash(promptValue, { coarse, fine, isMac = true, shortcut = "enter" } = {}) {
  return loadPromptKey({
    promptValue,
    coarse,
    fine,
    isMac,
    shortcut,
    rows: [highlighted("/help")],
    menuHidden: false,
  });
}

test("touch Cmd+Enter with a slash menu open sends the typed command", () => {
  const mac = openSlash("/help", { coarse: true, fine: false, isMac: true });
  const cmd = keydown({ metaKey: true });
  mac.element.dispatch(cmd);
  assert.deepEqual(mac.sent, ["/help"]);
  assert.deepEqual(mac.picks, []);
  assert.equal(mac.prompt.value, "/help");
  assert.equal(mac.composerMenu.hidden, true);
  assert.equal(cmd.defaultPrevented, true);

  const win = openSlash("/help", { coarse: true, fine: false, isMac: false });
  const ctrl = keydown({ ctrlKey: true });
  win.element.dispatch(ctrl);
  assert.deepEqual(win.sent, ["/help"]);
  assert.deepEqual(win.picks, []);
  assert.equal(win.prompt.value, "/help");

  const partial = openSlash("/he", { coarse: true, fine: false, isMac: true });
  const partialCmd = keydown({ metaKey: true });
  partial.element.dispatch(partialCmd);
  assert.deepEqual(partial.sent, ["/he"]);
  assert.deepEqual(partial.picks, []);
  assert.equal(partial.prompt.value, "/he");
});

test("touch Alt+Cmd+Enter with a menu open sends and opens a new thread", () => {
  const mac = openSlash("/help", { coarse: true, fine: false, isMac: true });
  const chord = keydown({ altKey: true, metaKey: true });
  mac.element.dispatch(chord);
  assert.deepEqual(mac.sent, ["new-thread /help"]);
  assert.deepEqual(mac.picks, []);
  assert.equal(mac.prompt.value, "/help");
  assert.equal(chord.defaultPrevented, true);
});

test("Shift+Cmd+Enter with a menu open stays a newline", () => {
  const phone = openSlash("/help", { coarse: true, fine: false });
  const enter = keydown({ shiftKey: true, metaKey: true });
  phone.element.dispatch(enter);
  assert.deepEqual(phone.picks, []);
  assert.deepEqual(phone.sent, []);
  assert.equal(phone.prompt.value, "/help");
  assert.equal(phone.composerMenu.hidden, false);
  assert.equal(enter.defaultPrevented, false);
});

test("a composing Enter with an open menu does not pick", () => {
  const phone = openSlash("/help", { coarse: true, fine: false, isMac: true });
  const enter = keydown({ isComposing: true });
  phone.element.dispatch(enter);
  assert.deepEqual(phone.picks, []);
  assert.deepEqual(phone.sent, []);
  assert.equal(phone.prompt.value, "/help");
  assert.equal(phone.composerMenu.hidden, false);
  assert.equal(enter.defaultPrevented, false);
});

test("desktop with an open menu still sends an exact command and picks a shorter one", () => {
  const exact = openSlash("/help", { coarse: false, fine: true, isMac: true });
  const enter = keydown();
  exact.element.dispatch(enter);
  assert.deepEqual(exact.sent, ["/help"]);
  assert.deepEqual(exact.picks, []);
  assert.equal(exact.prompt.value, "/help");
  assert.equal(exact.composerMenu.hidden, true);

  const chorded = openSlash("/help", { coarse: false, fine: true, isMac: true });
  const chord = keydown({ metaKey: true });
  chorded.element.dispatch(chord);
  assert.deepEqual(chorded.sent, ["/help"]);
  assert.deepEqual(chorded.picks, []);
  assert.equal(chorded.prompt.value, "/help");
  assert.equal(chorded.composerMenu.hidden, true);

  const partial = openSlash("/he", { coarse: false, fine: true, isMac: true });
  const plain = keydown();
  partial.element.dispatch(plain);
  assert.deepEqual(partial.picks, ["/help"]);
  assert.deepEqual(partial.sent, []);
  assert.equal(partial.prompt.value, "/help ");
});

test("Cmd+Enter with a shorter highlight sends the typed text", () => {
  const desk = openSlash("/he", { coarse: false, fine: true, isMac: true });
  const chord = keydown({ metaKey: true });
  desk.element.dispatch(chord);
  assert.deepEqual(desk.sent, ["/he"]);
  assert.deepEqual(desk.picks, []);
  assert.equal(desk.prompt.value, "/he");
});

test("a composing Alt+Cmd+Enter with a menu open does not send", () => {
  for (const surface of [
    { coarse: false, fine: true },
    { coarse: true, fine: false },
  ]) {
    const menu = openSlash("/he", { ...surface, isMac: true });
    const chord = keydown({ altKey: true, metaKey: true, isComposing: true });
    menu.element.dispatch(chord);
    assert.deepEqual(menu.sent, []);
    assert.deepEqual(menu.picks, []);
    assert.equal(menu.prompt.value, "/he");
    assert.equal(menu.composerMenu.hidden, false);
    assert.equal(chord.defaultPrevented, false);
  }
});

test("keyCode 229 does not send or pick", () => {
  const chords = [{}, { metaKey: true }, { altKey: true, metaKey: true }, { key: "Tab" }];
  for (const extra of chords) {
    const menu = openSlash("/he", { coarse: false, fine: true, isMac: true });
    const event = keydown({ ...extra, keyCode: 229, isComposing: false });
    menu.element.dispatch(event);
    assert.deepEqual(menu.sent, []);
    assert.deepEqual(menu.picks, []);
    assert.equal(menu.prompt.value, "/he");
    assert.equal(menu.composerMenu.hidden, false);
    assert.equal(event.defaultPrevented, false);
  }
});

test("Option+Enter with a menu open picks and does not send", () => {
  const desk = openSlash("/he", { coarse: false, fine: true, isMac: true });
  const option = keydown({ altKey: true });
  desk.element.dispatch(option);
  assert.deepEqual(desk.picks, ["/help"]);
  assert.deepEqual(desk.sent, []);
  assert.equal(desk.prompt.value, "/help ");
  assert.equal(option.defaultPrevented, true);

  const phone = openSlash("/he", { coarse: true, fine: false, isMac: true });
  const phoneOption = keydown({ altKey: true });
  phone.element.dispatch(phoneOption);
  assert.deepEqual(phone.picks, ["/help"]);
  assert.deepEqual(phone.sent, []);
  assert.equal(phone.prompt.value, "/help ");
  assert.equal(phoneOption.defaultPrevented, true);

  const macCtrl = openSlash("/he", { coarse: false, fine: true, isMac: true });
  const ctrl = keydown({ ctrlKey: true });
  macCtrl.element.dispatch(ctrl);
  assert.deepEqual(macCtrl.picks, ["/help"]);
  assert.deepEqual(macCtrl.sent, []);

  const winMeta = openSlash("/he", { coarse: false, fine: true, isMac: false });
  const meta = keydown({ metaKey: true });
  winMeta.element.dispatch(meta);
  assert.deepEqual(winMeta.picks, ["/help"]);
  assert.deepEqual(winMeta.sent, []);
});

test("Cmd+Enter and Alt+Cmd+Enter with a menu open still send", () => {
  const desk = openSlash("/he", { coarse: false, fine: true, isMac: true });
  const cmd = keydown({ metaKey: true });
  desk.element.dispatch(cmd);
  assert.deepEqual(desk.sent, ["/he"]);
  assert.deepEqual(desk.picks, []);
  assert.equal(desk.prompt.value, "/he");
  assert.equal(desk.composerMenu.hidden, true);

  const deskNew = openSlash("/he", { coarse: false, fine: true, isMac: true });
  const alt = keydown({ altKey: true, metaKey: true });
  deskNew.element.dispatch(alt);
  assert.deepEqual(deskNew.sent, ["new-thread /he"]);
  assert.deepEqual(deskNew.picks, []);
  assert.equal(deskNew.prompt.value, "/he");
  assert.equal(alt.defaultPrevented, true);

  const phone = openSlash("/he", { coarse: true, fine: false, isMac: true });
  const phoneAlt = keydown({ altKey: true, metaKey: true });
  phone.element.dispatch(phoneAlt);
  assert.deepEqual(phone.sent, ["new-thread /he"]);
  assert.deepEqual(phone.picks, []);
  assert.equal(phone.prompt.value, "/he");

  const win = openSlash("/he", { coarse: false, fine: true, isMac: false });
  const ctrl = keydown({ ctrlKey: true });
  win.element.dispatch(ctrl);
  assert.deepEqual(win.sent, ["/he"]);
  assert.deepEqual(win.picks, []);
  assert.equal(win.composerMenu.hidden, true);

  const configured = openSlash("/he", { coarse: false, fine: true, isMac: true, shortcut: "mod-enter" });
  const configuredCmd = keydown({ metaKey: true });
  configured.element.dispatch(configuredCmd);
  assert.deepEqual(configured.sent, ["/he"]);
  assert.deepEqual(configured.picks, []);

  const configuredPlain = openSlash("/he", { coarse: false, fine: true, isMac: true, shortcut: "mod-enter" });
  const plain = keydown();
  configuredPlain.element.dispatch(plain);
  assert.deepEqual(configuredPlain.picks, ["/help"]);
  assert.deepEqual(configuredPlain.sent, []);
});

test("keyCode 229 does not send", () => {
  const phone = loadPromptKey({ promptValue: "hello", coarse: true, fine: false, isMac: true });
  const cmd = keydown({ metaKey: true, keyCode: 229, isComposing: false });
  phone.element.dispatch(cmd);
  assert.deepEqual(phone.sent, []);
  assert.equal(cmd.defaultPrevented, false);

  const desk = loadPromptKey({ promptValue: "hello", coarse: false, fine: true, isMac: true });
  const enter = keydown({ keyCode: 229, isComposing: false });
  desk.element.dispatch(enter);
  assert.deepEqual(desk.sent, []);
  assert.equal(enter.defaultPrevented, false);
});

function loadSendChrome({ coarse, fine, isMac }) {
  const src = readFileSync(new URL("./gui-composer.js", import.meta.url), "utf8");
  const head = src.slice(src.indexOf("function enterNeedsMod"), src.indexOf("function bindSendShortcut"));
  const titleStart = src.indexOf("  const touch = touchComposer();");
  const titleEnd = src.indexOf('  send.setAttribute("aria-label", send.title);');
  const send = {};
  const context = {
    matchMedia: mq(coarse, fine),
    IS_MAC: isMac,
    sendShortcut: "enter",
    prompt: { value: "hello" },
    tab: null,
    blocked: false,
    working: false,
    composer: { dataset: {} },
    send,
    t: null,
  };
  vm.createContext(context);
  vm.runInContext(
    `${head}\n${src.slice(titleStart, titleEnd)}\nglobalThis.sendShortcutHtml = sendShortcutHtml;`,
    context,
  );
  return { title: send.title, html: context.sendShortcutHtml() };
}

test("touch send copy names the hardware-keyboard chord", () => {
  const mac = loadSendChrome({ coarse: true, fine: false, isMac: true });
  assert.equal(mac.title, "Send (⌘Enter)");
  assert.equal(
    mac.html,
    `<div class="look"><label>Send shortcut</label><p class="meta">Enter adds a new line here. ⌘ + Enter on a hardware keyboard sends, and the send button does too.</p></div>`,
  );
  const win = loadSendChrome({ coarse: true, fine: false, isMac: false });
  assert.equal(win.title, "Send (CtrlEnter)");
  assert.equal(
    win.html,
    `<div class="look"><label>Send shortcut</label><p class="meta">Enter adds a new line here. Ctrl + Enter on a hardware keyboard sends, and the send button does too.</p></div>`,
  );
  const desk = loadSendChrome({ coarse: false, fine: true, isMac: true });
  assert.equal(desk.title, "Send (Enter)");
  assert.match(desk.html, /id="send-shortcut"/);
});
