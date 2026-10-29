/** T3 Code's Keybindings settings (settings/KeybindingsSettings.tsx): every rule of `keybindingRules`
 * by group, searchable, each shortcut recorded by pressing it, its `when` written or built from
 * conditions, a changed default reset and an added rule removed. */

const KB_GROUPS = [
  ["navigation", "Navigation", ["sidebar", "rightPanel", "commandPalette", "filePicker", "projectSearch", "editor"]],
  ["threads", "Threads", ["thread", "chat", "pullRequest"]],
  ["composer", "Composer", ["composer", "modelPicker"]],
  ["terminal", "Terminal", ["terminal"]],
  ["preview", "Preview & diff", ["preview", "diff"]],
  ["appearance", "Appearance", ["theme", "appearance", "themeEditor"]],
  ["scripts", "Project scripts", ["script"]],
];
const KB_CORE_WHEN = ["terminalFocus", "terminalOpen", "isWeb", "isDesktop", "true", "false"];
const kbWhenVariables = () => {
  const names = new Set(KB_CORE_WHEN);
  const collect = (node) => node && (node.type === "identifier" ? names.add(node.name) : node.type === "not" ? collect(node.node) : (collect(node.left), collect(node.right)));
  for (const rule of DEFAULT_KEYBINDINGS) collect(rule.when && parseWhen(rule.when));
  return [...names].sort((a, b) => ((KB_CORE_WHEN.indexOf(a) + 1 || 99) - (KB_CORE_WHEN.indexOf(b) + 1 || 99)) || a.localeCompare(b));
};

/** KeybindingsSettings.logic's commandLabel: "chat.newLocal" reads "Chat: New Local". */
function commandTitle(command) {
  const special = { "thread.copyReference": "Pull Request: Copy Link or Thread ID", "view.reopenClosed": "Reopen Closed Tab", "preview.focusUrl": "Preview: Focus URL" };
  if (special[command]) return special[command];
  return command.split(".").map((part) => part.replace(/([a-z0-9])([A-Z])/g, "$1 $2").split(/[-_\s]+/).filter(Boolean).map((w) => w[0].toUpperCase() + w.slice(1)).join(" ")).join(": ");
}
const kbRuleKey = (rule) => JSON.stringify([rule.command, rule.key, rule.when ?? ""]);
const kbSameRule = (a, b) => a.command === b.command && a.key === b.key && (a.when ?? "") === (b.when ?? "");
/** whenAstToExpression: a node back as text, groups in parentheses. */
function whenText(node) {
  if (!node) return "";
  const wrap = (n) => (n.type === "identifier" || n.type === "not" ? whenText(n) : `(${whenText(n)})`);
  if (node.type === "identifier") return node.name;
  if (node.type === "not") return `!${wrap(node.node)}`;
  return `${wrap(node.left)} ${node.type === "and" ? "&&" : "||"} ${wrap(node.right)}`;
}
const kbConflictsWhen = (a, b) => !a || !b || a === b;

/** buildKeybindingRows: one row per rule, Default when it is one of the defaults as is, with the
 * other commands its chord and `when` can collide with. */
function keybindingRows() {
  const rows = new Map();
  for (const rule of keybindingRules) {
    const when = whenText(rule.whenAst);
    const row = { id: kbRuleKey({ ...rule, when }), command: rule.command, key: rule.key, when, shortcut: rule.shortcut };
    const defaults = DEFAULT_KEYBINDINGS.filter((d) => d.command === rule.command);
    row.source = defaults.some((d) => d.key === rule.key && (d.when ?? "") === when) ? "Default" : "Custom";
    const fallback = defaults.find((d) => d.key === rule.key && (d.when ?? "") === when) ?? defaults.find((d) => (d.when ?? "") === when) ?? defaults[0];
    row.defaultKey = fallback?.key ?? null;
    row.defaultWhen = fallback?.when ?? "";
    rows.set(row.id, row);
  }
  const all = [...rows.values()];
  for (const row of all) row.conflicts = kbConflicts(all, row.id, row.key, row.when);
  return all.sort((a, b) => a.command.localeCompare(b.command) || a.key.localeCompare(b.key));
}
const kbConflicts = (rows, id, key, when) => (key.trim() ? [...new Set(rows.filter((r) => r.id !== id && r.key === key && kbConflictsWhen(r.when, when)).map((r) => commandTitle(r.command)))].sort() : []);

