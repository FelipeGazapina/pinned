document.body.insertAdjacentHTML("afterbegin", `<div id="pierre-diffs-sprite" aria-hidden="true">${PIERRE_DIFFS.sprite}</div>`);

/** InteractionManager.js's line selection: pressing a line number selects it and dragging extends
 * it, pressing the one selected line clears it, and Shift extends from the nearer end. Pressing the
 * gutter's "+" and dragging picks the lines a new comment covers, starting from the selection.
 * Nothing is selected while a comment is being written, as AnnotatableCodeView turns it off. */
let diffDrag = null;
/** The drag's file as drawn now, since the panel may draw again while a drag goes on. */
const dragFile = (d) => panelBody.querySelector(`.diff[data-key="${CSS.escape(d.key)}"] .file[data-path="${CSS.escape(d.path)}"]`);
function markSelection(d) {
  const file = dragFile(d);
  if (!file) return;
  const [start, end] = dragRange(d);
  const side = d.side;
  for (const no of file.querySelectorAll(".dl .no[data-line]")) {
    const n = Number(no.dataset.line);
    const on = no.dataset.side === side && n >= start && n <= end;
    const row = no.closest(".dl");
    if (!row.classList.contains("split")) row.classList.toggle("sel", on);
    else {
      no.classList.toggle("sel", on);
      no.nextElementSibling?.classList.toggle("sel", on);
    }
  }
}
const dragRange = (d) => [Math.min(d.anchor, d.current), Math.max(d.anchor, d.current)];
panelBody.addEventListener("pointerdown", (e) => {
  const no = e.target.closest?.(".diff .no[data-line]");
  if (!no || (e.pointerType === "mouse" && e.button !== 0) || ui.commentAt) return;
  const file = no.closest(".file");
  const key = no.closest(".diff").dataset.key;
  const [path, side, n] = [file.dataset.path, no.dataset.side, Number(no.dataset.line)];
  const sel = ui.selection?.key === key && ui.selection.path === path && ui.selection.side === side ? ui.selection : null;
  const drag = { key, path, side, anchor: n, current: n, mode: "selecting", pr: !no.closest(".diff[data-comments]"), pointer: e.pointerId };
  if (e.target.closest("[data-comment-line]")) {
    Object.assign(drag, { mode: "gutter", anchor: sel?.start ?? n, current: sel?.end ?? n });
    e.preventDefault();
  } else if (e.shiftKey && sel) drag.anchor = n >= sel.start ? sel.start : sel.end;
  else if (sel?.start === n && sel.end === n) drag.mode = "unselect";
  diffDrag = drag;
  panelBody.classList.add("line-selecting");
  if (drag.mode !== "unselect") markSelection(drag);
});
document.addEventListener("pointermove", (e) => {
  const d = diffDrag;
  if (!d || e.pointerId !== d.pointer) return;
  const row = document.elementFromPoint(e.clientX, e.clientY)?.closest(".dl");
  const no = row && row.closest(".file") === dragFile(d) ? row.querySelector(`.no[data-side="${d.side}"][data-line]`) : null;
  if (!no || Number(no.dataset.line) === d.current) return;
  if (d.mode === "unselect") d.mode = "selecting";
  d.current = Number(no.dataset.line);
  markSelection(d);
});
document.addEventListener("pointerup", (e) => {
  const d = diffDrag;
  if (!d || e.pointerId !== d.pointer) return;
  diffDrag = null;
  panelBody.classList.remove("line-selecting");
  const [start, end] = dragRange(d);
  const at = { key: d.key, path: d.path, side: d.side, start, end };
  ui.selection = d.mode === "unselect" ? null : { ...at, anchor: d.anchor };
  if (d.mode === "gutter") ui.commentAt = { ...at, text: "", focus: true, pr: d.pr };
  renderPanel();
});
document.addEventListener("pointercancel", (e) => {
  if (!diffDrag || e.pointerId !== diffDrag.pointer) return;
  diffDrag = null;
  panelBody.classList.remove("line-selecting");
  renderPanel();
});
panelBody.addEventListener("input", (e) => {
  if (e.target.matches("[data-comment-text]")) e.target.closest(".dcomment").querySelector("[data-comment-save]").disabled = !e.target.value.trim();
});

