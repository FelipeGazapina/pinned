Object.assign(ICON_PATHS, {
  "square-split-horizontal": `<path d="M8 19H5c-1 0-2-1-2-2V7c0-1 1-2 2-2h3"/><path d="M16 5h3c1 0 2 1 2 2v10c0 1-1 2-2 2h-3"/><line x1="12" x2="12" y1="4" y2="20"/>`,
  "square-split-vertical": `<path d="M5 8V5c0-1 1-2 2-2h10c1 0 2 1 2 2v3"/><path d="M19 16v3c0 1-1 2-2 2H7c-1 0-2-1-2-2v-3"/><line x1="4" x2="20" y1="12" y2="12"/>`,
  "globe-2": `<path d="M21.54 15H17a2 2 0 0 0-2 2v4.54"/><path d="M7 3.34V5a3 3 0 0 0 3 3a2 2 0 0 1 2 2c0 1.1.9 2 2 2a2 2 0 0 0 2-2c0-1.1.9-2 2-2h3.17"/><path d="M11 21.95V18a2 2 0 0 0-2-2a2 2 0 0 1-2-2v-1a2 2 0 0 0-2-2H2.05"/><circle cx="12" cy="12" r="10"/>`,
  "git-pull-request-arrow": `<circle cx="5" cy="6" r="3"/><path d="M5 9v12"/><circle cx="19" cy="18" r="3"/><path d="m15 9-3-3 3-3"/><path d="M12 6h5a2 2 0 0 1 2 2v7"/>`,
  "git-pull-request-closed": `<circle cx="6" cy="6" r="3"/><path d="M6 9v12"/><path d="m21 3-6 6"/><path d="m21 9-6-6"/><path d="M18 11.5V15"/><circle cx="18" cy="18" r="3"/>`,
  "git-pull-request-draft": `<circle cx="18" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><path d="M18 6V5"/><path d="M18 11v-1"/><line x1="6" x2="6" y1="9" y2="21"/>`,
  "eye-off": `<path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/>`,
  "copy-plus": `<line x1="15" x2="15" y1="12" y2="18"/><line x1="12" x2="18" y1="15" y2="15"/><rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>`,
});

/** The brand marks T3 draws on a Device tab, from Simple Icons (CC0) and T3's Android glyph. */
const BRAND_MARKS = {
  apple: `<path d="M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701"/>`,
  android: `<path d="m8 3-1-2m9 2 1-2" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><path fill-rule="evenodd" d="M5 10a7 7 0 0 1 14 0H5Zm4-4a.8.8 0 1 0 0 1.6A.8.8 0 0 0 9 6Zm6 0a.8.8 0 1 0 0 1.6A.8.8 0 0 0 15 6Z"/><path d="M5 11h14v7a2 2 0 0 1-2 2v2a1.5 1.5 0 0 1-3 0v-2h-4v2a1.5 1.5 0 0 1-3 0v-2a2 2 0 0 1-2-2v-7Z"/><rect x="1" y="11" width="3" height="8" rx="1.5"/><rect x="20" y="11" width="3" height="8" rx="1.5"/>`,
};
const brandMark = (name, cls = "") => `<svg class="i brand ${cls}" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">${BRAND_MARKS[name]}</svg>`;

/** A pull request's state as T3 draws it: its glyph, its tone and its name. */
const PR_PRESENTATION = {
  open: ["git-pull-request-arrow", "Open"],
  draft: ["git-pull-request-draft", "Draft"],
  closed: ["git-pull-request-closed", "Closed"],
  merged: ["git-merge", "Merged"],
};
const prGlyph = (state, cls = "") => {
  const [name, label] = PR_PRESENTATION[state] ?? PR_PRESENTATION.open;
  return `<span class="pr-glyph ${cls}" data-state="${state}" role="img" aria-label="${label}" title="${label}">${icon(name)}</span>`;
};

/** A tab's icon, as T3's RightPanelTabs draws it: the file's own type icon, a pull request's state,
 * a device's platform. */
function surfaceIcon(tab, s) {
  if (s.kind === "file") return pierreIcon(s.path, "kind");
  if (s.kind === "pr") {
    const d = prCache.get(`${tab.key}:${s.url}`)?.data;
    return d ? `<span class="kind pr-tone" data-state="${prState(d)}">${icon(PR_PRESENTATION[prState(d)]?.[0] ?? "git-pull-request-arrow")}</span>` : icon("git-pull-request-arrow", "kind muted");
  }
  if (s.kind === "device" && s.target?.platform) return brandMark(s.target.platform === "ios" ? "apple" : "android", "kind");
  return null;
}

/** A device tab's tooltip adds its machine and version under its name, as T3's does. */
const surfaceTip = (tab, s) => (s.kind === "device" && s.target ? `${surfaceTitle(tab, s)}\n${machineName(tab.machine)} · ${s.target.version || (s.target.platform === "ios" ? "iOS" : "Android")}` : null);

