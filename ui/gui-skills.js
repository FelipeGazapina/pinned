const skillsUi = { byMachine: new Map(), filter: "", open: null, loading: false };

const UPDATE_REQUIRED = /can't answer that/;
const copyId = (c) => `${c.machine}\n${c.dir}`;
const scopeId = (s) => `${s.kind}\n${s.checkout ?? s.plugin ?? ""}`;
const rowId = (c) => `${scopeId(c.scope)}\n${c.key}`;

async function fetchSkills(only = onlineMachines()) {
  skillsUi.loading = true;
  renderSkillsPage();
  const got = await Promise.allSettled(only.map((m) => callOn(m, { cmd: "skills" })));
  only.forEach((m, i) => skillsUi.byMachine.set(m, got[i].status === "fulfilled" ? got[i].value : { error: String(got[i].reason) }));
  for (const m of skillsUi.byMachine.keys()) if (!machines.get(m)?.view.online) skillsUi.byMachine.delete(m);
  const o = skillsUi.open;
  const row = o?.got && openRow();
  if (row) refreshChecks(o, row);
  skillsUi.loading = false;
  renderSkillsPage();
}

/** One row per scope and key across machines. `agree` when every copy has the same tree. */
function skillRows() {
  const rows = new Map();
  for (const [machine, listing] of skillsUi.byMachine) for (const c of listing.copies ?? []) {
    const id = rowId(c);
    const row = rows.get(id) ?? { id, key: c.key, scope: c.scope, description: c.description, copies: [] };
    row.copies.push({ machine, ...c });
    rows.set(id, row);
  }
  const order = { personal: 0, project: 1, plugin: 2, builtin: 3 };
  return [...rows.values()]
    .map((r) => ({ ...r, agree: new Set(r.copies.map((c) => c.tree)).size === 1 }))
    .sort((a, b) => order[a.scope.kind] - order[b.scope.kind] || a.key.localeCompare(b.key));
}

const baseName = (path) => String(path).split(/[\\/]/).filter(Boolean).pop() ?? path;
const readerNames = (c, provider) => [...new Set(c.readers.filter((r) => r.provider === provider).map((r) => r.account))].join(", ");
function placeLabel(c) {
  return [...new Set([c.place, ...(c.also ?? [])].map((p) => onePlaceLabel(c, p)))].join(" + ");
}
function onePlaceLabel(c, p) {
  const who = (provider) => (readerNames(c, provider) ? ` (${readerNames(c, provider)})` : "");
  return {
    "claude-personal": `Claude${who("claude")}`,
    "claude-project": `Project ${baseName(p.checkout)}`,
    "plugin-cache": `Plugin cache ${p.version ? `v${p.version}` : ""}`.trim(),
    "plugin-source": `Plugin source in ${p.marketplace}`,
    "codex-personal": `Codex${who("codex")}`,
    "cursor-personal": "Cursor",
    "cursor-plugin": `Cursor plugin ${p.plugin}`,
    agents: "~/.agents",
    "claude-account-synced": "claude.ai",
    "synced-plugin": "Synced plugin",
    pinned: "Pinned",
    "codex-system": "Codex built-in",
    "cursor-builtin": "Cursor built-in",
  }[p.kind] ?? p.kind;
}
const scopeLabel = (s) => ({ personal: "Personal", project: `Project ${baseName(s.checkout ?? "")}`, plugin: `Plugin ${s.plugin}`, builtin: "Built in" })[s.kind];
const whereLabel = (c) => `${skillsUi.byMachine.size > 1 || c.machine !== LOCAL ? `${machineName(c.machine)} · ` : ""}${placeLabel(c)}`;

