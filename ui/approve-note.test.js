import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

// "Approve with note" on claude's plan card: the desktop card (gui-composer.js) and the phone's ask
// sheet (gui-phone-thread.js). The real functions run in a vm with the page's I/O stubbed.

const read = (name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8");
const composer = read("gui-composer.js");
const phone = read("gui-phone-thread.js");
const terminal = read("terminal-menu.js");
const decl = (src, start) => {
  const from = src.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  const close = src.slice(from).search(/\n\}+;?\n/);
  return src.slice(from, from + close + src.slice(from + close).indexOf("\n", 1) + 1);
};
const line = (src, start) => {
  const from = src.indexOf(start);
  assert.ok(from >= 0, `missing ${start}`);
  return src.slice(from, src.indexOf("\n", from) + 1);
};

/** A terminal holding `lines` as its screen. `show(next)` writes a new screen and tells the
 * onWriteParsed listeners, as xterm does after it parses a write. */
function term(lines) {
  const parsed = new Set();
  const t = {
    rows: lines.length,
    buffer: { active: { baseY: 0, getLine: (i) => ({ translateToString: () => lines[i] ?? "", isWrapped: false, getCell: () => ({ isBgDefault: () => true }) }) } },
    onWriteParsed: (fn) => (parsed.add(fn), { dispose: () => parsed.delete(fn) }),
    show(next) {
      lines = next;
      t.rows = next.length;
      for (const fn of [...parsed]) fn();
    },
  };
  return t;
}

const RULE = "────────────────────────────────────────";
/** claude's plan menu, as its screen draws it: the plan's question, then the options. */
const PLAN = [
  RULE,
  " Here is Claude's plan:",
  " 1. Keep the v1 route and add v2 beside it.",
  " 2. Move the handler into its own file.",
  "",
  " Would you like to proceed?",
  " ❯ 1. Yes, and auto-accept edits",
  "   2. Yes, and manually approve edits",
  "   3. No, keep planning",
  "   4. Tell Claude what to change",
  " Esc to cancel",
];
/** A plan menu with no "Yes" to approve. */
const PLAN_NO_YES = [
  RULE,
  " Here is Claude's plan:",
  " 1. Keep the v1 route and add v2 beside it.",
  "",
  " Would you like to proceed?",
  " ❯ 1. Keep planning",
  "   2. Tell Claude what to change",
  " Esc to cancel",
];
/** The screen once claude has taken the approval and works. */
const WORKING = ["⏺ Working through the plan…", "", "> "];

/** The page's real rules around one tab that waits on a plan menu, with the page's I/O stubbed. */
function page({ screen = PLAN, base = false } = {}) {
  const log = [];
  const notes = [];
  const tab = {
    key: "mini:t1",
    machine: "mini",
    view: { id: "t1", running: true, status: "needs-input" },
    term: term(screen),
    menu: null,
    answering: false,
    answered: null,
    thread: { items: [], open: new Set(), approveNote: undefined },
  };
  const context = {
    console,
    tab,
    active: tab,
    log,
    notes,
    prompt: { value: "", focus() {}, setSelectionRange() {} },
    composer: { dataset: {} },
    approval: { hidden: true, dataset: {}, innerHTML: "", toggleAttribute() {} },
    document: { body: { dataset: {} }, querySelector: () => null },
    LOCAL: "local",
    machines: new Map(),
    PENDING_STALE_MS: 30_000,
    setTimeout(fn, ms) {
      const timer = globalThis.setTimeout(fn, ms);
      if (ms > 1000) timer.unref();
      return timer;
    },
    clearTimeout: globalThis.clearTimeout,
    threadOf: (t) => t.thread,
    renderComposer() {},
    renderSidebar() {},
    renderThread() {},
    fetchThread() {},
    autosize() {},
    saveDraft() {},
    showComposerNote: (text, error) => notes.push([text, !!error]),
    waitingKindOf: () => null,
    needsYouItems: () => [],
    phoneSvg: () => "",
    icon: () => "",
    esc: (s) => String(s),
    durationLabel: () => "",
    callOn: async (machine, call) => {
      if (call.cmd === "write_input") {
        log.push(["write", call.data]);
        if (call.data === "\r") tab.term.show(WORKING);
        return null;
      }
      log.push(["prompt", call.text]);
      return null;
    },
  };
  vm.createContext(context);
  vm.runInContext(
    [
      terminal,
      line(composer, "const ARROW_DOWN"),
      line(composer, "const ARROW_UP"),
      line(composer, "const declineOption"),
      line(composer, "const freeOption"),
      line(composer, "const TAB_ROW"),
      line(composer, "const multiSelect"),
      line(composer, "const ticked"),
      decl(composer, "function menuKind"),
      line(composer, "const writeTo"),
      decl(composer, "const arrowsTo"),
      decl(composer, "async function until"),
      decl(composer, "function redrawAsk"),
      line(composer, "const sameMenu"),
      line(composer, "const answeredKey"),
      line(composer, "const ANSWERED_MS"),
      decl(composer, "function holdAnswered"),
      decl(composer, "function chooseOption"),
      decl(composer, "async function answerWith"),
      decl(composer, "async function closeClaudeDialog"),
      decl(composer, "function sendBlocked"),
      line(composer, "let pendingSeq"),
      line(composer, "let queuedNoteFor"),
      line(composer, "const LOST"),
      decl(composer, "function clearQueuedNote"),
      decl(composer, "function markSent"),
      decl(composer, "function raisePending"),
      decl(composer, "function dropPending"),
      ...(base ? [] : [
        decl(composer, "function planApprove"),
        decl(composer, "function approveNoteField"),
        decl(composer, "function openApproveNote"),
        decl(composer, "function keepNote"),
        decl(composer, "async function sendNote"),
        decl(composer, "async function approveWithNote"),
        decl(composer, "function approveNoteEvent"),
        decl(composer, "function renderApproval"),
        decl(phone, "function askPlace"),
        decl(phone, "function askSheetHtml"),
      ]),
    ].join("\n"),
    context,
  );
  tab.menu = vm.runInContext("terminalMenu(tab)", context);
  return { tab, log, notes, ctx: context, run: (code) => vm.runInContext(code, context) };
}