/** "Terminal 1", "Terminal 2": T3's labels for a thread's terminals, from their ids. */
const terminalLabel = (id) => (/^term-(\d+)$/.exec(id) ? `Terminal ${id.slice(5)}` : id);

/** The composer takes a file as a mention chip, as T3's Add to chat and a dragged row do. */
function addMention(tab, path) {
  if (!tab || tab !== active) return;
  const chips = chipsOf(tab);
  const base = `@${path.replace(/\/$/, "").split("/").pop()}`;
  const word = chips[base] && chips[base].value !== path ? `@${path}` : base;
  chips[word] = { kind: "mention", value: path };
  prompt.value = `${prompt.value}${prompt.value && !/\s$/.test(prompt.value) ? " " : ""}${word} `;
  prompt.setSelectionRange(prompt.value.length, prompt.value.length);
  saveDraft(tab, prompt.value);
  autosize();
  prompt.focus();
}
const MENTION_DRAG = "text/x-pinned-mention";
composer.addEventListener("dragover", (e) => {
  if (!e.dataTransfer.types.includes(MENTION_DRAG)) return;
  e.preventDefault();
  e.dataTransfer.dropEffect = "copy";
  composer.dataset.mentionDrop = "";
});
composer.addEventListener("dragleave", (e) => {
  if (!composer.contains(e.relatedTarget)) delete composer.dataset.mentionDrop;
});
composer.addEventListener("drop", (e) => {
  delete composer.dataset.mentionDrop;
  const paths = e.dataTransfer.getData(MENTION_DRAG);
  if (!paths) return;
  e.preventDefault();
  e.stopPropagation();
  for (const path of paths.split("\n").filter(Boolean)) addMention(active, path);
});

/** What the Files tree and a File tab offer for a path, as T3's file context menu does: open it with
 * the system, show it in the file manager, open it in an editor, and mention it. */
function fileMenu(tab, path) {
  const editors = editorsOn(tab.machine);
  const finder = editors.find((e) => e.id === "finder");
  const open = (editor, how = {}) => callOn(tab.machine, { cmd: "open_in", tab: tab.view.id, editor, path, folder: true, ...how }).catch((e) => flash(String(e)));
  const others = editors.filter((e) => e.id !== "finder");
  const mention = `@${path}`;
  return [
    ...(finder ? [{ label: "Open", icon: "pencil", run: () => open("default") }, { label: finder.name === "Finder" ? "Reveal in Finder" : "Reveal in Files", icon: "folder-tree", run: () => open("finder", { reveal: true }) }] : []),
    ...(others.length ? [{ label: "Open with", icon: "square-arrow-out-up-right", sub: others.map((ed) => ({ label: ed.name, run: () => open(ed.id) })) }] : []),
    ...(finder || others.length ? ["-"] : []),
    { label: "Copy mention", icon: "copy", run: () => copyText(mention, `Mention copied: ${path}`) },
    { label: "Add to chat", icon: "message-square", run: () => addMention(tab, path) },
  ];
}
/** A menu at the pointer, for a right-click. */
const pointerAnchor = (e) => ({ getBoundingClientRect: () => new DOMRect(e.clientX, e.clientY, 0, 0), contains: () => false });
panelBody.addEventListener("contextmenu", (e) => {
  const row = e.target.closest(".files-tree [data-files-open], .files-tree [data-files-dir]");
  if (!row || !active) return;
  e.preventDefault();
  popMenu(pointerAnchor(e), fileMenu(active, row.dataset.filesOpen ?? row.dataset.filesDir));
});
panelBody.addEventListener("dragstart", (e) => {
  const row = e.target.closest?.(".files-tree [data-files-open], .files-tree [data-files-dir]");
  if (!row) return;
  e.dataTransfer.setData(MENTION_DRAG, row.dataset.filesOpen ?? row.dataset.filesDir);
  e.dataTransfer.effectAllowed = "copy";
});

/** HTML and PDF files draw from a loopback URL of the machine's, so a page loads its own styles,
 * scripts and images, as T3's workspace asset URLs do. That URL only works on the machine itself. */
const pageUrls = new Map();
const servesHere = (tab) => tab.machine === LOCAL || /^(localhost|127\.0\.0\.1|\[::1\])$/.test(location.hostname);
async function fileUrl(tab, s) {
  const key = `${tab.key}\n${s.root ? "/" : ""}${s.path}`;
  if (!pageUrls.has(key)) pageUrls.set(key, callOn(tab.machine, { cmd: "file_url", tab: tab.view.id, path: s.path, root: !!s.root }));
  try {
    return await pageUrls.get(key);
  } catch (e) {
    pageUrls.delete(key);
    throw e;
  }
}
async function openFileInBrowser(tab, s) {
  try {
    addSurface(tab, newBrowser(await fileUrl(tab, s)));
  } catch (e) {
    flash(`Unable to open file in browser: ${String(e)}`);
  }
}
function mountFileFrame(tab, s) {
  const frame = panelBody.querySelector("iframe.html-view[data-src]");
  if (!frame) return;
  fileUrl(tab, s).then((url) => {
    if (frame.isConnected) frame.src = url;
  }, () => {
    const text = s.draft ?? fileTexts.get(fileTextKey(tab, s))?.file?.text ?? "";
    frame.removeAttribute("data-src");
    frame.srcdoc = text;
  });
}

