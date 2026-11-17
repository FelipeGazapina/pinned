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
const settings = read("settings.js");
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
/** The same plan with the highlight on option `n`. */
const PLAN_AT = (n) =>
  PLAN.map((l) => (l.startsWith(" ❯ ") ? `   ${l.slice(3)}` : l.replace(/^   (\d)\. /, (m, d) => (d === String(n) ? ` ❯ ${d}. ` : m))));
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
/** A second plan, which is not the first one. */
const PLAN_OTHER = PLAN.map((l) => (l.includes("Keep the v1 route") ? " 1. Replace the whole router in one pass." : l));
/** The screen once claude has taken the approval and works. */
const WORKING = ["⏺ Working through the plan…", "", "> "];

/** The page's real rules around one tab that waits on a plan menu, with the page's I/O stubbed.
 * `stuck` keeps the menu up after its Yes; `writeError` makes a write fail; `refuse` makes `prompt`
 * fail, after showing `refuse` as the screen when it is an array. */
function page({ screen = PLAN, stuck = false, writeError = null, refuse = false } = {}) {
  const log = [];
  const notes = [];
  const nodes = {};
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
    nodes,
    prompt: { value: "", focus() {}, setSelectionRange() {} },
    composer: { dataset: {} },
    approval: { hidden: true, dataset: {}, innerHTML: "", toggleAttribute() {} },
    document: {
      body: { dataset: {} },
      querySelector: (sel) => (nodes[sel] ?? [])[0] ?? null,
      querySelectorAll: (sel) => nodes[sel] ?? [],
    },
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
    phoneSvg: (name) => `<svg data-svg="${name}"></svg>`,
    icon: () => "",
    expandChips: (_tab, text) => text,
    compactOffer: () => null,
    durationLabel: () => "",
    callOn: async (machine, call) => {
      if (call.cmd === "write_input") {
        log.push(["write", call.data]);
        if (writeError) throw new Error(writeError);
        if (call.data.endsWith("\r") && !stuck) tab.term.show(WORKING);
        return null;
      }
      log.push(["prompt", call.text]);
      if (refuse) {
        if (Array.isArray(refuse)) tab.term.show(refuse);
        throw new Error("prompt refused");
      }
      return null;
    },
  };
  vm.createContext(context);
  vm.runInContext(
    [
      terminal,
      line(settings, "const esc"),
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
      decl(composer, "async function send()"),
      line(composer, "let pendingSeq"),
      line(composer, "let queuedNoteFor"),
      line(composer, "const LOST"),
      decl(composer, "function clearQueuedNote"),
      decl(composer, "function markSent"),
      decl(composer, "function raisePending"),
      decl(composer, "function dropPending"),
      decl(composer, "const planApprove"),
      line(composer, "const visibleOf"),
      decl(composer, "function approveNoteOf"),
      decl(composer, "function clearApproveNote"),
      decl(composer, "function approveNoteField"),
      decl(composer, "function syncApproveNote"),
      decl(composer, "function drawApproveNote"),
      decl(composer, "function openApproveNote"),
      decl(composer, "function keepNote"),
      decl(composer, "async function sendNote"),
      decl(composer, "async function approveWithNote"),
      decl(composer, "function approveNoteEvent"),
      decl(composer, "function renderApproval"),
      decl(phone, "function askPlace"),
      decl(phone, "function askSheetHtml"),
    ].join("\n"),
    context,
  );
  tab.menu = vm.runInContext("terminalMenu(tab)", context);
  return { tab, log, notes, nodes, ctx: context, run: (code) => vm.runInContext(code, context) };
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
const keydown = (input, key) => ({ type: "keydown", key, target: input, preventDefault() { this.defaultPrevented = true; } });
const enter = (input) => keydown(input, "Enter");
const tick = () => new Promise((r) => setTimeout(r, 5));

/** Sets the note the way opening the field does: the text, and the plan menu it was typed under. */
function typed(w, note) {
  Object.assign(w.tab.thread, { approveNote: note, approveNoteMenu: w.run("terminalMenu(tab)") });
}
/** Opens the plan's note field with `note` typed in it, the way the card draws it: the live input,
 * the Approve button and the error line are the page's own elements, which the page sets. */