function machineNotesHtml() {
  const notes = [...skillsUi.byMachine].filter(([, l]) => l.error).map(([m, l]) =>
    `<p class="skills-note${UPDATE_REQUIRED.test(l.error) ? "" : " error"}">${esc(machineName(m))}: ${UPDATE_REQUIRED.test(l.error) ? "Update required. This pinned does not list skills yet." : esc(l.error)}</p>`);
  const hollow = [...skillsUi.byMachine].flatMap(([m, l]) => (l.hollow ?? []).map((dir) => `<li><b>${esc(machineName(m))}</b> ${esc(dir)}</li>`));
  if (hollow.length) notes.push(`<details class="skills-note warn"><summary>${hollow.length} skill folder${hollow.length > 1 ? "s have" : " has"} no SKILL.md</summary><ul>${hollow.join("")}</ul></details>`);
  return notes.join("");
}

function skillRowHtml(r) {
  const open = skillsUi.open?.row === r.id;
  const places = r.copies.map((c) => `<span class="place${c.edit === "no" ? " fixed" : ""}" title="${esc(c.dir)}">${esc(whereLabel(c))}</span>`).join("");
  const agree = r.copies.length < 2 ? "" : r.agree ? `<span class="agree">${icon("check")} Same everywhere</span>` : `<span class="agree differ">${new Set(r.copies.map((c) => c.tree)).size} versions</span>`;
  return `<button type="button" class="skill-row${open ? " selected" : ""}" data-skill-row="${esc(r.id)}">
    <span class="name">${icon("sparkles")}<b>${esc(r.key)}</b>${agree}</span>
    <span class="about">${esc(r.description)}</span><span class="places">${places}</span></button>`;
}

function listHtml(rows) {
  const q = skillsUi.filter.trim().toLowerCase();
  const shown = q ? rows.filter((r) => `${r.key} ${r.description}`.toLowerCase().includes(q)) : rows;
  const groups = new Map();
  for (const r of shown) (groups.get(scopeId(r.scope)) ?? groups.set(scopeId(r.scope), { scope: r.scope, rows: [] }).get(scopeId(r.scope))).rows.push(r);
  if (!shown.length) return `<p class="muted note">${skillsUi.loading ? "Looking for skills..." : rows.length ? "No skill matches." : "No skills found."}</p>`;
  return [...groups.values()].map((g) => `<section class="skill-group"><h3>${esc(scopeLabel(g.scope))}<span class="count">${g.rows.length}</span><span class="rule"></span></h3>${g.rows.map(skillRowHtml).join("")}</section>`).join("");
}

/** A unified patch of `before` against `after`, from their longest common run of lines, for `ui/diff.js`. */
function textPatch(path, before, after) {
  const lines = (text) => (text ? text.replace(/\n$/, "").split("\n") : []);
  const a = lines(before);
  const b = lines(after);
  const out = [];
  if (a.length * b.length > 4e6) out.push(...a.map((l) => `-${l}`), ...b.map((l) => `+${l}`));
  else {
    const len = Array.from({ length: a.length + 1 }, () => new Uint32Array(b.length + 1));
    for (let i = a.length - 1; i >= 0; i--) for (let j = b.length - 1; j >= 0; j--) len[i][j] = a[i] === b[j] ? len[i + 1][j + 1] + 1 : Math.max(len[i + 1][j], len[i][j + 1]);
    let i = 0;
    let j = 0;
    while (i < a.length || j < b.length) {
      if (i < a.length && j < b.length && a[i] === b[j]) out.push(` ${a[i++]}`), j++;
      else if (i < a.length && (j === b.length || len[i + 1][j] >= len[i][j + 1])) out.push(`-${a[i++]}`);
      else out.push(`+${b[j++]}`);
    }
  }
  return `diff --git a/${path} b/${path}\n--- a/${path}\n+++ b/${path}\n@@ -1,${a.length} +1,${b.length} @@\n${out.join("\n")}\n`;
}
const diffHtml = (path, before, after) =>
  `<div class="diff unified">${filesHtml(parseDiff(textPatch(path, before, after)), `skill:${path}`, { contents: () => after.replace(/\n$/, "").split("\n") })}</div>`;

