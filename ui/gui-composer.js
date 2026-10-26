Object.assign(ICON_PATHS, {
  lock: `<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>`,
  "lock-open": `<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 9.9-1"/>`,
  "pen-line": `<path d="M13 21h8"/><path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/>`,
  sparkles: `<path d="M11.017 2.814a1 1 0 0 1 1.966 0l1.051 5.558a2 2 0 0 0 1.594 1.594l5.558 1.051a1 1 0 0 1 0 1.966l-5.558 1.051a2 2 0 0 0-1.594 1.594l-1.051 5.558a1 1 0 0 1-1.966 0l-1.051-5.558a2 2 0 0 0-1.594-1.594l-5.558-1.051a1 1 0 0 1 0-1.966l5.558-1.051a2 2 0 0 0 1.594-1.594z"/><path d="M20 2v4"/><path d="M22 4h-4"/><circle cx="4" cy="20" r="2"/>`,
  "pencil-ruler": `<path d="M13 7 8.7 2.7a2.41 2.41 0 0 0-3.4 0L2.7 5.3a2.41 2.41 0 0 0 0 3.4L7 13"/><path d="m8 6 2-2"/><path d="m18 16 2-2"/><path d="m17 11 4.3 4.3c.94.94.94 2.46 0 3.4l-2.6 2.6c-.94.94-2.46.94-3.4 0L11 17"/><path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>`,
  "loader-circle": `<path d="M21 12a9 9 0 1 1-6.219-8.56"/>`,
  file: `<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>`,
  bookmark: `<path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z"/>`,
  "git-commit": `<circle cx="12" cy="12" r="3"/><line x1="3" x2="9" y1="12" y2="12"/><line x1="15" x2="21" y1="12" y2="12"/>`,
  "cloud-upload": `<path d="M12 13v8"/><path d="M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242"/><path d="m8 17 4-4 4 4"/>`,
  "cloud-download": `<path d="M12 13v8l-4-4"/><path d="m12 21 4-4"/><path d="M4.393 15.269A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.436 8.284"/>`,
  star: `<path d="M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z"/>`,
  "corner-up-right": `<path d="m15 14 5-5-5-5"/><path d="M4 20v-7a4 4 0 0 1 4-4h12"/>`,
  "chevron-left": `<path d="m15 18-6-6 6-6"/>`,
  "list-todo": `<path d="M13 5h8"/><path d="M13 12h8"/><path d="M13 19h8"/><path d="m3 17 2 2 4-4"/><rect x="3" y="4" width="6" height="6" rx="1"/>`,
  circle: `<circle cx="12" cy="12" r="10"/>`,
  "circle-dot": `<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="1"/>`,
  gauge: `<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>`,
  "folder-git-2": `<path d="M18 19a5 5 0 0 1-5-5v8"/><path d="M9 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v5"/><circle cx="13" cy="12" r="2"/><circle cx="20" cy="19" r="2"/>`,
});

/** Claude's models as T3 Code lists them, newest first. `menu` names the row of claude's own /model
 * menu that picks one for the session alone; a model it doesn't list restarts claude with --model. */
document.body.insertAdjacentHTML("afterbegin", `<div id="file-icon-sprite" aria-hidden="true">${fileIcons.sprite}</div>`);
/** T3 Code's file-type icon for `path`, or a folder's. */
function fileTypeIcon(path, cls = "") {
  if (path.endsWith("/")) return icon("folder", cls);
  const { symbol, color } = fileIcons.of(path);
  return `<svg class="i ftype ${cls}" viewBox="0 0 16 16" style="color:${color}" aria-hidden="true"><use href="#${symbol}"/></svg>`;
}

const REASONING = { low: "Low", medium: "Medium", high: "High", xhigh: "Extra High", max: "Max", ultracode: "Ultracode", ultrathink: "Ultrathink" };
const ALL_REASONING = Object.keys(REASONING);
const NO_ULTRACODE = ALL_REASONING.filter((e) => e !== "ultracode");
const CLAUDE_MODELS = [
  { slug: "claude-fable-5-1", name: "Claude Fable 5.1", menu: "Fable", efforts: ALL_REASONING, context: 1e6 },
  { slug: "claude-opus-5-5", name: "Claude Opus 5.5", menu: "Opus", efforts: ALL_REASONING, fast: true, context: 1e6 },
  { slug: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", menu: "Sonnet", efforts: NO_ULTRACODE, context: 1e6 },
  { slug: "claude-haiku-5-5", name: "Claude Haiku 5.5", menu: "Haiku", efforts: NO_ULTRACODE, context: 1e6, isNew: true },
  { slug: "claude-fable-5", name: "Claude Fable 5", legacy: true, efforts: ALL_REASONING, context: 1e6 },
  { slug: "claude-opus-5", name: "Claude Opus 5", legacy: true, efforts: ALL_REASONING, fast: true, context: 1e6 },
  { slug: "claude-opus-4-8", name: "Claude Opus 4.8", legacy: true, efforts: ALL_REASONING, fast: true, context: 1e6 },
  { slug: "claude-opus-4-7", name: "Claude Opus 4.7", legacy: true, efforts: NO_ULTRACODE, context: 1e6 },
  { slug: "claude-opus-4-6", name: "Claude Opus 4.6", legacy: true, efforts: ["low", "medium", "high", "max", "ultrathink"], fast: true, context: 2e5 },
  { slug: "claude-opus-4-5", name: "Claude Opus 4.5", legacy: true, efforts: ["low", "medium", "high", "max"], fast: true, context: 2e5 },
  { slug: "claude-sonnet-5", name: "Claude Sonnet 5", legacy: true, efforts: NO_ULTRACODE, context: 1e6 },
  { slug: "claude-sonnet-4-6", name: "Claude Sonnet 4.6", legacy: true, efforts: ["low", "medium", "high", "max", "ultrathink"], context: 2e5 },
  { slug: "claude-haiku-4-5", name: "Claude Haiku 4.5", legacy: true, efforts: [], context: 2e5 },
];
const ULTRATHINK = "Ultrathink:\n";

/** Claude's permission modes, named and drawn as T3 Code's runtime modes. Plan is T3's separate
 * Build / Plan toggle; in claude it is one more stop of the Shift+Tab cycle. */
const MODES = {
  manual: { label: "Supervised", icon: "lock", hint: "Ask before commands and file changes." },
  accept: { label: "Auto-accept edits", icon: "pen-line", hint: "Auto-approve edits, ask before other actions." },
  auto: { label: "Auto", icon: "sparkles", hint: "Supported providers approve routine actions; others still ask." },
  bypass: { label: "Full access", icon: "lock-open", hint: "Allow commands and edits without prompts." },
  plan: { label: "Plan", icon: "pencil-ruler" },
};
const ACCESS = ["manual", "accept", "auto", "bypass"];

let drafts = {};
try {
  drafts = sessionsOnly(JSON.parse(load("gui-drafts")));
} catch {}
let stash = [];
try {
  stash = JSON.parse(load("gui-stash")) ?? [];
} catch {}
const STASH_MAX = 20;
/** T3 Code's send shortcut setting: Enter, ⌘Enter once the prompt has a second line, or ⌘Enter always. */
let sendShortcut = load("gui-send-shortcut") ?? (load("gui-send-mod") === "true" ? "mod-enter" : "enter");
const needsMod = (text) => sendShortcut === "mod-enter" || (sendShortcut === "mod-enter-multiline" && /\n/.test(text));
const SEND_MOD = IS_MAC ? "⌘" : "Ctrl";
const sendShortcutHtml = () => `<div class="look"><label for="send-shortcut">Send shortcut</label><p class="meta">Choose when Enter sends a prompt or inserts a new line</p>
  <select id="send-shortcut">${[["enter", "Enter"], ["mod-enter-multiline", `${SEND_MOD} + Enter for multiline prompts`], ["mod-enter", `${SEND_MOD} + Enter always`]].map(([v, l]) => `<option value="${v}"${sendShortcut === v ? " selected" : ""}>${l}</option>`).join("")}</select></div>`;
function bindSendShortcut() {
  settingsPage.querySelector("#send-shortcut").onchange = (e) => {
    sendShortcut = e.target.value;
    save("gui-send-shortcut", sendShortcut);
    if (active) renderComposer(active);
  };
}

/** The thread has text or files in its composer that weren't sent. */
const hasDraft = (tab) => !!(drafts[threadKey(tab)]?.trim() || threadOf(tab).attachments?.length);
onRekey((from, to) => {
  if (!(from in drafts)) return;
  drafts[to] = drafts[from];
  delete drafts[from];
  save("gui-drafts", JSON.stringify(drafts));
});
function saveDraft(tab, text) {
  if (text.trim()) drafts[threadKey(tab)] = text;
  else delete drafts[threadKey(tab)];
  save("gui-drafts", JSON.stringify(drafts));
}

const composerNote = document.getElementById("composer-note");
let composerNoteTimer;
function showComposerNote(text, error = false) {
  clearTimeout(composerNoteTimer);
  composerNote.textContent = text;
  composerNote.toggleAttribute("data-error", error);
  composerNote.hidden = !text;
  if (error) composerNoteTimer = setTimeout(() => (composerNote.hidden = true), 6000);
}

/** What claude's menu asks: a permission, which typed text declines; its plan, which typed text
 * refines; or a question, which typed text answers when it has a free-text option. */
function menuKind(menu) {
  const lines = menu.question.split("\n");
  if (menu.options.some((o) => /^Tell Claude what to change/.test(o.head))) return "plan";
  return lines.length > 1 && /proceed|allow|make this edit|permission/i.test(lines.at(-1)) ? "approval" : "question";
}
const declineOption = (menu) => menuKind(menu) === "approval" && menu.options.findLast((o) => /^No\b/.test(o.head));
const freeOption = (menu) => menuKind(menu) !== "approval" && menu.options.find((o) => /^(Type something|Tell Claude what to change)/.test(o.head));

function placeholderFor(tab, t) {
  if (composer.dataset.collapsed === "true") return "Ask anything...";
  const menu = tab.menu;
  if (menu && menuKind(menu) === "approval") {
    if (!declineOption(menu)) return "Choose an option above";
    return menu.amend ? "Send feedback to decline this request" : "Send a message to decline and redirect the agent";
  }
  if (menu && menuKind(menu) === "plan") return "Add feedback to refine the plan, or leave this blank to implement it";
  if (menu) return freeOption(menu) ? "Type your own answer, or leave this blank to use the selected option" : "Choose an option above";
  if (machines.get(tab.machine)?.view.online === false) return "Ask for changes, send follow-ups, or attach images";
  if (!tab.view.running) return "claude isn't running. Press Enter to start it.";
  return "Ask anything, @tag files/folders, $use skills, or / for commands";
}

function renderComposer(tab) {
  if (composer.dataset.tab !== (tab?.key ?? "")) switchDraft(tab);
  if (tab && tab.mode === undefined) tab.mode = terminalMode(tab);
  if (tab && tab.traits === undefined) tab.traits = traitsOn(screenLines(tab));
  const t = tab && threadOf(tab);
  const working = !!tab && phaseOf(tab) === "working";
  const empty = !prompt.value.trim() && !t?.attachments?.length;
  composer.dataset.working = working;
  composer.dataset.empty = empty;
  document.getElementById("chat").dataset.hero = !!tab && t.loaded && !t.items.length && !tab.menu && !tab.dialog;
  const send = document.getElementById("send");
  const blocked = tab && sendBlocked(tab);
  const spinning = composer.dataset.sending === "true" || blocked === "Connecting" || blocked === "Preparing worktree";
  send.innerHTML = icon(spinning ? "loader-circle" : working && !empty ? "list-plus" : "arrow-up");
  composer.dataset.spinning = spinning;
  send.disabled = !!blocked;
  const keys = needsMod(prompt.value) ? `${SEND_MOD}Enter` : "Enter";
  const compacting = tab && compactOffer(tab) && !t.keepHistory;
  send.title = blocked || (composer.dataset.sending === "true" ? "Submitting message" : compacting ? `Summarize ${formatTokens(compactOffer(tab))} tokens of history, then send` : working ? `Queue message (${keys})` : `Send (${keys})`);
  send.setAttribute("aria-label", send.title);
  renderApproval(tab);
  renderBanners(tab);
  renderTasks(tab);
  renderPills(tab);
  rest();
  renderAttachments(t);
  renderMirror();
  renderStash();
  renderControls(tab);
  renderContext(tab);
  renderCompactPill(tab);
  renderGitAction(tab);
  renderScripts(tab);
  renderDetails(tab);
  renderDrawer();
  if (tab) drainIdle(tab);
  const footer = document.getElementById("composer-footer");
  if (!tab) return (footer.innerHTML = "");
  const pr = prOf(tab);
  const ws = t.workspace;
  footer.innerHTML = `${workspaceHtml(tab)}
    <span class="spacer"></span>
    ${prBadge(tab, "button", pr)}
    ${ws?.branch ? `<button type="button" class="branch" title="${esc(ws.branch)} · ${bindingLabel("composer.branch") ?? ""}">${icon("git-branch")}<span>${esc(ws.branch)}</span>${icon("chevron-down", "chev")}</button>` : ""}`;
  const branchLabel = footer.querySelector(".branch > span");
  if (branchLabel) branchLabel.innerHTML = middle(ws.branch, 24);
  hostControls();
  renderHero(tab);
  prompt.placeholder = placeholderFor(tab, t);
}

function switchDraft(tab) {
  composer.dataset.tab = tab?.key ?? "";
  prompt.value = (tab && drafts[threadKey(tab)]) ?? "";
  if (tab) threadOf(tab).recall = null;
  if (tab?.view.running) fetchList(tab, "commands");
  composerMenu.hidden = true;
  prompt.style.height = "auto";
  prompt.style.height = `${Math.min(prompt.scrollHeight, 208)}px`;
}

function watchMenu(tab) {
  let queued = false;
  tab.term.onWriteParsed(() => {
    if (queued || !guiOn()) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      const lines = screenLines(tab);
      const menu = tab.answering ? null : menuOn(lines);
      const dialog = menu || tab.answering ? null : terminalDialog(tab);
      // claude hides its mode line and effort marks while a menu or a dialog is up.
      const covered = menu || dialog || tab.answering;
      const mode = modeOn(lines) ?? (covered ? tab.mode ?? null : null);
      const traits = traitsOn(lines) ?? (covered ? tab.traits ?? null : null);
      if (dialog !== (tab.dialog ?? null)) {
        const opened = !tab.dialog;
        tab.dialog = dialog;
        if (tab === active) {
          renderDialog(tab, opened);
          renderComposer(tab);
        }
      }
      if (JSON.stringify([menu, mode, traits]) === JSON.stringify([tab.menu ?? null, tab.mode ?? null, tab.traits ?? null])) return;
      tab.menu = menu;
      tab.mode = mode;
      tab.traits = traits;
      // Shift+Tab passes other modes on its way to the one picked; only where it stops counts.
      if (mode && mode !== "plan" && !tab.cycling) threadOf(tab).access = mode;
      if (tab === active) renderComposer(tab);
      renderSidebar();
    });
  });
}

const ARROW_DOWN = "\x1b[B";
const ARROW_UP = "\x1b[A";
const SHIFT_TAB = "\x1b[Z";
const writeTo = (tab, data) => callOn(tab.machine, { cmd: "write_input", tab: tab.view.id, data });
/** Presses Esc until claude's dialog closes, a few times at most for one opened from another. Keys
 * typed into a dialog would move its cursor or pick an entry instead of reaching the prompt. */
async function closeClaudeDialog(tab) {
  for (let i = 0; i < 3 && terminalDialog(tab); i++) {
    await writeTo(tab, "\x1b");
    await until(() => !terminalDialog(tab), 1000);
  }
  return !terminalDialog(tab);
}
const arrowsTo = (menu, n) => {
  const from = menu.options.find((o) => o.chosen)?.n ?? 1;
  return (n > from ? ARROW_DOWN : ARROW_UP).repeat(Math.abs(n - from));
};
async function until(test, ms = 1500) {
  for (const end = Date.now() + ms; !test(); ) {
    if (Date.now() > end) return false;
    await new Promise((r) => setTimeout(r, 40));
  }
  return true;
}

function chooseOption(tab, n, label) {
  const menu = terminalMenu(tab);
  if (menu?.options[n - 1]?.label !== label) {
    tab.menu = menu;
    return renderComposer(tab);
  }
  tab.menu = null;
  renderComposer(tab);
  renderSidebar();
  return writeTo(tab, arrowsTo(menu, n) + "\r");
}

/** Answers claude's menu with `text`. A question takes it as its free-text option. A permission is
 * declined with it: claude 2.1 opens a line for it with Tab on "No"; an older one declines on "No"
 * and takes the text as the next prompt. */
async function answerWith(tab, text) {
  const menu = terminalMenu(tab);
  const option = menu && (freeOption(menu) || declineOption(menu));
  if (!option) return false;
  const line = text.replace(/\s*\n\s*/g, " ");
  const shown = () => terminalMenu(tab)?.options[option.n - 1]?.head.includes(line.slice(0, 20));
  tab.answering = true;
  tab.menu = null;
  renderComposer(tab);
  try {
    await writeTo(tab, arrowsTo(menu, option.n));
    await until(() => terminalMenu(tab)?.options.find((o) => o.chosen)?.n === option.n);
    if (menuKind(menu) !== "approval" || menu.amend) {
      if (menuKind(menu) === "approval") {
        await writeTo(tab, "\t");
        await until(() => terminalMenu(tab)?.amend === false);
      }
      await writeTo(tab, line);
      await until(shown);
      await writeTo(tab, "\r");
    } else {
      await writeTo(tab, "\r");
      // Keys typed into a menu that stayed up could pick one of its options.
      if (!(await until(() => !terminalMenu(tab), 4000))) {
        showComposerNote("Claude's menu didn't close. Your message is still here.", true);
        return false;
      }
      await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text });
    }
  } finally {
    tab.answering = false;
  }
  return true;
}

/** claude's row of question tabs over a set of questions, "←  ☐ Color  ☒ Sizes  ✔ Submit  →":
 * each header, and whether it is answered. */
const TAB_ROW = /^←\s+(.*?)\s+→\s*$/;
function questionTabsOf(text) {
  const row = text.match(TAB_ROW);
  const items = row ? [...row[1].matchAll(/([☐☒✔])\s(.+?)(?=\s{2,}[☐☒✔]|$)/g)].filter((m) => m[1] !== "✔") : [];
  return items.length ? { headers: items.map((m) => m[2].trim()), answered: items.map((m) => m[1] === "☒") } : null;
}
/** The tabs on the tab's screen, and the one shown, which claude draws on a background; one past
 * the last question is its review of the answers. */
function questionTabs(tab) {
  const buffer = tab.term.buffer.active;
  for (let y = buffer.baseY + tab.term.rows - 1; y >= buffer.baseY; y--) {
    const line = buffer.getLine(y);
    const text = line?.translateToString(true) ?? "";
    const tabs = questionTabsOf(text);
    if (!tabs) continue;
    const marks = [...text].flatMap((c, x) => ("☐☒✔".includes(c) ? [x] : []));
    const at = marks.findIndex((x) => line.getCell(x) && !line.getCell(x).isBgDefault());
    return { ...tabs, at: Math.max(0, at) };
  }
  return null;
}
const multiSelect = (menu) => menu.options.some((o) => /^\[[ ✔]\]/.test(o.head));
const ticked = (o) => /^\[✔\]/.test(o.head) || /\s✔$/.test(o.head);

/** claude heads a permission menu with what it wants to run, such as "Bash command", then a
 * description and its own tips, the command in a dashed box, and the question last, such as "Do you
 * want to proceed?". A question's options each have a description under them; T3 Code shows it
 * with its header, a 1-9 hint or a check per option, and the tab it is on out of all. */
function renderApproval(tab) {
  const menu = !tab?.answering && tab?.menu;
  approval.hidden = !menu;
  if (!menu) return (approval.dataset.key = "");
  const kind = menuKind(menu);
  const tabs = kind === "question" ? questionTabs(tab) : null;
  const t = threadOf(tab);
  const key = JSON.stringify([tab.key, menu, tabs, t.picking ?? null, t.collapsed ?? null]);
  if (approval.dataset.key === key) return;
  approval.dataset.key = key;
  const lines = menu.question.split("\n").filter((l) => !TAB_ROW.test(l));
  const permission = kind === "approval";
  approval.dataset.kind = permission ? "approval" : "input";
  if (kind === "question") return renderQuestion(tab, menu, tabs, lines);
  delete approval.dataset.collapsed;
  const label = kind === "plan" ? "Plan ready" : /command/i.test(lines[0]) ? "Command approval" : /file|edit/i.test(lines[0]) ? "File change approval" : "Permission approval";
  if (permission) return renderPermission(menu, lines, label);
  const options = menu.options.map((o) => `<button type="button" data-option="${o.n}" class="choice"${o.chosen ? " data-chosen" : ""}${o.n <= 9 ? ` data-key="${o.n}"` : ""}><span class="label">${esc(o.head)}</span>${o.more ? `<span class="about">${esc(o.more)}</span>` : ""}</button>`);
  approval.innerHTML = `${icon("circle-alert", "lead")}<div class="body"><p class="kind">${esc(label)}</p>
      <p class="what">${esc(menu.question)}</p><div class="options choices">${options.join("")}</div></div>
    <div class="actions"><button type="button" data-show-terminal class="outline">Terminal</button></div>`;
}

/** T3 Code's answers to a permission, each one the option of claude's menu that means it: Approve
 * is its first "Yes", Decline its "No", Always allow the "Yes" that stops it asking, and Cancel its
 * Esc. Claude's other options, such as "Yes, and switch to auto mode", stay in the "…" menu. */