function attach(w, note = "") {
  const send = { disabled: false, textContent: "", title: "", getClientRects: () => [{}] };
  const error = { textContent: "", hidden: true };
  const input = element("[data-approve-note-input]", {
    value: "",
    focus() { this.focused = true; },
    getClientRects: () => [{}],
    form: { querySelector: () => send },
  });
  const root = { querySelector: (sel) => ({ "[data-approve-note-input]": input, "[data-approve-note-send]": send, "[data-approve-note-error]": error })[sel] ?? null, getClientRects: () => [{}] };
  w.nodes[".approve-note"] = [root];
  w.nodes["[data-approve-note-input]"] = [input];
  w.nodes["[data-approve-note-send]"] = [send];
  w.nodes["[data-approve-note-error]"] = [error];
  typed(w, note);
  w.run("renderApproval(tab)");
  return { input, send, error };
}
const form = () => element("[data-approve-note-form]");
const submitEvent = () => ({ type: "submit", target: form(), preventDefault() {} });
const escEvent = (input) => keydown(input, "Escape");

test("Approve with note types the Yes option's key, then sends the note as the next message", async () => {
  const w = page();
  const { send } = attach(w, "keep the old API");
  await fire(w, { type: "click", target: element("[data-approve-note-send]") });
  assert.deepEqual(w.log, [
    ["write", "\r"],
    ["prompt", "keep the old API"],
  ]);
  assert.equal(send.disabled, false);
});