/** The server's upsert and remove, on the rules this browser keeps: a rule replaces the one it edits. */
function upsertKeybinding({ command, key, when, replace }) {
  const rule = when ? { command, key: key.trim(), when: when.trim() } : { command, key: key.trim() };
  saveKeybindings([...keybindingRules.filter((r) => !kbSameRule(r, rule) && !(replace && kbSameRule(r, replace))), rule]);
  renderKeybindings();
}
function removeKeybinding(row) {
  saveKeybindings(keybindingRules.filter((r) => !kbSameRule(r, row)));
  renderKeybindings();
}

/** keybindingFromKeyboardEvent: the chord pressed as "mod+shift+k", or null for a modifier alone. */
function recordedChord(e) {
  let key = e.altKey || e.key.length !== 1 ? [...eventKeys(e)].at(-1) : e.key.toLowerCase();
  if (e.shiftKey && SHIFTED_KEYS[key] && SHIFTED_KEYS[key] === CODE_KEYS[e.code]) key = SHIFTED_KEYS[key];
  key = { " ": "space", escape: "esc" }[key] ?? key;
  if (["meta", "control", "shift", "alt", "os"].includes(key) || !(key.length === 1 || /^f\d{1,2}$|^(arrow(up|down|left|right)|enter|tab|backspace|delete|home|end|pageup|pagedown|esc|space)$/.test(key))) return null;
  const parts = IS_MAC ? [e.metaKey && "mod", e.ctrlKey && "ctrl"] : [e.ctrlKey && "mod", e.metaKey && "meta"];
  return [...parts, e.altKey && "alt", e.shiftKey && "shift", key].filter(Boolean).join("+");
}
/** KeybindingPill: each part of the chord as a key cap. */
const kbPill = (key) =>
  `<span class="kb-pill">${key.split("+").map((part, i, all) => (part === "" && all[i + 1] === "" ? null : part === "" ? "+" : part)).filter((p) => p !== null).map((part) => `<kbd>${esc(
    part === "mod" ? (IS_MAC ? "⌘" : "Ctrl") : part === "shift" ? "⇧" : part === "alt" ? (IS_MAC ? "⌥" : "Alt") : part === "ctrl" ? "⌃" : part.length === 1 ? part.toUpperCase() : part)}</kbd>`).join("")}</span>`;