const openRow = () => skillRows().find((r) => r.id === skillsUi.open?.row);
const editText = (o) => o.draft ?? o.got?.text ?? "";
const skillDirty = (o) => o?.got != null && o.draft != null && o.draft !== o.got.text;

function openChecks(o, row) {
  o.treeShown = new Map(row.copies.map((c) => [copyId(c), c.tree]));
  o.treeShown.set(copyId(o.copy), o.got.tree);
  o.checked = new Set(row.copies.filter((c) => c.edit !== "no" && o.treeShown.get(copyId(c)) === o.got.tree).map(copyId));
  o.changed = new Set();
}

function refreshChecks(o, row) {
  for (const c of row.copies) {
    const id = copyId(c);
    if (!o.treeShown.has(id)) o.treeShown.set(id, c.tree);
    else if (o.treeShown.get(id) !== c.tree) {
      o.checked.delete(id);
      o.compared.delete(id);
      o.changed.add(id);
      o.treeShown.set(id, c.tree);
    }
  }
}

function sawCopy(o, c, got) {
  o.compared.set(copyId(c), got);
  if (got.tree) o.treeShown.set(copyId(c), got.tree);
  o.changed.delete(copyId(c));
}

function writtenAnyway(o, row, c) {
  if (o.checked.has(copyId(c))) return "";
  const checked = row.copies.filter((x) => o.checked.has(copyId(x)));
  if (c.linked && checked.some((x) => x.linked === c.linked)) return "Sync replaces this copy with this save, checked or not.";
  const follows = (x) => x.machine === c.machine && x.place.kind === "plugin-source" && x.place.plugin === c.place.plugin && x.place.home === c.place.home && baseName(x.dir) === baseName(c.dir);
  if (c.place.kind === "plugin-cache" && !c.place.loaded && checked.some(follows)) return "Saving the plugin's source copy writes this copy too.";
  return "";
}

const copyUnseen = (o, c) => !o.compared.has(copyId(c)) && (o.changed.has(copyId(c)) || (copyId(c) !== copyId(o.copy) && c.tree !== o.got.tree));

function copyCheck(o, row, c) {
  const id = copyId(c);
  const fixed = c.edit === "no";
  const hidden = !fixed && copyUnseen(o, c);
  const disabled = fixed || hidden;
  const anyway = writtenAnyway(o, row, c);
  const why = fixed
    ? c.why
    : o.changed.has(id) && hidden
      ? "Changed since you opened it. Compare it first."
      : hidden
        ? "Differs from the copy you opened. Compare it first."
        : anyway || (c.edit === "lost-on-update" ? "Lost on the next plugin update." : "");
  return { checked: !disabled && o.checked.has(id), disabled, compare: hidden || o.compared.has(id), why };
}

const saveBlocker = (o, row) => row.copies.find((c) => c.edit !== "no" && writtenAnyway(o, row, c) && copyUnseen(o, c));

function saveTargets(o, row) {
  const byMachine = new Map();
  for (const c of row.copies) {
    if (!copyCheck(o, row, c).checked) continue;
    byMachine.set(c.machine, [...(byMachine.get(c.machine) ?? []), { dir: c.dir, since: o.treeShown.get(copyId(c)) }]);
  }
  return byMachine;
}

function skillsChanged(machine) {
  if (document.body.dataset.page !== "skills" || !machines.get(machine)?.view.online) return;
  (skillsChanged.machines ??= new Set()).add(machine);
  clearTimeout(skillsChanged.timer);
  skillsChanged.timer = setTimeout(() => {
    const changed = [...skillsChanged.machines];
    skillsChanged.machines.clear();
    fetchSkills(changed);
  }, 300);
}