function approvalChoices(menu) {
  const accept = menu.options.find((o) => /^Yes\b/.test(o.head));
  const always = menu.options.find((o) => o !== accept && /^Yes\b/.test(o.head) && /always|don't ask again|during this session/i.test(o.label));
  const decline = declineOption(menu) || null;
  return { accept, decline, always, rest: menu.options.filter((o) => ![accept, always, decline].includes(o)) };
}
/** As T3 Code's approval panel: what is asked, the command or file, and why, then Decline,
 * Approve and "…" at its right. */
function renderPermission(menu, lines, label) {
  const question = lines.at(-1);
  const middle = lines.slice(1, -1).filter((l) => !/^Tip:/.test(l));
  const detail = menu.box || middle.join("\n");
  const about = menu.box ? middle.join("\n") : "";
  const { accept, decline } = approvalChoices(menu);
  approval.innerHTML = `${icon("shield", "lead")}<div class="body"><p class="kind">${esc(label)}</p>
      ${detail ? `<pre tabindex="0">${esc(detail)}</pre>` : ""}${about ? `<p class="about">${esc(about)}</p>` : ""}${/^Do you want to proceed\?$/.test(question) ? "" : `<p class="about">${esc(question)}</p>`}</div>
    <div class="actions">${decline ? `<button type="button" class="outline" data-approval="decline">Decline</button>` : ""}${accept ? `<button type="button" class="primary" data-approval="accept">Approve</button>` : ""}
      <button type="button" class="outline icon" data-approval-more aria-label="More approval options" title="More approval options">${icon("ellipsis")}</button></div>`;
}
function approvalMore(tab, anchor) {
  const menu = tab.menu;
  const { always, rest } = approvalChoices(menu);
  const pick = (o) => () => tab.menu === menu && chooseOption(tab, o.n, o.label);
  const entries = [{ label: "Cancel", run: () => tab.menu === menu && cancelMenu(tab) }];
  if (always) entries.push(/session/i.test(always.label) ? { label: "Always allow this session", run: pick(always) } : { label: "Always allow", hint: always.label, run: pick(always) });
  entries.push(...rest.map((o) => ({ label: o.label, run: pick(o) })), { rule: true }, { label: "Show in terminal", run: () => toggleDrawer(true) });
  openMenu(anchor, entries, "approval");
  const box = composerMenu.offsetParent.getBoundingClientRect();
  const at = anchor.getBoundingClientRect();
  Object.assign(composerMenu.style, { left: "", right: `${Math.max(0, box.right - at.right)}px`, top: "", bottom: `${box.bottom - at.top + 4}px` });
}
function cancelMenu(tab) {
  tab.menu = null;
  renderComposer(tab);
  return writeTo(tab, "\x1b");
}
function renderQuestion(tab, menu, tabs, lines) {
  const t = threadOf(tab);
  const review = tabs && tabs.at >= tabs.headers.length;
  const own = !tabs && lines[0]?.match(/^[☐☒✔]\s+(.+)$/);
  const header = review ? "Review your answers" : tabs ? tabs.headers[tabs.at] : own ? own[1] : "Question";
  const question = (own ? lines.slice(1) : review ? lines.filter((l) => !/^Review your answers$/.test(l)) : lines).join("\n").trim();
  const multi = multiSelect(menu);
  const collapsed = t.collapsed === header;
  approval.toggleAttribute("data-collapsed", collapsed);
  const count = tabs && !review ? `<span class="q-count">${tabs.at + 1}/${tabs.headers.length}</span>` : "";
  const options = menu.options.map((o) => {
    const on = t.picking === o.n || ticked(o);
    const label = o.head.replace(/^\[[ ✔]\]\s*/, "").replace(/\s✔$/, "");
    return `<button type="button" data-option="${o.n}" class="choice"${o.chosen ? " data-chosen" : ""}${on ? " data-picked" : ""}${o.n <= 9 && !on ? ` data-key="${o.n}"` : ""}><span class="label">${esc(label)}</span>${o.more ? `<span class="about">${esc(o.more)}</span>` : ""}${on ? icon("check", "tick") : ""}</button>`;
  });
  approval.innerHTML = `${icon("message-circle-question", "lead")}<div class="body">
      <div class="q-head" data-toggle role="button" tabindex="0" aria-expanded="${!collapsed}"><span class="q-header">${esc(header)}</span>${collapsed ? `<span class="q-text">${esc(question.split("\n")[0])}</span>` : ""}
        <span class="spacer"></span>${count}${icon("chevron-down", "q-chev")}<button type="button" data-dismiss class="icon" aria-label="Dismiss question without answering" title="Dismiss question without answering">${icon("x")}</button></div>
      <div class="q-body"><p class="what">${esc(question)}</p>${multi ? `<p class="q-multi">Select one or more options.</p>` : ""}<div class="options choices">${options.join("")}</div></div></div>
    <div class="actions"><button type="button" data-show-terminal class="outline">Terminal</button></div>`;
}

/** Picks option `n` the way T3 Code does: a multi-select option toggles; a single one shows its
 * check, then answers 200ms later. */
function pickOption(tab, n) {
  const menu = tab.menu;
  const option = menu?.options[n - 1];
  if (!option) return;
  if (multiSelect(menu)) return writeTo(tab, `${arrowsTo(menu, n)} `);
  const t = threadOf(tab);
  t.picking = n;
  renderComposer(tab);
  setTimeout(() => {
    t.picking = null;
    if (tab.menu === menu) chooseOption(tab, n, option.label);
  }, 200);
}

approval.addEventListener("click", (e) => {
  const tab = active;
  if (!tab) return;
  const answer = e.target.closest("[data-approval]");
  if (answer && tab.menu) {
    const choice = approvalChoices(tab.menu)[answer.dataset.approval];
    if (answer.dataset.approval === "decline" && prompt.value.trim()) return send();
    return choice && chooseOption(tab, choice.n, choice.label);
  }
  if (e.target.closest("[data-approval-more]") && tab.menu) return composerMenu.hidden || composerMenu.dataset.kind !== "approval" ? approvalMore(tab, e.target.closest("[data-approval-more]")) : (composerMenu.hidden = true);
  const option = e.target.closest("[data-option]");
  if (option && tab.menu) {
    if (menuKind(tab.menu) === "question") return pickOption(tab, Number(option.dataset.option));
    return chooseOption(tab, Number(option.dataset.option), tab.menu.options[option.dataset.option - 1]?.label);
  }
  if (e.target.closest("[data-dismiss]") && tab.menu) return cancelMenu(tab);
  if (e.target.closest("[data-toggle]")) {
    const t = threadOf(tab);
    const header = approval.querySelector(".q-header")?.textContent;
    t.collapsed = t.collapsed === header ? null : header;
    return renderComposer(tab);
  }
  if (e.target.closest("[data-show-terminal]")) toggleDrawer(true);
});

// A question's options, and the plan's, take 1-9 while the prompt is empty, as T3 Code's do.
window.addEventListener("keydown", (e) => {
  const menu = active?.menu;
  if (!guiOn() || !menu || menuKind(menu) === "approval" || e.metaKey || e.ctrlKey || e.altKey || !/^[1-9]$/.test(e.key)) return;
  if (e.target.closest?.("input, textarea, [contenteditable], #terms") && !(e.target === prompt && !prompt.value)) return;
  const option = menu.options[e.key - 1];
  if (!option) return;
  e.preventDefault();
  if (menuKind(menu) === "question") pickOption(active, option.n);
  else chooseOption(active, option.n, option.label);
}, true);

/** The send slot while claude asks, as T3 Code's: Previous and Next question / Submit for its
 * questions, Refine or Implement for its plan. */
const pillActions = document.getElementById("pill-actions");
function renderPills(tab) {
  const menu = tab && !tab.answering && tab.menu;
  const kind = menu && menuKind(menu);
  const compact = composer.dataset.compact === "true";
  let html = "";
  if (kind === "question") {
    const tabs = questionTabs(tab);
    const n = tabs?.headers.length ?? 1;
    const at = tabs?.at ?? 0;
    const last = at >= n - 1;
    const label = last ? (compact ? "Submit" : n > 1 ? "Submit answers" : "Submit answer") : compact ? "Next" : "Next question";
    const previous = at > 0 ? (compact ? `<button type="button" class="outline round-sm" data-q="previous" aria-label="Previous question">${icon("chevron-left")}</button>` : `<button type="button" class="outline" data-q="previous">Previous</button>`) : "";
    html = `${previous}<button type="button" class="pill" data-q="${last ? "submit" : "next"}">${label}</button>`;
  } else if (kind === "plan") {
    html = prompt.value.trim() || threadOf(tab).attachments?.length
      ? `<button type="button" class="pill" data-plan="refine">Refine</button>`
      : `<span class="split"><button type="button" class="pill" data-plan="implement">Implement</button><button type="button" class="pill more" data-plan="more" aria-label="Implementation actions">${icon("chevron-down")}</button></span>`;
  }
  composer.dataset.pills = !!html;
  pillActions.hidden = !html;
  if (pillActions.dataset.html !== html) {
    pillActions.dataset.html = html;
    pillActions.innerHTML = html;
  }
}
pillActions.addEventListener("click", (e) => {
  const tab = active;
  const b = e.target.closest("button");
  if (!tab || !b) return;
  if (b.dataset.q) return questionStep(tab, b.dataset.q);
  if (b.dataset.plan === "more") return openMenu(b, [{ label: "Implement in a new thread", run: () => implementInNewThread(tab) }], "plan");
  send();
});

/** Moves through claude's questions: back a tab, on to the next, or answers and submits, picking
 * "Submit answers" on its review tab for a set of them. */
async function questionStep(tab, step) {
  const menu = tab.menu;
  if (!menu) return;
  const tabs = questionTabs(tab);
  if (step === "previous") return writeTo(tab, "\x1b[D");
  const text = expandChips(tab, prompt.value).trim();
  if (tabs && tabs.at >= tabs.headers.length) {
    const submit = menu.options.find((o) => /^Submit answers/.test(o.head));
    return submit && chooseOption(tab, submit.n, submit.label);
  }
  if (text && freeOption(menu)) {
    if (await answerWith(tab, text)) clearComposer(tab);
  } else if (multiSelect(menu) || (tabs && tabs.answered[tabs.at])) await writeTo(tab, "\x1b[C");
  else {
    const chosen = menu.options.find((o) => o.chosen);
    if (chosen) await chooseOption(tab, chosen.n, chosen.label);
  }
  if (step !== "submit" || !tabs) return;
  if (!(await until(() => { const shown = questionTabs(tab); return shown && shown.at >= shown.headers.length && terminalMenu(tab); }, 3000))) return;
  const review = terminalMenu(tab);
  const submit = review.options.find((o) => /^Submit answers/.test(o.head));
  if (submit) chooseOption(tab, submit.n, submit.label);
}

/** T3 Code's "Implement in a new thread": claude's plan menu closes here, and a new thread in the
 * project gets the plan to implement. */
async function implementInNewThread(tab) {
  const plan = [...threadOf(tab).items].reverse().find((i) => i.kind === "tool" && i.name === "ExitPlanMode")?.detail || tab.menu?.box;
  if (!plan) return showComposerNote("Claude's plan isn't in the thread yet.", true);
  await writeTo(tab, "\x1b");
  await newThreadIn(tab);
  const next = active;
  if (next === tab) return;
  const ready = () => next.view.running && (terminalMode(next) || screenLines(next).some((l) => /^[>❯]$/.test(l.trim())));
  if (!(await until(ready, 15000))) return showComposerNote("claude didn't start in the new thread.", true);
  const title = plan.match(/^#+\s+(.+)$/m)?.[1]?.trim();
  sideMarks.titles[threadKey(next)] = title ? `Implement ${title}` : "Implement plan";
  saveMarks();
  await callOn(next.machine, { cmd: "prompt", tab: next.view.id, text: `PLEASE IMPLEMENT THIS PLAN:\n${plan.trim()}` });
  setTimeout(() => fetchThread(next), 400);
  renderSidebar();
}

/** T3 Code's notices over the composer: the front one attached to it, the rest behind a cap that
 * shows them on hover or a tap. */
const composerBanners = Object.assign(document.createElement("div"), { id: "composer-banners", hidden: true });
approval.before(composerBanners);
const bannerDismissed = new Set();
let bannersOpen = false;
let restoringBranch = null;
/** The usage limit that stopped the thread's turn, from the reset time claude wrote, or from its
 * message when it wrote none, such as for a monthly spend limit. */
function limitBanner(tab) {
  if (phaseOf(tab) !== "failed") return null;
  const at = tab.view.resume_at;
  const last = threadOf(tab).items.findLast((i) => i.kind === "error" || i.kind === "user");
  if (!at && !(last?.kind === "error" && /hit your .*limit|usage limit/i.test(last.text))) return null;
  const key = threadKey(tab);
  const snoozed = at && sideMarks.snoozed[key]?.until === at;
  const actions = at && at > (tab.view.updated ?? 0) ? `<button type="button" class="ghost" data-banner-act="resume">${tab.view.held ? "Resume at reset" : "Cancel auto-resume"}</button>${snoozed ? "" : `<button type="button" class="ghost" data-banner-act="snooze"${at <= Date.now() / 1000 ? " disabled" : ""}>Snooze until reset</button>`}` : "";
  return { id: `limit:${at}`, variant: "warning", icon: "gauge", title: esc("Usage limit reached"), description: esc(at ? `Resets ${new Date(at * 1000).toLocaleString()}` : "Reset time unavailable; retry manually"), actions };
}
/** As T3 Code: the local checkout moved off the branch the thread last ran on. */
function branchBanner(tab) {
  const t = threadOf(tab);
  const ws = t.workspace;
  if (!ws?.branch || ws.worktree || !t.branch || t.branch === ws.branch) return null;
  const id = `branch:${threadKey(tab)}:${t.branch}:${ws.branch}`;
  if (bannerDismissed.has(id)) return null;
  const busy = ["working", "delegating"].includes(phaseOf(tab)) || !!tab.menu || restoringBranch === tab;
  return {
    id, variant: "info", icon: "git-branch", dismiss: "Dismiss branch change notice",
    title: `<span class="muted">Branch changed from</span><code title="This thread last ran on ${esc(t.branch)}. Sending will continue on ${esc(ws.branch)}.">${esc(t.branch)}</code>`,
    actions: `<button type="button" class="ghost" data-banner-act="restore"${busy ? " disabled" : ""}>${restoringBranch === tab ? "Restoring..." : "Restore branch"}</button>`,
  };
}
const bannerHtml = (b, attached) => `<div class="banner${attached ? " attached" : ""}" role="alert" data-banner="${esc(b.id)}" data-variant="${b.variant}">${icon(b.icon, "lead")}<span class="content"><span class="title">${b.title}</span>${b.description ? `<span class="description">${b.description}</span>` : ""}</span>
    ${b.actions || b.dismiss ? `<span class="actions">${b.actions ?? ""}${b.dismiss ? `<button type="button" class="ghost icon" data-banner-act="dismiss" aria-label="${esc(b.dismiss)}" title="${esc(b.dismiss)}">${icon("x")}</button>` : ""}</span>` : ""}</div>`;
function renderBanners(tab) {
  const items = tab ? [limitBanner(tab), branchBanner(tab)].filter(Boolean) : [];
  // Warnings go in front, as T3 Code orders them.
  items.sort((a, b) => (a.variant === "warning" ? 0 : 1) - (b.variant === "warning" ? 0 : 1));
  if (items.length < 2) bannersOpen = false;
  composerBanners.hidden = !items.length;
  const [front, ...rest] = items;
  const html = !front ? "" : `${rest.length ? `<div class="stack${bannersOpen ? " open" : ""}"><button type="button" class="peek" data-banner-act="peek" aria-label="Show other notices" aria-expanded="${bannersOpen}"></button><div class="others" role="group" aria-label="Other notices">${rest.map((b) => bannerHtml(b, false)).join("")}</div></div>` : ""}${bannerHtml(front, true)}`;
  if (composerBanners.dataset.html !== html) {
    composerBanners.dataset.html = html;
    composerBanners.innerHTML = html;
  }
  composer.dataset.banners = items.length > 0;
}
composerBanners.addEventListener("pointerenter", (e) => e.pointerType !== "touch" && composerBanners.querySelector(".stack") && !bannersOpen && ((bannersOpen = true), renderBanners(active)));
composerBanners.addEventListener("pointerleave", (e) => e.pointerType !== "touch" && bannersOpen && ((bannersOpen = false), renderBanners(active)));
composerBanners.addEventListener("click", async (e) => {
  const tab = active;
  const b = e.target.closest("[data-banner-act]");
  if (!tab || !b || b.disabled) return;
  const act = b.dataset.bannerAct;
  if (act === "peek") {
    bannersOpen = !bannersOpen;
    return renderBanners(tab);
  }
  if (act === "dismiss") {
    bannerDismissed.add(b.closest("[data-banner]").dataset.banner);
    return renderBanners(tab);
  }
  if (act === "snooze") return snooze(tab, tab.view.resume_at);
  if (act === "resume") {
    b.disabled = true;
    b.textContent = "Saving...";
    await callOn(tab.machine, { cmd: "auto_resume", tab: tab.view.id, on: !!tab.view.held }).catch((err) => showComposerNote(`Could not change auto-resume: ${err}`, true));
    return refreshTabs(tab.machine);
  }
  if (act === "restore") restoreBranch(tab);
});
/** Switches the checkout back to the thread's branch, asking first while it has changes, as T3 Code does. */
async function restoreBranch(tab) {
  const t = threadOf(tab);
  const branch = t.branch;
  if (t.git?.dirty && !(await confirmRestore(branch))) return;
  restoringBranch = tab;
  renderBanners(tab);
  try {
    await callOn(tab.machine, { cmd: "checkout", tab: tab.view.id, branch, create: false });
  } catch (err) {
    showComposerNote(String(err), true);
  } finally {
    restoringBranch = null;
    Object.assign(t, { workspaceFor: null, gitFor: null });
    await fetchWorkspace(tab);
    if (tab === active) renderComposer(tab);
  }
}
function confirmRestore(branch) {
  composerSheet.innerHTML = `<form class="sheet" role="alertdialog" aria-label="Switch to ${esc(branch)}?"><h2>Switch to <code>${esc(branch)}</code>?</h2><p class="about">You have uncommitted changes. They'll carry over to the other branch, or block the switch if they conflict.</p>
    <footer><span class="spacer"></span><button type="button" class="outline" data-answer="">Cancel</button><button type="button" class="primary" data-answer="yes">Switch branch</button></footer></form>`;
  composerSheet.hidden = false;
  return new Promise((resolve) => {
    composerSheet.querySelector("form").onclick = (e) => {
      const b = e.target.closest("[data-answer]");
      if (!b) return;
      closeSheet();
      resolve(!!b.dataset.answer);
    };
  });
}

/** T3 Code's Tasks tab under the notices: the list claude keeps with TodoWrite, while its turn runs
 * or until every step is done, its current step and progress, and the whole list when opened. */
const composerDock = Object.assign(document.createElement("div"), { id: "composer-dock" });
const tasksTab = Object.assign(document.createElement("div"), { id: "composer-tasks", hidden: true });
approval.after(composerDock);
composerDock.append(tasksTab, document.getElementById("stash"));
let tasksOpen = false;
function tasksOf(tab) {
  const t = threadOf(tab);
  const todos = t.todos;
  if (!todos?.steps.length) return null;
  const since = t.items.findLast((i) => i.kind === "user")?.at ?? "";
  const live = phaseOf(tab) === "working" && todos.at >= since;
  if (!live && todos.steps.every((s) => s.status === "completed")) return null;
  const done = todos.steps.filter((s) => s.status === "completed").length;
  const current = todos.steps.find((s) => s.status === "in_progress") ?? todos.steps.find((s) => s.status === "pending") ?? todos.steps.at(-1);
  return { steps: todos.steps, done, total: todos.steps.length, step: current.step, live };
}
const TASK_ICONS = { completed: "check", in_progress: "circle-dot", pending: "circle" };
const TASK_LABELS = { completed: "Completed", in_progress: "Running", pending: "Pending" };
function renderTasks(tab) {
  const tasks = tab && approval.hidden ? tasksOf(tab) : null;
  if (!tasks) tasksOpen = false;
  tasksTab.hidden = !tasks;
  const segments = tasks && tasks.total > 1 && tasks.total <= 10 ? `<span class="segments" aria-hidden="true">${tasks.steps.map((s) => `<span data-status="${s.status}"></span>`).join("")}</span>` : "";
  const html = !tasks ? "" : `<button type="button" class="row" data-tasks-toggle aria-expanded="${tasksOpen}" aria-label="${tasksOpen ? "Collapse tasks" : "Tasks"}: ${tasks.done} of ${tasks.total} complete. Current task: ${esc(tasks.step)}">
      ${icon("list-todo", "lead")}<span class="name">Tasks</span><span class="current">${esc(tasks.step)}</span><span class="count${tasks.done >= tasks.total ? " done" : ""}">${tasks.done}/${tasks.total}</span>${segments}${icon("chevron-down", `chev${tasksOpen ? "" : " up"}`)}</button>
    ${tasksOpen ? `<ul class="steps" role="list" aria-label="Task list. ${tasks.done} of ${tasks.total} complete.">${tasks.steps.map((s) => `<li data-status="${s.status}">${icon(TASK_ICONS[s.status], "lead")}<span class="text"><span class="sr-only">${TASK_LABELS[s.status]}: </span>${esc(s.step)}</span><span class="when">${s.status === "in_progress" && tasks.live ? "now" : ""}</span></li>`).join("")}</ul>` : ""}`;
  if (tasksTab.dataset.html !== html) {
    tasksTab.dataset.html = html;
    tasksTab.innerHTML = html;
  }
  tasksTab.toggleAttribute("data-open", tasksOpen);
}
tasksTab.addEventListener("pointerdown", (e) => e.target.closest("[data-tasks-toggle]") && e.preventDefault());
tasksTab.addEventListener("click", (e) => {
  if (!e.target.closest("[data-tasks-toggle]") || !active) return;
  tasksOpen = !tasksOpen;
  renderTasks(active);
});

/** With the thread scrolled up and a one-line prompt not being typed in, the composer rests as one
 * row, as T3 Code's does, to show more of the thread. Like T3's, it comes to rest on a scroll, not
 * on a blur: pressing Send takes the focus, and a blur would move Send out from under the press.
 * On a phone it collapses whenever the prompt isn't focused, to a row that opens it, as T3's does;
 * a press in the composer holds it open until the press ends. */
let composerPressed = false;
function rest() {
  const away = threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight > 40;
  const t = active && threadOf(active);
  const typing = document.activeElement === prompt || prompt.value.includes("\n");
  const phone = narrow(640);
  const asked = active?.menu && !active.answering && menuKind(active.menu) === "question";
  const collapsed = phone && !!t && !typing && !composerPressed && !asked && composerMenu.hidden && document.getElementById("chat").dataset.hero !== "true";
  const resting = collapsed || (!phone && !!t && away && !typing && approval.hidden && composerMenu.hidden && !t.attachments?.length);
  if (composer.dataset.resting !== String(resting)) composer.dataset.resting = resting;
  if (composer.dataset.collapsed !== String(collapsed)) {
    composer.dataset.collapsed = collapsed;
    if (active) prompt.placeholder = placeholderFor(active, threadOf(active));
  }
  hostControls();
}
let pressTimer;
const releasePress = (ms) => {
  clearTimeout(pressTimer);
  pressTimer = setTimeout(() => {
    composerPressed = false;
    rest();
  }, ms);
};
composer.addEventListener("pointerdown", () => (composerPressed = true), true);
// A tap's click can come a task after its pointerup, so the click lets go, or a moment later.
for (const end of ["pointerup", "pointercancel"]) window.addEventListener(end, () => composerPressed && releasePress(400), true);
window.addEventListener("click", () => composerPressed && releasePress(0), true);
prompt.addEventListener("blur", () => setTimeout(rest, 0));
/** A resting composer hands its controls to the context strip under it, as T3 Code's does. */
const composerControls = composer.querySelector(".controls");
function hostControls() {
  const controls = composerControls;
  const footer = document.getElementById("composer-footer");
  const host = composer.dataset.resting === "true" && footer.querySelector(".spacer");
  if (host && controls.parentElement !== footer) host.before(controls);
  else if (!host && controls.parentElement !== composer.querySelector(".tools")) composer.querySelector(".tools").prepend(controls);
}
threadView.addEventListener("scroll", rest, { passive: true });
prompt.addEventListener("focus", rest);

function autosize() {
  prompt.style.height = "auto";
  prompt.style.height = `${Math.min(prompt.scrollHeight, 208)}px`;
  if (active) renderComposer(active);
}
prompt.addEventListener("input", () => {
  if (active) {
    saveDraft(active, prompt.value);
    const t = threadOf(active);
    if (t.recall !== null && prompt.value !== recallable(active)[t.recall]) t.recall = null;
  }
  autosize();
  showTrigger();
});
prompt.addEventListener("keydown", (e) => {
  if (isTrigger() && triggerKey(e)) return e.preventDefault();
  if (deleteChip(e)) return e.preventDefault();
  if (promptCodeKey(e)) return e.preventDefault();
  if (e.key === "Enter" && e.altKey && !e.shiftKey && (IS_MAC ? e.metaKey : e.ctrlKey)) {
    e.preventDefault();
    return sendAndNewThread();
  }
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing && (!needsMod(prompt.value) || (IS_MAC ? e.metaKey : e.ctrlKey))) {
    e.preventDefault();
    send();
  } else if (e.key === "Escape" && active && phaseOf(active) === "working" && !prompt.value) {
    e.preventDefault();
    stop();
  } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !e.shiftKey && !e.altKey && !e.metaKey && !e.ctrlKey && recall(e.key === "ArrowUp" ? -1 : 1)) {
    e.preventDefault();
  } else if (e.key === "Tab" && e.shiftKey && active?.view.running && !active.menu && !active.dialog) {
    e.preventDefault();
    writeTo(active, SHIFT_TAB);
  }
});
document.getElementById("send").onclick = () => send();
document.getElementById("stop").onclick = () => stop();