const kb = { query: "", drafts: new Map(), adding: null, popover: null };
/** The Keybindings settings page, as T3 Code's settings/keybindings route. */
function showKeybindings() {
  settingsPage.innerHTML = keybindingsSettingsHtml();
  bindKeybindingsSettings();
}
settingsSection({ page: "keybindings", label: "Keybindings", icon: "keyboard", show: () => showKeybindings(), search: [{ title: "Keybindings", terms: ["shortcuts keys chords commands keyboard"] }] });
function keybindingsSettingsHtml() {
  const web = !TAURI ? `<span class="kb-note" role="img" tabindex="0" aria-label="The browser may claim some shortcuts first. The desktop app receives them all." title="The browser may claim some shortcuts first. The desktop app receives them all.">${icon("triangle-alert")}</span>` : "";
  return `<h2>Keybindings${web}</h2><section id="keybindings">
    <div class="kb-tools"><label class="kb-search">${icon("search")}<input type="search" placeholder="Search keybindings" aria-label="Search keybindings" autocomplete="off" spellcheck="false"></label>
    <button type="button" class="kb-button" data-kb-add>${icon("plus")}Add keybinding</button></div><div class="kb-groups"></div></section>`;
}
function bindKeybindingsSettings() {
  const root = document.getElementById("keybindings");
  if (!root) return;
  const search = root.querySelector(".kb-search input");
  search.value = kb.query;
  search.oninput = () => {
    kb.query = search.value;
    renderKeybindings();
  };
  search.onkeydown = (e) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    if (search.value) {
      search.value = kb.query = "";
      renderKeybindings();
    } else search.blur();
  };
  root.querySelector("[data-kb-add]").onclick = () => {
    kb.adding = { command: "", key: "", when: undefined, recording: false };
    renderKeybindings();
  };
  root.addEventListener("click", keybindingsClick);
  root.addEventListener("keydown", keybindingsKeydown, true);
  root.addEventListener("focusin", (e) => e.target.matches("[data-keybinding-capture]") && setRecording(e.target, true));
  root.addEventListener("focusout", (e) => e.target.matches("[data-keybinding-capture]") && setRecording(e.target, false));
  root.addEventListener("input", (e) => e.target.matches("[data-keybinding-capture]") && (draftFor(e.target).key = e.target.value));
  root.addEventListener("change", (e) => {
    if (e.target.matches("[data-kb-command]")) {
      kb.adding.command = e.target.value;
      renderKeybindings();
    }
  });
  renderKeybindings();
}

/** A row's edits before Save: `{ key, when, recording, invalid }`, `when` an AST or undefined. */
const draftOf = (row) => kb.drafts.get(row.id) ?? { key: row.key, when: row.when ? parseWhen(row.when) : undefined, recording: false, invalid: false };
const rowById = (id) => keybindingRows().find((r) => r.id === id);
function draftFor(el) {
  const id = el.closest("[data-kb-row]")?.dataset.kbRow;
  if (!id) return kb.adding;
  if (!kb.drafts.has(id)) kb.drafts.set(id, draftOf(rowById(id)));
  return kb.drafts.get(id);
}
function setRecording(input, on) {
  const draft = draftFor(input);
  if (draft.recording === on) return;
  draft.recording = on;
  input.value = on ? "" : draft.key;
  input.placeholder = on ? "Press shortcut" : "Unassigned";
}
const isDirty = (row, draft) => draft.key !== row.key || whenText(draft.when) !== row.when;

function keybindingRowHtml(row, rows, anchor) {
  const draft = draftOf(row);
  const dirty = isDirty(row, draft);
  const conflicts = kbConflicts(rows, row.id, draft.key, whenText(draft.when));
  const canReset = row.source === "Custom" && row.defaultKey !== null;
  const canRemove = row.source !== "Default";
  const keyControl = !draft.recording && draft.key === row.key && row.key && !dirty
    ? `<button type="button" class="kb-key" data-kb-record aria-label="Edit shortcut for ${esc(commandTitle(row.command))}: ${esc(shortcutText(row.shortcut))}">${kbPill(row.key)}</button>`
    : `<input class="kb-input" data-keybinding-capture aria-label="Keybinding for ${esc(commandTitle(row.command))}" value="${draft.recording ? "" : esc(draft.key)}" placeholder="${draft.recording ? "Press shortcut" : "Unassigned"}" spellcheck="false" autocomplete="off">`;
  return `<div class="kb-row" data-kb-row="${esc(row.id)}"${anchor ? ` id="${esc(`keybinding-${row.command}`)}"` : ""}>
    <div class="kb-text"><span class="kb-title" title="${esc(row.command)}">${esc(commandTitle(row.command))}${row.source === "Custom" ? `<span class="kb-badge">Custom</span>` : ""}</span>
      <span class="kb-when"><span class="kb-when-label">When</span>${whenButton(commandTitle(row.command), whenText(draft.when))}</span></div>
    <div class="kb-control">${conflictIcon(conflicts)}${canReset || canRemove ? `<button type="button" class="kb-menu" data-kb-menu aria-label="Actions for ${esc(commandTitle(row.command))}">${icon("ellipsis")}</button>` : ""}
      ${dirty ? `<button type="button" class="kb-button primary" data-kb-save${!draft.key.trim() || draft.invalid ? " disabled" : ""}>Save</button>` : ""}${keyControl}</div></div>`;
}
const whenButton = (label, expression) =>
  `<button type="button" class="kb-when-button${expression ? "" : " muted"}" data-kb-when aria-label="Edit when clause for ${esc(label)}"><span>${esc(expression || "Always")}</span>${icon("chevron-down")}</button>`;