/** T3's compact Open-in group on a File tab: the preferred editor's mark opens the file there, and
 * its chevron picks another. */
function fileOpenInHtml(tab) {
  const editor = editorFor(tab.machine);
  if (!editor) return "";
  return `<span class="open-in-file" role="group" aria-label="Open in editor"><button type="button" class="main" data-file-open-in aria-label="Open file in preferred editor" title="Open in ${esc(editor.name)}">${editorIcon(editor.id) || icon("square-arrow-out-up-right")}</button><button type="button" class="pick" data-file-editors aria-label="Choose editor">${icon("chevron-down")}</button></span>`;
}
function openFileIn(tab, s) {
  const editor = editorFor(tab.machine);
  if (!editor) return;
  const how = editor.id === "finder" ? { reveal: true } : {};
  callOn(tab.machine, { cmd: "open_in", tab: tab.view.id, editor: editor.id, path: s.path, ...(s.root ? {} : { folder: true }), ...how }).then(() => flash(`Opened ${s.path} in ${editor.name}`), (err) => flash(String(err)));
}
function fileEditorsMenu(tab, anchor) {
  const chosen = editorFor(tab.machine);
  const list = editorsOn(tab.machine);
  toggleMenu(anchor, list.length
    ? list.map((ed) => ({ label: ed.name, checked: ed.id === chosen?.id, kbd: ed.id === chosen?.id ? `${IS_MAC ? "⌘" : "Ctrl+"}O` : "", run: () => (save(`gui-editor:${tab.machine}`, ed.id), renderOpenIn(tab), renderPanel()) }))
    : [{ label: "No installed editors found", disabled: true }]);
}

function pierreSprite() {
  if (window.PIERRE_ICONS && !document.getElementById("pierre-sprite")) document.body.insertAdjacentHTML("afterbegin", `<div id="pierre-sprite" aria-hidden="true">${PIERRE_ICONS.sprite}</div>`);
  return !!window.PIERRE_ICONS;
}
/** Pierre's chevron, which T3's file tree turns to show a folder open. */
const pierreChevron = () => (pierreSprite() ? `<svg class="i pierre chevron" viewBox="0 0 16 16" aria-hidden="true"><use href="#file-tree-icon-chevron"/></svg>` : icon("chevron-right", "chevron"));
/** A path's icon from T3's Pierre set in its light or dark color, as the theme is. */
function pierreIcon(path, cls = "") {
  if (!pierreSprite()) return icon("file", cls);
  const { name, token } = PIERRE_ICONS.resolve(path);
  const [light, dark] = PIERRE_ICONS.colors[token ?? "default"] ?? PIERRE_ICONS.colors.default;
  return `<svg class="i pierre toned ${cls}" viewBox="0 0 16 16" style="--tone-light:${light};--tone-dark:${dark}" aria-hidden="true"><use href="#${esc(name)}"/></svg>`;
}

/** Terminal tabs: login shells in the thread's folder beside its claude, as T3's thread terminals
 * are. A tab holds up to four, side by side or stacked; each keeps its xterm across tab switches,
 * and closing one ends its shell. */