/** Fires `event` through the field's handler, as the card and the sheet do. */
function fire(w, event) {
  w.ctx.event = event;
  return w.run("approveNoteEvent(tab, event)");
}
/** An element that answers to `match` and to the selectors its `closest` finds. */
function element(match, extra = {}) {
  const el = { ...extra, matches: (sel) => sel === match, closest: (sel) => (sel === match ? el : null) };
  return el;
}
const field = (value = "") => {
  const submit = { disabled: false };
  return element("[data-approve-note-input]", { value, form: { querySelector: () => submit }, submit });
};
const form = () => element("[data-approve-note-form]");
const submitEvent = () => ({ type: "submit", target: form(), preventDefault() {} });
const escEvent = (input) => ({ type: "keydown", key: "Escape", target: input, preventDefault() {} });

test("Approve with note types the Yes option's key, then sends the note as the next message", async () => {
  const w = page();
  w.tab.thread.approveNote = "keep the old API";
  await w.run(`approveWithNote(tab, "keep the old API")`);
  assert.deepEqual(w.log, [
    ["write", "\r"],
    ["prompt", "keep the old API"],
  ]);
});

test("the note never picks Tell Claude what to change and never reaches the menu", async () => {
  const w = page();
  await w.run(`approveWithNote(tab, "keep the old API")`);
  assert.deepEqual(
    w.log.filter(([kind]) => kind === "write"),
    [["write", "\r"]],
    "only the Yes key is typed into the menu",
  );
  assert.deepEqual(
    w.log.filter(([kind]) => kind === "prompt"),
    [["prompt", "keep the old API"]],
  );
});

test("Enter in the note field does the same as Approve", async () => {
  const w = page();
  w.tab.thread.approveNote = "keep the old API";
  await fire(w, submitEvent());
  assert.deepEqual(w.log, [
    ["write", "\r"],
    ["prompt", "keep the old API"],
  ]);
});

test("an empty note cannot be submitted, typing blank disables Approve, and Esc closes the field and sends nothing", async () => {
  const w = page();
  w.tab.thread.approveNote = "";
  assert.equal(await fire(w, submitEvent()), false, "an empty note does not submit");
  assert.deepEqual(w.log, [], "nothing is typed or sent for an empty note");

  const input = field("   ");
  fire(w, { type: "input", target: input });
  assert.equal(input.form.querySelector().disabled, true, "Approve is disabled while the field is blank");

  w.tab.thread.approveNote = "half a note";
  const esc = escEvent(field("half a note"));
  esc.defaultPrevented = false;
  esc.preventDefault = () => (esc.defaultPrevented = true);
  fire(w, esc);
  assert.equal(w.tab.thread.approveNote, null, "Esc closes the field");
  assert.equal(esc.defaultPrevented, true);
  assert.deepEqual(w.log, [], "Esc sends nothing");
});

test("the plain Approve still picks the Yes option and sends no message", async () => {
  const w = page({ base: true });
  await w.run(`chooseOption(tab, 1, "Yes, and auto-accept edits")`);
  assert.deepEqual(w.log, [["write", "\r"]]);
});

test("the desktop plan card offers Approve with note beside Terminal only when the menu has a Yes", () => {
  const w = page();
  w.run("renderApproval(tab)");
  assert.match(w.ctx.approval.innerHTML, /data-approve-note>Approve with note</);
  const none = page({ screen: PLAN_NO_YES });
  none.run("renderApproval(tab)");
  assert.doesNotMatch(none.ctx.approval.innerHTML, /data-approve-note/);
});

test("the desktop plan card opens a note field with the typed text, and Approve is disabled while it is blank", () => {
  const w = page();
  w.tab.thread.approveNote = "keep the old API";
  w.run("renderApproval(tab)");
  assert.match(w.ctx.approval.innerHTML, /data-approve-note-form/);
  assert.match(w.ctx.approval.innerHTML, /value="keep the old API"/);
  assert.doesNotMatch(w.ctx.approval.innerHTML, /type="submit"[^>]*\sdisabled/);
  assert.doesNotMatch(w.ctx.approval.innerHTML, /data-approve-note>/, "the open field replaces the button");

  w.tab.thread.approveNote = "";
  w.run("renderApproval(tab)");
  assert.match(w.ctx.approval.innerHTML, /type="submit"[^>]*\sdisabled/);
});

test("the phone plan sheet offers Approve with note as a row only when the menu has a Yes", () => {
  const w = page();
  const html = w.run("askSheetHtml(tab)");
  assert.match(html, /data-approve-note/);
  assert.match(html, /Approve with note/);
  const none = page({ screen: PLAN_NO_YES });
  assert.doesNotMatch(none.run("askSheetHtml(tab)"), /data-approve-note/);
});

test("the phone sheet's note field sends the note the same way the card does", async () => {
  const w = page();
  const opened = fire(w, { type: "click", target: element("[data-approve-note]") });
  assert.equal(opened, true);
  assert.equal(w.tab.thread.approveNote, "", "the click opens the field");
  assert.match(w.run("askSheetHtml(tab)"), /data-approve-note-form/);

  w.tab.thread.approveNote = "keep the old API";
  await fire(w, submitEvent());
  assert.deepEqual(w.log, [
    ["write", "\r"],
    ["prompt", "keep the old API"],
  ]);
});