function conflictIcon(labels) {
  if (!labels.length) return "";
  const text = labels.length === 1 ? `Conflicts with ${labels[0]}.` : `Conflicts with ${labels.slice(0, 3).join(", ")}${labels.length > 3 ? ", and more" : ""}.`;
  return `<span class="kb-warn" role="img" tabindex="0" aria-label="${esc(text)}" title="${esc(`${text} The most recent matching binding wins when both conditions can apply.`)}">${icon("triangle-alert")}</span>`;
}
function newRowHtml(rows) {
  const draft = kb.adding;
  const label = draft.command ? commandTitle(draft.command) : "new keybinding";
  const commands = [...new Set([...DEFAULT_KEYBINDINGS.map((r) => r.command), ...keybindingRules.map((r) => r.command)])].sort((a, b) => commandTitle(a).localeCompare(commandTitle(b)));
  const canSave = draft.command && draft.key.trim() && !draft.invalid;
  return `<div class="kb-group"><div class="kb-row new" data-kb-new>
    <div class="kb-text"><span class="kb-title">New keybinding</span><span class="kb-when"><span class="kb-when-label">When</span>${whenButton(label, whenText(draft.when))}</span></div>
    <div class="kb-control"><select class="kb-select" data-kb-command aria-label="Command"><option value="" disabled${draft.command ? "" : " selected"}>Command</option>${commands.map((c) => `<option value="${esc(c)}"${c === draft.command ? " selected" : ""}>${esc(commandTitle(c))}</option>`).join("")}</select>
      ${conflictIcon(kbConflicts(rows, "new", draft.key, whenText(draft.when)))}
      <input class="kb-input" data-keybinding-capture aria-label="Keybinding for ${esc(label)}" value="${draft.recording ? "" : esc(draft.key)}" placeholder="${draft.recording ? "Press shortcut" : "Unassigned"}" spellcheck="false" autocomplete="off">
      <button type="button" class="kb-button primary" data-kb-save-new${canSave ? "" : " disabled"}>Save</button>
      <button type="button" class="kb-icon" data-kb-cancel aria-label="Cancel new keybinding" title="Cancel">${icon("x")}</button></div></div></div>`;
}
function renderKeybindings() {
  const root = document.getElementById("keybindings");
  if (!root) return;
  const rows = keybindingRows();
  const q = kb.query.trim().toLowerCase();
  const shown = q ? rows.filter((r) => [r.command, commandTitle(r.command), r.key, r.when, r.source].some((f) => f.toLowerCase().includes(q))) : rows;
  const groups = [...KB_GROUPS, ["other", "Other", []]].map(([id, title, prefixes]) => [id, title, shown.filter((r) => (KB_GROUPS.find(([, , p]) => p.includes(r.command.split(".")[0]))?.[0] ?? "other") === id)]).filter(([, , list]) => list.length);
  const anchored = new Set();
  const focused = document.activeElement?.closest?.("#keybindings") && document.activeElement.matches("[data-keybinding-capture]") ? document.activeElement.closest("[data-kb-row], [data-kb-new]") : null;
  const focusKey = focused?.dataset.kbRow ?? (focused ? "new" : null);
  root.querySelector(".kb-groups").innerHTML = `${kb.adding ? newRowHtml(rows) : ""}${groups.length
    ? groups.map(([id, title, list]) => `<div class="kb-group" id="keybindings-${id}"><h3>${esc(title)}</h3>${list.map((r) => keybindingRowHtml(r, rows, !anchored.has(r.command) && anchored.add(r.command))).join("")}</div>`).join("")
    : `<div class="kb-group"><p class="kb-empty">No keybindings match your search.</p></div>`}`;
  root.querySelector("[data-kb-add]").disabled = !!kb.adding;
  if (focusKey) (focusKey === "new" ? root.querySelector("[data-kb-new] [data-keybinding-capture]") : root.querySelector(`[data-kb-row="${CSS.escape(focusKey)}"] [data-keybinding-capture]`))?.focus();
}