const shellTerms = new Map();
const MAX_SPLIT = 4;
/** A tab saved before terminals split holds one, named by its old number. */
function terminalsOf(s) {
  if (!s.terminals?.length) s.terminals = [`term-${s.n ?? 1}`];
  if (!s.terminals.includes(s.active)) s.active = s.terminals[0];
  return s;
}
/** The lowest number no terminal of the thread has, as T3's nextTerminalId picks. */
function nextTerminalId(tab) {
  const used = new Set(panelOf(tab).surfaces.filter((s) => s.kind === "terminal").flatMap((s) => terminalsOf(s).terminals));
  let n = 1;
  while (used.has(`term-${n}`)) n++;
  return `term-${n}`;
}
function newTerminalSurface(tab) {
  const id = nextTerminalId(tab);
  return addSurface(tab, { id: `terminal:${Date.now().toString(36)}`, kind: "terminal", terminals: [id], active: id });
}
function splitTerminal(tab, s, direction) {
  terminalsOf(s);
  if (s.terminals.length >= MAX_SPLIT) return;
  const id = nextTerminalId(tab);
  s.terminals.push(id);
  s.active = id;
  s.split = direction;
  savePanel(tab);
  renderPanel();
}
function activateTerminal(tab, s, id) {
  if (s.active === id) return;
  s.active = id;
  savePanel(tab);
  renderPanelTabs(tab);
  for (const pane of panelBody.querySelectorAll(".term-pane")) pane.classList.toggle("active", pane.dataset.term === id);
  for (const row of panelBody.querySelectorAll(".term-list [data-term-pick]")) row.classList.toggle("active", row.dataset.termPick === id);
}
/** Closing the last terminal closes its tab. The desktop app asks first, as T3's does. */
function closeTerminal(tab, s, id, ask = TAURI) {
  const label = terminalLabel(id);
  if (ask) {
    showDialog(`<div class="dialog-head"><h2>Close terminal “${esc(label)}”?</h2><p class="muted">This stops the running process and clears its history.</p></div>
      <div class="theme-dialog-actions"><button type="button" class="outline" data-dialog-cancel>Cancel</button><button type="button" class="danger" data-dialog-close>Close terminal</button></div>`);
    shellDialog.querySelector("[data-dialog-cancel]").onclick = closeDialog;
    shellDialog.querySelector("[data-dialog-close]").onclick = () => {
      closeDialog();
      closeTerminal(tab, s, id, false);
    };
    return;
  }
  terminalsOf(s);
  if (s.terminals.length === 1) return closeSurface(s.id);
  const i = s.terminals.indexOf(id);
  s.terminals.splice(i, 1);
  if (s.active === id) s.active = s.terminals.at(-1);
  if (s.terminals.length === 1) delete s.split;
  savePanel(tab);
  closeGoneShells();
  renderPanel();
}
const shownTerminal = () => (active && shownSurface(active)?.kind === "terminal" ? shownSurface(active) : null);
const panelTerminalFocused = () => !!shownTerminal() && !!document.activeElement?.closest?.("#panel-body .term-pane");
/** ⌘W in a focused terminal closes that terminal, not its whole tab. */
function closeFocusedTerminal() {
  if (!panelTerminalFocused()) return false;
  const s = shownTerminal();
  closeTerminal(active, s, document.activeElement.closest(".term-pane").dataset.term);
  return true;
}
onGuiKey("d", () => splitTerminal(active, shownTerminal(), "horizontal"), { when: panelTerminalFocused });
onGuiKey("d", () => splitTerminal(active, shownTerminal(), "vertical"), { shift: true, when: panelTerminalFocused });
onGuiKey("n", () => newTerminalSurface(active), { when: panelTerminalFocused });

const shellKey = (s, id) => `${s.id}\n${id}`;
function shellTerm(tab, s, id) {
  const key = shellKey(s, id);
  if (shellTerms.has(key)) return shellTerms.get(key);
  const el = document.createElement("div");
  el.className = "shell-term";
  const look = terminalLook();
  const term = new Terminal({ ...look, cursorBlink: true, allowTransparency: true, rightClickSelectsWord: false, theme: { background: "#00000000" } });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  const entry = { el, term, fit, tab, id, shell: null, opened: false, typed: [], look: JSON.stringify(look) };
  shellTerms.set(key, entry);
  wireTerminalLinks(term);
  // Keys typed while the shell starts wait for it.
  term.onData((data) => (entry.shell ? callOn(tab.machine, { cmd: "write_input", tab: entry.shell, data }) : entry.typed.push(data)));
  term.onResize(({ cols, rows }) => entry.shell && callOn(tab.machine, { cmd: "resize_pty", tab: entry.shell, cols, rows }).catch(() => {}));
  el.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    popMenu(pointerAnchor(e), shellMenu(entry, true));
  });
  // A selection made with the mouse offers itself to the chat, as T3's selection action does.
  el.addEventListener("mouseup", (e) => {
    if (e.button !== 0) return;
    setTimeout(() => term.hasSelection() && term.getSelection().trim() && popMenu(pointerAnchor(e), shellMenu(entry, false)), e.detail >= 2 ? 500 : 0);
  });
  return entry;
}
function shellMenu(entry, full) {
  const { term } = entry;
  const selected = term.hasSelection() && term.getSelection().trim() !== "";
  const copy = { label: "Copy", disabled: !selected, run: () => copyText(term.getSelection(), "Copied terminal selection") };
  const add = { label: "Add to chat", disabled: !selected, run: () => addTerminalContext(entry) };
  if (!full) return [add, copy];
  return [add, copy,
    { label: "Paste", run: () => navigator.clipboard.readText().then((text) => text && term.paste(text), () => flash("Unable to read the clipboard")) },
    { label: "Select all", run: () => (term.selectAll(), term.focus()) },
    { label: "Jump to latest", run: () => (term.scrollToBottom(), term.focus()) },
  ];
}
/** The selected lines go with the next prompt as T3's `<terminal_context>` block, a chip until then. */
function addTerminalContext(entry) {
  const { term, tab, id } = entry;
  const pos = term.getSelectionPosition();
  const text = term.getSelection().replace(/\r\n/g, "\n").replace(/^\n+|\n+$/g, "");
  if (!pos || !text || tab !== active) return;
  const start = pos.start.y + 1;
  const lines = text.split("\n");
  const end = Math.max(start, start + lines.length - 1);
  const range = start === end ? `line ${start}` : `lines ${start}-${end}`;
  const label = terminalLabel(id);
  const block = `<terminal_context>\n- ${label} ${range}:\n${lines.map((line, i) => `  ${start + i} | ${line}`).join("\n")}\n</terminal_context>`;
  const t = threadOf(tab);
  t.attachments = [...(t.attachments ?? []), { path: `terminal:${id}:${start}-${end}:${Date.now()}`, name: `${label} ${range}`, icon: "square-terminal", text: block }];
  term.clearSelection();
  renderComposer(tab);
  prompt.focus();
}