/** The thread's prompts, oldest first, without a prompt repeated right after itself. */
const recallable = (tab) => threadOf(tab).items.filter((i) => i.kind === "user" && i.text.trim()).map((i) => i.text).filter((text, i, all) => text !== all[i - 1]);

/** Steps through the thread's prompts, as T3 Code does: only from an empty prompt or one recalled,
 * with the caret on its first line going up or its last going down, and nothing attached. */
function recall(step) {
  const tab = active;
  if (!tab) return false;
  const t = threadOf(tab);
  const sent = recallable(tab);
  const at = t.recall ?? null;
  if (!sent.length || t.attachments?.length || (at === null ? prompt.value !== "" : prompt.value !== sent[at])) return false;
  if (step < 0 ? prompt.value.slice(0, prompt.selectionStart).includes("\n") : prompt.value.slice(prompt.selectionEnd).includes("\n")) return false;
  if (at === null && step > 0) return false;
  const next = at === null ? sent.length - 1 : at + step;
  if (next < 0) return true;
  t.recall = next < sent.length ? next : null;
  prompt.value = t.recall === null ? "" : sent[t.recall];
  prompt.setSelectionRange(prompt.value.length, prompt.value.length);
  autosize();
  return true;
}

// Typing into a menu would filter its options or pick one, so the prompt waits for an answer.
function waitsOnMenu(tab) {
  if (!tab.menu) return false;
  approval.animate([{ transform: "translateX(-4px)" }, { transform: "translateX(4px)" }, { transform: "none" }], 180);
  return true;
}

async function send() {
  const tab = active;
  if (!tab) return toggleProjects();
  if (!tab.view.running) return startTab(tab);
  if (tab.answering || composer.dataset.sending === "true") return;
  const t = threadOf(tab);
  const files = t.attachments ?? [];
  const text = [expandChips(tab, prompt.value).trim(), ...files.map((f) => f.text ?? escapePath(f.path))].filter(Boolean).join(" ");
  if (tab.menu && menuKind(tab.menu) === "question" && !files.length) {
    const tabs = questionTabs(tab);
    return questionStep(tab, !tabs || tabs.at >= tabs.headers.length - 1 ? "submit" : "next");
  }
  if (tab.menu) {
    const menu = tab.menu;
    const chosen = menu.options.find((o) => o.chosen);
    if (!text && freeOption(menu) && chosen && chosen !== freeOption(menu)) return chooseOption(tab, chosen.n, chosen.label);
    if (!text || files.length || !(await answerWith(tab, text))) return waitsOnMenu(tab);
    return clearComposer(tab);
  }
  if (!text || sendBlocked(tab)) return;
  prompts++;
  const draft = prompt.value;
  const chips = { ...chipsOf(tab) };
  const compactFirst = !!compactOffer(tab) && !t.keepHistory && !/^\/compact\b/.test(text);
  composer.dataset.sending = "true";
  if (!(await closeClaudeDialog(tab))) {
    delete composer.dataset.sending;
    return showComposerNote("Claude's dialog didn't close. Your message is still here.", true);
  }
  clearComposer(tab, false);
  try {
    // claude takes the message typed while it compacts as the next prompt.
    if (compactFirst) await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text: "/compact" });
    await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text });
    for (const f of files) if (f.url) URL.revokeObjectURL(f.url);
  } catch (e) {
    Object.assign(t, { attachments: files });
    Object.assign(chipsOf(tab), chips);
    saveDraft(tab, draft);
    if (active === tab) prompt.value = draft;
    showComposerNote(String(e), true);
  } finally {
    delete composer.dataset.sending;
    autosize();
  }
  setTimeout(() => fetchThread(tab), 300);
}

/** Why Send can't go now, in T3 Code's words: the hub link is down, or the machine is offline. */
function sendBlocked(tab) {
  if (threadOf(tab).preparing) return "Preparing worktree";
  if (tab.machine !== LOCAL && document.body.dataset.hub === "offline") return "Connecting";
  if (machines.get(tab.machine)?.view.online === false) return "Environment disconnected";
  return null;
}

/** T3 Code's ⌘⌥Enter: sends the prompt, then opens a new thread in the same project. */
async function sendAndNewThread() {
  const tab = active;
  if (!tab || (!prompt.value.trim() && !threadOf(tab).attachments?.length)) return;
  await send();
  if (active === tab && !prompt.value.trim()) newThreadIn(tab);
}

function clearComposer(tab, revoke = true) {
  const t = threadOf(tab);
  if (revoke) for (const f of t.attachments ?? []) if (f.url) URL.revokeObjectURL(f.url);
  t.attachments = [];
  t.recall = null;
  saveDraft(tab, "");
  if (tab === active) {
    prompt.value = "";
    autosize();
  }
}

// Esc ends the turn but leaves background agents and workflows running, so claude's ctrl+x ctrl+k
// stops them first. Their "stopped by the user" notices then wake claude for a turn of its own,
// which a second Esc ends, unless a prompt went out in between.
const KILL_AGENTS = "\x18\x0b";
let prompts = 0;
async function stop() {
  const tab = active;
  if (!tab) return;
  const sent = prompts;
  for (const keys of [KILL_AGENTS, KILL_AGENTS, "\x1b"]) {
    await writeTo(tab, keys);
    await new Promise((r) => setTimeout(r, 300));
  }
  await new Promise((r) => setTimeout(r, 1200));
  if (prompts === sent) writeTo(tab, "\x1b");
}

const escapePath = (p) => p.replace(/[^\w@%+=:,./-]/g, "\\$&");

/** Files attached to the active thread show above the prompt and go out as escaped paths with it,
 * which claude reads as attachments. */
function insertPaths(paths, files = []) {
  const t = threadOf(active);
  t.attachments = [...(t.attachments ?? []), ...paths.map((path, i) => ({
    path,
    name: files[i]?.name ?? path.split(/[\\/]/).pop(),
    url: files[i]?.type.startsWith("image/") ? URL.createObjectURL(files[i]) : null,
    size: files[i]?.size ?? null,
    file: files[i] ?? null,
  }))];
  renderComposer(active);
  renderSidebar();
  prompt.focus();
}
document.getElementById("composer-attach").onclick = () => document.getElementById("attach").click();
// A picture pasted into the composer joins the strip above it, on this machine too, where the
// terminal view instead hands claude the clipboard with Ctrl+V.
window.addEventListener("paste", (e) => {
  const types = [...(e.clipboardData?.types ?? [])];
  if (!guiOn() || !active || e.target !== prompt || types.includes("text/plain") || !types.includes("Files")) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  attachFiles(active, [...e.clipboardData.files]);
}, true);

const attachmentsRow = document.getElementById("attachments");
const formatSize = (n) => (n == null ? "" : n < 1024 ? `${n} B` : n < 1024 ** 2 ? `${(n / 1024).toFixed(1).replace(/\.0$/, "")} KB` : `${(n / 1024 ** 2).toFixed(1).replace(/\.0$/, "")} MB`);
/** As T3 Code shows them: pictures as 64px tiles that open full size, other files as rows. */
function renderAttachments(t) {
  const files = t?.attachments ?? [];
  attachmentsRow.hidden = !files.length;
  const key = JSON.stringify(files.map((f) => [f.path, f.name]));
  if (attachmentsRow.dataset.key === key) return;
  attachmentsRow.dataset.key = key;
  const tiles = files.map((f, i) => [f, i]).filter(([f]) => f.url);
  const rows = files.map((f, i) => [f, i]).filter(([f]) => !f.url);
  attachmentsRow.innerHTML = `${tiles.length ? `<div class="tiles">${tiles.map(([f, i]) => `<span class="tile" title="${esc(f.path)}"><button type="button" class="zoom" data-zoom="${i}" aria-label="Preview ${esc(f.name)}"><img src="${esc(f.url)}" alt="${esc(f.name)}"></button>
      <button type="button" class="remove" data-remove="${i}" aria-label="Remove ${esc(f.name)}" title="Remove ${esc(f.name)}">${icon("x")}</button></span>`).join("")}</div>` : ""}
    ${rows.length ? `<div class="rows">${rows.map(([f, i]) => `<div class="file" title="${esc(f.path)}">${f.icon ? icon(f.icon) : fileTypeIcon(f.name)}<button type="button" class="name" data-preview="${i}"${f.file ? "" : " disabled"}>${esc(f.name)}</button>
      <span class="size">${formatSize(f.size)}</span>${f.action ? `<button type="button" class="remove" data-attachment-action="${i}" aria-label="${esc(f.action.label)}" title="${esc(f.action.label)}">${icon(f.action.icon)}</button>` : ""}<button type="button" class="remove" data-remove="${i}" aria-label="Remove ${esc(f.name)}" title="Remove ${esc(f.name)}">${icon("x")}</button></div>`).join("")}</div>` : ""}`;
}
const filePreview = Object.assign(document.createElement("div"), { id: "file-preview", hidden: true });
document.body.append(filePreview);
/** T3 Code's preview of an attached file: its text, or a note that it can't be shown. */
async function previewFile(f) {
  const text = f.size <= 1 << 20 ? await f.file.text().catch(() => null) : null;
  const shown = text !== null && !/\u0000/.test(text.slice(0, 4096));
  filePreview.innerHTML = `<div class="sheet" role="dialog" aria-label="${esc(f.name)}"><header>${fileTypeIcon(f.name)}<span class="name">${esc(f.name)}</span><span class="size">${formatSize(f.size)}</span>
      <button type="button" class="icon" data-close aria-label="Close">${icon("x")}</button></header>${shown ? `<pre>${esc(text)}</pre>` : `<p class="none">No preview for this file.</p>`}</div>`;
  filePreview.hidden = false;
}
filePreview.onclick = (e) => (e.target === filePreview || e.target.closest("[data-close]")) && (filePreview.hidden = true);
window.addEventListener("keydown", (e) => e.key === "Escape" && !filePreview.hidden && ((filePreview.hidden = true), e.stopPropagation()), true);
attachmentsRow.addEventListener("click", (e) => {
  const t = active && threadOf(active);
  const zoom = e.target.closest("[data-zoom]");
  if (zoom && t) return zoomImage(t.attachments[zoom.dataset.zoom].url);
  const preview = e.target.closest("[data-preview]");
  if (preview && t) return previewFile(t.attachments[preview.dataset.preview]);
  const remove = e.target.closest("[data-remove]");
  if (!remove || !t) return;
  const [gone] = t.attachments.splice(Number(remove.dataset.remove), 1);
  if (gone?.url) URL.revokeObjectURL(gone.url);
  renderComposer(active);
  prompt.focus();
});

const composerMenu = document.getElementById("composer-menu");
/** Opens a menu of `entries` above `anchor`: rows `{ label, hint, run, cls, icon, checked, badge,
 * remove }`, and `{ heading }`, `{ note }` and `{ rule: true }` between them. */
function openMenu(anchor, entries, kind = "") {
  composerMenu.dataset.kind = kind;
  composerMenu.replaceChildren(...entries.map((entry) => menuRow(entry)));
  composerMenu.hidden = false;
  placeMenu(anchor);
}
function menuRow({ label, hint, run, cls, icon: name, checked, remove, disabled, badge, heading, note, rule }, menu = composerMenu) {
  if (heading || note || rule) return Object.assign(document.createElement(rule ? "hr" : "p"), { className: heading ? "heading" : note ? "note" : "rule", textContent: heading ?? note ?? "" });
  const b = document.createElement("button");
  b.type = "button";
  b.className = `${cls ?? ""}${checked ? " checked" : ""}`;
  b.disabled = !!disabled;
  b.innerHTML = `${name ? icon(name, "lead") : ""}<span class="text"><span class="label">${esc(label)}${badge ? `<span class="badge">${esc(badge)}</span>` : ""}</span>${hint ? `<span class="hint">${esc(hint)}</span>` : ""}</span>
    ${checked ? icon("check", "check") : ""}${remove ? `<span class="remove" title="Delete">${icon("x")}</span>` : ""}`;
  b.onclick = (e) => {
    menu.hidden = true;
    if (remove && e.target.closest(".remove")) return remove();
    run();
  };
  return b;
}
function placeMenu(anchor) {
  composerMenu.style.top = composerMenu.style.bottom = "";
  const box = composerMenu.offsetParent?.getBoundingClientRect();
  const at = anchor.getBoundingClientRect();
  if (!box) return;
  composerMenu.style.left = "";
  composerMenu.style.right = "";
  const left = at.left - box.left;
  if (left + composerMenu.offsetWidth > box.width) composerMenu.style.right = "0px";
  else composerMenu.style.left = `${Math.max(0, left)}px`;
}
window.addEventListener("mousedown", (e) => {
  const keeps = isTrigger() ? "#composer-menu, #prompt" : "#composer-menu, #model-button, #more-button, #mode-button, #traits-button, #stash, #composer-footer .branch, #composer-footer .where, [data-hero-project], [data-approval-more]";
  if (!composerMenu.hidden && !e.target.closest(keeps)) composerMenu.hidden = true;
}, true);
window.addEventListener("keydown", (e) => {
  if (composerMenu.hidden || e.key !== "Escape" || isTrigger()) return;
  composerMenu.hidden = true;
  e.preventDefault();
  e.stopPropagation();
  prompt.focus();
}, true);

function sendSlash(tab, text) {
  if (!tab || waitsOnMenu(tab)) return false;
  (async () => {
    if (!(await closeClaudeDialog(tab))) throw "Claude's dialog didn't close.";
    await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text });
    setTimeout(() => fetchThread(tab), 400);
  })().catch((e) => showComposerNote(String(e), true));
  return true;
}

/** Runs `run` once claude takes a command: now while it waits for a prompt, else when the turn
 * or its menu ends, as T3 Code applies a pick to the next turn. */
function whenIdle(tab, run) {
  const t = threadOf(tab);
  if (phaseOf(tab) !== "working" && !tab.menu && !tab.answering && !t.onIdle?.length) return run();
  (t.onIdle ??= []).push(run);
  showComposerNote("This applies once claude finishes the turn.");
}
function drainIdle(tab) {
  const t = threadOf(tab);
  if (!t.onIdle?.length || t.draining || phaseOf(tab) === "working" || tab.menu || tab.answering) return;
  t.draining = true;
  showComposerNote("");
  (async () => {
    while (t.onIdle.length) await t.onIdle.shift()();
  })().finally(() => (t.draining = false));
}

/** Opens claude's own `command` menu with the banner held back, lets `steer` answer it once
 * `open` sees it on the screen, and waits for it to close. */
async function throughMenu(tab, command, open, steer) {
  tab.answering = true;
  tab.menu = null;
  renderComposer(tab);
  try {
    await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text: command });
    if (!(await until(() => open(screenLines(tab)), 5000))) throw `claude didn't open ${command}`;
    await steer(open(screenLines(tab)));
    if (!(await until(() => !open(screenLines(tab)), 3000))) throw `claude's ${command} menu didn't close`;
    return true;
  } catch (e) {
    if (open(screenLines(tab))) await writeTo(tab, "\x1b");
    showComposerNote(String(e), true);
    return false;
  } finally {
    tab.answering = false;
    tab.menu = terminalMenu(tab);
    if (tab === active) renderComposer(tab);
    renderSidebar();
    setTimeout(() => fetchThread(tab), 400);
  }
}

function modelName(tab) {
  const t = threadOf(tab);
  return t.picked ?? [...t.items].reverse().find((i) => i.kind === "model")?.name ?? "Default";
}
const modelOf = (tab) => CLAUDE_MODELS.find((m) => m.name === `Claude ${modelName(tab)}` || m.name === modelName(tab)) ?? null;
/** Claude's /model menu: the numbered rows under "Select model", ❯ on the chosen one. Its effort
 * and fast mode lines under the rows keep `menuOn` from reading it. */
function modelMenuOn(lines) {
  if (!/Esc to cancel/.test(lines.findLast((l) => l.trim()) ?? "")) return null;
  const top = lines.findLastIndex((l) => /^\s*Select model\s*$/.test(l));
  const rows = top < 0 ? [] : lines.slice(top + 1).map((l) => l.match(/^\s*(❯)?\s*(\d+)\.\s+(.*)$/)).filter(Boolean);
  return rows.length ? { options: rows.map((m) => ({ n: Number(m[2]), head: m[3].trim(), chosen: !!m[1] })) } : null;
}

/** A model T3 Code lists is picked from claude's /model menu with "s", for this session alone,
 * since Enter there saves it as every session's default. Any other restarts claude with --model. */
function pickModel(tab, model) {
  composerMenu.hidden = true;
  if (modelOf(tab) === model) return;
  const done = (ok) => {
    if (!ok) return;
    threadOf(tab).picked = model.name.replace(/^Claude /, "");
    if (tab === active) renderComposer(tab);
  };
  if (model.legacy) {
    refit(tab);
    return whenIdle(tab, () => callOn(tab.machine, { cmd: "set_model", tab: tab.view.id, model: model.slug, cols: tab.term.cols, rows: tab.term.rows }).then(() => done(true), (e) => showComposerNote(String(e), true)));
  }
  whenIdle(tab, async () =>
    done(await throughMenu(tab, "/model", modelMenuOn, async (menu) => {
      const option = menu.options.find((o) => o.head.split(/\s/)[0] === model.menu);
      if (!option) throw `claude's /model menu has no ${model.menu}`;
      await writeTo(tab, arrowsTo(menu, option.n));
      await until(() => modelMenuOn(screenLines(tab))?.options.find((o) => o.chosen)?.n === option.n);
      await writeTo(tab, "s");
    })),
  );
}

let modelFavorites = [];
try {
  modelFavorites = JSON.parse(load("gui-model-favorites")) ?? [];
} catch {}
const modelPicker = { provider: "claude", search: "", legacy: false, highlight: 0 };

/** T3 Code's model picker: a provider rail with Favorites, a search, and the models with their
 * NEW badge, a check on the current one and a star. */