function keybindingsKeydown(e) {
  const input = e.target.closest?.("[data-keybinding-capture]");
  if (!input) return;
  const draft = draftFor(input);
  if (e.key === "Tab" && !draft.recording) return;
  e.preventDefault();
  e.stopPropagation();
  const row = input.closest("[data-kb-row]");
  if (e.key === "Escape") {
    draft.key = row ? rowById(row.dataset.kbRow).key : "";
    draft.recording = false;
    input.blur();
    return renderKeybindings();
  }
  const chord = recordedChord(e);
  if (!chord) return;
  Object.assign(draft, { key: chord, recording: false });
  input.blur();
  renderKeybindings();
}
function keybindingsClick(e) {
  const row = e.target.closest("[data-kb-row]");
  const record = e.target.closest("[data-kb-record]");
  if (record) {
    kb.drafts.set(row.dataset.kbRow, { ...draftOf(rowById(row.dataset.kbRow)), recording: true });
    renderKeybindings();
    return document.querySelector(`#keybindings [data-kb-row="${CSS.escape(row.dataset.kbRow)}"] [data-keybinding-capture]`)?.focus();
  }
  if (e.target.closest("[data-kb-save]")) {
    const r = rowById(row.dataset.kbRow);
    const draft = draftOf(r);
    kb.drafts.delete(r.id);
    return upsertKeybinding({ command: r.command, key: draft.key, when: whenText(draft.when) || undefined, replace: { command: r.command, key: r.key, when: r.when || undefined } });
  }
  if (e.target.closest("[data-kb-save-new]")) {
    const { command, key, when } = kb.adding;
    kb.adding = null;
    return upsertKeybinding({ command, key, when: whenText(when) || undefined });
  }
  if (e.target.closest("[data-kb-cancel]")) {
    kb.adding = null;
    return renderKeybindings();
  }
  const menu = e.target.closest("[data-kb-menu]");
  if (menu) {
    const r = rowById(row.dataset.kbRow);
    const items = [
      r.source === "Custom" && r.defaultKey !== null && { label: "Reset to default", run: () => upsertKeybinding({ command: r.command, key: r.defaultKey, when: r.defaultWhen || undefined, replace: { command: r.command, key: r.key, when: r.when || undefined } }) },
      r.source !== "Default" && { label: "Remove", danger: true, run: () => removeKeybinding(r) },
    ].filter(Boolean);
    return openKbPopover(menu, `<div class="kb-actions">${items.map((it, i) => `<button type="button" data-kb-act="${i}"${it.danger ? ' class="danger"' : ""}>${esc(it.label)}</button>`).join("")}</div>`, (pop) =>
      pop.querySelectorAll("[data-kb-act]").forEach((b) => (b.onclick = () => (closeKbPopover(), items[b.dataset.kbAct].run()))));
  }
  const when = e.target.closest("[data-kb-when]");
  if (when) {
    const draft = draftFor(when);
    return openKbPopover(when, "", (pop) => whenBuilder(pop, draft));
  }
}