async function startShell(entry) {
  const { tab, term } = entry;
  try {
    entry.shell = await callOn(tab.machine, { cmd: "open_shell", tab: tab.view.id, cols: term.cols, rows: term.rows });
    await machines.get(tab.machine).transport.attach(entry.shell, { write: (bytes) => term.write(bytes), reset: () => term.reset() });
    if (entry.typed.length) callOn(tab.machine, { cmd: "write_input", tab: entry.shell, data: entry.typed.splice(0).join("") });
  } catch (e) {
    term.write(`\r\n${/unknown variant/.test(String(e)) ? `${machineName(tab.machine)} runs an older pinned, which has no terminals. Update it to open one.` : String(e)}\r\n`);
  }
}

const termAction = (act, ic, label, disabled = false) => `<button type="button" data-term-act="${act}" aria-label="${esc(label)}" title="${esc(label)}"${disabled ? ` aria-disabled="true" class="off"` : ""}>${icon(ic)}</button>`;
function terminalActions(s) {
  const full = s.terminals.length >= MAX_SPLIT;
  const key = (k) => `${IS_MAC ? "⌘" : "Ctrl+"}${k}`;
  return [
    termAction("split", "square-split-horizontal", full ? `Split Terminal Horizontally (max ${MAX_SPLIT} per group)` : `Split Terminal Horizontally (${key("D")})`, full),
    termAction("split-vertical", "square-split-vertical", full ? `Split Terminal Vertically (max ${MAX_SPLIT} per group)` : `Split Terminal Vertically (${key(IS_MAC ? "⇧D" : "Shift+D")})`, full),
    termAction("new", "plus", `New Terminal (${key("N")})`),
    termAction("close", "trash-2", `Close Terminal (${key("W")})`),
  ].join("");
}
function renderTerminalPanel(s) {
  const tab = active;
  terminalsOf(s);
  const split = s.split === "vertical" ? "vertical" : "horizontal";
  const many = s.terminals.length > 1;
  const group = split === "vertical" ? ["Stacked", "square-split-vertical"] : ["Side by side", "square-split-horizontal"];
  const panes = s.terminals.map((id) => `<div class="term-pane${id === s.active ? " active" : ""}" data-term="${esc(id)}"></div>`).join("");
  const side = many
    ? `<aside class="term-list"><div class="term-list-head">${terminalActions(s)}</div><div class="term-list-body">
        <button type="button" class="term-group" data-term-pick="${esc(s.active)}">${icon(group[1])}<span>${group[0]}</span><span class="count">${s.terminals.length}</span></button>
        ${s.terminals.map((id) => `<div class="term-row${id === s.active ? " active" : ""}" data-term-pick="${esc(id)}"><button type="button" class="lead" data-term-close="${esc(id)}" aria-label="Close ${esc(terminalLabel(id))}" title="Close ${esc(terminalLabel(id))}">${icon("square-terminal", "kind")}${icon("x", "x")}</button><button type="button" class="name">${esc(terminalLabel(id))}</button></div>`).join("")}</div></aside>`
    : `<div class="term-actions">${terminalActions(s)}</div>`;
  const html = `<div class="term-surface${many ? " many" : ""}" data-shell="${esc(s.id)}"><div class="term-panes" data-split="${split}" style="--count:${s.terminals.length}">${panes}</div>${side}</div>`;
  const focused = !!document.activeElement?.closest?.("#panel-body .term-pane");
  showPanel(html);
  for (const id of s.terminals) {
    const entry = shellTerm(tab, s, id);
    const pane = panelBody.querySelector(`.term-pane[data-term="${CSS.escape(id)}"]`);
    if (entry.el.parentElement !== pane) pane.append(entry.el);
    if (!entry.opened) {
      entry.opened = true;
      entry.term.open(entry.el);
    }
    const look = terminalLook();
    if (JSON.stringify(look) !== entry.look) {
      entry.look = JSON.stringify(look);
      Object.assign(entry.term.options, look);
    }
  }
  if (focused && !document.activeElement?.closest?.("#panel-body .term-pane")) shellTerms.get(shellKey(s, s.active))?.term.focus();
  requestAnimationFrame(() => {
    for (const id of s.terminals) {
      const entry = shellTerms.get(shellKey(s, id));
      if (!entry?.el.isConnected) continue;
      entry.fit.fit();
      if (!entry.shell && !entry.starting) {
        entry.starting = true;
        startShell(entry);
      }
    }
    if (document.activeElement === document.body || !document.activeElement) shellTerms.get(shellKey(s, s.active))?.term.focus();
  });
}
panelBody.addEventListener("click", (e) => {
  const s = shownTerminal();
  if (!s || !e.target.closest(".term-surface")) return;
  const tab = active;
  const act = e.target.closest("[data-term-act]");
  const close = e.target.closest("[data-term-close]");
  const pick = e.target.closest("[data-term-pick]");
  if (act?.getAttribute("aria-disabled") === "true") return;
  if (act?.dataset.termAct === "split") splitTerminal(tab, s, "horizontal");
  else if (act?.dataset.termAct === "split-vertical") splitTerminal(tab, s, "vertical");
  else if (act?.dataset.termAct === "new") newTerminalSurface(tab);
  else if (act?.dataset.termAct === "close") closeTerminal(tab, s, s.active);
  else if (close) closeTerminal(tab, s, close.dataset.termClose);
  else if (pick) {
    activateTerminal(tab, s, pick.dataset.termPick);
    shellTerms.get(shellKey(s, pick.dataset.termPick))?.term.focus();
  }
});
panelBody.addEventListener("pointerdown", (e) => {
  const s = shownTerminal();
  const pane = e.target.closest?.(".term-pane");
  if (s && pane) activateTerminal(active, s, pane.dataset.term);
});
panelBody.addEventListener("focusin", (e) => {
  const s = shownTerminal();
  const pane = e.target.closest?.(".term-pane");
  if (s && pane) activateTerminal(active, s, pane.dataset.term);
});
new ResizeObserver(() => {
  const s = shownTerminal();
  if (s) for (const id of terminalsOf(s).terminals) shellTerms.get(shellKey(s, id))?.fit.fit();
}).observe(panelBody);