/** Opens `copy`: reads it, and checks every editable copy whose tree is the one read. */
async function openSkill(row, copy, file = "SKILL.md") {
  skillsUi.opened = (skillsUi.opened ?? 0) + 1;
  skillsUi.open = { id: skillsUi.opened, row: row.id, copy, file, got: null, draft: null, view: "edit", checked: new Set(), treeShown: new Map(), changed: new Set(), compared: new Map(), report: null, saving: false, leaving: null };
  renderSkillsPage();
  const o = skillsUi.open;
  try {
    o.got = await callOn(copy.machine, { cmd: "read_skill", dir: copy.dir, file });
  } catch (e) {
    o.error = String(e);
    return renderSkillsPage();
  }
  openChecks(o, openRow() ?? row);
  if (skillsUi.open === o) renderSkillsPage();
}

/** Runs `go` now, or once the user agrees to drop the unsaved draft. */
function leaveThen(go) {
  const o = skillsUi.open;
  if (!skillDirty(o)) return go();
  o.leaving = go;
  renderSkillsPage();
}

async function compareCopy(c) {
  const o = skillsUi.open;
  try {
    sawCopy(o, c, await callOn(c.machine, { cmd: "read_skill", dir: c.dir, file: o.file }));
  } catch (e) {
    o.compared.set(copyId(c), { error: String(e) });
  }
  o.view = `compare:${copyId(c)}`;
  renderSkillsPage();
}

function checklistHtml(row, o) {
  const items = row.copies.map((c) => {
    const id = copyId(c);
    const state = copyCheck(o, row, c);
    const note = c.place.kind === "plugin-cache" && !c.place.loaded ? " Sessions here load the plugin's source copy. A save to the source writes this copy too, and plugin sync sends it to your other machines." : "";
    return `<li class="${c.edit === "no" ? "fixed" : ""}"><label><input type="checkbox" data-skill-check="${esc(id)}"${state.checked ? " checked" : ""}${state.disabled ? " disabled" : ""}>
      <span class="where">${esc(whereLabel(c))}</span>${copyId(o.copy) === id ? `<span class="tag">open</span>` : ""}${c.edited ? `<span class="tag warn">edited</span>` : ""}</label>
      ${state.compare ? `<button type="button" class="link" data-skill-compare="${esc(id)}">Compare</button>` : ""}
      <span class="why">${esc(state.why + note)}</span><span class="dir" title="${esc(c.dir)}">${esc(c.dir)}</span></li>`;
  });
  return `<section class="save-to"><h3>Save to</h3><ul>${items.join("")}</ul></section>`;
}

function reportHtml(report) {
  const copies = report.flatMap((r) => (r.copies ?? []).map((c) => ({ machine: r.machine, ...c })));
  const sessions = report.flatMap((r) => (r.sessions ?? []).map((s) => ({ machine: r.machine, ...s })));
  const saved = copies.filter((c) => c.outcome === "saved");
  const count = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
  const now = sessions.filter((s) => s.nudge === "reloaded").length;
  const later = sessions.filter((s) => s.nudge === "after-turn").length;
  const restart = sessions.filter((s) => s.nudge === "next-start");
  const parts = [`Saved ${count(saved.length, "copy", "copies")} on ${count(new Set(saved.map((c) => c.machine)).size, "machine")}.`];
  const reach = [now && `Reloaded ${count(now, "session")}`, later && `${later} reload${later === 1 ? "s" : ""} after ${later === 1 ? "its" : "their"} turn`, restart.length && `${count(restart.length, "Codex or Cursor session")} need${restart.length === 1 ? "s" : ""} a restart`].filter(Boolean);
  if (reach.length) parts.push(`${reach.join(", ")}.`);
  else if (saved.length) parts.push("No running session loads it.");
  const where = (c) => {
    const listed = skillRows().flatMap((r) => r.copies).find((x) => x.machine === c.machine && x.dir === c.dir);
    return listed ? whereLabel(listed) : `${machineName(c.machine)} · ${c.dir}`;
  };
  const problems = [
    ...report.filter((r) => r.error).map((r) => `<li>${esc(machineName(r.machine))}: ${esc(UPDATE_REQUIRED.test(r.error) ? "Update required." : r.error)}</li>`),
    ...copies.filter((c) => c.outcome !== "saved").map((c) => `<li>${esc(where(c))}: ${esc({ changed: "Changed on disk since you opened it. Not written.", "read-only": c.why, missing: "No longer listed. Not written.", failed: c.error }[c.outcome] ?? c.outcome)}</li>`),
  ];
  const buttons = restart.map((s) => `<button type="button" class="outline small" data-skill-restart="${esc(`${s.machine}\n${s.tab}`)}">Restart ${esc(s.title || s.tab)}</button>`).join("");
  return `<div class="skills-report" role="status"><p>${esc(parts.join(" "))}</p>${buttons ? `<p class="buttons">${buttons}</p>` : ""}${problems.length ? `<ul>${problems.join("")}</ul>` : ""}</div>`;
}