function openKbPopover(anchor, html, bind) {
  closeKbPopover();
  const pop = Object.assign(document.createElement("div"), { className: "kb-popover" });
  pop.innerHTML = html;
  document.body.append(pop);
  bind(pop);
  const at = anchor.getBoundingClientRect();
  const width = pop.offsetWidth;
  pop.style.left = `${Math.max(16, Math.min(at.left, innerWidth - width - 16))}px`;
  pop.style.top = `${Math.min(at.bottom + 6, innerHeight - pop.offsetHeight - 16)}px`;
  kb.popover = { pop, anchor };
}
function closeKbPopover() {
  kb.popover?.pop.remove();
  kb.popover = null;
}
addEventListener("mousedown", (e) => kb.popover && !kb.popover.pop.contains(e.target) && !kb.popover.anchor.contains(e.target) && (closeKbPopover(), renderKeybindings()), true);
addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || !kb.popover) return;
  e.preventDefault();
  e.stopPropagation();
  closeKbPopover();
  renderKeybindings();
}, true);

/** WhenExpressionBuilder: the expression as text, and the same conditions as rows to toggle, pick,
 * group and remove. Either edits `draft.when`. */
function whenBuilder(pop, draft) {
  const variables = kbWhenVariables();
  const condition = () => ({ type: "identifier", name: variables.find((v) => v !== "true" && v !== "false") ?? "terminalFocus" });
  const group = (op) => ({ type: op, left: condition(), right: { type: "not", node: condition() } });
  const flatten = (node, op) => (node.type === op ? [...flatten(node.left, op), ...flatten(node.right, op)] : [node]);
  const build = (children, op) => children.reduce((left, right) => (left ? { type: op, left, right } : right), null);
  const parts = (node) => (node.type === "identifier" ? { name: node.name, negated: false } : node.type === "not" && node.node.type === "identifier" ? { name: node.node.name, negated: true } : null);
  let text = whenText(draft.when);
  const syncAnchor = () => {
    const label = kb.popover?.anchor.querySelector("span");
    if (label && !draft.invalid) label.textContent = whenText(draft.when) || "Always";
  };
  const set = (value) => {
    draft.when = value ?? undefined;
    draft.invalid = false;
    text = whenText(draft.when);
    draw();
    syncAnchor();
  };
  const removeLabel = (node, depth) => (depth === 0 ? "Clear all conditions" : parts(node) ? "Remove condition" : "Remove group and its conditions");
  const remove = (label) => `<button type="button" class="kb-icon" data-act="remove" aria-label="${label}" title="${label}">${icon("x")}</button>`;
  /** Each node draws with the edits that replace it, kept by index in `edits`. */
  const edits = [];
  const nodeHtml = (node, depth, replace, drop) => {
    const at = edits.push({ node, replace, drop }) - 1;
    const c = parts(node);
    if (c) {
      const options = variables.includes(c.name) ? variables : [c.name, ...variables];
      return `<div class="kb-cond" data-node="${at}"><button type="button" class="kb-toggle" data-act="not" aria-pressed="${c.negated}" aria-label="Negate ${esc(c.name)}">Not</button>
        <select data-act="var" aria-label="Condition">${options.map((v) => `<option${v === c.name ? " selected" : ""}>${esc(v)}</option>`).join("")}</select>${variables.includes(c.name) ? "" : `<span class="kb-warn" title="Pinned does not recognize this condition yet. It can still be saved, but it may not match unless the runtime provides it.">${icon("triangle-alert")}</span>`}${drop ? remove(removeLabel(node, depth)) : ""}</div>`;
    }
    if (node.type === "not")
      return `<div class="kb-group-node" data-node="${at}"><div class="kb-group-head"><button type="button" class="kb-toggle" data-act="unnot" aria-pressed="true" aria-label="Negate group">Not</button>${drop ? remove(removeLabel(node, depth)) : ""}</div>
        <div class="kb-branch">${nodeHtml(node.node, depth + 1, (next) => replace({ type: "not", node: next }), null)}</div></div>`;
    const op = node.type;
    const children = flatten(node, op);
    const swap = (i, next) => replace(build(children.map((ch, j) => (j === i ? next : ch)), op));
    const cut = (i) => replace(build(children.filter((_, j) => j !== i), op) ?? condition());
    return `<div class="kb-group-node" data-node="${at}"><div class="kb-group-head"><select data-act="op" aria-label="Operator"><option${op === "and" ? " selected" : ""}>and</option><option${op === "or" ? " selected" : ""}>or</option></select>
      <button type="button" class="kb-button" data-act="add-cond">${icon("plus")}Condition</button><button type="button" class="kb-button" data-act="add-group">${icon("plus")}Group</button>${drop ? remove(removeLabel(node, depth)) : ""}</div>
      ${children.map((ch, i) => `<div class="kb-branch">${nodeHtml(ch, depth + 1, (next) => swap(i, next), () => cut(i))}</div>`).join("")}</div>`;
  };
  function draw() {
    edits.length = 0;
    const parsed = text.trim() ? parseWhen(text.trim()) : null;
    const error = text.trim() && !parsed;
    const unknown = [];
    const collect = (n) => n && (n.type === "identifier" ? !variables.includes(n.name) && unknown.push(n.name) : n.type === "not" ? collect(n.node) : (collect(n.left), collect(n.right)));
    collect(parsed);
    const tree = draft.when ? nodeHtml(draft.when, 0, set, () => set(null)) : `<div class="kb-empty-when"><button type="button" class="kb-button primary" data-act="root-cond">${icon("plus")}Condition</button><button type="button" class="kb-button" data-act="root-group">${icon("plus")}Group</button></div>`;
    pop.innerHTML = `<div class="kb-when-builder"><div class="kb-builder-head"><b>When</b><span><button type="button" class="kb-button" data-act="root-cond">${icon("plus")}Condition</button><button type="button" class="kb-button" data-act="root-group">${icon("plus")}Group</button></span></div>
      <label class="kb-expr"><input value="${esc(text)}" placeholder="Always" aria-label="When expression" aria-invalid="${!!error}" spellcheck="false" autocomplete="off">${unknown.length ? `<span class="kb-warn" title="${esc(unknown.length === 1 ? `Unknown condition: ${unknown[0]}` : `Unknown conditions: ${unknown.join(", ")}`)}">${icon("triangle-alert")}</span>` : ""}</label>
      ${error ? `<p class="kb-error">${icon("circle-x")}Use variables with !, &&, ||, and parentheses.</p>` : ""}
      <div class="kb-tree">${tree}${error ? `<p class="kb-blocked">Fix the expression above to continue editing visually.</p>` : ""}</div></div>`;
    const input = pop.querySelector(".kb-expr input");
    input.oninput = () => {
      text = input.value;
      const ast = text.trim() ? parseWhen(text.trim()) : null;
      draft.invalid = !!text.trim() && !ast;
      if (!draft.invalid) draft.when = ast ?? undefined;
      syncAnchor();
      const caret = input.selectionStart;
      draw();
      const again = pop.querySelector(".kb-expr input");
      again.focus();
      again.setSelectionRange(caret, caret);
    };
  }
  pop.addEventListener("click", (e) => {
    const b = e.target.closest("[data-act]");
    if (!b || b.tagName === "SELECT") return;
    const act = b.dataset.act;
    const edit = edits[b.closest("[data-node]")?.dataset.node];
    if (act === "root-cond") set(draft.when ? { type: "and", left: draft.when, right: condition() } : condition());
    else if (act === "root-group") set(draft.when ? { type: "and", left: draft.when, right: group("or") } : group("or"));
    else if (act === "remove") edit.drop();
    else if (act === "not") {
      const c = parts(edit.node);
      edit.replace(c.negated ? { type: "identifier", name: c.name } : { type: "not", node: { type: "identifier", name: c.name } });
    } else if (act === "unnot") edit.replace(edit.node.node);
    else if (act === "add-cond") edit.replace(build([...flatten(edit.node, edit.node.type), condition()], edit.node.type));
    else if (act === "add-group") edit.replace(build([...flatten(edit.node, edit.node.type), group(edit.node.type === "and" ? "or" : "and")], edit.node.type));
  });
  pop.addEventListener("change", (e) => {
    const edit = edits[e.target.closest("[data-node]")?.dataset.node];
    if (e.target.dataset.act === "var") {
      const c = parts(edit.node);
      const next = { type: "identifier", name: e.target.value };
      edit.replace(c.negated ? { type: "not", node: next } : next);
    } else if (e.target.dataset.act === "op") edit.replace(build(flatten(edit.node, edit.node.type), e.target.value));
  });
  draw();
}