/** Ends the shells of terminals that closed. */
function closeGoneShells() {
  const keep = new Set([...panelStates.values()].flatMap((p) => p.surfaces.filter((s) => s.kind === "terminal").flatMap((s) => terminalsOf(s).terminals.map((id) => shellKey(s, id)))));
  for (const [key, entry] of shellTerms) {
    if (keep.has(key)) continue;
    shellTerms.delete(key);
    if (entry.shell) {
      machines.get(entry.tab.machine)?.transport.detach(entry.shell);
      callOn(entry.tab.machine, { cmd: "close_shell", shell: entry.shell }).catch(() => {});
    }
    entry.term.dispose();
  }
}

/** The Pull requests tab: each linked pull request as T3's ThreadPullRequestsPanel lists it, with its
 * state, checks, review, author, repository, branches and age, a watch that wakes claude, and the
 * quick actions Shift shows. */
const PR_SOURCES = { manual: "Linked by you", agent: "Linked by the agent" };
const PR_CHECKS_TONE = { passing: ["circle-check", "ok", "All checks have passed"], failing: ["circle-x", "fail", "Some checks were not successful"], pending: ["circle-dot", "pending", "Some checks haven't completed yet"] };
const PR_REVIEW_TONE = { APPROVED: ["user-check", "ok", "Approved"], CHANGES_REQUESTED: ["user-round-x", "pending", "Changes requested"], REVIEW_REQUIRED: ["user-round", "quiet", "Awaiting review"] };
function prChecksState(d) {
  const states = (d.statusCheckRollup ?? []).map(checkState);
  if (!states.length) return null;
  return states.includes("fail") ? "failing" : states.includes("pending") ? "pending" : "passing";
}
const prWatched = new Map();
function watchedPrs(tab) {
  if (!prWatched.has(tab.key)) {
    prWatched.set(tab.key, new Set());
    callOn(tab.machine, { cmd: "watch_pr", tab: tab.view.id }).then((urls) => {
      prWatched.set(tab.key, new Set(urls));
      if (tab === active && shownSurface(tab)?.kind === "prs") renderPanel();
    }, () => {});
  }
  return prWatched.get(tab.key);
}
async function setWatching(tab, url, watching) {
  try {
    prWatched.set(tab.key, new Set(await callOn(tab.machine, { cmd: "watch_pr", tab: tab.view.id, pr: url, watching })));
  } catch (e) {
    flash(/unknown variant/.test(String(e)) ? `${machineName(tab.machine)} runs an older pinned, which can't watch pull requests.` : String(e));
  }
  if (tab === active) renderPanel();
}
const middleTruncate = (text, cls = "") => `<span class="mid ${cls}" title="${esc(text)}"><span class="start">${esc(text.slice(0, Math.ceil(text.length / 2)))}</span><span class="end">${esc(text.slice(Math.ceil(text.length / 2)))}</span></span>`;
function prRowHtml(tab, pr, watched) {
  const entry = prCache.get(`${tab.key}:${pr.url}`);
  if (!entry) loadPr(tab, pr.url, true);
  const d = entry?.data;
  const state = d ? prState(d) : null;
  const open = !d || state === "open" || state === "draft";
  const repo = pr.url.match(/github\.com\/([^/]+\/[^/]+)\/pull/)?.[1] ?? "";
  const tone = ([ic, cls, label]) => `<span class="sig ${cls}" role="img" aria-label="${label}" title="${label}">${icon(ic)}</span>`;
  const checks = d && prChecksState(d);
  const review = d && PR_REVIEW_TONE[d.reviewDecision];
  const conflict = d && state === "open" && d.mergeable === "CONFLICTING" ? `<span class="conflict" role="img" aria-label="Conflicts with ${esc(d.baseRefName)}" title="Conflicts with ${esc(d.baseRefName)}">${icon("triangle-alert")}</span>` : "";
  const stack = d?.stack && d.stack.below.length + d.stack.above.length > 0 ? d.stack.below.length + d.stack.above.length + 1 : 0;
  const login = d?.author?.login ?? "ghost";
  const signals = open && d ? `${watched ? `<span class="sig" role="img" aria-label="Watching" title="Watching: the agent wakes when checks finish, someone comments, or the branch conflicts">${icon("eye")}</span>` : ""}${checks ? tone(PR_CHECKS_TONE[checks]) : ""}${review ? tone(review) : ""}` : "";
  const quick = d && [
    open && `<button type="button" class="danger" data-prs-quick="close" aria-label="Close #${esc(pr.number)}" title="Close immediately">${icon("git-pull-request-closed")}Close</button>`,
    state === "open" && `<button type="button" data-prs-quick="merge" aria-label="Merge #${esc(pr.number)}"${stack ? ` disabled title="Open this pull request to merge its stack"` : ` title="Merge immediately"`}>${icon("git-merge")}Merge</button>`,
    state === "draft" && `<button type="button" data-prs-quick="ready" aria-label="Ready for review #${esc(pr.number)}" title="Ready for review immediately">${icon("git-pull-request-arrow")}Ready for review</button>`,
    state === "closed" && `<button type="button" data-prs-quick="reopen" aria-label="Reopen #${esc(pr.number)}" title="Reopen immediately">${icon("git-pull-request-arrow")}Reopen</button>`,
  ].filter(Boolean).join("");
  return `<div class="pr-row${quick ? " has-quick" : ""}" data-prs-open="${esc(pr.url)}" draggable="${d ? "true" : "false"}">
    <span class="glyph">${d ? prGlyph(state) : `<span class="pr-glyph waiting" role="img" aria-label="Waiting for host state">${icon("git-pull-request-arrow")}</span>`}${conflict}</span>
    <span class="what"><span class="line"><span class="num" title="${esc(PR_SOURCES[pr.source])}${pr.at ? ` · ${esc(agoText(new Date(pr.at).toISOString()))}` : ""}">#${esc(pr.number)}</span><span class="name">${esc(d?.title ?? repo)}</span>${signals ? `<span class="signals">${signals}</span>` : ""}${d ? `<span class="stat"><span class="add">+${d.additions}</span><span class="del">-${d.deletions}</span></span>` : ""}</span>
      <span class="line meta">${stack ? `<span class="stack" title="${stack} pull requests chained by base branch.">${icon("layers")}${stack}</span>` : ""}${d ? `<span class="author" title="${esc(d.author?.name && d.author.name !== login ? `${d.author.name} (@${login})` : login)}"><span class="avatar">${esc(login.slice(0, 1).toUpperCase())}</span><span class="login">${esc(login)}</span></span>${middleTruncate(repo, "repo")}<span class="branches">${middleTruncate(d.headRefName ?? "")}<span class="arrow">→</span>${middleTruncate(d.baseRefName ?? "", "base")}</span>` : `<span class="repo-plain">github.com/${esc(repo)}</span>`}${d?.updatedAt ? `<span class="when">${esc(agoText(d.updatedAt))}</span>` : ""}</span></span>
    ${quick ? `<span class="quick" role="group" aria-label="Quick actions for pull request #${esc(pr.number)}">${quick}</span>` : ""}
    <span class="more"><button type="button" class="icon" data-prs-more="${esc(pr.url)}" aria-label="Actions for #${esc(pr.number)}">${icon("ellipsis")}</button></span></div>`;
}
function renderPrsPanel(s) {
  const tab = active;
  const prs = linkedPrs(tab);
  if (!prs.length) {
    return showPanel(`<div class="prs-empty">${icon("link-2", "big")}<p class="title">No linked pull requests</p><p>Pull requests the agent opens from this thread land here. Link one yourself from a URL or a number.</p>
      <button type="button" class="outline" data-prs-link>${icon("plus")}Link pull request</button></div>`);
  }
  const watched = watchedPrs(tab);
  const rows = prs.map((pr) => prRowHtml(tab, pr, watched.has(pr.url))).join("");
  const open = prs.filter((pr) => ["open", "draft"].includes(prState(prCache.get(`${tab.key}:${pr.url}`)?.data ?? { state: "OPEN" }))).length;
  const synced = Math.max(0, ...prs.map((pr) => prCache.get(`${tab.key}:${pr.url}`)?.at ?? 0));
  showPanel(`<div class="prs"><div class="prs-list">${rows}</div><footer><span>${open} open · ${prs.length} linked${synced ? ` · synced ${esc(ago(synced / 1000))}${ago(synced / 1000) === "now" ? "" : " ago"}` : ""}</span><button type="button" class="ghost" data-prs-link>${icon("plus")}Link</button></footer></div>`);
}
function prRowMenu(tab, url) {
  const pr = linkedPrs(tab).find((p) => p.url === url);
  const state = prCache.get(`${tab.key}:${url}`)?.data;
  const open = !state || ["open", "draft"].includes(prState(state));
  const watching = watchedPrs(tab).has(url);
  return [
    { label: "Copy link", icon: "link-2", run: () => copyText(url, "Link copied") },
    { label: "Open", icon: "arrow-up-right", run: () => openPr(tab, url) },
    ...(open ? [{ label: watching ? "Stop watching" : "Watch for changes", icon: watching ? "eye-off" : "eye", run: () => setWatching(tab, url, !watching) }] : []),
    { label: "Unlink from thread", icon: "unlink", run: () => (pr && watching && setWatching(tab, url, false), unlinkPr(tab, url), renderPanel()) },
  ];
}
async function quickPr(tab, url, action) {
  try {
    await callOn(tab.machine, { cmd: "pr_action", tab: tab.view.id, pr: url, action });
    await loadPr(tab, url, true);
  } catch (e) {
    flash(`Couldn't ${action === "ready" ? "mark it ready" : action} the pull request: ${String(e)}`);
  }
  if (tab === active) renderPanel();
}
function prsClick(e) {
  const tab = active;
  const more = e.target.closest("[data-prs-more]");
  if (more) return toggleMenu(more, prRowMenu(tab, more.dataset.prsMore));
  if (e.target.closest("[data-prs-link]")) return linkPrDialog(tab);
  const quick = e.target.closest("[data-prs-quick]");
  if (quick) return quick.disabled || quickPr(tab, quick.closest("[data-prs-open]").dataset.prsOpen, quick.dataset.prsQuick);
  const row = e.target.closest("[data-prs-open]");
  if (row) return e.metaKey || e.ctrlKey ? openOutside(row.dataset.prsOpen) : openPr(tab, row.dataset.prsOpen);
}
panelBody.addEventListener("click", (e) => {
  if (e.target.closest(".prs, .prs-empty")) prsClick(e);
});
panelBody.addEventListener("contextmenu", (e) => {
  const row = e.target.closest(".prs [data-prs-open]");
  if (!row || !active) return;
  e.preventDefault();
  popMenu(pointerAnchor(e), prRowMenu(active, row.dataset.prsOpen));
});
const PR_DRAG = "text/x-pinned-pr";
panelBody.addEventListener("dragstart", (e) => {
  const row = e.target.closest?.(".prs [data-prs-open]");
  if (!row) return;
  // The link rides along for drops outside the app; the composer takes it as a chip.
  e.dataTransfer.setData(PR_DRAG, row.dataset.prsOpen);
  e.dataTransfer.setData("text/uri-list", row.dataset.prsOpen);
  e.dataTransfer.effectAllowed = "copy";
});
/** The composer takes a pull request as its `#N` chip. */
function addPrChip(tab, url) {
  if (!tab || tab !== active) return;
  const word = `#${url.match(/\/pull\/(\d+)/)?.[1] ?? "?"}`;
  chipsOf(tab)[word] = { kind: "pr", value: url };
  prompt.value = `${prompt.value}${prompt.value && !/\s$/.test(prompt.value) ? " " : ""}${word} `;
  prompt.setSelectionRange(prompt.value.length, prompt.value.length);
  saveDraft(tab, prompt.value);
  autosize();
  prompt.focus();
}
composer.addEventListener("dragover", (e) => {
  if (!e.dataTransfer.types.includes(PR_DRAG)) return;
  e.preventDefault();
  e.dataTransfer.dropEffect = "copy";
  composer.dataset.mentionDrop = "";
});
composer.addEventListener("drop", (e) => {
  const url = e.dataTransfer.getData(PR_DRAG);
  if (!url) return;
  delete composer.dataset.mentionDrop;
  e.preventDefault();
  e.stopPropagation();
  addPrChip(active, url);
});
