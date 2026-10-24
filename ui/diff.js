const sum = (files, key) => files.reduce((n, f) => n + Number(f[key]), 0);

function parseDiff(patch) {
  const files = [];
  let file = null;
  let hunk = null;
  let oldNo = 0;
  let newNo = 0;
  for (const line of patch.split("\n")) {
    if (line.startsWith("diff --git ")) {
      file = { path: line.match(/ b\/(.*)$/)?.[1] ?? line.slice(11), status: "changed", added: 0, removed: 0, hunks: [], binary: false };
      files.push(file);
      hunk = null;
    } else if (!file) continue;
    else if (!hunk && line.startsWith("new file")) file.status = "added";
    else if (!hunk && line.startsWith("deleted file")) file.status = "deleted";
    else if (!hunk && line.startsWith("Binary files")) file.binary = true;
    else if (line.startsWith("@@")) {
      const m = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
      oldNo = Number(m?.[1] ?? 0);
      newNo = Number(m?.[2] ?? 0);
      hunk = { oldStart: oldNo, lines: [] };
      file.hunks.push(hunk);
    } else if (hunk && !line.startsWith("\\")) {
      const type = line[0] === "+" ? "add" : line[0] === "-" ? "del" : "ctx";
      if (type === "ctx" && line === "") continue;
      if (type === "add") file.added++;
      if (type === "del") file.removed++;
      hunk.lines.push({ type, old: type === "add" ? null : oldNo++, new: type === "del" ? null : newNo++, text: line.slice(1) });
    }
  }
  return files;
}

function diffRows(hunk) {
  if (ui.diffView === "unified") {
    return hunk.lines.map((l) => `<div class="dl ${l.type}"><span class="no">${l.type === "del" ? l.old : l.new}</span><span class="code">${esc(l.text) || " "}</span></div>`).join("");
  }
  const rows = [];
  let dels = [];
  let adds = [];
  const flush = () => {
    for (let i = 0; i < Math.max(dels.length, adds.length); i++) rows.push([dels[i], adds[i]]);
    dels = [];
    adds = [];
  };
  for (const l of hunk.lines) {
    if (l.type === "del") dels.push(l);
    else if (l.type === "add") adds.push(l);
    else {
      flush();
      rows.push([l, l]);
    }
  }
  flush();
  const side = (l, n) => (l ? `<span class="no ${l.type}">${n}</span><span class="code ${l.type}">${esc(l.text) || " "}</span>` : `<span class="no none"></span><span class="code none"></span>`);
  return rows.map(([a, b]) => `<div class="dl split">${side(a, a?.old)}${side(b, b?.new)}</div>`).join("");
}

function filesHtml(files, key) {
  if (!files.length) return `<p class="panel-empty">No net changes in this selection.</p>`;
  return files.map((f) => {
    const id = `${key}:${f.path}`;
    const collapsed = ui.collapsed.has(id);
    let next = 1;
    const hunks = f.hunks.map((h) => {
      const gap = h.oldStart - next;
      next = h.oldStart + h.lines.filter((l) => l.type !== "add").length;
      return `${gap > 0 ? `<div class="gap">${gap} unmodified line${gap === 1 ? "" : "s"}</div>` : ""}${diffRows(h)}`;
    }).join("");
    return `<section class="file" data-id="${esc(id)}" data-status="${f.status}"${collapsed ? " data-collapsed" : ""}>
      <header>${icon(collapsed ? "chevron-right" : "chevron-down", "fold")}<span class="change"></span><span class="path" title="${esc(f.path)}"><bdi>${esc(f.path)}</bdi></span>
        <button class="copy" data-copy="${esc(f.path)}" title="Copy path">${icon("copy")}</button><span class="spacer"></span><span class="del">-${f.removed}</span><span class="add">+${f.added}</span></header>
      ${collapsed ? "" : `<div class="hunks">${f.binary ? `<p class="panel-empty">Binary file</p>` : hunks}</div>`}</section>`;
  }).join("");
}

function diffToolbar(added, removed, scope) {
  const on = (b) => (b ? " on" : "");
  return `<div class="diff-bar">${scope}<span class="spacer"></span><span class="add">+${added}</span><span class="del">-${removed}</span>
    <button data-diff="collapse" class="icon" title="Collapse or expand all files">${icon("chevrons-up-down")}</button>
    <span class="seg"><button data-diff="unified" class="icon${on(ui.diffView === "unified")}" title="Stacked">${icon("rows-3")}</button><button data-diff="split" class="icon${on(ui.diffView === "split")}" title="Split">${icon("columns-2")}</button></span>
    <button data-diff="wrap" class="icon${on(ui.diffWrap)}" title="Wrap lines">${icon("text-wrap")}</button></div>`;
}