/** The editor pane in three parts, so a refresh can redraw the parts around an open editor and leave
 * the textarea, with its undo history and caret, alone. */
function editorParts(row, o) {
  const head = `<div class="skill-head"><h2>${esc(row.key)}</h2><span class="muted">${esc(whereLabel(o.copy))}</span><span class="spacer"></span>
    ${o.got?.files?.length > 1 ? `<select data-skill-file aria-label="File">${o.got.files.map((f) => `<option${f === o.file ? " selected" : ""}>${esc(f)}</option>`).join("")}</select>` : ""}</div>`;
  if (o.error) return { top: `${head}<p class="skills-note error">${esc(o.error)}</p>`, body: null, bottom: "" };
  if (!o.got) return { top: `${head}<div class="file-loading" role="status" aria-label="Loading skill"><span class="spinner"></span></div>`, body: null, bottom: "" };
  const locked = o.copy.edit === "no";
  const text = editText(o);
  const compare = o.view.startsWith("compare:") ? o.compared.get(o.view.slice(8)) : null;
  const tabs = [["edit", "Edit"], ["preview", "Preview"], ...(o.got.original != null ? [["original", "Changes from the installed plugin"]] : [])];
  if (compare) tabs.push([o.view, "Compare"]);
  const bar = `<div class="skill-tabs" role="tablist">${tabs.map(([v, label]) => `<button type="button" role="tab" data-skill-view="${esc(v)}" aria-selected="${o.view === v}">${esc(label)}</button>`).join("")}</div>`;
  let body;
  if (o.view === "preview") body = `<div class="md file-markdown">${markdown(text.replace(/^---\n[\s\S]*?\n---\n/, ""))}</div>`;
  else if (o.view === "original") body = diffHtml(o.file, o.got.original, text);
  else if (compare?.error) body = `<p class="skills-note error">${esc(compare.error)}</p>`;
  else if (compare) body = `<p class="muted">That copy's ${esc(o.file)} against the text you are saving.</p>${diffHtml(o.file, compare.text, text)}`;
  else body = editorHtml(o.file, text, { locked, attrs: ` data-skill-editor="${o.id}"` });
  const targets = [...saveTargets(o, row).values()].flat().length;
  const checkedRows = row.copies.filter((c) => copyCheck(o, row, c).checked);
  const lost = checkedRows.some((c) => c.edit === "lost-on-update") && !checkedRows.some((c) => c.place.kind === "plugin-source");
  const blocker = locked ? null : saveBlocker(o, row);
  const notes = [
    o.leaving ? `<p class="skills-note warn">${esc(o.file)} has changes you have not saved. <button type="button" class="link" data-skill-leave="discard">Discard them</button> <button type="button" class="link" data-skill-leave="stay">Keep editing</button></p>` : "",
    locked ? `<p class="skills-note">${esc(o.copy.why)}</p>` : "",
    lost ? `<p class="skills-note warn">A checked plugin cache copy is replaced on the next plugin update, and this edit with it.</p>` : "",
    blocker ? `<p class="skills-note warn">This save also replaces ${esc(whereLabel(blocker))}, which differs from the copy you opened. Compare it before you save.</p>` : "",
  ].join("");
  const save = locked ? "" : `<button type="button" class="primary" data-skill-save${!skillDirty(o) || !targets || o.saving || blocker ? " disabled" : ""} title="Save (${MOD}S)">${o.saving ? "Saving..." : `Save to ${targets} ${targets === 1 ? "copy" : "copies"}`}</button>`;
  return { top: `${head}${bar}${notes}`, body, bottom: `<div class="skill-actions">${save}</div>${o.report ? reportHtml(o.report) : ""}${checklistHtml(row, o)}` };
}