function openModels(anchor) {
  const tab = active;
  if (!tab) return;
  Object.assign(modelPicker, { search: "", highlight: 0, legacy: !!modelOf(tab)?.legacy });
  composerMenu.dataset.kind = "models";
  composerMenu.innerHTML = `<nav class="providers" aria-label="Providers"><button type="button" data-provider="favorites" title="Favorites" aria-label="Favorites">${icon("star")}</button><span class="rule"></span>
      <button type="button" data-provider="claude" title="Claude" aria-label="Claude">${icon("claude")}</button></nav>
    <div class="pane"><input class="search" placeholder="Search models..." autocomplete="off" spellcheck="false"><div class="list" role="listbox"></div></div>`;
  composerMenu.hidden = false;
  placeMenu(anchor);
  const search = composerMenu.querySelector(".search");
  search.oninput = () => {
    Object.assign(modelPicker, { search: search.value.trim().toLowerCase(), highlight: 0 });
    renderModels(tab);
  };
  search.onkeydown = (e) => {
    const rows = [...composerMenu.querySelectorAll(".list [data-row]")];
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      modelPicker.highlight = (modelPicker.highlight + (e.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length;
      renderModels(tab);
    } else if (e.key === "Enter") {
      e.preventDefault();
      rows[modelPicker.highlight]?.click();
    } else if (e.key === "Escape") {
      composerMenu.hidden = true;
      prompt.focus();
    } else if (e.key === "ArrowLeft" && !search.value) composerMenu.querySelector(`[data-provider="${modelPicker.provider}"]`).focus();
    e.stopPropagation();
  };
  composerMenu.querySelector(".providers").onclick = (e) => {
    const b = e.target.closest("[data-provider]");
    if (!b) return;
    Object.assign(modelPicker, { provider: b.dataset.provider, highlight: 0 });
    renderModels(tab);
    search.focus();
  };
  composerMenu.querySelector(".list").onclick = (e) => {
    const fav = e.target.closest("[data-fav]");
    if (fav) {
      const slug = fav.dataset.fav;
      modelFavorites = modelFavorites.includes(slug) ? modelFavorites.filter((s) => s !== slug) : [...modelFavorites, slug];
      save("gui-model-favorites", JSON.stringify(modelFavorites));
      return renderModels(tab);
    }
    const row = e.target.closest("[data-row]");
    if (!row) return;
    if (row.dataset.row === "legacy") {
      modelPicker.legacy = !modelPicker.legacy;
      return renderModels(tab);
    }
    pickModel(tab, CLAUDE_MODELS.find((m) => m.slug === row.dataset.row));
  };
  renderModels(tab);
  search.focus();
}
function renderModels(tab) {
  const { provider, search, legacy, highlight } = modelPicker;
  for (const b of composerMenu.querySelectorAll("[data-provider]")) b.setAttribute("aria-pressed", b.dataset.provider === provider);
  const current = modelOf(tab);
  const row = (m) => `<div class="model" data-row="${m.slug}" role="option"><div class="text"><div class="name"><span>${esc(m.name)}</span>${m.isNew ? `<span class="new" aria-label="New model">New</span>` : ""}</div>
      <div class="sub">${icon("claude")}<span>Claude</span></div></div>${m === current ? icon("check", "check") : ""}
      <button type="button" class="fav${modelFavorites.includes(m.slug) ? " on" : ""}" data-fav="${m.slug}" aria-label="${modelFavorites.includes(m.slug) ? "Remove from favorites" : "Add to favorites"}" title="${modelFavorites.includes(m.slug) ? "Remove from favorites" : "Add to favorites"}">${icon("star")}</button></div>`;
  let rows;
  if (search) rows = CLAUDE_MODELS.filter((m) => `${m.name} ${m.slug}`.toLowerCase().includes(search)).map(row);
  else if (provider === "favorites") rows = CLAUDE_MODELS.filter((m) => modelFavorites.includes(m.slug)).map(row);
  else {
    const favored = (m) => modelFavorites.includes(m.slug);
    const shown = [...CLAUDE_MODELS.filter(favored), ...CLAUDE_MODELS.filter((m) => !favored(m) && !m.legacy)];
    const old = CLAUDE_MODELS.filter((m) => m.legacy && !favored(m));
    rows = shown.map(row);
    rows.push(`<div class="model legacy" data-row="legacy" role="option" aria-expanded="${legacy}"><div class="text"><div class="name">Legacy models</div><div class="sub"><span>${old.length} models</span></div></div>${icon("chevron-right", "chev")}</div>`);
    if (legacy) rows.push(...old.map(row));
  }
  const list = composerMenu.querySelector(".list");
  list.innerHTML = rows.join("") || `<p class="none">${provider === "favorites" && !search ? "Star a model to keep it here." : "No models found"}</p>`;
  const lit = list.querySelectorAll("[data-row]")[Math.min(highlight, list.querySelectorAll("[data-row]").length - 1)];
  lit?.classList.add("highlight");
  lit?.scrollIntoView({ block: "nearest" });
  markModelJumps(list);
}
document.getElementById("model-button").onclick = (e) => (composerMenu.hidden || composerMenu.dataset.kind !== "models" ? openModels(e.currentTarget) : (composerMenu.hidden = true));
onGuiKey("m", () => document.getElementById("model-button").click(), { shift: true });

/** Claude's effort marks by its prompt box, such as "◐ medium · ultracode · /effort", and
 * "↯ /fast" on its rule while fast mode is on. */
function traitsOn(lines) {
  const bottom = lines.filter((l) => l.trim()).slice(-8);
  const at = bottom.findLastIndex((l) => MARK.test(l));
  if (at < 0) return null;
  const [, effort, more] = bottom[at].match(MARK);
  return { effort, ultracode: more.includes("ultracode"), fast: /↯ \/fast/.test(bottom[at + 1] ?? "") };
}
const MARK = /[○◐●◉◈] (low|medium|high|xhigh|max)((?: · [\w-]+)*?) · \/effort/;
/** Claude's /effort menu: its levels, the one its ▲ points at, and whether Ultracode is on. */
function effortMenuOn(lines) {
  if (!/Esc to cancel/.test(lines.findLast((l) => l.trim()) ?? "")) return null;
  const row = lines.findLastIndex((l) => /^\s*low\s+medium\b/.test(l));
  if (row < 1 || !lines[row - 1].includes("▲")) return null;
  const levels = [...lines[row].matchAll(/\b(low|medium|high|xhigh|max)\b/g)];
  const arrow = lines[row - 1].indexOf("▲");
  const off = (m) => Math.abs(m.index + m[1].length / 2 - arrow);
  const at = levels.reduce((best, m, i) => (off(m) < off(levels[best]) ? i : best), 0);
  return { levels: levels.map((m) => m[1]), at, ultracode: /Ultracode\s+on\b/.test(lines[row - 1]) };
}

const ultrathinking = () => prompt.value.startsWith(ULTRATHINK);
/** "ultrathink" typed in the prompt itself, which claude reads as the deepest thinking. */
const ultrathinkInText = () => /\bultrathink\b/i.test(ultrathinking() ? prompt.value.slice(ULTRATHINK.length) : prompt.value);
function traitsLabel(tab) {
  const traits = tab.traits;
  if (!traits) return null;
  const effort = ultrathinking() || ultrathinkInText() ? "Ultrathink" : traits.ultracode ? "Ultracode" : REASONING[traits.effort];
  return traits.fast ? `${effort} Fast` : effort;
}
const effortChoice = (tab) => (ultrathinking() || ultrathinkInText() ? "ultrathink" : tab.traits?.ultracode ? "ultracode" : tab.traits?.effort);

/** Ultrathink goes into the prompt as T3 Code puts it; the rest go through claude's /effort menu
 * with "s", which sets them for this session alone, as T3 Code keeps them per thread. */
function pickEffort(tab, effort) {
  const t = threadOf(tab);
  if (effort === "ultrathink") {
    if (!ultrathinking()) prompt.value = ULTRATHINK + prompt.value;
    saveDraft(tab, prompt.value);
    return autosize();
  }
  if (ultrathinking()) {
    prompt.value = prompt.value.slice(ULTRATHINK.length);
    saveDraft(tab, prompt.value);
    autosize();
  }
  const level = effort === "ultracode" ? "xhigh" : effort;
  const ultracode = effort === "ultracode";
  if (tab.traits?.effort === level && tab.traits.ultracode === ultracode) return;
  whenIdle(tab, () =>
    throughMenu(tab, "/effort", effortMenuOn, async (menu) => {
      const to = menu.levels.indexOf(level);
      if (to < 0) throw `${modelName(tab)} has no ${REASONING[level]} effort`;
      await writeTo(tab, (to > menu.at ? "\x1b[C" : "\x1b[D").repeat(Math.abs(to - menu.at)));
      await until(() => effortMenuOn(screenLines(tab))?.at === to);
      if (menu.ultracode !== ultracode) {
        await writeTo(tab, "\t");
        await until(() => effortMenuOn(screenLines(tab))?.ultracode === ultracode);
      }
      await writeTo(tab, "s");
    }),
  );
  t.effortAsked = effort;
}
/** Fast mode, which claude's /fast turns on for the session and keeps as the default for new ones. */
function pickFast(tab, on) {
  if (!!tab.traits?.fast === on) return;
  whenIdle(tab, async () => {
    await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text: `/fast ${on ? "on" : "off"}` });
    await until(() => !!traitsOn(screenLines(tab))?.fast === on, 4000);
  });
}

/** The Reasoning and Fast Mode groups, as T3 Code's traits menu and its "…" menu list them. */
function traitEntries(tab) {
  const model = modelOf(tab);
  const efforts = model?.efforts ?? ALL_REASONING;
  const chosen = effortChoice(tab);
  const locked = ultrathinkInText() && !ultrathinking();
  const entries = [];
  if (efforts.length) {
    entries.push({ heading: "Reasoning" });
    if (locked) entries.push({ note: 'Your prompt contains "ultrathink" in the text. Remove it to change this option.' });
    entries.push(...efforts.map((e) => ({
      label: REASONING[e],
      badge: e === "medium" ? "Default" : "",
      hint: e === "ultracode" ? "xhigh effort plus multi-agent workflow orchestration" : "",
      checked: chosen === e,
      disabled: locked,
      run: () => pickEffort(tab, e),
    })));
  }
  if (model?.fast ?? !model) {
    if (entries.length) entries.push({ rule: true });
    entries.push({ heading: "Fast Mode" }, ...[true, false].map((on) => ({ label: on ? "On" : "Off", checked: !!tab.traits?.fast === on, run: () => pickFast(tab, on) })));
  }
  return entries;
}
const traitsButton = document.getElementById("traits-button");
traitsButton.onclick = (e) => {
  if (active && !composerMenu.hidden && composerMenu.dataset.kind === "traits") return (composerMenu.hidden = true);
  if (active?.traits) openMenu(e.currentTarget, traitEntries(active), "traits");
};
onGuiKey("e", () => {
  if (!traitsButton.closest("[hidden]") && getComputedStyle(traitsButton).display !== "none") traitsButton.click();
  else if (active?.traits) document.getElementById("more-button").click();
}, { shift: true });

const modeButton = document.getElementById("mode-button");
const planToggle = document.getElementById("plan-toggle");
function renderControls(tab) {
  const t = tab && threadOf(tab);
  const name = tab ? modelName(tab) : "Default";
  const modelButton = document.getElementById("model-button");
  modelButton.querySelector(".name").textContent = name;
  modelButton.title = `${name} · ${bindingLabel("modelPicker.toggle") ?? ""}`;
  const label = tab?.view.running && traitsLabel(tab);
  const traitsBlock = document.getElementById("traits-block");
  traitsBlock.hidden = !label || !traitEntries(tab).length;
  if (!traitsBlock.hidden) traitsButton.innerHTML = `${icon("brain")}<span class="name">${esc(label)}</span>${icon("chevron-down")}`;
  traitsButton.setAttribute("aria-label", label || "Reasoning");
  const mode = tab?.view.running && MODES[tab.mode] && tab.mode;
  document.getElementById("mode-block").hidden = !mode;
  document.getElementById("more-button").hidden = !(tab?.view.running && (mode || label)) || composer.dataset.compact !== "true";
  if (!mode) return;
  const access = mode === "plan" ? t.access ?? "manual" : mode;
  const shown = JSON.stringify([access, mode]);
  if (modeButton.dataset.mode === shown) return;
  modeButton.dataset.mode = shown;
  modeButton.innerHTML = `${icon(MODES[access].icon)}<span class="name">${esc(MODES[access].label)}</span>${icon("chevron-down")}`;
  modeButton.title = `${MODES[access].hint} (${bindingLabel("composer.mode") ?? ""})`;
  const plan = mode === "plan";
  planToggle.setAttribute("aria-pressed", plan);
  planToggle.title = plan ? "Plan mode. Click to return to normal build mode" : "Default mode. Click to enter plan mode";
  planToggle.innerHTML = `${icon(plan ? "pencil-ruler" : "bot")}<span class="name">${plan ? "Plan" : "Build"}</span>`;
}
const accessEntries = (tab) => {
  const mode = tab.mode === "plan" ? threadOf(tab).access ?? "manual" : tab.mode;
  return ACCESS.map((m) => ({ ...MODES[m], checked: mode === m, run: () => pickAccess(tab, m) }));
};
modeButton.onclick = (e) => {
  if (!composerMenu.hidden && composerMenu.dataset.kind === "access") return (composerMenu.hidden = true);
  if (active?.mode) openMenu(e.currentTarget, accessEntries(active), "access");
};
planToggle.onclick = () => active && togglePlan(active);
onGuiKey("a", () => {
  if (document.getElementById("mode-block").hidden) return;
  if (composer.dataset.compact === "true") document.getElementById("more-button").click();
  else modeButton.click();
}, { shift: true });

/** Full access needs claude restarted with permission checks off; the other modes are stops of
 * its Shift+Tab cycle. Picked while in Plan, a mode waits for the return to Build, as in T3 Code. */
async function pickAccess(tab, m) {
  const t = threadOf(tab);
  if (m === "bypass" && !tab.view.bypass) return setBypass(tab, true);
  if (tab.mode === "plan" && m !== "bypass") {
    t.access = m;
    return renderComposer(tab);
  }
  await pickMode(tab, m);
}
function togglePlan(tab) {
  pickMode(tab, tab.mode === "plan" ? threadOf(tab).access ?? "manual" : "plan");
}

/** Presses Shift+Tab, which cycles claude's mode, until the screen shows `want`, or the cycle comes
 * round without it. */
async function pickMode(tab, want) {
  if (waitsOnMenu(tab)) return;
  tab.cycling = true;
  const seen = new Set();
  while (terminalMode(tab) !== want && !seen.has(terminalMode(tab)) && !tab.menu) {
    const was = terminalMode(tab);
    seen.add(was);
    await writeTo(tab, SHIFT_TAB);
    if (!(await until(() => terminalMode(tab) !== was))) break;
  }
  tab.cycling = false;
  tab.mode = terminalMode(tab) ?? tab.mode;
  if (tab.mode && tab.mode !== "plan") threadOf(tab).access = tab.mode;
  if (tab === active) renderComposer(tab);
}

/** T3 Code folds Traits, Mode and Access into "…" once the composer is too narrow for them. */
document.getElementById("more-button").onclick = (e) => {
  const tab = active;
  if (!tab) return;
  if (!composerMenu.hidden && composerMenu.dataset.kind === "more") return (composerMenu.hidden = true);
  const traits = tab.traits ? traitEntries(tab) : [];
  const entries = traits.length ? [...traits, { rule: true }] : [];
  if (tab.mode) {
    const plan = tab.mode === "plan";
    entries.push({ heading: "Mode" }, { label: "Chat", checked: !plan, run: () => plan && togglePlan(tab) }, { label: "Plan", checked: plan, run: () => !plan && togglePlan(tab) });
    entries.push({ rule: true }, { heading: "Access" }, ...accessEntries(tab).map(({ label, checked, run }) => ({ label, checked, run })));
  }
  openMenu(e.currentTarget, entries, "more");
};
new ResizeObserver(([entry]) => {
  const width = entry.contentRect.width;
  composer.dataset.compact = width < 620;
  composer.dataset.narrow = width < 640;
  if (active) renderControls(active);
}).observe(composer.querySelector(".box"));

const formatTokens = (n) => (n < 1000 ? `${Math.round(n)}` : n < 10_000 ? `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k` : n < 999_500 ? `${Math.round(n / 1000)}k` : `${(n / 1e6).toFixed(1).replace(/\.0$/, "")}m`);
const formatPercent = (p) => (p < 10 ? `${p.toFixed(1).replace(/\.0$/, "")}%` : `${Math.round(p)}%`);
/** How full the context window is after the last answer, as T3 Code's ring shows it. */
function contextOf(tab) {
  const context = threadOf(tab).context;
  if (!context) return null;
  const max = modelOf(tab)?.context ?? 1e6;
  return { ...context, max, percent: Math.min(100, (context.tokens / max) * 100) };
}
const contextMeter = document.getElementById("context-meter");
function renderContext(tab) {
  const context = tab && contextOf(tab);
  contextMeter.hidden = !context;
  if (!context) return;
  const circumference = 2 * Math.PI * 9.75;
  contextMeter.dataset.over = context.percent > 90;
  contextMeter.setAttribute("aria-label", `Context window ${formatPercent(context.percent)} used`);
  contextMeter.innerHTML = `<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9.75" class="track"/><circle cx="12" cy="12" r="9.75" class="used" stroke-dasharray="${circumference}" stroke-dashoffset="${circumference * (1 - context.percent / 100)}"/></svg>`;
  if (!contextCard.hidden) renderContextCard(tab);
}
const contextCard = Object.assign(document.createElement("div"), { id: "context-card", hidden: true });
composer.querySelector(".box").append(contextCard);
function renderContextCard(tab) {
  const context = contextOf(tab);
  if (!context) return (contextCard.hidden = true);
  const busy = phaseOf(tab) === "working" || !!tab.menu;
  contextCard.innerHTML = `<div class="head"><span class="title">Context Window</span><span class="value">${formatPercent(context.percent)}<span class="dot">·</span>${formatTokens(context.tokens)}/${formatTokens(context.max)}</span></div>
    <div class="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(context.percent)}" aria-label="Context window usage"><span style="width:${context.percent}%"></span></div>
    <p class="note">Context for ${esc(modelName(tab))} compacts automatically when needed.</p>
    <button type="button" class="outline" data-compact${busy ? " disabled" : ""}>${icon("minimize-2")}Compact context</button>${busy ? `<p class="note">Wait for claude to finish the turn.</p>` : ""}`;
  contextCard.hidden = false;
}
let contextTimer;
const showContextCard = (on, wait) => {
  clearTimeout(contextTimer);
  contextTimer = setTimeout(() => (on && active ? renderContextCard(active) : (contextCard.hidden = true)), wait);
};
for (const el of [contextMeter, contextCard]) {
  el.addEventListener("pointerenter", (e) => e.pointerType === "mouse" && showContextCard(true, 150));
  el.addEventListener("pointerleave", (e) => e.pointerType === "mouse" && showContextCard(false, 150));
}
contextMeter.onclick = () => (contextCard.hidden ? active && renderContextCard(active) : (contextCard.hidden = true));
contextCard.onclick = (e) => {
  if (!e.target.closest("[data-compact]") || !active) return;
  contextCard.hidden = true;
  sendSlash(active, "/compact");
};
window.addEventListener("mousedown", (e) => !contextCard.hidden && !e.target.closest("#context-card, #context-meter") && (contextCard.hidden = true), true);

/** T3 Code offers to compact a long conversation first once claude's prompt cache has gone cold,
 * so the next turn doesn't read the whole history again at full price. */
const COMPACT_OFFER_TOKENS = 200_000;
const CACHE_COLD_MS = 70 * 60_000;
function compactOffer(tab) {
  const context = contextOf(tab);
  if (!context || context.tokens < COMPACT_OFFER_TOKENS || tab.menu || phaseOf(tab) === "working") return null;
  return Date.now() - Date.parse(context.at) >= CACHE_COLD_MS ? context.tokens : null;
}
const compactPill = document.getElementById("compact-pill");
function renderCompactPill(tab) {
  const tokens = tab && !/^\/compact\b/.test(prompt.value.trim()) && compactOffer(tab);
  compactPill.hidden = !tokens;
  if (!tokens) return;
  const keep = !!threadOf(tab).keepHistory;
  compactPill.dataset.keep = keep;
  compactPill.setAttribute("aria-pressed", !keep);
  compactPill.setAttribute("aria-label", `Compact ${formatTokens(tokens)} tokens of history before sending`);
  compactPill.title = `${keep ? `Next send keeps all ${formatTokens(tokens)} tokens.\nClick to compact first` : `Next send compacts ${formatTokens(tokens)} tokens first.\nClick to keep full history`}`;
  compactPill.innerHTML = `${icon("minimize-2")}${keep ? "Full" : "Compact"}<span>${formatTokens(tokens)}</span>`;
}
compactPill.onclick = () => {
  if (!active) return;
  const t = threadOf(active);
  t.keepHistory = !t.keepHistory;
  renderComposer(active);
};

const listsOf = new Map();
/** The tab's slash commands or files, kept until `fresh` asks the machine again. */
function fetchList(tab, cmd, fresh = false) {
  const key = `${cmd} ${tab.key} ${tab.view.cwd}`;
  if (fresh || !listsOf.has(key)) listsOf.set(key, callOn(tab.machine, { cmd, tab: tab.view.id }).catch(() => (listsOf.delete(key), [])));
  return listsOf.get(key);
}

const TRIGGER_KINDS = ["slash", "mention", "skill", "pr"];
const isTrigger = () => TRIGGER_KINDS.includes(composerMenu.dataset.kind) && !composerMenu.hidden;
/** What the word before the caret asks for, as T3 Code's triggers read it: `/` at the start,
 * `@` files and folders, `$` skills, `#` pull requests. */