test("the note never picks Tell Claude what to change and never reaches the menu", async () => {
  const w = page();
  attach(w, "keep the old API");
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

test("the note field is a div, not a form: a form inside the composer's form is dropped by the parser", () => {
  const w = page();
  typed(w, "");
  const open = w.run("approveNoteField(tab)");
  assert.doesNotMatch(open, /<form/i);
  assert.match(open, /class="approve-note"/);
});

test("Enter in the note field approves with the note, as a click on Approve does", async () => {
  const w = page();
  const { input } = attach(w, "keep the old API");
  const pressed = enter(input);
  await fire(w, pressed);
  assert.equal(pressed.defaultPrevented, true, "Enter does not submit anything else");
  assert.deepEqual(w.log, [
    ["write", "\r"],
    ["prompt", "keep the old API"],
  ]);
});

test("an empty note cannot be submitted, typing blank disables Approve, and Esc closes the field and sends nothing", async () => {
  const w = page();
  const { input, send } = attach(w, "");
  await fire(w, enter(input));
  assert.deepEqual(w.log, [], "nothing is typed or sent for an empty note");

  const blank = attach(w, "");
  blank.input.value = "   ";
  await fire(w, { type: "input", target: blank.input });
  assert.equal(blank.send.disabled, true, "Approve is disabled while the field is blank");

  typed(w, "half a note");
  const esc = escEvent(element("[data-approve-note-input]"));
  fire(w, esc);
  assert.equal(w.tab.thread.approveNote, null, "Esc closes the field");
  assert.equal(esc.defaultPrevented, true);
  assert.deepEqual(w.log, [], "Esc sends nothing");
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
  const { input, send } = attach(w, "keep the old API");
  assert.match(w.ctx.approval.innerHTML, /class="approve-note"/);
  assert.equal(input.value, "keep the old API", "the typed text is set on the live input");
  assert.equal(send.disabled, false);
  assert.doesNotMatch(w.ctx.approval.innerHTML, /data-approve-note>/, "the open field replaces the button");

  const blank = page();
  const field = attach(blank, "");
  assert.equal(field.send.disabled, true);
});

test("the sheet's markup does not depend on the typed note: a keystroke never rebuilds the field", () => {
  const w = page();
  typed(w, "keep");
  const first = w.run("askSheetHtml(tab)");
  typed(w, "keep the old API, and more");
  assert.equal(w.run("askSheetHtml(tab)"), first);
  typed(w, " ");
  assert.equal(w.run("askSheetHtml(tab)"), first, "a blank note does not change the markup either");
});

test("the typed text and the disabled Approve are set on the live field, not in its markup", () => {
  const w = page();
  const { input, send } = attach(w, "");
  typed(w, "keep the old API");
  w.run("renderApproval(tab)");
  assert.equal(input.value, "keep the old API");
  assert.equal(send.disabled, false);
  assert.doesNotMatch(w.ctx.approval.innerHTML, /value="/);
});

test("the phone plan sheet offers Approve with note as a row only when the menu has a Yes", () => {
  const w = page();
  const html = w.run("askSheetHtml(tab)");
  assert.match(html, /data-approve-note/);
  assert.match(html, /Approve with note/);
  const none = page({ screen: PLAN_NO_YES });
  assert.doesNotMatch(none.run("askSheetHtml(tab)"), /data-approve-note/);
});

test("the phone's Approve with note row has its own check icon, not the mic", () => {
  const w = page();
  const row = w.run("askSheetHtml(tab)").match(/<button[^>]*\sdata-approve-note>[\s\S]*?<\/button>/)[0];
  assert.match(row, /data-svg="check"/);
  assert.doesNotMatch(row, /data-svg="mic"/);
});

test("the phone sheet's note field sends the note the same way the card does", async () => {
  const w = page();
  const opened = fire(w, { type: "click", target: element("[data-approve-note]") });
  assert.equal(await opened, true);
  assert.equal(w.tab.thread.approveNote, "", "the click opens the field");
  assert.match(w.run("askSheetHtml(tab)"), /class="approve-note"/);

  attach(w, "keep the old API");
  await fire(w, enter(element("[data-approve-note-input]")));
  assert.deepEqual(w.log, [
    ["write", "\r"],
    ["prompt", "keep the old API"],
  ]);
});

test("a note with markup in it is escaped where it is drawn, and the field holds it as typed", () => {
  const w = page();
  const note = `<b>"x"</b>`;
  const { input } = attach(w, note);
  assert.doesNotMatch(w.run("askSheetHtml(tab)"), /<b>/);
  assert.doesNotMatch(w.ctx.approval.innerHTML, /<b>/);
  assert.equal(input.value, note);
});

test("the plain Implement still picks the highlighted option and sends no message", async () => {
  const w = page({ screen: PLAN_AT(2) });
  await w.run("send()");
  assert.deepEqual(w.log, [["write", "\r"]]);
});

test("Approve with note picks the highlighted option when it is a Yes, and the first Yes otherwise", async () => {
  const highlighted = page({ screen: PLAN_AT(2) });
  attach(highlighted, "keep the old API");
  await fire(highlighted, enter(element("[data-approve-note-input]")));
  assert.deepEqual(highlighted.log, [
    ["write", "\r"],
    ["prompt", "keep the old API"],
  ]);

  const other = page({ screen: PLAN_AT(3) });
  attach(other, "keep the old API");
  await fire(other, enter(element("[data-approve-note-input]")));
  assert.deepEqual(other.log, [
    ["write", "\x1b[A\x1b[A\r"],
    ["prompt", "keep the old API"],
  ]);
});

test("the field's Approve says which option it picks, in full in its title", () => {
  const w = page({ screen: PLAN_AT(2) });
  const { send } = attach(w, "keep the old API");
  assert.equal(send.textContent, "Approve: Yes, and manually approve edits");
  assert.equal(send.title, "Approve: Yes, and manually approve edits");
});

test("Enter twice types the Yes key once and sends the note once", async () => {
  const w = page();
  const { input } = attach(w, "keep the old API");
  await Promise.all([fire(w, enter(input)), fire(w, enter(input))]);
  assert.deepEqual(w.log, [
    ["write", "\r"],
    ["prompt", "keep the old API"],
  ]);
});

test("a choice that changed since the field drew types nothing and keeps the note", async () => {
  const w = page();
  attach(w, "keep the old API");
  w.tab.term.show(PLAN_AT(2));
  w.tab.menu = w.run("terminalMenu(tab)");
  await w.run(`approveWithNote(tab, "keep the old API")`);
  assert.deepEqual(w.log, [], "the Yes key is not typed when the choice moved");
  assert.equal(w.tab.thread.approveNote, "keep the old API");
  assert.match(String(w.tab.thread.approveError), /changed/);
});

test("when the menu does not close in time the note stays in the field, with the reason", async () => {
  const w = page({ stuck: true });
  attach(w, "keep the old API");
  await w.run(`approveWithNote(tab, "keep the old API")`);
  assert.deepEqual(w.log, [["write", "\r"]]);
  assert.equal(w.ctx.prompt.value, "", "the note is not put into the composer");
  assert.equal(w.tab.thread.approveNote, "keep the old API");
  assert.match(String(w.tab.thread.approveError), /didn't close/);
});

test("a prompt refused while a menu is up keeps the note in the field, never in the composer", async () => {
  const w = page({ refuse: PLAN_OTHER });
  attach(w, "keep the old API");
  await w.run(`approveWithNote(tab, "keep the old API")`);
  assert.equal(w.ctx.prompt.value, "", "Enter there would refine or decline with the note");
  assert.equal(w.tab.thread.approveNote, "keep the old API");
  assert.match(String(w.tab.thread.approveError), /refused/);
});

test("a prompt refused with no menu up still goes back to the composer, as before", async () => {
  const w = page({ refuse: true });
  attach(w, "keep the old API");
  await w.run(`approveWithNote(tab, "keep the old API")`);
  assert.equal(w.ctx.prompt.value, "keep the old API");
});

test("a write the hub refuses leaves the note in the field with the reason, and sends nothing", async () => {
  const w = page({ writeError: "lost the connection to the hub" });
  attach(w, "keep the old API");
  assert.equal(await w.run(`approveWithNote(tab, "keep the old API")`), false);
  assert.deepEqual(w.log, [["write", "\r"]]);
  assert.equal(w.tab.thread.approveNote, "keep the old API");
  assert.match(String(w.tab.thread.approveError), /lost the connection/);
});

test("the note goes with the plan menu it was typed under: the next plan opens with its button, not the old note", () => {
  const w = page();
  attach(w, "stale note");
  w.tab.term.show(PLAN_OTHER);
  w.tab.menu = w.run("terminalMenu(tab)");
  w.run("renderApproval(tab)");
  assert.doesNotMatch(w.ctx.approval.innerHTML, /class="approve-note"/);
  assert.match(w.ctx.approval.innerHTML, /data-approve-note>Approve with note</);
});

test("a note whose menu left the screen is gone when the same plan comes back", () => {
  const w = page();
  attach(w, "stale note");
  w.tab.term.show(WORKING);
  w.tab.menu = null;
  w.run("renderApproval(tab)");
  w.tab.term.show(PLAN);
  w.tab.menu = w.run("terminalMenu(tab)");
  w.run("renderApproval(tab)");
  assert.doesNotMatch(w.ctx.approval.innerHTML, /class="approve-note"/);
});

test("a note that starts with / is sent as send() sends it: no pending row", async () => {
  const w = page();
  const { input } = attach(w, "/clear");
  await fire(w, enter(input));
  assert.deepEqual(w.log, [
    ["write", "\r"],
    ["prompt", "/clear"],
  ]);
  assert.deepEqual(w.tab.thread.pending ?? [], []);
});

test("Esc closes the note field and puts the focus back on Approve with note", async () => {
  const w = page();
  const { input } = attach(w, "keep the old API");
  const button = { focused: false, focus() { this.focused = true; }, getClientRects: () => [{}] };
  w.nodes["[data-approve-note]"] = [button];
  fire(w, escEvent(input));
  await tick();
  assert.equal(w.tab.thread.approveNote, null, "the field closes");
  assert.equal(button.focused, true);
  assert.deepEqual(w.log, []);
});

test("opening the note focuses the field on the visible surface, not a hidden copy", async () => {
  const w = page();
  const hidden = { focused: false, focus() { this.focused = true; }, getClientRects: () => [] };
  const visible = { focused: false, focus() { this.focused = true; }, getClientRects: () => [{}] };
  w.nodes["[data-approve-note-input]"] = [hidden, visible];
  await fire(w, { type: "click", target: element("[data-approve-note]") });
  await tick();
  assert.equal(visible.focused, true);
  assert.equal(hidden.focused, false);
});