function editorPaneHtml(row, o) {
  if (!row) return `<p class="muted note">This skill is no longer listed.</p>`;
  const { top, body, bottom } = editorParts(row, o);
  return `<div class="skill-top">${top}</div>${body == null ? "" : `<div class="skill-body">${body}</div>`}<div class="skill-bottom">${bottom}</div>`;
}

function renderSkillsPage() {
  if (document.body.dataset.page !== "skills") return;
  const rows = skillRows();
  const o = skillsUi.open;
  const row = o && rows.find((r) => r.id === o.row);
  const shown = pageView.querySelector(".skills-editor [data-skill-editor]");
  const parts = row && editorParts(row, o);
  if (shown && parts?.body?.includes(`data-skill-editor="${o.id}"`) && shown.dataset.skillEditor === String(o.id)) {
    pageView.querySelector(".skills-notes").innerHTML = machineNotesHtml();
    pageView.querySelector(".skills-rows").innerHTML = listHtml(rows);
    pageView.querySelector(".skill-top").innerHTML = parts.top;
    pageView.querySelector(".skill-bottom").innerHTML = parts.bottom;
    pageView.querySelector("[data-skills-refresh]")?.classList.toggle("spinning", skillsUi.loading);
    return;
  }
  const editing = document.activeElement?.matches?.("#page-view .code-edit") && [document.activeElement.selectionStart, document.activeElement.selectionEnd];
  const filtering = document.activeElement?.matches?.("[data-skills-filter]") && document.activeElement.selectionStart;
  const refresh = `<button type="button" class="icon refresh${skillsUi.loading ? " spinning" : ""}" data-skills-refresh aria-label="Refresh skills">${icon("refresh-cw")}</button>`;
  setPage(`${headHtml("Skills", `<span class="spacer"></span>${refresh}`)}
    <div class="page-body"><div class="page-wrap skills${o ? " with-editor" : ""}"><div class="skills-notes">${machineNotesHtml()}</div>
      <div class="skills-split"><div class="skills-list"><input type="search" class="skills-filter" data-skills-filter placeholder="Filter skills" value="${esc(skillsUi.filter)}" aria-label="Filter skills"><div class="skills-rows">${listHtml(rows)}</div></div>
      ${o ? `<div class="skills-editor">${editorPaneHtml(row, o)}</div>` : ""}</div></div></div>`);
  if (editing) {
    const area = pageView.querySelector(".code-edit");
    area?.focus();
    area?.setSelectionRange(editing[0], editing[1]);
  }
  if (filtering !== false && filtering != null) {
    const input = pageView.querySelector("[data-skills-filter]");
    input.focus();
    input.setSelectionRange(filtering, filtering);
  }
}

async function saveSkill() {
  const o = skillsUi.open;
  const row = openRow();
  if (!skillDirty(o) || o.saving || !row || saveBlocker(o, row)) return;
  const byMachine = saveTargets(o, row);
  if (!byMachine.size) return;
  o.saving = true;
  renderSkillsPage();
  const ms = [...byMachine.keys()];
  const text = o.draft;
  const got = await Promise.allSettled(ms.map((m) => callOn(m, { cmd: "save_skill", file: o.file, text, copies: byMachine.get(m) })));
  o.report = ms.map((m, i) => ({ machine: m, ...(got[i].status === "fulfilled" ? got[i].value : { error: String(got[i].reason) }) }));
  o.saving = false;
  for (const r of o.report) for (const c of r.copies ?? []) if (c.outcome === "saved") o.treeShown.set(`${r.machine}\n${c.dir}`, c.tree);
  const mine = o.report.find((r) => r.machine === o.copy.machine)?.copies?.find((c) => c.dir === o.copy.dir);
  if (mine?.outcome === "saved") {
    o.got = { ...o.got, text, tree: mine.tree };
    if (o.draft === text) o.draft = null;
  }
  await fetchSkills();
}