/** fileContextMenu.ts on a diff file's name: Open in its app, Reveal in the file manager, and Open
 * with each editor the machine has. */
panelBody.addEventListener("contextmenu", (e) => {
  const title = e.target.closest?.(".diff[data-comments] .file > header .path");
  const tab = active;
  if (!title || !tab) return;
  e.preventDefault();
  const path = title.closest(".file").dataset.path;
  const editors = editorsOn(tab.machine);
  const finder = editors.find((ed) => ed.id === "finder");
  const others = editors.filter((ed) => ed !== finder);
  const open = (editor, failed, reveal = false) => callOn(tab.machine, { cmd: "open_in", tab: tab.view.id, editor, path, ...(reveal ? { reveal } : {}) }).catch(() => flash(`${failed}: ${path}`));
  const items = [
    ...(finder ? [
      { label: "Open", icon: "pencil", run: () => open("finder", "Could not open file") },
      { label: finder.name === "Finder" ? "Reveal in Finder" : "Reveal in Files", icon: "folder-tree", run: () => open("finder", "Unable to reveal file", true) },
    ] : []),
    ...(others.length ? [{ label: "Open with", children: () => others.map((ed) => ({ label: ed.name, run: () => open(ed.id, `Could not open in ${ed.name}`) })) }] : []),
  ];
  if (items.length) showMenu(items, { x: e.clientX, y: e.clientY });
});

/** AppearanceSettingsPanel.tsx's Motion section: how long panels take to open and close, 0 to 400
 * ms in 25 ms steps, with a preview that replays it and a reset to the default, off. */
const PANEL_ANIMATION = { min: 0, max: 400, step: 25 };
function panelSettingsHtml() {
  return `<h2 class="looks-title">Motion</h2><div class="look" id="panel-animation-row"><div class="contrast-head"><label for="panel-animation">Panel animations</label><button type="button" class="reset" data-panel-animation-reset aria-label="Reset panel animations to default" title="Reset to default">${icon("undo-2")}</button></div>
    <p class="meta">Set how fast panels open and close.</p>
    <div class="motion-control"><button type="button" class="motion-preview open" data-panel-animation-preview aria-label="Replay panel animation preview"><span class="side"></span><span class="main"><span class="lines"><span></span><span></span><span></span></span><span class="bottom"><span></span></span></span><span class="right"></span></button>
    <div class="contrast-control"><output for="panel-animation"></output><input type="range" class="settings-slider" id="panel-animation" aria-label="Panel animation duration" min="${PANEL_ANIMATION.min}" max="${PANEL_ANIMATION.max}" step="${PANEL_ANIMATION.step}"></div></div></div>`;
}
function showPanelAnimation() {
  const row = document.getElementById("panel-animation-row");
  if (!row) return;
  const ms = panelAnimation();
  const ratio = (ms - PANEL_ANIMATION.min) / (PANEL_ANIMATION.max - PANEL_ANIMATION.min);
  const input = row.querySelector("input");
  input.value = ms;
  input.style.setProperty("--settings-slider-progress", `${ratio * 100}%`);
  input.style.setProperty("--settings-slider-fill-offset", `${0.5 - ratio}rem`);
  row.querySelector("output").textContent = `${ms} ms`;
  row.querySelector("[data-panel-animation-reset]").hidden = ms === 0;
  row.querySelector("[data-panel-animation-preview]").style.setProperty("--preview-duration", `${ms}ms`);
}
function bindPanelSettings() {
  const row = document.getElementById("panel-animation-row");
  if (!row) return;
  row.querySelector("input").oninput = (e) => {
    const ms = Number(e.target.value);
    if (!Number.isInteger(ms) || ms < PANEL_ANIMATION.min || ms > PANEL_ANIMATION.max) return;
    save("gui-panel-animation", ms ? String(ms) : null);
    showPanelAnimation();
  };
  row.querySelector("[data-panel-animation-reset]").onclick = () => {
    save("gui-panel-animation", null);
    showPanelAnimation();
  };
  row.querySelector("[data-panel-animation-preview]").onclick = (e) => e.currentTarget.classList.toggle("open");
  showPanelAnimation();
}
