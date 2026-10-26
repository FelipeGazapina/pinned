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
});

const MODELS_MENU = [
  ["default", "Default", "The model your Claude Code settings pick"],
  ["opus", "Opus", "Most capable for complex work"],
  ["sonnet", "Sonnet", "Fast and capable for everyday tasks"],
  ["haiku", "Haiku", "Fastest for quick answers"],
];

/** Claude's permission modes, in the order Shift+Tab cycles them, named as T3 Code names its own. */
const MODES = {
  manual: { label: "Supervised", icon: "lock", hint: "Ask before commands and file changes" },
  accept: { label: "Auto-accept edits", icon: "pen-line", hint: "Auto-approve edits, ask before other actions" },
  plan: { label: "Plan", icon: "pencil-ruler", hint: "Plan first, change nothing until you approve" },
  auto: { label: "Auto", icon: "sparkles", hint: "Approve routine actions, ask before risky ones" },
  bypass: { label: "Full access", icon: "lock-open", hint: "Allow commands and edits without prompts" },
};

const GIT_ACTIONS = {
  commit: { label: "Commit", icon: "git-commit", prompt: "Commit the current changes with a clear commit message." },
  commit_push: { label: "Commit & push", icon: "cloud-upload", prompt: "Commit the current changes with a clear commit message, then push the branch." },
  commit_push_pr: { label: "Commit, push & PR", icon: "github", prompt: "Commit the current changes with a clear commit message, push the branch, and open a pull request with gh pr create." },
  push: { label: "Push", icon: "cloud-upload", prompt: "Push the current branch." },
  create_pr: { label: "Create PR", icon: "github", prompt: "Push the current branch if it isn't, then open a pull request with gh pr create." },
  push_pr: { label: "Push & create PR", icon: "github", prompt: "Push the current branch, then open a pull request with gh pr create." },
  pull: { label: "Pull", icon: "cloud-download", prompt: "Pull the latest changes into the current branch." },
};

let drafts = {};
try {
  drafts = sessionsOnly(JSON.parse(load("gui-drafts")));
} catch {}
let stash = [];
try {
  stash = JSON.parse(load("gui-stash")) ?? [];
} catch {}
const STASH_MAX = 20;
let sendWithMod = load("gui-send-mod") === "true";

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
  const menu = tab.menu;
  if (menu && menuKind(menu) === "approval") {
    if (!declineOption(menu)) return "Choose an option above";
    return menu.amend ? "Send feedback to decline this request" : "Send a message to decline and redirect the agent";
  }
  if (menu && menuKind(menu) === "plan") return "Add feedback to refine the plan, or leave this blank to implement it";
  if (menu) return freeOption(menu) ? "Type your own answer, or leave this blank to use the selected option" : "Choose an option above";
  if (!tab.view.running) return "claude isn't running. Press Enter to start it.";
  return t.items.length ? "Ask for changes, send follow-ups, or attach images" : "Ask anything, or / for commands";
}