pageView.addEventListener("click", (e) => {
  if (document.body.dataset.page !== "skills") return;
  const t = e.target.closest("[data-skill-row], [data-skill-view], [data-skill-save], [data-skill-compare], [data-skill-leave], [data-skill-restart], [data-skills-refresh]");
  if (!t) return;
  const o = skillsUi.open;
  if (t.dataset.skillRow != null) {
    const row = skillRows().find((r) => r.id === t.dataset.skillRow);
    const pick = row.copies.find((c) => c.machine === LOCAL && c.edit === "yes") ?? row.copies.find((c) => c.edit !== "no") ?? row.copies[0];
    return leaveThen(() => openSkill(row, pick));
  }
  if (t.dataset.skillLeave != null) {
    const go = o.leaving;
    o.leaving = null;
    return t.dataset.skillLeave === "discard" ? go() : renderSkillsPage();
  }
  if (t.dataset.skillView != null) return (o.view = t.dataset.skillView), renderSkillsPage();
  if (t.dataset.skillSave != null) return saveSkill();
  if (t.dataset.skillCompare != null) return compareCopy(openRow().copies.find((c) => copyId(c) === t.dataset.skillCompare));
  if (t.dataset.skillsRefresh != null) return fetchSkills();
  const [machine, tab] = t.dataset.skillRestart.split("\n");
  const open = tabByKey(keyOf(machine, tab));
  if (open) restartAgent(open);
  else callOn(machine, { cmd: "restart_claude", tab, cols: 120, rows: 32 }).then(() => tell("<b>Agent session will restart</b>", 3000), (err) => tell(esc(String(err))));
});
pageView.addEventListener("change", (e) => {
  if (document.body.dataset.page !== "skills") return;
  const o = skillsUi.open;
  if (e.target.matches("[data-skill-check]")) {
    if (e.target.checked) o.checked.add(e.target.dataset.skillCheck);
    else o.checked.delete(e.target.dataset.skillCheck);
    return renderSkillsPage();
  }
  if (e.target.matches("[data-skill-file]")) {
    const file = e.target.value;
    return leaveThen(() => openSkill(openRow(), o.copy, file));
  }
});
let skillDraftTimer = null;
pageView.addEventListener("input", (e) => {
  if (document.body.dataset.page !== "skills") return;
  if (e.target.matches("[data-skills-filter]")) return (skillsUi.filter = e.target.value), renderSkillsPage();
  if (!e.target.matches(".code-edit")) return;
  const o = skillsUi.open;
  const was = skillDirty(o);
  o.draft = e.target.value;
  if (was !== skillDirty(o)) return renderSkillsPage();
  clearTimeout(skillDraftTimer);
  skillDraftTimer = setTimeout(() => {
    const lines = pageView.querySelector(".skills-editor .code-lines");
    if (skillsUi.open !== o || !lines || o.draft == null) return;
    lines.innerHTML = plainRows(o.file, o.draft);
    lines.closest(".file").style.setProperty("--digits", String(o.draft.split("\n").length).length);
  }, 60);
});
pageView.addEventListener("keydown", (e) => {
  if (document.body.dataset.page === "skills" && e.target.matches(".code-edit") && e.key === "Tab" && !e.shiftKey) {
    e.preventDefault();
    document.execCommand("insertText", false, "  ");
  }
});

SHELL_PAGES.skills = { show: () => (renderSkillsPage(), fetchSkills()) };
onGuiKey("s", () => saveSkill(), { when: () => document.body.dataset.page === "skills" && !!document.activeElement?.closest?.("#page-view .code-edit") });