function triggerAt() {
  const before = prompt.value.slice(0, prompt.selectionStart);
  const slash = prompt.selectionStart === prompt.value.length ? prompt.value.match(/^\/(\S*)$/)?.[1] : undefined;
  if (slash !== undefined) return { kind: "slash", query: slash };
  const word = before.match(/(?:^|\s)([@$#])([^\s@$#]*)$/);
  if (!word) return null;
  const kind = { "@": "mention", $: "skill", "#": "pr" }[word[1]];
  if (kind === "pr" && word[2] && !/^[\p{L}\p{N}][\p{L}\p{N}_-]*$/u.test(word[2])) return null;
  if (kind === "skill" && /^\d/.test(word[2])) return null;
  return { kind, query: word[2] };
}
const TRIGGER_EMPTY = { slash: "No matching command.", mention: "No matching files or folders.", skill: "No skills found. Try / to browse provider commands.", pr: "No matching pull requests." };

/** `/` at the start of the prompt lists claude's commands and skills, and `@` before the caret the
 * folder's files and folders, each filtered as more is typed, as T3 Code's trigger menus do. */
async function showTrigger() {
  const tab = active;
  const trigger = tab && !tab.menu && !blockAt(prompt.value.split("\n"), caretLine(prompt.value, prompt.selectionStart)) && triggerAt();
  if (!trigger) {
    if (isTrigger()) composerMenu.hidden = true;
    return;
  }
  const value = prompt.value;
  const { kind } = trigger;
  const query = trigger.query.toLowerCase();
  const cmd = { slash: "commands", mention: "files", skill: "commands", pr: "pr_list" }[kind];
  let list = await fetchList(tab, cmd, query === "");
  if (active !== tab || prompt.value !== value) return;
  if (kind === "skill") list = list.filter((c) => c.skill);
  const score = { slash: slashScore, mention: fileScore, skill: slashScore, pr: prScore }[kind];
  const ranked = list
    .map((item) => ({ item, score: score(item, query) }))
    .filter((r) => r.score !== null)
    .sort((a, b) => a.score - b.score)
    .slice(0, 60)
    .map((r) => r.item);
  composerMenu.dataset.kind = kind;
  composerMenu.replaceChildren(...(ranked.length ? ranked.map((item, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.classList.toggle("highlight", i === 0);
    if (kind === "slash") {
      b.dataset.pick = `/${item.name}`;
      b.innerHTML = `<span class="name">/${esc(item.name)}</span><span class="about">${esc(item.description)}</span>${item.source !== "builtin" ? `<span class="source">${esc(item.skill ? "skill" : item.source)}</span>` : ""}`;
    } else if (kind === "skill") {
      b.dataset.pick = `$${item.name}`;
      b.innerHTML = `${icon("sparkles", "lead")}<span class="name">${esc(item.name)}</span><span class="about">${esc(item.description)}</span><span class="source">${esc(item.source === "builtin" ? "system" : item.source)} skill</span>`;
    } else if (kind === "pr") {
      b.dataset.pick = `#${item.number}`;
      b.dataset.url = item.url;
      b.innerHTML = `${icon(PR_STATE_ICONS[prMenuState(item)], `lead pr-${prMenuState(item)}`)}<span class="name">#${item.number}</span><span class="about">${esc(item.title)}</span>`;
    } else {
      b.dataset.pick = `@${item}`;
      const [dir, base] = item.replace(/\/$/, "").match(/^(.*\/)?([^/]*)$/).slice(1);
      b.innerHTML = `${fileTypeIcon(item, "lead")}<span class="name">${esc(base)}</span><span class="about">${esc(dir ?? "")}</span>`;
    }
    b.onmousedown = (e) => e.preventDefault();
    b.onclick = () => pickTrigger(b.dataset.pick, b.dataset.url);
    return b;
  }) : [Object.assign(document.createElement("p"), { className: "none", textContent: TRIGGER_EMPTY[kind] })]));
  composerMenu.hidden = false;
  composerMenu.style.left = composerMenu.style.right = composerMenu.style.top = composerMenu.style.bottom = "";
}
const prMenuState = (pr) => (pr.isDraft ? "draft" : (pr.state ?? "").toLowerCase());
const PR_STATE_ICONS = { open: "git-pull-request", draft: "git-pull-request", merged: "git-merge", closed: "git-pull-request" };
function prScore(pr, query) {
  if (!query) return 0;
  if (String(pr.number).startsWith(query)) return 1;
  const title = pr.title.toLowerCase();
  if (title.startsWith(query)) return 2;
  return title.includes(query) || pr.headRefName?.toLowerCase().includes(query) ? 3 : null;
}
/** Ranks a command for `/query` as T3 Code does: its name exactly, then by prefix, at a word
 * boundary, anywhere, then its description. */
function slashScore(c, query) {
  if (!query) return 0;
  const name = c.name.toLowerCase();
  const about = c.description.toLowerCase();
  if (name === query) return 0;
  if (name.startsWith(query)) return 2;
  if (["-", "_", "/", ":"].some((mark) => name.includes(mark + query))) return 4;
  if (name.includes(query)) return 6;
  if (about.startsWith(query)) return 22;
  if (about.includes(` ${query}`)) return 24;
  return about.includes(query) ? 26 : null;
}
/** A path ranks by its last part, then anywhere in it, shallower first. */
function fileScore(path, query) {
  const lower = path.toLowerCase();
  const base = lower.replace(/\/$/, "").split("/").pop();
  const depth = path.split("/").length / 100;
  if (!query) return depth;
  if (base.startsWith(query)) return 2 + depth;
  if (base.includes(query)) return 4 + depth;
  return lower.includes(query) ? 6 + depth : null;
}
/** Puts `pick` in place of the `/` or `@` word before the caret. */
/** Puts `pick` in place of the trigger word before the caret. A file, skill or pull request goes in
 * as a chip: a short word the composer draws as T3 Code's chip and sends as what it stands for. */
function pickTrigger(pick, url) {
  composerMenu.hidden = true;
  const at = prompt.selectionStart;
  const start = pick.startsWith("/") ? 0 : prompt.value.slice(0, at).search(/[@$#][^\s@$#]*$/);
  let word = pick;
  if (active && !pick.startsWith("/") && !pick.endsWith("/")) {
    const chips = chipsOf(active);
    const value = pick.startsWith("@") ? pick.slice(1) : pick.startsWith("$") ? pick.slice(1) : url;
    const kind = { "@": "mention", $: "skill", "#": "pr" }[pick[0]];
    if (kind === "mention") {
      const base = `@${value.split("/").pop()}`;
      word = chips[base] && chips[base].value !== value ? pick : base;
    }
    chips[word] = { kind, value };
  }
  const insert = pick.endsWith("/") ? pick : `${word} `;
  prompt.value = prompt.value.slice(0, start) + insert + prompt.value.slice(at);
  prompt.setSelectionRange(start + insert.length, start + insert.length);
  if (active) saveDraft(active, prompt.value);
  autosize();
  prompt.focus();
  if (pick.endsWith("/")) showTrigger();
}
let chipsSaved = {};
try {
  chipsSaved = sessionsOnly(JSON.parse(load("gui-chips")));
} catch {}
onRekey((from, to) => {
  if (!(from in chipsSaved)) return;
  chipsSaved[to] = chipsSaved[from];
  delete chipsSaved[from];
  save("gui-chips", JSON.stringify(chipsSaved));
});
const chipsOf = (tab) => (chipsSaved[threadKey(tab)] ??= {});
const CHIP_WORD = /(^|\s)([@$#][^\s]+)(?=\s|$)/g;
/** The chips the prompt still holds, keeping the saved ones in step with the text. */
function liveChips(tab) {
  const chips = chipsOf(tab);
  const words = new Set([...prompt.value.matchAll(CHIP_WORD)].map((m) => m[2]));
  for (const word of Object.keys(chips)) if (!words.has(word)) delete chips[word];
  if (!Object.keys(chips).length) delete chipsSaved[threadKey(tab)];
  save("gui-chips", JSON.stringify(chipsSaved));
  return chips;
}
/** What claude gets for the prompt: a file chip's whole path, a skill as its slash command, a
 * pull request as its URL. */
function expandChips(tab, text) {
  const chips = chipsOf(tab);
  return text.replace(CHIP_WORD, (whole, space, word) => {
    const chip = chips[word];
    if (!chip) return whole;
    return space + (chip.kind === "mention" ? `@${chip.value}` : chip.kind === "skill" ? `/${chip.value}` : chip.value);
  });
}
const mirror = Object.assign(document.createElement("div"), { id: "prompt-mirror", ariaHidden: "true" });
prompt.before(mirror);
const CHIP_ICONS = { skill: "sparkles", pr: "git-pull-request" };
/** The textarea can't draw chips or code blocks, so a copy of its text under it does, line for line
 * and letter for letter: each chip's sigil swapped for its icon, each code block framed and
 * highlighted. The textarea's own text turns transparent above it. */
function renderMirror() {
  const tab = active;
  const chips = tab ? liveChips(tab) : {};
  const lines = prompt.value.split("\n");
  const blocks = promptBlocks(lines);
  const on = (Object.keys(chips).length > 0 || blocks.length > 0) && composer.dataset.composing !== "true";
  composer.dataset.chips = on;
  if (!on) {
    mirror.innerHTML = "";
    blockPickers.innerHTML = "";
    return;
  }
  const line = (text) => {
    let html = "";
    let last = 0;
    for (const m of text.matchAll(CHIP_WORD)) {
      const chip = chips[m[2]];
      if (!chip) continue;
      const at = m.index + m[1].length;
      html += esc(text.slice(last, at));
      const mark = chip.kind === "mention" ? fileTypeIcon(chip.value) : icon(CHIP_ICONS[chip.kind]);
      html += `<span class="chip ${chip.kind}"><span class="sigil">${esc(m[2][0])}${mark}</span>${esc(m[2].slice(1))}</span>`;
      last = at + m[2].length;
    }
    return `<div class="pl">${html}${esc(text.slice(last)) || "\u200b"}</div>`;
  };
  let html = "";
  let next = 0;
  for (const b of blocks) {
    html += lines.slice(next, b.open).map(line).join("");
    const code = lines.slice(b.open + 1, b.close ?? lines.length);
    const lang = languageOf(b.info);
    const colored = highlightLines(code, (languageEntry(lang)?.id ?? lang.toLowerCase()) || null);
    html += `<div class="pblock"><div class="pl fence fence-open">${esc(lines[b.open])}</div>${code.map((c, i) => `<div class="pl code">${c ? colored[i] : "\u200b"}</div>`).join("")}${b.close === null ? "" : `<div class="pl fence">${esc(lines[b.close])}</div>`}</div>`;
    next = (b.close ?? lines.length - 1) + 1;
  }
  mirror.innerHTML = html + lines.slice(next).map(line).join("");
  Object.assign(mirror.style, { top: `${prompt.offsetTop}px`, left: `${prompt.offsetLeft}px`, width: `${prompt.clientWidth}px`, height: `${prompt.clientHeight}px` });
  mirror.scrollTop = prompt.scrollTop;
  renderBlockPickers(blocks);
}
document.addEventListener("highlighter", () => active && renderMirror());
/** T3 Code's code blocks in the prompt: a fence on a line of its own opens one, closed by a fence
 * of the same kind or by the end of the prompt. The textarea keeps the fences as typed, which is the
 * markdown claude gets; the copy under it draws the block framed and highlighted, with the language
 * picker on the fence line. */
const PROMPT_LANGUAGES = [["", "Plain text"], ["bash", "Bash"], ["c", "C"], ["cpp", "C++"], ["csharp", "C#"], ["css", "CSS"], ["dart", "Dart"], ["diff", "Diff"], ["dockerfile", "Dockerfile"], ["elixir", "Elixir"], ["go", "Go"], ["graphql", "GraphQL"], ["haskell", "Haskell"], ["html", "HTML"], ["java", "Java"], ["javascript", "JavaScript"], ["json", "JSON"], ["jsx", "JSX"], ["kotlin", "Kotlin"], ["lua", "Lua"], ["markdown", "Markdown"], ["php", "PHP"], ["powershell", "PowerShell"], ["python", "Python"], ["r", "R"], ["ruby", "Ruby"], ["rust", "Rust"], ["scala", "Scala"], ["scss", "SCSS"], ["sql", "SQL"], ["svelte", "Svelte"], ["swift", "Swift"], ["toml", "TOML"], ["tsx", "TSX"], ["typescript", "TypeScript"], ["vue", "Vue"], ["xml", "XML"], ["yaml", "YAML"], ["zig", "Zig"]].map(([id, label]) => ({ id, label }));
const LANGUAGE_ALIASES = { cs: "csharp", "c++": "cpp", js: "javascript", md: "markdown", py: "python", rb: "ruby", rs: "rust", sh: "bash", shell: "bash", ts: "typescript", yml: "yaml" };
/** A file name of each language, for its icon. */
const LANGUAGE_FILES = { bash: "a.sh", c: "a.c", cpp: "a.cpp", csharp: "a.cs", css: "a.css", dart: "a.dart", diff: "a.diff", dockerfile: "Dockerfile", elixir: "a.ex", go: "a.go", graphql: "a.graphql", haskell: "a.hs", html: "a.html", java: "a.java", javascript: "a.js", json: "a.json", jsx: "a.jsx", kotlin: "a.kt", lua: "a.lua", markdown: "a.md", php: "a.php", powershell: "a.ps1", python: "a.py", r: "a.r", ruby: "a.rb", rust: "a.rs", scala: "a.scala", scss: "a.scss", sql: "a.sql", svelte: "a.svelte", swift: "a.swift", toml: "a.toml", tsx: "a.tsx", typescript: "a.ts", vue: "a.vue", xml: "a.xml", yaml: "a.yaml", zig: "a.zig" };
const languageOf = (info) => info.trim().split(/\s+/, 1)[0] ?? "";
const languageEntry = (lang) => {
  const id = lang === "text" ? "" : LANGUAGE_ALIASES[lang.toLowerCase()] ?? lang.toLowerCase();
  return PROMPT_LANGUAGES.find((l) => l.id === id) ?? null;
};
const languageLabel = (lang) => languageEntry(lang)?.label ?? lang;
/** The info string with `language` in place of its first word; plain text clears it unless more follows. */
function withLanguage(info, language) {
  const rest = info.slice(languageOf(info).length + (/^\s*/.exec(info)[0].length));
  if (language) return `${language}${rest}`;
  return rest.trim() ? `text${rest}` : "";
}
const OPENING_FENCE = /^(`{3,}|~{3,})([^\n]*)$/;
const closes = (line, fence) => {
  const run = /^(`{3,}|~{3,})[ \t]*$/.exec(line)?.[1];
  return !!run && run[0] === fence[0] && run.length >= fence.length;
};
/** The prompt's code blocks by line: where each opens, its fence and info string, and the line that
 * closes it, or none when it runs to the end. A fence opens a block only once a line follows it. */
function promptBlocks(lines) {
  const blocks = [];
  for (let i = 0; i < lines.length - 1; i++) {
    const open = OPENING_FENCE.exec(lines[i]);
    if (!open || (open[1][0] === "`" && open[2].includes("`"))) continue;
    let end = i + 1;
    while (end < lines.length && !closes(lines[end], open[1])) end++;
    blocks.push({ open: i, close: end < lines.length ? end : null, fence: open[1], info: open[2] });
    i = end;
  }
  return blocks;
}
/** The block the caret's line is in, from its fence through its closing one. */
function blockAt(lines, line) {
  return promptBlocks(lines).find((b) => line >= b.open && line <= (b.close ?? lines.length - 1)) ?? null;
}
const caretLine = (text, at) => text.slice(0, at).split("\n").length - 1;

/** The language buttons sit on the fence lines, over the textarea. */
const blockPickers = Object.assign(document.createElement("div"), { id: "prompt-blocks" });
prompt.after(blockPickers);
function renderBlockPickers(blocks) {
  const fences = [...mirror.querySelectorAll(".pl.fence-open")];
  blockPickers.innerHTML = blocks.map((b, i) => {
    const lang = languageOf(b.info);
    const id = languageEntry(lang)?.id ?? lang;
    return `<button type="button" class="block-lang" data-block="${i}" aria-label="Code language: ${esc(languageLabel(lang) || "Plain text")}">${fileTypeIcon(LANGUAGE_FILES[id] ?? "a.txt")}<span>${esc(languageLabel(lang) || "Plain text")}</span>${icon("chevron-down")}</button>`;
  }).join("");
  Object.assign(blockPickers.style, { top: `${prompt.offsetTop}px`, left: `${prompt.offsetLeft}px`, width: `${prompt.clientWidth}px`, height: `${prompt.clientHeight}px` });
  const top = mirror.getBoundingClientRect().top;
  [...blockPickers.children].forEach((b, i) => fences[i] && (b.style.top = `${fences[i].getBoundingClientRect().top - top}px`));
}
blockPickers.addEventListener("mousedown", (e) => e.target.closest(".block-lang") && e.preventDefault());
blockPickers.addEventListener("click", (e) => {
  const b = e.target.closest(".block-lang");
  if (b && active) openLanguages(b, Number(b.dataset.block));
});
/** T3 Code's language picker: a searchable list, each language with its file icon and a check on the current one. */
function openLanguages(anchor, index) {
  const block = promptBlocks(prompt.value.split("\n"))[index];
  if (!block) return;
  const lang = languageOf(block.info);
  const current = languageEntry(lang)?.id ?? lang;
  const items = PROMPT_LANGUAGES.some((l) => l.id === current) ? PROMPT_LANGUAGES : [{ id: current, label: current }, ...PROMPT_LANGUAGES];
  composerMenu.dataset.kind = "languages";
  composerMenu.innerHTML = `<input class="search" placeholder="Search languages" aria-label="Search languages" autocomplete="off" spellcheck="false"><div class="list" role="listbox"></div>`;
  composerMenu.hidden = false;
  const box = composerMenu.offsetParent.getBoundingClientRect();
  const at = anchor.getBoundingClientRect();
  Object.assign(composerMenu.style, { right: "", left: `${Math.max(0, at.left - box.left)}px`, top: "", bottom: `${box.bottom - at.top + 4}px` });
  const search = composerMenu.querySelector(".search");
  const list = () => {
    const needle = search.value.trim().toLowerCase();
    const shown = needle ? items.filter((l) => l.label.toLowerCase().includes(needle) || l.id.includes(needle)) : items;
    composerMenu.querySelector(".list").innerHTML = shown.map((l, i) => `<button type="button" data-lang="${esc(l.id)}" role="option"${i === 0 ? ` class="highlight"` : ""}>${fileTypeIcon(LANGUAGE_FILES[l.id] ?? "a.txt", "lead")}<span class="label">${esc(l.label)}</span>${l.id === current ? icon("check", "check") : ""}</button>`).join("") || `<p class="none">No matching languages.</p>`;
  };
  list();
  search.oninput = list;
  search.onkeydown = (e) => {
    const rows = [...composerMenu.querySelectorAll("[data-lang]")];
    const at = rows.findIndex((r) => r.classList.contains("highlight"));
    if (e.key === "Escape") {
      composerMenu.hidden = true;
      prompt.focus();
    } else if (e.key === "Enter") rows[at]?.click();
    else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      rows[at]?.classList.remove("highlight");
      const next = rows[(at + (e.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length];
      next?.classList.add("highlight");
      next?.scrollIntoView({ block: "nearest" });
    } else return;
    e.stopPropagation();
  };
  composerMenu.querySelector(".list").onclick = (e) => {
    const row = e.target.closest("[data-lang]");
    if (!row) return;
    composerMenu.hidden = true;
    if (row.dataset.lang !== current) setBlockLanguage(index, row.dataset.lang);
    else prompt.focus();
  };
  search.focus();
}
/** Rewrites the block's fence for `language`, then puts the caret at the end of its code, as T3 Code does. */
function setBlockLanguage(index, language) {
  const lines = prompt.value.split("\n");
  const block = promptBlocks(lines)[index];
  if (!block) return;
  lines[block.open] = `${block.fence}${withLanguage(block.info, language)}`;
  const last = (block.close ?? lines.length) - 1;
  prompt.value = lines.join("\n");
  const caret = lines.slice(0, last + 1).join("\n").length;
  prompt.focus();
  prompt.setSelectionRange(caret, caret);
  prompt.dispatchEvent(new Event("input"));
}

/** Enter and Tab in a code block, as in T3 Code's: Enter on a fence line opens a closed block with
 * the caret in it, Enter in a block keeps the line's indent, on its closing fence leaves it, and
 * after two blank lines at the end of an open one closes it; Tab indents and Shift+Tab outdents. */
function promptCodeKey(e) {
  if ((e.key !== "Enter" && e.key !== "Tab") || e.metaKey || e.ctrlKey || e.altKey || e.isComposing) return false;
  const text = prompt.value;
  const [from, to] = [prompt.selectionStart, prompt.selectionEnd];
  const lines = text.split("\n");
  const at = caretLine(text, from);
  const block = blockAt(lines, at);
  const lineStart = text.lastIndexOf("\n", from - 1) + 1;
  const lineEnd = text.indexOf("\n", from) < 0 ? text.length : text.indexOf("\n", from);
  const replace = (start, end, insert, caret = start + insert.length) => {
    prompt.setRangeText(insert, start, end, "end");
    prompt.setSelectionRange(caret, caret);
    prompt.dispatchEvent(new Event("input"));
    return true;
  };
  if (e.key === "Tab") {
    if (!block || at === block.open || at === block.close) return false;
    if (!e.shiftKey && from === to) return replace(from, to, "  ");
    const end = text.indexOf("\n", Math.max(from, to - 1)) < 0 ? text.length : text.indexOf("\n", Math.max(from, to - 1));
    const original = text.slice(lineStart, end).split("\n");
    const next = original.map((l) => (e.shiftKey ? (l.startsWith("  ") ? l.slice(2) : l.replace(/^[\t ]/, "")) : l ? `  ${l}` : l));
    const firstDelta = next[0].length - original[0].length;
    const joined = next.join("\n");
    prompt.setRangeText(joined, lineStart, end, "preserve");
    prompt.setSelectionRange(Math.max(lineStart, from + firstDelta), Math.max(lineStart, to + joined.length - original.join("\n").length));
    prompt.dispatchEvent(new Event("input"));
    return true;
  }
  if (e.shiftKey || from !== to) return false;
  if (!block) {
    const open = OPENING_FENCE.exec(text.slice(lineStart, lineEnd));
    if (!open || from !== lineEnd || (open[1][0] === "`" && open[2].includes("`"))) return false;
    return replace(from, from, `\n\n${open[1]}`, from + 1);
  }
  if (at === block.close && from === lineEnd) return replace(from, from, "\n");
  if (block.close === null && from === text.length && /\n[ \t]*\n[ \t]*$/.test(text.slice(0, from)) && at > block.open + 2) {
    const trailing = /\n[ \t]*\n[ \t]*$/.exec(text)[0];
    return replace(from - trailing.length, from, `\n${block.fence}\n`);
  }
  return replace(from, from, `\n${/^[ \t]*/.exec(text.slice(lineStart, from))[0]}`);
}

prompt.addEventListener("scroll", () => {
  mirror.scrollTop = prompt.scrollTop;
  if (blockPickers.childElementCount) renderBlockPickers(promptBlocks(prompt.value.split("\n")));
});
prompt.addEventListener("compositionstart", () => {
  composer.dataset.composing = "true";
  renderMirror();
});
prompt.addEventListener("compositionend", () => {
  delete composer.dataset.composing;
  renderMirror();
});
/** Backspace right after a chip takes the whole chip, as T3 Code's atomic chips go. */
function deleteChip(e) {
  if (e.key !== "Backspace" || !active || prompt.selectionStart !== prompt.selectionEnd) return false;
  const before = prompt.value.slice(0, prompt.selectionStart);
  const word = before.match(/(?:^|\s)([@$#][^\s]+)$/)?.[1];
  if (!word || !chipsOf(active)[word]) return false;
  const start = prompt.selectionStart - word.length;
  prompt.setRangeText("", start, prompt.selectionStart, "end");
  prompt.dispatchEvent(new Event("input"));
  return true;
}

function triggerKey(e) {
  const rows = [...composerMenu.querySelectorAll("[data-pick]")];
  const at = rows.findIndex((r) => r.classList.contains("highlight"));
  if (e.key === "Escape") composerMenu.hidden = true;
  else if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey) {
    // A command typed in full, or none matching, goes out as typed.
    if (rows[at] && prompt.value !== rows[at].dataset.pick) pickTrigger(rows[at].dataset.pick, rows[at].dataset.url);
    else if (e.key === "Enter") return !(composerMenu.hidden = true);
  } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    const next = rows[(at + (e.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length];
    rows[at]?.classList.remove("highlight");
    next?.classList.add("highlight");
    next?.scrollIntoView({ block: "nearest" });
  } else return false;
  return true;
}

document.getElementById("stash").insertAdjacentHTML("afterbegin", icon("bookmark"));
function renderStash() {
  const badge = document.getElementById("stash");
  badge.hidden = !stash.length || !approval.hidden;
  badge.querySelector(".count").textContent = stash.length;
}
/** ⌘S sets the draft aside, as T3 Code's stash does, and the Stash tab above the composer brings it back. */
function stashDraft() {
  const tab = active;
  const text = prompt.value.trim();
  if (!tab || (!text && !threadOf(tab).attachments?.length)) return;
  const files = (threadOf(tab).attachments ?? []).map(({ path, name }) => ({ path, name }));
  stash = [{ text, files, at: Date.now() }, ...stash].slice(0, STASH_MAX);
  save("gui-stash", JSON.stringify(stash));
  clearComposer(tab);
  const badge = document.getElementById("stash");
  renderStash();
  badge.classList.remove("pulse");
  void badge.offsetWidth;
  badge.classList.add("pulse");
}
onGuiKey("s", stashDraft);
document.getElementById("stash").onclick = (e) => {
  openMenu(e.currentTarget, stash.map((entry, i) => ({
    label: entry.text.split("\n")[0].slice(0, 80) || entry.files.map((f) => f.name).join(", "),
    hint: ago(entry.at / 1000) === "now" ? "just now" : `${ago(entry.at / 1000)} ago`,
    run: () => {
      if (!active) return;
      if (prompt.value.trim()) stash.push({ text: prompt.value.trim(), files: [], at: Date.now() });
      stash.splice(stash.indexOf(entry), 1);
      save("gui-stash", JSON.stringify(stash));
      const t = threadOf(active);
      t.attachments = [...(t.attachments ?? []), ...(entry.files ?? []).map((f) => ({ ...f, url: null }))];
      prompt.value = entry.text;
      saveDraft(active, entry.text);
      autosize();
      prompt.focus();
    },
    remove: () => {
      stash.splice(i, 1);
      save("gui-stash", JSON.stringify(stash));
      renderStash();
    },
  })), "stash");
};

/** Whether the thread's workspace can still change, as in T3 Code: until its first prompt goes out. */
const workspaceOpen = (tab) => tab.view.running && !tab.view.connector && !threadOf(tab).items.some((i) => i.kind === "user") && !tab.menu && phaseOf(tab) !== "working";
/** The most recently active other thread of the project that works in a worktree, which T3 Code
 * offers as "Previous worktree". */
function previousWorktree(tab) {
  const ws = threadOf(tab).workspace;
  const others = tabs.filter((o) => o !== tab && o.machine === tab.machine && threadOf(o).workspace?.worktree && threadOf(o).workspace.project === ws?.project && o.view.cwd !== tab.view.cwd);
  const latest = others.sort((a, b) => (b.view.updated ?? 0) - (a.view.updated ?? 0))[0];
  return latest && { path: latest.view.cwd, branch: threadOf(latest).workspace.branch };
}
function workspaceLabel(tab, open) {
  const t = threadOf(tab);
  if (!t.workspace?.worktree) return { label: open ? "Current checkout" : "Local checkout", icon: "folder" };
  if (open && t.madeWorktree === tab.view.cwd) return { label: "New worktree", icon: "folder-git-2" };
  return { label: open ? "Current worktree" : "Worktree", icon: "folder-git" };
}
/** The context strip's left end, as T3 Code's: where the thread runs, a menu until it starts. A
 * thread on another machine shows that machine, its workspace as a second icon. */
function workspaceHtml(tab) {
  const open = workspaceOpen(tab);
  const { label, icon: name } = workspaceLabel(tab, open);
  const remote = tab.machine !== LOCAL;
  const content = `${remote ? icon("monitor") : ""}<span class="ws-icon" title="${esc(label)}">${icon(name)}</span><span class="label">${esc(remote ? machineName(tab.machine) : label)}</span>`;
  return open
    ? `<button type="button" class="where" title="${esc(tab.view.cwd)} · ${bindingLabel("composer.workspace") ?? ""}">${content}${icon("chevron-down", "chev")}</button>`
    : `<span class="where" title="${esc(tab.view.cwd)}">${content}</span>`;
}
/** T3 Code's run context menu: the machines it can run on, and Current checkout, New worktree or
 * the previous worktree. */
function openWorkspace(anchor) {
  const tab = active;
  if (!tab || !workspaceOpen(tab)) return;
  const t = threadOf(tab);
  const ws = t.workspace;
  const entries = [{ heading: "Run on" }, ...onlineMachines().map((m) => ({ label: machineName(m), icon: "monitor", checked: m === tab.machine, run: () => m !== tab.machine && runOn(tab, m) }))];
  if (ws?.repo) {
    const previous = previousWorktree(tab);
    const fresh = t.madeWorktree === tab.view.cwd;
    entries.push({ rule: true }, { heading: "Workspace" },
      { label: ws.worktree && !fresh ? "Current worktree" : "Current checkout", icon: ws.worktree && !fresh ? "folder-git" : "folder", checked: !fresh, run: () => fresh && setWorkspace(tab, null) },
      { label: "New worktree", icon: "folder-git-2", checked: fresh, run: () => !fresh && setWorkspace(tab, "new") });
    if (previous) entries.push({ label: previous.branch ? `Previous worktree (${previous.branch})` : "Previous worktree", icon: "folder-git", run: () => setWorkspace(tab, previous.path) });
  }
  openMenu(anchor, entries, "workspace");
}
/** T3 Code's Run on picker (⌘⇧H): the machines alone, where the composer is wide enough to show it
 * apart from the workspace; a narrow one has the combined menu. */
function openHosts(anchor) {
  const tab = active;
  if (!tab || !anchor || !workspaceOpen(tab)) return;
  if (composer.offsetWidth < 768) return anchor.click();
  openMenu(anchor, [{ heading: "Run on" }, ...onlineMachines().map((m) => ({ label: machineName(m), icon: "monitor", checked: m === tab.machine, run: () => m !== tab.machine && runOn(tab, m) }))], "hosts");
}
/** Moves an unstarted thread to a new worktree, another, or the project's checkout; claude starts
 * over there. Send waits meanwhile, as T3 Code's does while it prepares a worktree. */
async function setWorkspace(tab, worktree) {
  const t = threadOf(tab);
  t.preparing = true;
  renderComposer(tab);
  refit(tab);
  try {
    const cwd = await callOn(tab.machine, { cmd: "set_workspace", tab: tab.view.id, worktree, cols: tab.term.cols, rows: tab.term.rows });
    t.madeWorktree = worktree === "new" ? cwd : null;
    tab.view = { ...tab.view, cwd };
    Object.assign(t, { workspaceFor: null, gitFor: null });
    await fetchWorkspace(tab);
    if (worktree === "new") runSetupScripts(tab);
  } catch (e) {
    showComposerNote(String(e), true);
  } finally {
    t.preparing = false;
    if (tab === active) renderComposer(tab);
  }
}
/** Runs an unstarted thread on another machine: the same project there, found or cloned by its
 * origin, in a new thread that takes this one's place. */
async function runOn(tab, machine) {
  const remote = await callOn(tab.machine, { cmd: "git_remote", path: threadOf(tab).workspace?.project ?? tab.view.cwd }).catch(() => null);
  if (!remote) return showComposerNote("This project has no origin to find it by on another machine.", true);
  try {
    const folder = await callOn(machine, { cmd: "clone", repo: remote });
    await openRemote(machine, folder, tab.view.bypass);
    closeTab(tab);
  } catch (e) {
    showComposerNote(String(e), true);
  }
}
const pickPreviousWorktree = () => {
  const tab = active;
  const previous = tab && workspaceOpen(tab) && previousWorktree(tab);
  if (previous) setWorkspace(tab, previous.path);
};
onGuiKey("x", () => document.querySelector("#composer-footer button.where")?.click(), { shift: true });
onGuiKey("h", () => openHosts(document.querySelector("#composer-footer button.where")), { shift: true });
onGuiKey("l", pickPreviousWorktree, { shift: true });

/** T3 Code's empty-thread headline, "What should we build in <project>?", whose project opens a
 * new thread in another one; a thread without a project asks "What should we work on?" over its
 * project picker. */
const heroHead = Object.assign(document.createElement("div"), { id: "hero-head" });
composer.prepend(heroHead);
function renderHero(tab) {
  const on = document.getElementById("chat").dataset.hero === "true";
  heroHead.hidden = !on;
  if (!on) return;
  const name = projectName(tab);
  const html = threadOf(tab).workspace?.scratch
    ? `<h1>What should we work on?</h1><p><button type="button" class="project picker" data-hero-project>No project${icon("chevron-down")}</button></p>`
    : `<h1><span class="ask">What should we build<span class="q">?</span><span class="in"> in</span></span>
      <span class="where"><button type="button" class="project" data-hero-project title="${esc(name)}">${esc(name)}${icon("chevron-down")}</button><span class="in">?</span></span></h1>
    <p><button type="button" class="scratch" data-hero-scratch>or start without a project</button></p>`;
  if (heroHead.dataset.html !== html) {
    heroHead.dataset.html = html;
    heroHead.innerHTML = html;
  }
}
heroHead.addEventListener("click", (e) => {
  if (e.target.closest("[data-hero-scratch]")) return newThreadWithoutProject();
  const trigger = e.target.closest("[data-hero-project]");
  if (trigger) openHeroProjects(trigger);
});
/** Opens a thread in the machine's folder for threads without a project, as T3 Code's Scratch. */
async function openScratchTab(machine) {
  const id = await callOn(machine, { cmd: "open_scratch", bypass: false }).catch((e) => (/unknown variant/.test(String(e)) ? callOn(machine, { cmd: "open_tab", path: null, bypass: false }) : Promise.reject(e)));
  await refreshTabs(machine);
  const tab = tabByKey(keyOf(machine, id));
  activate(tab);
  await startTab(tab);
  return tab;
}
async function openHeroProjects(anchor) {
  const tab = active;
  if (!tab) return;
  composerMenu.dataset.kind = "projects";
  composerMenu.innerHTML = `<input class="search" placeholder="Search projects..." aria-label="Search projects" autocomplete="off" spellcheck="false"><div class="list"><p class="none">Loading projects…</p></div>`;
  composerMenu.hidden = false;
  const box = composerMenu.offsetParent.getBoundingClientRect();
  const at = anchor.getBoundingClientRect();
  composerMenu.style.right = "";
  composerMenu.style.left = `${Math.max(0, at.left - box.left + at.width / 2 - 144)}px`;
  composerMenu.style.top = `${at.bottom - box.top + 6}px`;
  composerMenu.style.bottom = "auto";
  const search = composerMenu.querySelector(".search");
  search.focus();
  const projects = await callOn(tab.machine, { cmd: "list_projects" }).catch(() => []);
  const current = threadOf(tab).workspace?.project ?? tab.view.cwd;
  const list = () => {
    const needle = search.value.trim().toLowerCase();
    const shown = projects.filter((p) => p.name.toLowerCase().includes(needle));
    const rows = shown.map((p) => `<button type="button" data-path="${esc(p.path)}">${projectIcon(p.name)}<span class="label">${esc(p.name)}</span>${p.path === current ? icon("check", "check") : ""}</button>`);
    if (!needle) rows.push(`<button type="button" data-add>${icon("folder-plus", "lead")}<span class="label">Add project</span></button>`);
    composerMenu.querySelector(".list").innerHTML = rows.join("") || `<p class="none">No matching projects.</p>`;
  };
  list();
  search.oninput = list;
  search.onkeydown = (e) => {
    if (e.key === "Escape") composerMenu.hidden = true;
    else if (e.key === "Enter") composerMenu.querySelector(".list button")?.click();
  };
  composerMenu.querySelector(".list").onclick = async (e) => {
    const row = e.target.closest("button");
    if (!row) return;
    composerMenu.hidden = true;
    if (row.dataset.add !== undefined) return addProject();
    if (row.dataset.path === current) return;
    const empty = !threadOf(tab).items.length && !hasDraft(tab);
    try {
      if (tab.machine === LOCAL) await openTab(LOCAL, row.dataset.path);
      else await openRemote(tab.machine, row.dataset.path, false);
      if (empty) closeTab(tab);
    } catch (err) {
      showComposerNote(String(err), true);
    }
  };
}

/** The branch picker: the folder's branches, searchable, and a new branch named as typed. git
 * switches the folder claude works in, so the machine refuses while a turn runs. */
async function openBranches(anchor) {
  const tab = active;
  if (!tab) return;
  composerMenu.dataset.kind = "branches";
  composerMenu.innerHTML = `<input class="search" placeholder="Search refs..." autocomplete="off" spellcheck="false"><div class="refs"><p class="none">Loading refs…</p></div>`;
  composerMenu.hidden = false;
  composerMenu.style.left = composerMenu.style.top = composerMenu.style.bottom = "";
  composerMenu.style.right = "0px";
  const search = composerMenu.querySelector(".search");
  search.focus();
  let git;
  try {
    git = await callOn(tab.machine, { cmd: "branches", tab: tab.view.id });
  } catch (e) {
    composerMenu.querySelector(".refs").innerHTML = `<p class="none">${esc(String(e))}</p>`;
    return;
  }
  threadOf(tab).git = git;
  const delegating = phaseOf(tab) === "delegating";
  const busy = phaseOf(tab) === "working" || !!tab.menu || !!tab.dialog || delegating;
  // A fresh worktree, as T3 Code's draft one, picks the ref it starts from, which may be the remote's.
  const base = threadOf(tab).madeWorktree === tab.view.cwd && workspaceOpen(tab);
  const current = git.refs.find((r) => r.current)?.name;
  let fromRemote = false;
  const list = () => {
    const needle = search.value.trim();
    const shown = git.refs.filter((r) => r.name.toLowerCase().includes(needle.toLowerCase())).slice(0, 80);
    const pr = needle.match(/^#?(\d+)$/)?.[1] ?? (/^https:\/\/\S+\/pull\/\d+/.test(needle) ? needle : null);
    const fresh = needle && !pr && !git.refs.some((r) => r.name === needle);
    const rows = shown.map((r) => `<button type="button" data-ref="${esc(r.name)}"${(r.current && !base) || r.worktree || busy ? " disabled" : ""}>${icon("git-branch", "lead")}<span class="label">${middle(r.name, 32)}</span>
      ${r.current ? `<span class="badge">current</span>` : r.worktree ? `<span class="badge">worktree</span>` : r.remote ? `<span class="badge">remote</span>` : r.name === git.default ? `<span class="badge">default</span>` : ""}</button>`);
    if (pr) rows.unshift(`<button type="button" data-pr-checkout="${esc(pr)}"${busy ? " disabled" : ""}>${icon("git-pull-request", "lead")}<span class="label">Checkout PR ${/^\d+$/.test(pr) ? `#${pr}` : esc(pr)}</span></button>`);
    if (fresh) rows.push(`<button type="button" data-create="${esc(needle)}"${busy ? " disabled" : ""}>${icon("plus", "lead")}<span class="label">Create new ref "${esc(needle)}"</span></button>`);
    composerMenu.querySelector(".refs").innerHTML = (busy ? `<p class="none">${delegating ? "claude's agents are working. Switch branches when they finish." : "claude is working. Switch branches when the turn ends."}</p>` : "") + (rows.join("") || `<p class="none">No refs found.</p>`)
      + (base ? `<label class="from-remote"><span>Start from remote</span><input type="checkbox" role="switch"${fromRemote ? " checked" : ""}></label>` : "");
    const toggle = composerMenu.querySelector(".from-remote input");
    if (toggle) toggle.onchange = () => (fromRemote = toggle.checked);
  };
  list();
  search.oninput = list;
  search.onkeydown = (e) => {
    if (e.key === "Escape") {
      composerMenu.hidden = true;
      prompt.focus();
    } else if (e.key === "Enter") composerMenu.querySelector(".refs button:not([disabled])")?.click();
  };
  composerMenu.querySelector(".refs").onclick = async (e) => {
    const row = e.target.closest("[data-ref], [data-create], [data-pr-checkout]");
    if (!row || row.disabled) return;
    composerMenu.hidden = true;
    const create = !!row.dataset.create;
    const ref = git.refs.find((r) => r.name === row.dataset.ref);
    const name = create ? row.dataset.create : row.dataset.prCheckout ? `PR ${row.dataset.prCheckout}` : ref.remote ? ref.name.slice(ref.name.indexOf("/") + 1) : ref.name;
    try {
      if (row.dataset.prCheckout) await callOn(tab.machine, { cmd: "checkout_pr", tab: tab.view.id, pr: row.dataset.prCheckout });
      else if (base && !create) await callOn(tab.machine, { cmd: "checkout", tab: tab.view.id, branch: current, create: true, from: fromRemote && !ref.remote ? `origin/${ref.name}` : ref.name });
      else await callOn(tab.machine, { cmd: "checkout", tab: tab.view.id, branch: name, create });
      showComposerNote(base && !create && !row.dataset.prCheckout ? `Starting from ${fromRemote && !ref.remote ? `origin/${ref.name}` : ref.name}` : `Switched to ${name}`);
      setTimeout(() => showComposerNote(""), 2500);
    } catch (err) {
      showComposerNote(String(err), true);
    }
    Object.assign(threadOf(tab), { workspaceFor: null, gitFor: null });
    fetchWorkspace(tab);
    prompt.focus();
  };
}
document.getElementById("composer-footer").addEventListener("click", (e) => {
  const where = e.target.closest("button.where");
  if (where) return composerMenu.hidden || composerMenu.dataset.kind !== "workspace" ? openWorkspace(where) : (composerMenu.hidden = true);
  const branch = e.target.closest(".branch");
  if (branch) return composerMenu.hidden || composerMenu.dataset.kind !== "branches" ? openBranches(branch) : (composerMenu.hidden = true);
  const pr = e.target.closest("[data-pr]");
  if (pr && active) e.metaKey || e.ctrlKey ? openOutside(pr.dataset.pr) : openPr(active, pr.dataset.pr);
});
onGuiKey("g", () => document.querySelector("#composer-footer .branch")?.click(), { shift: true });

/** The header's Git action, as T3 Code's: a quick action picked from the branch's state, which runs
 * git and gh here, commits with a message a headless claude writes, and a menu of the rest. */
function quickAction(git, workspace, hasPr) {
  if (gitRun) return { label: "Commit", disabled: true, hint: "Git action in progress." };
  if (!workspace?.repo) return { label: "Initialize Git", action: "init", icon: "git-branch-plus" };
  if (!git) return { label: "Commit", disabled: true, hint: "Git status is unavailable." };
  const branch = git.refs.find((r) => r.current)?.name;
  const isDefault = branch === git.default;
  if (!branch || git.detached) return { label: "Commit", disabled: true, hint: "Create and checkout a ref before pushing or opening a PR." };
  const ahead = git.ahead > 0;
  const behind = git.behind > 0;
  if (git.dirty) {
    if (!git.upstream && !git.remote) return { label: "Commit", action: "commit" };
    if (hasPr || isDefault) return { label: "Commit & push", action: "commit_push" };
    return { label: "Commit, push & PR", action: "commit_push_pr" };
  }
  if (!git.upstream) {
    if (!git.remote) return { label: "Publish repository", action: "publish" };
    if (!ahead && !(git.ahead_of_default > 0)) return hasPr ? { label: "Commit", disabled: true, hint: "Branch is up to date. No action needed." } : { label: "Push", disabled: true, hint: "No local commits to push." };
    if (hasPr || isDefault) return { label: "Push", action: isDefault ? "commit_push" : "push" };
    return { label: "Push & create PR", action: "create_pr" };
  }
  if (ahead && behind) return { label: "Sync ref", disabled: true, hint: "Branch has diverged from upstream. Rebase/merge first." };
  if (behind) return { label: "Pull", action: "pull" };
  if (ahead) return hasPr || isDefault ? { label: "Push", action: isDefault ? "commit_push" : "push" } : { label: "Push & create PR", action: "create_pr" };
  if (hasPr) return { label: "Commit", disabled: true, hint: "Branch is up to date. No action needed." };
  if ((git.ahead_of_default ?? 0) > 0 && !isDefault) return { label: "Create PR", action: "create_pr" };
  return { label: "Commit", disabled: true, hint: "Branch is up to date. No action needed." };
}
const GIT_ICONS = { commit: "git-commit", commit_push: "cloud-upload", push: "cloud-upload", commit_push_pr: "github", create_pr: "github", pull: "cloud-download", publish: "cloud-upload", init: "git-branch-plus" };
/** The steps an action runs, each with the label T3 Code shows while it does. */
const GIT_STEPS = {
  commit: ["message", "commit"],
  commit_push: ["message", "commit", "push"],
  commit_push_pr: ["message", "commit", "push", "create_pr"],
  push: ["push"],
  create_pr: ["push", "create_pr"],
  pull: ["pull"],
  init: ["init"],
};
const STEP_LABELS = { message: "Generating commit message...", commit: "Committing...", push: "Pushing...", create_pr: "Creating pull request...", pull: "Pulling latest changes...", init: "Initializing...", publish: "Publishing repository..." };
let gitRun = null;
const gitAction = document.getElementById("git-action");
function renderGitAction(tab) {
  const t = tab && threadOf(tab);
  gitAction.hidden = !t?.workspace;
  if (gitAction.hidden) return;
  const key = JSON.stringify([tab.key, tab.view.cwd, t.workspace.branch, statusOf(tab)]);
  if (t.gitFor !== key && t.workspace.repo) {
    t.gitFor = key;
    callOn(tab.machine, { cmd: "branches", tab: tab.view.id }).then((git) => {
      t.git = git;
      if (tab === active) renderGitAction(tab);
    }, () => (t.git = null));
  }
  const main = gitAction.querySelector(".main");
  const more = gitAction.querySelector(".more");
  if (gitRun?.tab === tab) {
    main.disabled = true;
    main.dataset.action = "";
    main.title = gitRun.status;
    main.innerHTML = `${icon("loader-circle", "spin")}<span class="progress">${esc(gitRun.status)}</span><span class="elapsed">${Math.floor((Date.now() - gitRun.at) / 1000)}s</span>`;
    more.hidden = true;
    return renderDetails(tab);
  }
  const quick = quickAction(t.git, t.workspace, !!prOf(tab));
  // As T3 Code's: a disabled quick action says why in a popover under it.
  main.disabled = !!quick.disabled && !quick.hint;
  if (quick.disabled) main.setAttribute("aria-disabled", "true");
  else main.removeAttribute("aria-disabled");
  if (quick.disabled && quick.hint) Object.assign(main.dataset, { tip: quick.hint, tipSide: "bottom", tipDelay: "300" });
  else for (const k of ["tip", "tipSide", "tipDelay"]) delete main.dataset[k];
  main.dataset.action = quick.disabled ? "" : quick.action ?? "";
  main.removeAttribute("title");
  main.innerHTML = `${icon(GIT_ICONS[quick.action] ?? "git-commit")}<span class="label">${esc(quick.label)}</span>`;
  more.hidden = !t.workspace.repo;
  renderDetails(tab);
}
setInterval(() => gitRun && active === gitRun.tab && renderGitAction(active), 1000);
function gitMenuEntries(tab) {
  const git = threadOf(tab).git;
  if (!git) return [{ note: "Reading the branch…" }];
  const busy = !!gitRun;
  const branch = git.refs.find((r) => r.current)?.name;
  const hasPr = !!prOf(tab);
  const canPush = !busy && branch && !git.behind && git.ahead > 0 && (git.upstream || git.remote);
  const canPr = !busy && branch && !hasPr && (git.ahead_of_default ?? git.ahead) > 0 && !git.behind && (git.upstream || git.remote);
  const entries = [{ label: "Commit", icon: "git-commit", disabled: busy || !git.dirty, title: !git.dirty ? "No changes to commit." : "", run: () => openCommitDialog(tab) }];
  if (git.remote) {
    entries.push({ label: "Push", icon: "cloud-upload", disabled: !canPush, title: canPush ? "" : git.behind ? "Behind upstream. Pull/rebase first." : "No local commits to push.", run: () => runGitAction(tab, "push") });
    if (!hasPr) entries.push({ label: "Create PR", icon: "github", disabled: !canPr, title: canPr ? "" : "Push commits that the default branch lacks first.", run: () => runGitAction(tab, "create_pr") });
  } else entries.push({ label: "Publish repository...", icon: "cloud-upload", run: () => openPublishDialog(tab) });
  if (git.detached) entries.push({ note: "Detached HEAD: create and check out a branch to enable push and pull request actions." });
  else if (git.behind > 0) entries.push({ note: "Behind upstream. Pull/rebase first." });
  return entries;
}
gitAction.querySelector(".main").onclick = (e) => {
  const tab = active;
  const action = e.currentTarget.dataset.action;
  if (!tab || !action) return;
  if (action === "publish") return openPublishDialog(tab);
  runGitAction(tab, action);
};
gitAction.querySelector(".more").onclick = (e) => {
  const tab = active;
  if (!tab) return;
  if (!headerMenu.hidden && headerMenu.dataset.kind === "git") return (headerMenu.hidden = true);
  openHeaderMenu(e.currentTarget, gitMenuEntries(tab), "git");
};

/** Runs the steps of `action` in order, showing each as T3 Code does, and says how it went. */
async function runGitAction(tab, action, { message = "", files = [] } = {}) {
  if (gitRun) return;
  const git = threadOf(tab).git;
  const branch = git?.refs.find((r) => r.current)?.name;
  if (["push", "commit_push", "commit_push_pr", "create_pr"].includes(action) && branch && branch === git.default) {
    const answer = await confirmDefault(action, branch);
    if (!answer) return;
    if (answer === "branch") {
      try {
        await callOn(tab.machine, { cmd: "checkout", tab: tab.view.id, branch: `pinned/feature-${Date.now().toString(36)}`, create: true });
      } catch (e) {
        return showComposerNote(String(e), true);
      }
    }
  }
  gitRun = { tab, status: "", at: Date.now() };
  let url = null;
  try {
    for (const step of GIT_STEPS[action]) {
      if (step === "message" && message) continue;
      gitRun.status = STEP_LABELS[step];
      if (tab === active) renderGitAction(tab);
      const answer = await callOn(tab.machine, { cmd: "git", tab: tab.view.id, step: step === "message" ? { step, files } : step === "commit" ? { step, message, files } : { step } });
      if (step === "message") message = answer;
      if (step === "create_pr") url = answer;
    }
    showComposerNote(url ? `Opened ${url}` : { pull: "Pulled the latest changes.", init: "Initialized a git repository." }[action] ?? (action.includes("push") ? "Pushed." : "Committed."));
    setTimeout(() => showComposerNote(""), 10_000);
  } catch (e) {
    showComposerNote(String(e), true);
  } finally {
    gitRun = null;
    Object.assign(threadOf(tab), { gitFor: null, workspaceFor: null });
    fetchWorkspace(tab);
    if (tab === active) renderGitAction(tab);
  }
}
/** T3 Code asks before it pushes to the default branch, and offers a feature ref instead. */
function confirmDefault(action, branch) {
  const commits = action.startsWith("commit");
  const suffix = ` on "${branch}". You can continue on this ref or create a feature ref and run the same action there.`;
  const copy = action === "push" || action === "commit_push"
    ? commits ? ["Commit & push to default ref?", `This action will commit and push changes${suffix}`, `Commit & push to ${branch}`] : ["Push to default ref?", `This action will push local commits${suffix}`, `Push to ${branch}`]
    : commits ? ["Commit, push & create PR from default ref?", `This action will commit, push, and create a pull request${suffix}`, "Commit, push & create PR"] : ["Create PR from default ref?", `This action will create a pull request${suffix}`, "Create PR"];
  composerSheet.innerHTML = `<form class="sheet" role="alertdialog" aria-label="${esc(copy[0])}"><h2>${esc(copy[0])}</h2><p class="about">${esc(copy[1])}</p>
    <footer><span class="spacer"></span><button type="button" class="outline" data-answer="">Cancel</button><button type="button" class="outline" data-answer="branch">Create feature ref</button><button type="button" class="primary" data-answer="continue">${esc(copy[2])}</button></footer></form>`;
  composerSheet.hidden = false;
  return new Promise((resolve) => {
    composerSheet.querySelector("form").onclick = (e) => {
      const b = e.target.closest("[data-answer]");
      if (!b) return;
      closeSheet();
      resolve(b.dataset.answer || null);
    };
  });
}

/** T3 Code's commit dialog: the branch, the changed files to include, and an optional message. */
function openCommitDialog(tab) {
  const git = threadOf(tab).git;
  if (!git) return;
  const branch = git.refs.find((r) => r.current)?.name ?? "(detached)";
  composerSheet.innerHTML = `<form class="sheet" role="dialog" aria-label="Commit changes"><h2>Commit changes</h2><p class="about">Review and confirm your commit. Leave the message blank to auto-generate one.</p>
    <dl class="facts"><dt>Branch</dt><dd>${esc(branch)}${branch === git.default ? ` <span class="warn">Default branch</span>` : ""}</dd><dt>Files</dt><dd><ul class="files">${git.changed.map((f, i) => `<li><label><input type="checkbox" name="file" value="${i}" checked>${fileTypeIcon(f)}<span>${esc(f)}</span></label></li>`).join("") || "<li>none</li>"}</ul></dd></dl>
    <label class="field"><span>Commit message (optional)</span><textarea name="message" rows="3" placeholder="Leave empty to auto-generate"></textarea></label>
    <footer><span class="spacer"></span><button type="button" class="outline" data-cancel>Cancel</button><button type="button" class="outline" data-new-branch>Commit on new branch</button><button type="submit" class="primary">Commit</button></footer></form>`;
  composerSheet.hidden = false;
  const form = composerSheet.querySelector("form");
  const chosen = () => [...form.querySelectorAll("[name=file]:checked")].map((b) => git.changed[b.value]);
  const commit = async (onNewBranch) => {
    const files = chosen();
    if (!files.length) return;
    closeSheet();
    const message = form.elements.message.value.trim();
    if (onNewBranch) {
      const name = `pinned/${(message || "changes").toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 40) || "changes"}`;
      try {
        await callOn(tab.machine, { cmd: "checkout", tab: tab.view.id, branch: name, create: true });
      } catch (e) {
        return showComposerNote(String(e), true);
      }
    }
    runGitAction(tab, "commit", { message, files: files.length === git.changed.length ? [] : files });
  };
  form.onchange = () => {
    const none = !chosen().length;
    for (const b of form.querySelectorAll("[type=submit], [data-new-branch]")) b.disabled = none;
  };
  form.querySelector("[data-cancel]").onclick = closeSheet;
  form.querySelector("[data-new-branch]").onclick = () => commit(true);
  form.onsubmit = (e) => {
    e.preventDefault();
    commit(false);
  };
  form.elements.message.focus();
}
/** T3 Code's Publish repository dialog: a GitHub repository made with gh, pushed as origin. */
function openPublishDialog(tab) {
  const name = projectName(tab);
  composerSheet.innerHTML = `<form class="sheet" role="dialog" aria-label="Publish repository"><h2>Publish repository</h2><p class="about">Create a GitHub repository for this project and push it as origin.</p>
    <label class="field"><span>Repository</span><input name="name" value="${esc(name)}" autocomplete="off"></label>
    <label class="switch-row"><span>Public</span><input type="checkbox" role="switch" name="public"></label>
    <p class="error" hidden></p>
    <footer><span class="spacer"></span><button type="button" class="outline" data-cancel>Cancel</button><button type="submit" class="primary">Publish</button></footer></form>`;
  composerSheet.hidden = false;
  const form = composerSheet.querySelector("form");
  form.querySelector("[data-cancel]").onclick = closeSheet;
  form.onsubmit = async (e) => {
    e.preventDefault();
    closeSheet();
    if (gitRun) return;
    gitRun = { tab, status: STEP_LABELS.publish, at: Date.now() };
    renderGitAction(tab);
    try {
      await callOn(tab.machine, { cmd: "git", tab: tab.view.id, step: { step: "publish", name: form.elements.name.value.trim(), public: form.elements.public.checked } });
      showComposerNote("Published the repository.");
    } catch (err) {
      showComposerNote(String(err), true);
    } finally {
      gitRun = null;
      threadOf(tab).gitFor = null;
      renderGitAction(tab);
    }
  };
}

function toggleDrawer(open = !ui.drawer) {
  ui.drawer = open;
  document.body.dataset.drawer = open ? "open" : "closed";
  renderDrawer();
  if (open && active) refit(active);
  focusInput();
}
document.getElementById("toggle-terminal").onclick = () => toggleDrawer();
onGuiKey("j", () => toggleDrawer());

const DRAWER_MIN = 180;
const setDrawerHeight = (px) => {
  const room = document.getElementById("workspace").clientHeight;
  const height = Math.round(Math.max(DRAWER_MIN, room ? Math.min(px, room * 0.75) : px));
  document.documentElement.style.setProperty("--drawer-height", `${height}px`);
  return height;
};
setDrawerHeight(Number(load("gui-drawer-height")) || 280);
const drawerResize = Object.assign(document.createElement("span"), { id: "drawer-resize", title: "Drag to resize" });
document.getElementById("terms").prepend(drawerResize);
drawerResize.onpointerdown = (down) => {
  down.preventDefault();
  drawerResize.setPointerCapture(down.pointerId);
  const bottom = document.getElementById("workspace").getBoundingClientRect().bottom;
  document.body.classList.add("resizing-rows");
  drawerResize.onpointermove = (e) => setDrawerHeight(bottom - e.clientY);
  drawerResize.onpointerup = drawerResize.onpointercancel = (e) => {
    drawerResize.onpointermove = drawerResize.onpointerup = drawerResize.onpointercancel = null;
    document.body.classList.remove("resizing-rows");
    save("gui-drawer-height", String(setDrawerHeight(bottom - e.clientY)));
    if (active) refit(active);
  };
};
drawerResize.ondblclick = () => {
  save("gui-drawer-height", String(setDrawerHeight(280)));
  if (active) refit(active);
};

/** The header title opens the thread's menu under it, as T3 Code's does. The desktop app's native menu
 * takes the input while it is up, so there a click waits half a second for a double-click to rename. */
let titleClick;
crumbEl.addEventListener("click", (e) => {
  const title = e.target.closest("button.title");
  clearTimeout(titleClick);
  if (!active || !title || e.detail > 1) return;
  const tab = active;
  const open = () => {
    const at = title.getBoundingClientRect();
    if (tab === active && title.isConnected) showMenu(rowMenuItems(tab), { x: at.left, y: at.bottom + 4 });
  };
  if (!TAURI?.menu || e.detail === 0 || e.target.closest(".chev")) return open();
  titleClick = setTimeout(open, 500);
});
crumbEl.addEventListener("dblclick", () => clearTimeout(titleClick));

Object.assign(ICON_PATHS, {
  play: `<polygon points="6 3 20 12 6 21 6 3"/>`,
  "flask-conical": `<path d="M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2"/><path d="M6.453 15h11.094"/><path d="M8.5 2h7"/>`,
  "list-checks": `<path d="m3 17 2 2 4-4"/><path d="m3 7 2 2 4-4"/><path d="M13 6h8"/><path d="M13 12h8"/><path d="M13 18h8"/>`,
  bug: `<path d="m8 2 1.88 1.88"/><path d="M14.12 3.88 16 2"/><path d="M9 7.13v-1a3.003 3.003 0 1 1 6 0v1"/><path d="M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6"/><path d="M12 20v-9"/><path d="M6.53 9C4.6 8.8 3 7.1 3 5"/><path d="M6 13H2"/><path d="M3 21c0-2.1 1.7-3.9 3.8-4"/><path d="M20.97 5c0 2.1-1.6 3.8-3.5 4"/><path d="M22 13h-4"/><path d="M17.2 17c2.1.1 3.8 1.9 3.8 4"/>`,
  "square-menu": `<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 8h10"/><path d="M7 12h10"/><path d="M7 16h10"/>`,
  "git-branch-plus": `<path d="M6 3v12"/><path d="M18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6z"/><path d="M6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6z"/><path d="M15 6a9 9 0 0 0-9 9"/><path d="M18 15v6"/><path d="M21 18h-6"/>`,
  "triangle-alert": `<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>`,
});

/** T3 Code's project actions: scripts kept per project, run in a terminal of the drawer. */
const SCRIPT_ICONS = { play: ["Play", "play"], test: ["Test", "flask-conical"], lint: ["Lint", "list-checks"], configure: ["Configure", "wrench"], build: ["Build", "hammer"], debug: ["Debug", "bug"] };
const scriptIcon = (name) => icon(SCRIPT_ICONS[name]?.[1] ?? "play");
const scriptLabel = (s) => {
  const roles = [...(s.runOnWorktreeCreate ? ["setup"] : []), ...(s.runOnSettle ? ["on settle"] : [])];
  return roles.length ? `${s.name} (${roles.join(", ")})` : s.name;
};
const scriptsOf = new Map();
/** The scripts of the tab's project, `{ scripts, file }`, asked of its machine once and kept. */
function projectScripts(tab, fresh = false) {
  const key = `${tab.machine}:${threadOf(tab).workspace?.project ?? tab.view.cwd}`;
  if (fresh || !scriptsOf.has(key)) {
    scriptsOf.set(key, { scripts: [], file: [], loading: true });
    callOn(tab.machine, { cmd: "project_scripts", tab: tab.view.id }).then((got) => {
      scriptsOf.set(key, got);
      if (active === tab) renderScripts(tab);
    }, () => scriptsOf.set(key, { scripts: [], file: [] }));
  }
  return scriptsOf.get(key);
}
async function saveScripts(tab, scripts) {
  await callOn(tab.machine, { cmd: "save_project_scripts", tab: tab.view.id, scripts });
  scriptsOf.set(`${tab.machine}:${threadOf(tab).workspace?.project ?? tab.view.cwd}`, { ...projectScripts(tab), scripts });
  renderScripts(tab);
}
const primaryScript = (scripts) => scripts.find((s) => !s.runOnWorktreeCreate && !s.runOnSettle) ?? scripts.find((s) => !s.runOnSettle) ?? null;
const importable = ({ scripts, file }) => file.filter((f) => !scripts.some((s) => s.command === f.command || s.name.toLowerCase() === f.name.toLowerCase()));

const scriptsControl = document.getElementById("scripts-control");
function renderScripts(tab) {
  const known = tab && guiOn() && tab.view.running && threadOf(tab).workspace && projectScripts(tab);
  scriptsControl.hidden = !known || known.loading;
  if (scriptsControl.hidden) return;
  const primary = primaryScript(known.scripts);
  const html = primary
    ? `<button type="button" class="main outline" data-run="${esc(primary.id)}" data-tip="Run ${esc(primary.name)}" aria-label="Run ${esc(primary.name)}">${scriptIcon(primary.icon)}<span class="label">${esc(primary.name)}</span></button><button type="button" class="pick outline" aria-label="Script actions">${icon("chevron-down")}</button>`
    : known.scripts.length || importable(known).length
      ? `<button type="button" class="main outline solo" data-scripts-menu aria-label="Project actions">${icon("wrench")}<span class="label">Actions</span>${icon("chevron-down")}</button>`
      : `<button type="button" class="main outline solo" data-add-script aria-label="Add action" data-tip="Add action">${icon("plus")}<span class="label">Add action</span></button>`;
  if (scriptsControl.dataset.html !== html) {
    scriptsControl.dataset.html = html;
    scriptsControl.innerHTML = html;
  }
  renderDetails(tab);
}
/** The scripts menu, as T3 Code's: each script with its shortcut and a gear to edit it, the ones
 * t3.json offers to import, then "Add action". */
function scriptEntries(tab) {
  const known = projectScripts(tab);
  const rows = known.scripts.map((s) => ({ html: `${scriptIcon(s.icon)}<span class="label">${esc(scriptLabel(s))}</span>${s.keybinding ? `<kbd>${esc(shortcutLabel(s.keybinding))}</kbd>` : ""}<span role="button" class="gear" data-edit="${esc(s.id)}" aria-label="Edit ${esc(s.name)}" title="Edit ${esc(s.name)}">${icon("settings")}</span>`, run: () => runScript(tab, s) }));
  const imports = importable(known);
  if (imports.length) {
    if (rows.length) rows.push({ rule: true });
    rows.push({ heading: "From t3.json" }, ...imports.map((f) => ({ html: `${scriptIcon(f.icon)}<span class="label">${esc(f.name)}</span><span class="end">${icon("download")}</span>`, run: () => importScript(tab, f) })));
  }
  rows.push({ html: `${icon("plus")}<span class="label">Add action</span>`, run: () => openScriptEditor(tab) });
  return rows;
}
function openScriptsMenu(anchor) {
  const tab = active;
  if (!tab) return;
  const menu = document.getElementById("header-menu");
  if (!menu.hidden && menu.dataset.kind === "scripts") return (menu.hidden = true);
  openHeaderMenu(anchor, scriptEntries(tab), "scripts");
}
scriptsControl.addEventListener("click", (e) => {
  const tab = active;
  if (!tab) return;
  const run = e.target.closest("[data-run]");
  if (run) return runScript(tab, projectScripts(tab).scripts.find((s) => s.id === run.dataset.run));
  if (e.target.closest("[data-add-script]")) return openScriptEditor(tab);
  if (e.target.closest(".pick, [data-scripts-menu]")) openScriptsMenu(e.target.closest("button"));
});
async function importScript(tab, f) {
  const known = projectScripts(tab);
  await saveScripts(tab, [...known.scripts, { ...f, id: scriptId(f.name, known.scripts), icon: f.icon ?? "play" }]).catch((e) => showComposerNote(String(e), true));
}
function scriptId(name, scripts) {
  const base = name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 32) || "script";
  let id = base;
  for (let n = 2; scripts.some((s) => s.id === id); n++) id = `${base}-${n}`;
  return id;
}

/** The header's own menus, opened under their button. */
const headerMenu = document.getElementById("header-menu");
function openHeaderMenu(anchor, entries, kind) {
  headerMenu.dataset.kind = kind;
  headerMenu.replaceChildren(...entries.map((entry) => {
    if (entry.heading || entry.rule || entry.note) return Object.assign(document.createElement(entry.rule ? "hr" : "p"), { className: entry.heading ? "heading" : entry.note ? "note" : "rule", textContent: entry.heading ?? entry.note ?? "" });
    const b = document.createElement("button");
    b.type = "button";
    b.disabled = !!entry.disabled;
    // As T3 Code's: a disabled item says why in a popover at its left, or under it on a touch screen.
    if (entry.disabled && entry.title) {
      const why = Object.assign(document.createElement("span"), { className: "why" });
      Object.assign(why.dataset, { tip: entry.title, tipSide: "left", tipDelay: "300" });
      why.append(b);
      if (matchMedia("(pointer: coarse)").matches) why.append(Object.assign(document.createElement("p"), { className: "note", textContent: entry.title }));
      b.innerHTML = entry.html ?? `${entry.icon ? icon(entry.icon) : ""}<span class="label">${esc(entry.label)}</span>`;
      return why;
    }
    b.innerHTML = entry.html ?? `${entry.icon ? icon(entry.icon) : ""}<span class="label">${esc(entry.label)}</span>`;
    b.onclick = (e) => {
      const edit = e.target.closest("[data-edit]");
      headerMenu.hidden = true;
      if (edit) return openScriptEditor(active, projectScripts(active).scripts.find((s) => s.id === edit.dataset.edit));
      entry.run();
    };
    return b;
  }));
  headerMenu.hidden = false;
  const r = anchor.getBoundingClientRect();
  headerMenu.style.top = `${r.bottom + 4}px`;
  headerMenu.style.left = `${Math.max(8, Math.min(r.right - headerMenu.offsetWidth, innerWidth - headerMenu.offsetWidth - 8))}px`;
}
window.addEventListener("mousedown", (e) => !headerMenu.hidden && !e.target.closest("#header-menu, #scripts-control, #git-action, #details-card") && (headerMenu.hidden = true), true);
window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || headerMenu.hidden) return;
  headerMenu.hidden = true;
  e.stopPropagation();
}, true);

/** T3 Code's Add / Edit Action dialog, and the other dialogs of the header. */
const composerSheet = Object.assign(document.createElement("div"), { id: "composer-dialog", hidden: true });
document.body.append(composerSheet);
const closeSheet = () => (composerSheet.hidden = true);
composerSheet.addEventListener("mousedown", (e) => e.target === composerSheet && closeSheet());
window.addEventListener("keydown", (e) => e.key === "Escape" && !composerSheet.hidden && (closeSheet(), e.stopPropagation()), true);
function openScriptEditor(tab, script = null) {
  if (!tab) return;
  let chosenIcon = script?.icon ?? "play";
  let keybinding = script?.keybinding ?? "";
  const toggle = (name, label, on, off = false) => `<label class="switch-row${off ? " off" : ""}"><span>${label}</span><input type="checkbox" role="switch" name="${name}"${on ? " checked" : ""}${off ? " disabled" : ""}></label>`;
  composerSheet.innerHTML = `<form class="sheet" role="dialog" aria-label="${script ? "Edit Action" : "Add Action"}">
    <h2>${script ? "Edit Action" : "Add Action"}</h2><p class="about">Actions are project-scoped commands you can run from the top bar or keybindings.</p>
    <label class="field"><span>Name</span><span class="row"><button type="button" class="icon-pick outline" aria-label="Choose icon">${scriptIcon(chosenIcon)}</button><input name="name" placeholder="Test" autocomplete="off" value="${esc(script?.name ?? "")}"></span></label>
    <div class="icons" hidden>${Object.entries(SCRIPT_ICONS).map(([id, [label]]) => `<button type="button" data-icon="${id}"${id === chosenIcon ? ` class="on"` : ""}>${scriptIcon(id)}<span>${label}</span></button>`).join("")}</div>
    <label class="field"><span>Keybinding</span><input name="keybinding" placeholder="Press shortcut" readonly value="${esc(keybinding ? shortcutLabel(keybinding) : "")}"><small>Press a shortcut. Use <code>Backspace</code> to clear. Shortcuts are environment-wide. Projects using the same action share its shortcut.</small></label>
    <label class="field"><span>Command</span><textarea name="command" placeholder="bun test" rows="3">${esc(script?.command ?? "")}</textarea></label>
    <label class="field"><span>Preview URL (optional)</span><input name="previewUrl" placeholder="http://localhost:5173" autocomplete="off" value="${esc(script?.previewUrl ?? "")}"><small>Open this URL in the in-app preview when this action runs.</small></label>
    ${toggle("runOnWorktreeCreate", "Run automatically on worktree creation", script?.runOnWorktreeCreate)}
    ${toggle("wait", "Wait for it to finish before the agent starts", script?.runOnWorktreeCreate && script?.async === false, !script?.runOnWorktreeCreate)}
    ${toggle("runOnSettle", "Run in the thread's worktree when the thread settles", script?.runOnSettle)}
    ${toggle("autoOpenPreview", "Open preview automatically when this action runs", script?.autoOpenPreview, !script?.previewUrl)}
    <p class="error" hidden></p>
    <footer>${script ? `<button type="button" class="danger-outline" data-delete>Delete</button>` : ""}<span class="spacer"></span><button type="button" class="outline" data-cancel>Cancel</button><button type="submit" class="primary">${script ? "Save changes" : "Save action"}</button></footer></form>`;
  composerSheet.hidden = false;
  const form = composerSheet.querySelector("form");
  form.elements.name.focus();
  const sync = () => {
    form.elements.wait.disabled = !form.elements.runOnWorktreeCreate.checked;
    form.elements.wait.closest(".switch-row").classList.toggle("off", form.elements.wait.disabled);
    form.elements.autoOpenPreview.disabled = !form.elements.previewUrl.value.trim();
    form.elements.autoOpenPreview.closest(".switch-row").classList.toggle("off", form.elements.autoOpenPreview.disabled);
  };
  form.oninput = sync;
  form.querySelector(".icon-pick").onclick = () => (form.querySelector(".icons").hidden = !form.querySelector(".icons").hidden);
  form.querySelector(".icons").onclick = (e) => {
    const b = e.target.closest("[data-icon]");
    if (!b) return;
    chosenIcon = b.dataset.icon;
    form.querySelector(".icon-pick").innerHTML = scriptIcon(chosenIcon);
    for (const o of form.querySelectorAll("[data-icon]")) o.classList.toggle("on", o === b);
    form.querySelector(".icons").hidden = true;
  };
  form.elements.keybinding.onkeydown = (e) => {
    if (e.key === "Tab") return;
    e.preventDefault();
    if (e.key === "Backspace" || e.key === "Delete") keybinding = "";
    else if (!["Meta", "Control", "Shift", "Alt"].includes(e.key)) keybinding = [(IS_MAC ? e.metaKey : e.ctrlKey) && "mod", e.shiftKey && "shift", e.altKey && "alt", (e.altKey ? e.code.replace(/^(Key|Digit)/, "") : e.key).toLowerCase()].filter(Boolean).join("+");
    form.elements.keybinding.value = keybinding ? shortcutLabel(keybinding) : "";
  };
  form.querySelector("[data-cancel]").onclick = closeSheet;
  form.querySelector("[data-delete]")?.addEventListener("click", (e) => {
    const b = e.currentTarget;
    if (!b.dataset.armed) {
      b.dataset.armed = "true";
      b.textContent = `Delete action "${script.name}"?`;
      return;
    }
    closeSheet();
    saveScripts(tab, projectScripts(tab).scripts.filter((s) => s.id !== script.id)).catch((err) => showComposerNote(String(err), true));
  });
  form.onsubmit = async (e) => {
    e.preventDefault();
    const name = form.elements.name.value.trim();
    const command = form.elements.command.value.trim();
    const error = form.querySelector(".error");
    error.hidden = !!(name && command);
    if (!name || !command) return (error.textContent = !name ? "Name is required." : "Command is required.");
    const previewUrl = form.elements.previewUrl.value.trim();
    const scripts = projectScripts(tab).scripts;
    const next = {
      id: script?.id ?? scriptId(name, scripts),
      name,
      command,
      icon: chosenIcon,
      runOnWorktreeCreate: form.elements.runOnWorktreeCreate.checked,
      ...(form.elements.runOnWorktreeCreate.checked && form.elements.wait.checked ? { async: false } : {}),
      runOnSettle: form.elements.runOnSettle.checked,
      ...(previewUrl ? { previewUrl, autoOpenPreview: form.elements.autoOpenPreview.checked } : {}),
      ...(keybinding ? { keybinding } : {}),
    };
    // A script that takes over setup or settling releases it from the others, as in T3 Code.
    const others = scripts.filter((s) => s.id !== next.id).map((s) => ({ ...s, ...(next.runOnWorktreeCreate ? { runOnWorktreeCreate: false } : {}), ...(next.runOnSettle ? { runOnSettle: false } : {}) }));
    try {
      await saveScripts(tab, script ? scripts.map((s) => (s.id === script.id ? next : others.find((o) => o.id === s.id))) : [...others, next]);
      closeSheet();
    } catch (err) {
      error.hidden = false;
      error.textContent = String(err);
    }
  };
}
const shortcutLabel = (binding) => binding.split("+").map((k) => ({ mod: IS_MAC ? "⌘" : "Ctrl+", shift: IS_MAC ? "⇧" : "Shift+", alt: IS_MAC ? "⌥" : "Alt+" })[k] ?? k.toUpperCase()).join("");
// A script's own shortcut runs it in the active thread's project.
window.addEventListener("keydown", (e) => {
  const tab = active;
  if (!guiOn() || !tab || !(IS_MAC ? e.metaKey : e.ctrlKey) || settingsOpen() || !composerSheet.hidden) return;
  const pressed = ["mod", e.shiftKey && "shift", e.altKey && "alt", (e.altKey ? e.code.replace(/^(Key|Digit)/, "") : e.key).toLowerCase()].filter(Boolean).join("+");
  const script = projectScripts(tab).scripts?.find((s) => s.keybinding === pressed);
  if (!script) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  runScript(tab, script);
}, true);

/** The terminal drawer's tabs: claude's own session, and a shell for each script run, which T3
 * Code opens for a project action. */
const drawerTabs = Object.assign(document.createElement("nav"), { id: "drawer-tabs" });
document.getElementById("terms").prepend(drawerTabs);
const scriptShells = [];
async function runScript(tab, script, cwd = null, group = null) {
  if (!tab || !script) return;
  if (script.previewUrl && script.autoOpenPreview && tab === active) {
    setBrowserUrl(script.previewUrl);
    openPanel("browser");
  }
  toggleDrawer(true);
  const el = Object.assign(document.createElement("div"), { className: "term shell" });
  document.getElementById("terms").append(el);
  const term = new Terminal({ ...terminalLook(), cursorBlink: true, allowTransparency: true, theme: { background: "#00000000" } });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open(el);
  const shell = { tab, name: script.name, el, term, fit, id: null, done: null, group };
  group?.members.push(shell);
  shell.done = new Promise((resolve) => (shell.exited = resolve));
  scriptShells.push(shell);
  showDrawerView(tab, shell);
  fit.fit();
  try {
    shell.id = await callOn(tab.machine, { cmd: "open_shell", tab: tab.view.id, cols: term.cols, rows: term.rows });
    await machines.get(tab.machine).transport.attach(shell.id, { write: (bytes) => term.write(bytes), reset: () => term.reset() });
    term.onData((data) => callOn(tab.machine, { cmd: "write_input", tab: shell.id, data }));
    // A shell drops what is typed before its prompt is up.
    await until(() => term.buffer.active.getLine(term.buffer.active.cursorY + term.buffer.active.baseY)?.translateToString(true).trim(), 5000);
    await callOn(tab.machine, { cmd: "write_input", tab: shell.id, data: `${cwd ? `cd ${escapePath(cwd)} && ` : ""}${script.command}\r` });
  } catch (e) {
    term.write(`\r\n\x1b[31m[could not run ${script.name}: ${e}]\x1b[0m\r\n`);
  }
  return shell;
}
function closeShell(shell) {
  scriptShells.splice(scriptShells.indexOf(shell), 1);
  shell.el.remove();
  shell.term.dispose();
  shell.exited();
  if (shell.id) callOn(shell.tab.machine, { cmd: "close_shell", shell: shell.id }).catch(() => {});
  const t = threadOf(shell.tab);
  const group = shell.group?.members;
  group?.splice(group.indexOf(shell), 1);
  if (t.drawerShell === shell) t.drawerShell = group?.at(-1) ?? null;
  if (group?.length === 1) group[0] ? (group[0].group = null) : (t.claudeGroup = null);
  renderDrawer();
}
/** Shows claude's terminal or a script's shell in the drawer, and the tabs to switch them. */
function showDrawerView(tab, shell) {
  threadOf(tab).drawerShell = shell;
  renderDrawer();
}
function renderDrawer() {
  const tab = active;
  const own = scriptShells.filter((s) => s.tab === tab);
  const open = tab && document.body.dataset.drawer === "open";
  const shown = open ? threadOf(tab).drawerShell ?? null : null;
  const group = open ? (shown ? shown.group : threadOf(tab).claudeGroup) : null;
  const visible = group?.members ?? [shown];
  for (const s of scriptShells) s.el.hidden = !visible.includes(s);
  if (tab) layoutSplit([tab.el, ...own.map((s) => s.el)], group && group.members.map((s) => s?.el ?? tab.el), group?.vertical, own.length ? 38 : 6);
  if (tab && tab.el.hidden !== !visible.includes(null) && guiOn()) {
    tab.el.hidden = !visible.includes(null);
    if (!tab.el.hidden) refit(tab);
  } else if (tab && group && visible.includes(null)) refit(tab);
  for (const s of visible) {
    if (!s) continue;
    s.fit.fit();
    if (s.id) callOn(tab.machine, { cmd: "resize_pty", tab: s.id, cols: s.term.cols, rows: s.term.rows }).catch(() => {});
  }
  drawerTabs.hidden = !own.length;
  drawerTabs.innerHTML = own.length ? `<button type="button" data-shell=""${shown ? "" : " aria-pressed=\"true\""}>${icon("claude")}<span>claude</span></button>${own.map((s, i) => `<span class="shell-tab"><button type="button" data-shell="${i}"${s === shown ? " aria-pressed=\"true\"" : ""}>${icon("terminal")}<span>${esc(s.name)}</span></button><button type="button" class="close" data-close-shell="${i}" aria-label="Close ${esc(s.name)}" title="Close ${esc(s.name)}">${icon("x")}</button></span>`).join("")}` : "";
}
/** T3 Code's split terminals: a group's terminals share the drawer side by side, or stacked when `vertical`. */
function layoutSplit(all, members, vertical, top) {
  for (const el of all) {
    for (const side of ["top", "bottom", "left", "right"]) el.style[side] = "";
    el.classList.remove("split-next", "vertical");
  }
  if (!members || members.length < 2) return;
  const n = members.length;
  members.forEach((el, i) => {
    const [start, end, a, b] = vertical ? ["top", "bottom", top, 4] : ["left", "right", 8, 4];
    el.style[start] = `calc(${a}px + (100% - ${a + b}px) * ${i} / ${n})`;
    el.style[end] = `calc(${b}px + (100% - ${a + b}px) * ${n - 1 - i} / ${n})`;
    el.classList.toggle("split-next", i > 0);
    el.classList.toggle("vertical", !!vertical);
  });
}
drawerTabs.addEventListener("click", (e) => {
  const tab = active;
  const own = scriptShells.filter((s) => s.tab === tab);
  const close = e.target.closest("[data-close-shell]");
  if (close) return closeShell(own[close.dataset.closeShell]);
  const pick = e.target.closest("[data-shell]");
  if (pick) showDrawerView(tab, pick.dataset.shell === "" ? null : own[pick.dataset.shell]);
});

/** T3 Code runs a project's setup script in a worktree as it is made for a thread; one that must
 * finish first holds Send, as T3 holds the agent. */
async function runSetupScripts(tab) {
  const setup = projectScripts(tab).scripts.find((s) => s.runOnWorktreeCreate);
  if (!setup) return;
  // The shell echoes what it is given, so the mark it prints when done is spelled differently there.
  const shell = await runScript(tab, { ...setup, command: setup.async === false ? `${setup.command}; printf 'pinned%sdone\\n' -setup-` : setup.command });
  if (setup.async !== false || !shell) return;
  const t = threadOf(tab);
  t.preparing = true;
  renderComposer(tab);
  await new Promise((resolve) => {
    const seen = shell.term.onWriteParsed(() => {
      const buffer = shell.term.buffer.active;
      for (let y = 0; y < buffer.length; y++) if (buffer.getLine(y)?.translateToString(true).trim() === "pinned-setup-done") return (seen.dispose(), resolve());
    });
    shell.done.then(resolve);
  });
  t.preparing = false;
  if (tab === active) renderComposer(tab);
}
// T3 Code runs a project's settle script in a thread's worktree when the thread settles.
const settledSeen = new Set();
let settleWatching = false;
setInterval(() => {
  if (!guiOn()) return;
  for (const tab of tabs) {
    const key = threadKey(tab);
    const now = !!sideMarks.settled[key];
    if (settleWatching && now && !settledSeen.has(key) && threadOf(tab).workspace?.worktree) {
      const script = projectScripts(tab).scripts?.find((s) => s.runOnSettle);
      if (script) runScript(tab, script, tab.view.cwd);
    }
    if (now) settledSeen.add(key);
    else settledSeen.delete(key);
  }
  settleWatching = true;
}, 1000);

/** The header's Open button shows the chosen editor's own icon, and its menu each editor's, as
 * T3 Code's picker does. */
// gui-panel.js, which owns the Open button and its menu, runs after this file.
addEventListener("DOMContentLoaded", () => {
const openInMain = openIn.querySelector(".main");
new MutationObserver(() => {
  const id = openInMain.dataset.editor;
  if (!id || openInMain.dataset.icon === id) return;
  openInMain.dataset.icon = id;
  openInMain.querySelector("svg")?.remove();
  openInMain.insertAdjacentHTML("afterbegin", editorIcon(id) || icon("square-arrow-out-up-right"));
}).observe(openInMain, { attributes: true, attributeFilter: ["data-editor"] });
new MutationObserver(() => {
  if (panelMenu.hidden || menuAnchor !== openIn.querySelector(".pick") || panelMenu.dataset.editors === "true") return;
  panelMenu.dataset.editors = "true";
  const list = active ? editorsOn(active.machine) : [];
  for (const b of panelMenu.querySelectorAll("button[data-i]")) {
    const editor = list.find((e) => e.name === b.querySelector(".label")?.textContent);
    if (editor) b.querySelector(":scope > svg, :scope > .i")?.replaceWith(Object.assign(document.createElement("span"), { className: "editor-mark", innerHTML: editorIcon(editor.id) }));
  }
}).observe(panelMenu, { childList: true, attributes: true, attributeFilter: ["hidden"] });
new MutationObserver(() => panelMenu.hidden && delete panelMenu.dataset.editors).observe(panelMenu, { attributes: true, attributeFilter: ["hidden"] });
});

/** T3 Code's thread details (⌘⇧B): the thread's workspace, editor and project actions, then its
 * branch, Git action and pull requests, in a card under the header. */
const detailsButton = document.getElementById("toggle-details");
detailsButton.innerHTML = icon("square-menu");
const detailsCard = Object.assign(document.createElement("aside"), { id: "details-card", ariaLabel: "Thread details", hidden: true });
document.getElementById("chat").append(detailsCard);
function toggleDetails(open = detailsCard.hidden) {
  detailsCard.hidden = !open || !active;
  detailsButton.setAttribute("aria-pressed", !detailsCard.hidden);
  if (!detailsCard.hidden) renderDetails(active);
}
detailsButton.onclick = () => toggleDetails();
onGuiKey("b", () => toggleDetails(), { shift: true });
window.addEventListener("mousedown", (e) => !detailsCard.hidden && !e.target.closest("#details-card, #toggle-details, #head-more, #header-menu, #composer-menu, #panel-menu, #composer-dialog") && toggleDetails(false), true);
window.addEventListener("keydown", (e) => e.key === "Escape" && !detailsCard.hidden && headerMenu.hidden && composerSheet.hidden && (toggleDetails(false), e.stopPropagation()), true);
function renderDetails(tab) {
  if (detailsCard.hidden || !tab) return;
  const t = threadOf(tab);
  const ws = t.workspace;
  const open = workspaceOpen(tab);
  const where = workspaceLabel(tab, open);
  const editor = editorFor(tab.machine);
  const known = ws && projectScripts(tab);
  const primary = known && primaryScript(known.scripts);
  const quick = quickAction(t.git, ws, !!prOf(tab));
  const prs = [...new Map(t.items.filter((i) => i.kind === "pr").map((p) => [p.url, p])).values()];
  const row = (attrs, inner, chevron = false) => `<button type="button" class="row${/ class="more"/.test(attrs) ? " more" : ""}"${attrs.replace(' class="more"', "")}>${inner}${chevron ? icon("chevron-down", "chev") : ""}</button>`;
  const split = (main, more) => `<div class="split-row">${main}<span class="seam"></span>${more}</div>`;
  const html = `<section aria-label="Workspace">
      ${row(` data-act="workspace"${open ? "" : " disabled"} title="${esc(tab.view.cwd)}"`, `${icon(tab.machine !== LOCAL ? "monitor" : where.icon)}<span class="label">${esc(tab.machine !== LOCAL ? `${machineName(tab.machine)} · ${where.label}` : where.label)}</span>`, open)}
      ${editor ? split(row(` data-act="open-in"`, `<span class="editor-mark">${editorIcon(editor.id) || icon("square-arrow-out-up-right")}</span><span class="label">Open in ${esc(editor.name)}</span>`), row(` data-act="editors" class="more" aria-label="Choose the editor"`, icon("chevron-down"))) : ""}
      ${known && !known.loading ? primary ? split(row(` data-act="run" title="Run ${esc(primary.name)}"`, `${scriptIcon(primary.icon)}<span class="label">${esc(primary.name)}</span>`), row(` data-act="scripts" class="more" aria-label="Script actions"`, icon("chevron-down")))
        : known.scripts.length || importable(known).length ? row(` data-act="scripts" aria-label="Project actions"`, `${icon("wrench")}<span class="label">Actions</span>`, true)
        : row(` data-act="add-script"`, `${icon("plus")}<span class="label">Add project script</span>`) : ""}
    </section>
    ${ws?.repo || ws ? `<section aria-label="Version Control">
      ${ws.branch ? row(` data-act="branch"`, `${icon("git-branch")}<span class="label branch">${middle(ws.branch, 28)}</span>`, true) : ""}
      ${split(row(` data-act="git"${quick.disabled || gitRun ? " disabled" : ""} title="${esc(quick.hint ?? quick.label)}"`, `${icon(gitRun ? "loader-circle" : GIT_ICONS[quick.action] ?? "git-commit", gitRun ? "spin" : "")}<span class="label">${esc(gitRun?.tab === tab ? gitRun.status : quick.label)}</span>`), ws.repo ? row(` data-act="git-menu" class="more" aria-label="Git actions"`, icon("chevron-down")) : "")}
      ${prs.map((p) => row(` data-act="pr" data-url="${esc(p.url)}"`, `${icon("git-pull-request")}<span class="label">#${p.number}</span><span class="dim">${esc(p.repo)}</span>`)).join("")}
    </section>` : ""}`;
  // Each editor icon numbers its own ids, so the card compares what it shows without them.
  const key = html.replace(/ed\d+-/g, "");
  if (detailsCard.dataset.html !== key) {
    detailsCard.dataset.html = key;
    detailsCard.innerHTML = html;
  }
}
detailsCard.addEventListener("click", (e) => {
  const tab = active;
  const b = e.target.closest("[data-act]");
  if (!tab || !b || b.disabled) return;
  const act = b.dataset.act;
  if (act === "workspace") openWorkspace(document.querySelector("#composer-footer button.where") ?? b);
  else if (act === "open-in") openInEditor();
  else if (act === "editors") openHeaderMenu(b, editorsOn(tab.machine).map((ed) => ({ html: `<span class="editor-mark">${editorIcon(ed.id)}</span><span class="label">${esc(ed.name)}</span>${ed.id === editorFor(tab.machine)?.id ? icon("check", "end") : ""}`, run: () => (save(`gui-editor:${tab.machine}`, ed.id), renderOpenIn(tab), renderDetails(tab)) })), "editors");
  else if (act === "run") runScript(tab, primaryScript(projectScripts(tab).scripts));
  else if (act === "scripts") openHeaderMenu(b, scriptEntries(tab), "scripts");
  else if (act === "add-script") openScriptEditor(tab);
  else if (act === "branch") document.querySelector("#composer-footer .branch")?.click();
  else if (act === "git") gitAction.querySelector(".main").click();
  else if (act === "git-menu") openHeaderMenu(b, gitMenuEntries(tab), "git");
  else if (act === "pr") openPr(tab, b.dataset.url);
});