function renderComposer(tab) {
  if (composer.dataset.tab !== (tab?.key ?? "")) switchDraft(tab);
  if (tab && tab.mode === undefined) tab.mode = terminalMode(tab);
  const t = tab && threadOf(tab);
  const working = !!tab && phaseOf(tab) === "working";
  const empty = !prompt.value.trim() && !t?.attachments?.length;
  composer.dataset.working = working;
  composer.dataset.empty = empty;
  document.getElementById("chat").dataset.hero = !!tab && t.loaded && !t.items.length && !tab.menu;
  const send = document.getElementById("send");
  send.innerHTML = icon(composer.dataset.sending === "true" ? "loader-circle" : working && !empty ? "list-plus" : "arrow-up");
  const keys = sendWithMod ? "⌘Enter" : "Enter";
  send.title = composer.dataset.sending === "true" ? "Submitting message" : working ? `Queue message (${keys})` : `Send (${keys})`;
  renderApproval(tab);
  rest();
  renderAttachments(t);
  renderStash();
  renderModeButton(tab);
  renderGitAction(tab);
  document.querySelector("#model-button .name").textContent = tab ? modelName(tab) : "Default";
  const footer = document.getElementById("composer-footer");
  if (!tab) return (footer.innerHTML = "");
  const pr = prOf(tab);
  const ws = t.workspace;
  footer.innerHTML = `<span class="where" title="${esc(tab.view.cwd)}">${icon(ws?.worktree ? "folder-git" : "folder")}${ws?.worktree ? "Worktree" : "Local checkout"}</span>
    ${tab.machine !== LOCAL ? `<span class="machine">${icon("monitor")}${esc(machineName(tab.machine))}</span>` : ""}
    <span class="spacer"></span>
    ${pr ? `<button type="button" class="pr" data-pr="${esc(pr.url)}" title="Open pull request #${pr.number}">${icon("git-pull-request")}#${pr.number}</button>` : ""}
    ${ws?.branch ? `<button type="button" class="branch" title="${esc(ws.branch)} · ⌘⇧G">${icon("git-branch")}<span>${esc(ws.branch)}</span>${icon("chevron-down", "chev")}</button>` : ""}`;
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
      const menu = terminalMenu(tab);
      // claude hides its mode line while a menu is up.
      const mode = terminalMode(tab) ?? (menu ? tab.mode ?? null : null);
      if (JSON.stringify([menu, mode]) === JSON.stringify([tab.menu ?? null, tab.mode ?? null])) return;
      tab.menu = menu;
      tab.mode = mode;
      if (tab === active) renderComposer(tab);
      renderSidebar();
    });
  });
}

const ARROW_DOWN = "\x1b[B";
const ARROW_UP = "\x1b[A";
const SHIFT_TAB = "\x1b[Z";
const writeTo = (tab, data) => callOn(tab.machine, { cmd: "write_input", tab: tab.view.id, data });
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

/** claude heads a permission menu with what it wants to run, such as "Bash command", then a
 * description and its own tips, the command in a dashed box, and the question last, such as "Do you
 * want to proceed?". A question's options each have a description under them. */
function renderApproval(tab) {
  const menu = !tab?.answering && tab?.menu;
  approval.hidden = !menu;
  if (!menu) return (approval.dataset.key = "");
  const key = JSON.stringify([tab.key, menu]);
  if (approval.dataset.key === key) return;
  approval.dataset.key = key;
  const lines = menu.question.split("\n");
  const permission = menuKind(menu) === "approval";
  approval.dataset.kind = permission ? "approval" : "input";
  const kind = menuKind(menu) === "plan" ? "Plan ready" : !permission ? "Claude is asking" : /command/i.test(lines[0]) ? "Command approval" : /file|edit/i.test(lines[0]) ? "File change approval" : "Permission approval";
  const what = permission ? lines.at(-1) : menu.question;
  const middle = permission ? lines.slice(1, -1).filter((l) => !/^Tip:/.test(l)) : [];
  const detail = menu.box || middle.join("\n");
  const about = menu.box ? middle.join("\n") : "";
  const options = menu.options.map((o) =>
    permission
      ? `<button type="button" data-option="${o.n}" class="${o.chosen ? "primary" : "outline"}"${o.chosen ? " data-chosen" : ""}>${esc(o.label)}</button>`
      : `<button type="button" data-option="${o.n}" class="choice"${o.chosen ? " data-chosen" : ""}${o.n <= 9 ? ` data-key="${o.n}"` : ""}><span class="label">${esc(o.head)}</span>${o.more ? `<span class="about">${esc(o.more)}</span>` : ""}</button>`,
  );
  approval.innerHTML = `${icon(permission ? "shield" : "circle-alert", "lead")}<div class="body"><p class="kind">${esc(kind)}</p>
      <p class="what">${esc(what)}</p>${about ? `<p class="about">${esc(about)}</p>` : ""}${detail ? `<pre>${esc(detail)}</pre>` : ""}
      <div class="options${permission ? "" : " choices"}">${options.join("")}</div></div>
    <div class="actions">${permission ? "" : `<button type="button" data-dismiss class="icon" title="Dismiss question without answering">${icon("x")}</button>`}<button type="button" data-show-terminal class="outline">Terminal</button></div>`;
}

approval.addEventListener("click", (e) => {
  const option = e.target.closest("[data-option]");
  if (option && active) return chooseOption(active, Number(option.dataset.option), active.menu?.options[option.dataset.option - 1]?.label);
  if (e.target.closest("[data-dismiss]") && active?.menu) {
    active.menu = null;
    renderComposer(active);
    return writeTo(active, "\x1b");
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
  chooseOption(active, option.n, option.label);
}, true);

/** With the thread scrolled up and a one-line prompt not being typed in, the composer rests as one
 * row, as T3 Code's does, to show more of the thread. Like T3's, it comes to rest on a scroll, not
 * on a blur: pressing Send takes the focus, and a blur would move Send out from under the press. */
function rest() {
  const away = threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight > 40;
  const t = active && threadOf(active);
  const resting = !!t && away && document.activeElement !== prompt && !prompt.value.includes("\n") && approval.hidden && composerMenu.hidden && !t.attachments?.length;
  if (composer.dataset.resting !== String(resting)) composer.dataset.resting = resting;
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
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing && (!sendWithMod || (IS_MAC ? e.metaKey : e.ctrlKey))) {
    e.preventDefault();
    send();
  } else if (e.key === "Escape" && active && phaseOf(active) === "working" && !prompt.value) {
    e.preventDefault();
    stop();
  } else if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !e.shiftKey && !e.altKey && !e.metaKey && !e.ctrlKey && recall(e.key === "ArrowUp" ? -1 : 1)) {
    e.preventDefault();
  } else if (e.key === "Tab" && e.shiftKey && active?.view.running && !active.menu) {
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
  const text = [prompt.value.trim(), ...files.map((f) => escapePath(f.path))].filter(Boolean).join(" ");
  if (tab.menu) {
    const menu = tab.menu;
    const chosen = menu.options.find((o) => o.chosen);
    if (!text && freeOption(menu) && chosen && chosen !== freeOption(menu)) return chooseOption(tab, chosen.n, chosen.label);
    if (!text || files.length || !(await answerWith(tab, text))) return waitsOnMenu(tab);
    return clearComposer(tab);
  }
  if (!text) return;
  prompts++;
  const draft = prompt.value;
  composer.dataset.sending = "true";
  clearComposer(tab, false);
  try {
    await callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text });
    for (const f of files) if (f.url) URL.revokeObjectURL(f.url);
  } catch (e) {
    Object.assign(t, { attachments: files });
    saveDraft(tab, draft);
    if (active === tab) prompt.value = draft;
    showComposerNote(String(e), true);
  } finally {
    delete composer.dataset.sending;
    autosize();
  }
  setTimeout(() => fetchThread(tab), 300);
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
    name: files[i]?.name ?? path.split("/").pop(),
    url: files[i]?.type.startsWith("image/") ? URL.createObjectURL(files[i]) : null,
  }))];
  renderComposer(active);
  renderSidebar();
  prompt.focus();
}
document.getElementById("composer-attach").onclick = () => document.getElementById("attach").click();

const attachmentsRow = document.getElementById("attachments");
function renderAttachments(t) {
  const files = t?.attachments ?? [];
  attachmentsRow.hidden = !files.length;
  const key = JSON.stringify(files.map((f) => f.path));
  if (attachmentsRow.dataset.key === key) return;
  attachmentsRow.dataset.key = key;
  attachmentsRow.innerHTML = files.map((f, i) => `<span class="attachment" title="${esc(f.path)}">${f.url ? `<img src="${esc(f.url)}" alt="">` : icon("file")}<span class="name">${esc(f.name)}</span>
    <button type="button" class="remove" data-remove="${i}" title="Remove ${esc(f.name)}">${icon("x")}</button></span>`).join("");
}
attachmentsRow.addEventListener("click", (e) => {
  const remove = e.target.closest("[data-remove]");
  if (!remove || !active) return;
  const t = threadOf(active);
  const [gone] = t.attachments.splice(Number(remove.dataset.remove), 1);
  if (gone?.url) URL.revokeObjectURL(gone.url);
  renderComposer(active);
  prompt.focus();
});

const composerMenu = document.getElementById("composer-menu");
/** Opens a menu of `entries`, each `{ label, hint, run, cls, icon, checked, remove }`, above `anchor`. */
function openMenu(anchor, entries, kind = "") {
  composerMenu.dataset.kind = kind;
  composerMenu.replaceChildren(...entries.map((entry) => menuRow(entry)));
  composerMenu.hidden = false;
  placeMenu(anchor);
}
function menuRow({ label, hint, run, cls, icon: name, checked, remove, disabled }, menu = composerMenu) {
  const b = document.createElement("button");
  b.type = "button";
  b.className = `${cls ?? ""}${checked ? " checked" : ""}`;
  b.disabled = !!disabled;
  b.innerHTML = `${name ? icon(name, "lead") : ""}<span class="text"><span class="label">${esc(label)}</span>${hint ? `<span class="hint">${esc(hint)}</span>` : ""}</span>
    ${checked ? icon("check", "check") : ""}${remove ? `<span class="remove" title="Delete">${icon("x")}</span>` : ""}`;
  b.onclick = (e) => {
    menu.hidden = true;
    if (remove && e.target.closest(".remove")) return remove();
    run();
  };
  return b;
}
function placeMenu(anchor) {
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
  const keeps = isTrigger() ? "#composer-menu, #prompt" : "#composer-menu, #model-button, #more-button, #mode-button, #stash, #composer-footer .branch";
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
  callOn(tab.machine, { cmd: "prompt", tab: tab.view.id, text }).then(() => setTimeout(() => fetchThread(tab), 400), (e) => showComposerNote(String(e), true));
  return true;
}

function modelName(tab) {
  const t = threadOf(tab);
  return t.picked ?? [...t.items].reverse().find((i) => i.kind === "model")?.name ?? "Default";
}
document.getElementById("model-button").onclick = (e) => {
  const tab = active;
  if (!tab) return;
  const family = modelName(tab).split(" ")[0].toLowerCase();
  openMenu(e.currentTarget, MODELS_MENU.map(([id, label, hint]) => ({
    label,
    hint,
    checked: family === id || (id === "default" && !MODELS_MENU.some(([other]) => other === family)),
    run: () => {
      if (!sendSlash(tab, `/model ${id}`)) return;
      threadOf(tab).picked = label;
      renderComposer(tab);
    },
  })));
};
onGuiKey("m", () => document.getElementById("model-button").click(), { shift: true });

const modeButton = document.getElementById("mode-button");
function renderModeButton(tab) {
  const mode = tab?.view.running && MODES[tab.mode];
  modeButton.hidden = !mode;
  if (!mode || modeButton.dataset.mode === tab.mode) return;
  modeButton.dataset.mode = tab.mode;
  modeButton.innerHTML = `${icon(mode.icon)}<span class="name">${esc(mode.label)}</span>${icon("chevron-down")}`;
}
modeButton.onclick = (e) => {
  const tab = active;
  if (!tab) return;
  const modes = Object.keys(MODES).filter((m) => m !== "bypass" || tab.view.bypass || tab.mode === "bypass");
  openMenu(e.currentTarget, modes.map((m) => ({ ...MODES[m], checked: tab.mode === m, run: () => pickMode(tab, m) })));
};
onGuiKey("a", () => !modeButton.hidden && modeButton.click(), { shift: true });

/** Presses Shift+Tab, which cycles claude's mode, until the screen shows `want`, or the cycle comes
 * round without it. */
async function pickMode(tab, want) {
  if (waitsOnMenu(tab)) return;
  const seen = new Set();
  while (terminalMode(tab) !== want && !seen.has(terminalMode(tab)) && !tab.menu) {
    const was = terminalMode(tab);
    seen.add(was);
    await writeTo(tab, SHIFT_TAB);
    if (!(await until(() => terminalMode(tab) !== was))) break;
  }
  tab.mode = terminalMode(tab) ?? tab.mode;
  if (tab === active) renderComposer(tab);
}

document.getElementById("more-button").onclick = (e) => {
  const tab = active;
  if (!tab) return;
  openMenu(e.currentTarget, [
    tab.view.bypass
      ? { label: "Ask for permissions again", hint: "Restarts claude and resumes this conversation", run: () => setBypass(tab, false) }
      : { label: "Full access", hint: "Restarts claude with no permission checks and resumes", run: () => setBypass(tab, true), cls: "danger" },
    { label: "Compact the conversation", hint: "/compact", run: () => sendSlash(tab, "/compact") },
    {
      label: "Send with ⌘Enter",
      hint: "Enter adds a new line",
      checked: sendWithMod,
      run: () => {
        sendWithMod = !sendWithMod;
        save("gui-send-mod", String(sendWithMod));
        renderComposer(tab);
      },
    },
    { label: "Change folder…", run: () => document.getElementById("folder").click() },
    { label: "Show the terminal", hint: "⌘J", run: () => toggleDrawer(true) },
    { label: "Close thread", hint: "⌘W", run: () => closeTab(tab) },
  ]);
};

const listsOf = new Map();
/** The tab's slash commands or files, kept until `fresh` asks the machine again. */
function fetchList(tab, cmd, fresh = false) {
  const key = `${cmd} ${tab.key} ${tab.view.cwd}`;
  if (fresh || !listsOf.has(key)) listsOf.set(key, callOn(tab.machine, { cmd, tab: tab.view.id }).catch(() => (listsOf.delete(key), [])));
  return listsOf.get(key);
}

const isTrigger = () => ["slash", "mention"].includes(composerMenu.dataset.kind) && !composerMenu.hidden;

/** `/` at the start of the prompt lists claude's commands and skills, and `@` before the caret the
 * folder's files and folders, each filtered as more is typed, as T3 Code's trigger menus do. */
async function showTrigger() {
  const tab = active;
  const before = prompt.value.slice(0, prompt.selectionStart);
  const slash = prompt.selectionStart === prompt.value.length ? prompt.value.match(/^\/(\S*)$/)?.[1] : undefined;
  const mention = slash === undefined ? before.match(/(?:^|\s)@([^\s@]*)$/)?.[1] : undefined;
  if (!tab || tab.menu || (slash === undefined && mention === undefined)) {
    if (isTrigger()) composerMenu.hidden = true;
    return;
  }
  const value = prompt.value;
  const query = (slash ?? mention).toLowerCase();
  const list = await fetchList(tab, slash !== undefined ? "commands" : "files", query === "");
  if (active !== tab || prompt.value !== value) return;
  const score = slash !== undefined ? slashScore : fileScore;
  const ranked = list
    .map((item) => ({ item, score: score(item, query) }))
    .filter((r) => r.score !== null)
    .sort((a, b) => a.score - b.score)
    .slice(0, 60)
    .map((r) => r.item);
  composerMenu.dataset.kind = slash !== undefined ? "slash" : "mention";
  composerMenu.replaceChildren(...(ranked.length ? ranked.map((item, i) => {
    const b = document.createElement("button");
    b.type = "button";
    b.classList.toggle("highlight", i === 0);
    if (slash !== undefined) {
      b.dataset.pick = `/${item.name}`;
      b.innerHTML = `<span class="name">/${esc(item.name)}</span><span class="about">${esc(item.description)}</span>${item.source !== "builtin" ? `<span class="source">${esc(item.skill ? "skill" : item.source)}</span>` : ""}`;
    } else {
      b.dataset.pick = `@${item}`;
      const [dir, base] = item.replace(/\/$/, "").match(/^(.*\/)?([^/]*)$/).slice(1);
      b.innerHTML = `${icon(item.endsWith("/") ? "folder" : "file", "lead")}<span class="name">${esc(base)}</span><span class="about">${esc(dir ?? "")}</span>`;
    }
    b.onmousedown = (e) => e.preventDefault();
    b.onclick = () => pickTrigger(b.dataset.pick);
    return b;
  }) : [Object.assign(document.createElement("p"), { className: "none", textContent: slash !== undefined ? "No matching command." : "No matching files or folders." })]));
  composerMenu.hidden = false;
  composerMenu.style.left = composerMenu.style.right = "";
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
function pickTrigger(pick) {
  composerMenu.hidden = true;
  const at = prompt.selectionStart;
  const start = pick.startsWith("/") ? 0 : prompt.value.slice(0, at).lastIndexOf("@");
  const insert = pick.endsWith("/") ? pick : `${pick} `;
  prompt.value = prompt.value.slice(0, start) + insert + prompt.value.slice(at);
  prompt.setSelectionRange(start + insert.length, start + insert.length);
  if (active) saveDraft(active, prompt.value);
  autosize();
  prompt.focus();
  if (pick.endsWith("/")) showTrigger();
}
function triggerKey(e) {
  const rows = [...composerMenu.querySelectorAll("[data-pick]")];
  const at = rows.findIndex((r) => r.classList.contains("highlight"));
  if (e.key === "Escape") composerMenu.hidden = true;
  else if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey) {
    // A command typed in full, or none matching, goes out as typed.
    if (rows[at] && prompt.value !== rows[at].dataset.pick) pickTrigger(rows[at].dataset.pick);
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

/** The branch picker: the folder's branches, searchable, and a new branch named as typed. git
 * switches the folder claude works in, so the machine refuses while a turn runs. */
async function openBranches(anchor) {
  const tab = active;
  if (!tab) return;
  composerMenu.dataset.kind = "branches";
  composerMenu.innerHTML = `<input class="search" placeholder="Search branches…" autocomplete="off" spellcheck="false"><div class="refs"><p class="none">Loading branches…</p></div>`;
  composerMenu.hidden = false;
  composerMenu.style.left = "";
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
  const busy = phaseOf(tab) === "working" || !!tab.menu;
  const list = () => {
    const needle = search.value.trim();
    const shown = git.refs.filter((r) => r.name.toLowerCase().includes(needle.toLowerCase())).slice(0, 80);
    const fresh = needle && !git.refs.some((r) => r.name === needle);
    const rows = shown.map((r) => `<button type="button" data-ref="${esc(r.name)}"${r.current || r.worktree || busy ? " disabled" : ""}>${icon("git-branch", "lead")}<span class="label">${esc(r.name)}</span>
      ${r.current ? `<span class="badge">current</span>` : r.worktree ? `<span class="badge">worktree</span>` : r.remote ? `<span class="badge">remote</span>` : r.name === git.default ? `<span class="badge">default</span>` : ""}</button>`);
    if (fresh) rows.push(`<button type="button" data-create="${esc(needle)}"${busy ? " disabled" : ""}>${icon("plus", "lead")}<span class="label">Create new branch "${esc(needle)}"</span></button>`);
    composerMenu.querySelector(".refs").innerHTML = (busy ? `<p class="none">claude is working. Switch branches when the turn ends.</p>` : "") + (rows.join("") || `<p class="none">No branches found.</p>`);
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
    const row = e.target.closest("[data-ref], [data-create]");
    if (!row || row.disabled) return;
    composerMenu.hidden = true;
    const create = !!row.dataset.create;
    const ref = git.refs.find((r) => r.name === row.dataset.ref);
    const name = create ? row.dataset.create : ref.remote ? ref.name.slice(ref.name.indexOf("/") + 1) : ref.name;
    try {
      await callOn(tab.machine, { cmd: "checkout", tab: tab.view.id, branch: name, create });
      showComposerNote(`Switched to ${name}`);
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
  const branch = e.target.closest(".branch");
  if (branch) return composerMenu.hidden || composerMenu.dataset.kind !== "branches" ? openBranches(branch) : (composerMenu.hidden = true);
  const pr = e.target.closest("[data-pr]");
  if (pr && active) e.metaKey || e.ctrlKey ? openOutside(pr.dataset.pr) : openPr(active, pr.dataset.pr);
});
onGuiKey("g", () => document.querySelector("#composer-footer .branch")?.click(), { shift: true });

/** The header's Git action asks claude to commit, push or open a pull request, picked as T3 Code
 * picks its quick action from the branch's state. */
function quickAction(git, hasPr) {
  if (!git) return null;
  const branch = git.refs.find((r) => r.current)?.name;
  if (!branch) return null;
  if (git.dirty) return hasPr || branch === git.default ? "commit_push" : "commit_push_pr";
  if (git.behind > 0 && !git.ahead) return "pull";
  if (git.ahead > 0) return hasPr ? "push" : "push_pr";
  if (!git.upstream && branch !== git.default && !hasPr) return "push_pr";
  if (!hasPr && branch !== git.default && git.upstream) return "create_pr";
  return null;
}
const gitAction = document.getElementById("git-action");
function renderGitAction(tab) {
  const t = tab && threadOf(tab);
  gitAction.hidden = !t?.workspace?.repo;
  if (gitAction.hidden) return;
  const key = JSON.stringify([tab.key, tab.view.cwd, t.workspace.branch, statusOf(tab)]);
  if (t.gitFor !== key) {
    t.gitFor = key;
    callOn(tab.machine, { cmd: "branches", tab: tab.view.id }).then((git) => {
      t.git = git;
      if (tab === active) renderGitAction(tab);
    }, () => (t.git = null));
  }
  const action = quickAction(t.git, !!prOf(tab));
  const shown = GIT_ACTIONS[action ?? "commit"];
  const main = gitAction.querySelector(".main");
  main.disabled = !action;
  main.dataset.action = action ?? "";
  main.title = action ? shown.prompt : t.git ? "Branch is up to date. No action needed." : "Reading the branch…";
  main.innerHTML = `${icon(shown.icon)}<span>${esc(shown.label)}</span>`;
}
function askGit(action) {
  const tab = active;
  if (!tab || !GIT_ACTIONS[action]) return;
  if (!sendSlash(tab, GIT_ACTIONS[action].prompt)) return;
  showComposerNote(`Asked claude to ${GIT_ACTIONS[action].label.toLowerCase()}`);
  setTimeout(() => showComposerNote(""), 2500);
}
gitAction.querySelector(".main").onclick = (e) => askGit(e.currentTarget.dataset.action);
gitAction.querySelector(".more").onclick = (e) => {
  const tab = active;
  if (!tab) return;
  const menu = document.getElementById("git-menu");
  const hasPr = !!prOf(tab);
  const entries = [["commit", "Commit"], ["push", "Push"], ...(hasPr ? [] : [["create_pr", "Create PR"]])];
  menu.replaceChildren(...entries.map(([action, label]) => menuRow({ label, icon: GIT_ACTIONS[action].icon, run: () => askGit(action) }, menu)));
  menu.hidden = !menu.hidden;
  e.stopPropagation();
};
window.addEventListener("mousedown", (e) => {
  const menu = document.getElementById("git-menu");
  if (!menu.hidden && !e.target.closest("#git-menu, #git-action .more")) menu.hidden = true;
}, true);

function toggleDrawer(open = !ui.drawer) {
  ui.drawer = open;
  document.body.dataset.drawer = open ? "open" : "closed";
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