/** ModelPickerContent's jump keys: ⌘1-9 pick the first nine models the open picker lists, each
 * row showing its key. */
function markModelJumps(list) {
  [...list.querySelectorAll(".model[data-row]:not(.legacy)")].slice(0, 9).forEach((row, i) => {
    const label = bindingLabel(`modelPicker.jump.${i + 1}`, { modelPickerOpen: true });
    if (label) row.querySelector(".fav")?.insertAdjacentHTML("beforebegin", `<kbd class="jump">${esc(label)}</kbd>`);
  });
}
for (let n = 1; n <= 9; n++) onGuiCommand(`modelPicker.jump.${n}`, () => composerMenu.querySelectorAll(".list .model[data-row]:not(.legacy)")[n - 1]?.click());

/** The keys a focused terminal takes, as T3 Code's terminal drawer: ⌘D and ⌘⇧D split it, ⌘N opens
 * another terminal beside it and ⌘W closes it. A panel Terminal tab splits and closes its own
 * terminals and opens another tab; in the drawer a shell opens, and claude's own terminal hides the
 * drawer, since it can't close. */
const focusedTerminal = () => {
  const el = document.activeElement;
  if (el?.closest?.("#panel-body")) return { panel: true };
  const host = el?.closest?.("#terms .term");
  return { shell: scriptShells.find((s) => s.el === host) ?? null };
};
const nextTerminal = (tab) => {
  const n = scriptShells.filter((s) => s.tab === tab && /^Terminal( \d+)?$/.test(s.name)).length + 1;
  return { name: n === 1 ? "Terminal" : `Terminal ${n}`, command: "" };
};
onGuiCommand("terminal.new", () => {
  if (!active) return;
  if (focusedTerminal().panel) return newTerminalSurface(active);
  runScript(active, nextTerminal(active)).then((shell) => shell?.term.focus());
});
/** ⌘D and ⌘⇧D in the drawer: another terminal beside the focused one, or under it, four at most. */
const MAX_TERMINALS_PER_GROUP = 4;
function splitDrawerTerminal(vertical) {
  const at = focusedTerminal();
  if (!active) return;
  if (at.panel) return shownTerminal() && splitTerminal(active, shownTerminal(), vertical ? "vertical" : "horizontal");
  const t = threadOf(active);
  const group = (at.shell ? at.shell.group : t.claudeGroup) ?? { vertical, members: [at.shell] };
  if (group.members.length >= MAX_TERMINALS_PER_GROUP) return;
  if (at.shell) at.shell.group = group;
  else t.claudeGroup = group;
  runScript(active, nextTerminal(active), null, group).then((shell) => shell?.term.focus());
}
onGuiCommand("terminal.split", () => splitDrawerTerminal(false));
onGuiCommand("terminal.splitVertical", () => splitDrawerTerminal(true));
onGuiCommand("terminal.close", () => {
  const at = focusedTerminal();
  if (at.panel) return closeFocusedTerminal() || closeSurface();
  if (at.shell) return closeShell(at.shell);
  toggleDrawer(false);
});
