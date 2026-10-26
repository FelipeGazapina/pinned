function parseDiff(patch) {
  const files = [];
  let file = null;
  let hunk = null;
  let oldNo = 0;
  let newNo = 0;
  for (const line of patch.split("\n")) {
    if (line.startsWith("diff --git ")) {
      file = { path: line.match(/ b\/(.*)$/)?.[1] ?? line.slice(11), from: null, status: "changed", added: 0, removed: 0, hunks: [], binary: false };
      files.push(file);
      hunk = null;
    } else if (!file) continue;
    else if (!hunk && line.startsWith("new file")) file.status = "added";
    else if (!hunk && line.startsWith("deleted file")) file.status = "deleted";
    else if (!hunk && line.startsWith("rename from ")) Object.assign(file, { from: line.slice(12), status: "renamed" });
    else if (!hunk && line.startsWith("rename to ")) file.path = line.slice(10);
    else if (!hunk && line.startsWith("Binary files")) file.binary = true;
    else if (line.startsWith("@@")) {
      const m = line.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/);
      oldNo = Number(m?.[1] ?? 0);
      newNo = Number(m?.[3] ?? 0);
      // An empty side's start names the line before it, as `@@ -0,0 +1,2 @@` does.
      hunk = { oldStart: m?.[2] === "0" ? oldNo + 1 : oldNo, newStart: m?.[4] === "0" ? newNo + 1 : newNo, lines: [] };
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

/** What `git diff -w` would show of `files`: a run of removed and added lines that differ only in
 * whitespace becomes context, and a file left with no change drops out. */
function withoutWhitespace(files) {
  const bare = (lines) => lines.map((l) => l.text).join("").replace(/\s+/g, "");
  return files.flatMap((f) => {
    if (f.binary) return [f];
    const hunks = f.hunks.map((h) => {
      const lines = [];
      for (let i = 0; i < h.lines.length; ) {
        if (h.lines[i].type === "ctx") {
          lines.push(h.lines[i++]);
          continue;
        }
        const run = [];
        while (i < h.lines.length && h.lines[i].type !== "ctx") run.push(h.lines[i++]);
        const dels = run.filter((l) => l.type === "del");
        const adds = run.filter((l) => l.type === "add");
        if (dels.length === adds.length && bare(dels) === bare(adds)) lines.push(...adds.map((l, n) => ({ ...l, type: "ctx", old: dels[n].old })));
        else lines.push(...run);
      }
      return { ...h, lines };
    }).filter((h) => h.lines.some((l) => l.type !== "ctx"));
    if (!hunks.length && f.status === "changed") return [];
    const count = (type) => hunks.reduce((n, h) => n + h.lines.filter((l) => l.type === type).length, 0);
    return [{ ...f, hunks, added: count("add"), removed: count("del") }];
  });
}

/** The Shiki language pierre colors `path` as, from its file name, or null for plain text. */
const langOf = (path) => {
  const lang = PIERRE_DIFFS.filetype(path);
  return lang === "text" ? null : lang;
};

/** Shiki with T3's pierre-dark or pierre-light theme and each grammar, loaded on first use. A
 * language is settled once it is loaded or failed to; a "highlighter" event says when one settles. */
let shiki = null;
let shikiLoading = null;
const shikiLangs = new Set();
const shikiFailed = new Set();
const shikiWaiting = new Set();
const shikiSettle = () => {
  highlighted.clear();
  document.dispatchEvent(new Event("highlighter"));
};
function loadShiki() {
  shikiLoading ??= import("./vendor/shiki/shiki.js").then(async (mod) => {
    shiki = await mod.highlighter();
    for (const lang of shikiWaiting) await shiki.loadLanguage(lang).then(() => shikiLangs.add(lang), () => shikiFailed.add(lang));
    shikiWaiting.clear();
    shikiSettle();
  }, (e) => {
    console.warn("shiki didn't load", e);
    for (const lang of shikiWaiting) shikiFailed.add(lang);
    shikiWaiting.clear();
    shikiSettle();
  });
  return shikiLoading;
}
const SHIKI_PATIENCE = 4000;
function wantLang(lang) {
  if (!lang || shikiLangs.has(lang) || shikiFailed.has(lang) || shikiWaiting.has(lang)) return;
  shikiWaiting.add(lang);
  setTimeout(() => {
    if (!shikiWaiting.has(lang)) return;
    shikiFailed.add(lang);
    shikiSettle();
  }, SHIKI_PATIENCE);
  if (!shiki) return void loadShiki();
  shiki.loadLanguage(lang).then(() => shikiLangs.add(lang), () => shikiFailed.add(lang)).then(() => {
    shikiWaiting.delete(lang);
    shikiSettle();
  });
}
/** Whether `lang` is colored or never will be, asking for it otherwise. */
function langSettled(lang) {
  wantLang(lang);
  return !lang || shikiLangs.has(lang) || shikiFailed.has(lang);
}

const highlighted = new Map();
/** Each of `texts` as highlighted HTML, lexed together so a string or comment that spans lines keeps
 * its color; plain until the language settles. */
function highlightLines(texts, lang) {
  if (!lang || !langSettled(lang) || !shikiLangs.has(lang)) return texts.map((t) => esc(t));
  const source = texts.join("\n");
  const key = `${codeTheme()}\n${lang}\n${source}`;
  if (highlighted.has(key)) return highlighted.get(key);
  if (highlighted.size > 2000) highlighted.clear();
  const lines = shikiLines(source, lang);
  if (!lines) return texts.map((t) => esc(t));
  highlighted.set(key, lines);
  return lines;
}

function shikiLines(source, lang) {
  try {
    const { tokens } = shiki.codeToTokens(source, { lang, theme: codeTheme() });
    return tokens.map((line) => line.map((t) => {
      const style = `${t.color ? `color:${t.color}` : ""}${t.fontStyle & 1 ? ";font-style:italic" : ""}${t.fontStyle & 2 ? ";font-weight:600" : ""}`;
      return style ? `<span style="${style}">${esc(t.content)}</span>` : esc(t.content);
    }).join(""));
  } catch {
    return null;
  }
}

function lineHtml(lines, lang) {
  const sides = (type) => lines.filter((l) => l.type !== type);
  const newSide = sides("del");
  const oldSide = sides("add");
  const newHtml = highlightLines(newSide.map((l) => l.text), lang);
  const oldHtml = highlightLines(oldSide.map((l) => l.text), lang);
  const html = new Map();
  newSide.forEach((l, i) => html.set(l, newHtml[i]));
  oldSide.forEach((l, i) => l.type === "del" && html.set(l, oldHtml[i]));
  return (l) => html.get(l) || " ";
}

/** The numbers of the file's last line before each hunk, so the unchanged stretches between hunks
 * can be named and revealed. `total` is the new file's length once it was read. */
function gapsOf(f, total) {
  const gaps = [];
  let oldEnd = 0;
  let newEnd = 0;
  for (const h of f.hunks) {
    gaps.push({ oldFrom: oldEnd + 1, newFrom: newEnd + 1, size: Math.max(0, h.newStart - newEnd - 1), first: !gaps.length, last: false });
    const oldCount = h.lines.filter((l) => l.type !== "add").length;
    const newCount = h.lines.filter((l) => l.type !== "del").length;
    oldEnd = h.oldStart + oldCount - 1;
    newEnd = h.newStart + newCount - 1;
  }
  gaps.push({ oldFrom: oldEnd + 1, newFrom: newEnd + 1, size: total == null ? null : Math.max(0, total - newEnd), first: !f.hunks.length, last: true });
  return gaps;
}

const EXPAND_STEP = 100;

/** Rows of a gap: the lines revealed at its top and bottom, around a separator for the rest. A gap
 * after the last hunk has no known length until the file is read. */
function gapRows(f, gap, i, open, content, rowOpts) {
  if (gap.size === 0) return [];
  if (gap.size == null) return f.hunks.length && f.status !== "added" && f.status !== "deleted" ? [separatorHtml(gap, i, null)] : [];
  const shown = open?.[i] ?? { top: 0, bottom: 0 };
  const top = Math.min(shown.top, gap.size);
  const bottom = Math.min(shown.bottom, gap.size - top);
  const left = gap.size - top - bottom;
  const context = (from, count) => Array.from({ length: content ? count : 0 }, (_, k) => {
    const n = from + k;
    return { type: "ctx", old: gap.oldFrom + n, new: gap.newFrom + n, text: content[gap.newFrom + n - 1] ?? "" };
  });
  return [...(top ? diffRows(context(0, top), rowOpts) : []), ...(left ? [separatorHtml(gap, i, left)] : []), ...(bottom ? diffRows(context(gap.size - bottom, bottom), rowOpts) : [])];
}

/** "N unmodified lines" with pierre's chevrons (createSeparator.js). A stretch longer than
 * `EXPAND_STEP` gets one per side it touches, each opening `EXPAND_STEP` more lines on that side, and
 * "Expand all"; a shorter one a single chevron for all of it, two-way between hunks. */
function separatorHtml(gap, i, left) {
  const chunked = left == null || left > EXPAND_STEP;
  const pierreIcon = (name) => `<svg class="i" viewBox="0 0 16 16" aria-hidden="true"><use href="#${name}"/></svg>`;
  const button = (dir, label, single = false) => `<button type="button" class="expand" data-expand="${dir}"${single ? " data-whole" : ""} data-gap="${i}" aria-label="${label}">${pierreIcon(dir === "all" ? "diffs-icon-expand-all" : "diffs-icon-expand")}</button>`;
  const buttons = !chunked
    ? gap.first ? button("bottom", "Expand", true) : gap.last ? button("top", "Expand", true) : button("all", "Expand")
    : `${gap.first ? "" : button("top", `Show ${EXPAND_STEP} more lines`)}${gap.last ? "" : button("bottom", `Show ${EXPAND_STEP} more lines`)}`;
  const label = left == null ? "More unchanged context may be available" : `${left} unmodified line${left === 1 ? "" : "s"}`;
  const multi = chunked && !gap.first && !gap.last;
  return `<div class="gap${multi ? " multi" : ""}" data-gap="${i}"><span class="gutter">${buttons}</span><span class="label" data-expand="${gap.first ? "bottom" : gap.last ? "top" : "both"}" data-gap="${i}">${label}</span>${chunked && left != null ? `<button type="button" class="all" data-expand="all" data-gap="${i}">Expand all</button>` : ""}</div>`;
}

/** Each visual row of `lines` as HTML. `comments` holds a file's review comments by `side:end`;
 * `draft` is the one being written and `selection` the lines chosen for it. */
function diffRows(lines, { lang, comments, draft, selection, path }) {
  const code = lineHtml(lines, lang);
  const notes = (side, n) => [...(comments?.get(`${side}:${n}`) ?? []).map(commentHtml), draft?.side === side && draft.end === n ? draftHtml(draft) : ""].join("");
  const picked = (side, n) => selection && selection.path === path && selection.side === side && n >= selection.start && n <= selection.end;
  const no = (side, n, cls = "") => `<span class="no${cls}"${n == null ? "" : ` data-side="${side}" data-line="${n}"`}>${n ?? ""}${n == null ? "" : `<button type="button" class="plus" data-comment-line tabindex="-1" title="Comment on this line">${icon("plus")}</button>`}</span>`;
  if (ui.diffView === "unified") {
    return lines.map((l) => {
      const [side, n] = l.type === "del" ? ["old", l.old] : ["new", l.new];
      return `<div class="dl ${l.type}${picked(side, n) ? " sel" : ""}">${no(side, n)}<span class="code">${code(l)}</span></div>${notes(side, n)}`;
    });
  }
  const rows = [];
  let dels = [];
  let adds = [];
  const flush = () => {
    for (let i = 0; i < Math.max(dels.length, adds.length); i++) rows.push([dels[i], adds[i]]);
    dels = [];
    adds = [];
  };
  for (const l of lines) {
    if (l.type === "del") dels.push(l);
    else if (l.type === "add") adds.push(l);
    else {
      flush();
      rows.push([l, l]);
    }
  }
  flush();
  const half = (l, which, n) => (l ? `${no(which, n, ` ${l.type}${picked(which, n) ? " sel" : ""}`)}<span class="code ${l.type}${picked(which, n) ? " sel" : ""}">${code(l)}</span>` : `<span class="no none"></span><span class="code none"></span>`);
  return rows.map(([a, b]) => `<div class="dl split">${half(a, "old", a?.old)}${half(b, "new", b?.new)}</div>${a ? notes("old", a.old) : ""}${b ? notes("new", b.new) : ""}`);
}

const rangeLabel = (c) => (c.start === c.end ? `L${c.end}` : `L${c.start} to L${c.end}`);
/** DiffCommentAnnotation.tsx: a saved comment, and the draft being written. */
const commentHtml = (c) => c.html ?? `<div class="dcomment">${icon("message-circle")}<p>${esc(c.text)}</p><button type="button" class="icon" data-comment-delete="${esc(c.id)}" aria-label="Delete comment">${icon("trash-2")}</button></div>`;
const draftHtml = (d) => `<div class="dcomment draft"><textarea rows="3" placeholder="Add a comment…" aria-label="Comment on lines ${esc(rangeLabel(d))}" data-comment-text>${esc(d.text ?? "")}</textarea>
  <div class="row"><span class="hint">⌘/Ctrl Enter to send</span><button type="button" class="ghost" data-comment-cancel>Cancel</button><button type="button" class="primary" data-comment-save${(d.text ?? "").trim() ? "" : " disabled"}>Comment</button></div></div>`;

/** Rows past the first `EAGER_ROWS` of a diff wait in sized placeholders until they scroll near,
 * so a long diff opens without building or coloring every line. */
const EAGER_ROWS = 400;
const CHUNK_ROWS = 150;
const pendingChunks = new Map();
let chunkObserver = null;
function fillChunk(el) {
  const html = pendingChunks.get(el.dataset.chunk);
  if (html === undefined) return;
  el.outerHTML = html();
}
function watchChunks(root) {
  chunkObserver?.disconnect();
  chunkObserver = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting) fillChunk(e.target);
  }, { root: root.closest("#panel-body") ?? null, rootMargin: "1200px 0px" });
  for (const el of root.querySelectorAll(".rows-later")) chunkObserver.observe(el);
}
const rowHeight = (row) => (row.startsWith('<div class="gap"') ? 24 : row.startsWith('<div class="dl') ? 20 + (row.includes('class="dcomment') ? 64 : 0) : 64);

/** pierre's file header icon for each status (getIconForType.js), and the tree's git-status letter. */
const CHANGE_ICON = { changed: "diffs-icon-symbol-modified", added: "diffs-icon-symbol-added", deleted: "diffs-icon-symbol-deleted", renamed: "diffs-icon-symbol-moved" };
const pierreSymbol = (name, cls = "") => `<svg class="i ${cls}" viewBox="0 0 16 16" aria-hidden="true"><use href="#${name}"/></svg>`;

/** A file still waiting for its grammar shows these, as T3's viewer shows a loading file. */
const waitingRows = `<div class="file-waiting" aria-label="Loading">${["62%", "44%", "78%"].map((w) => `<span style="width:${w}"></span>`).join("")}</div>`;

/** `opts.comments` (Map path → Map `side:end` → comments) and `opts.draft` show review comments;
 * `opts.viewed(file)` answers a Code tab's "Viewed" state; `opts.contents(path)` the new file's
 * lines once read, for the unchanged stretches; `opts.expanded(path)` which of them are open;
 * `opts.partial` is the file a cut patch ends in. */
function filesHtml(files, key, opts = {}) {
  if (!files.length) return `<p class="panel-empty">No net changes in this selection.</p>`;
  pendingChunks.clear();
  let budget = EAGER_ROWS;
  return files.map((f) => {
    const id = `${key}:${f.path}`;
    const collapsed = ui.collapsed.has(id);
    const lang = collapsed ? null : langOf(f.path);
    const rowOpts = { lang, path: f.path, comments: opts.comments?.get(f.path), draft: opts.draft?.path === f.path ? opts.draft : null, selection: ui.selection?.key === key ? ui.selection : null };
    let body = "";
    if (!collapsed && !f.binary && !langSettled(lang)) body = waitingRows;
    else if (!collapsed && !f.binary) {
      const content = opts.contents?.(f.path);
      const open = opts.expanded?.(f.path);
      const gaps = gapsOf(f, content?.length);
      const rows = f.hunks.flatMap((h, i) => [...gapRows(f, gaps[i], i, open, content, rowOpts), ...diffRows(h.lines, rowOpts)]);
      rows.push(...gapRows(f, gaps.at(-1), gaps.length - 1, open, content, rowOpts));
      const eager = Math.min(rows.length, Math.max(budget, 0));
      budget -= rows.length;
      body = rows.slice(0, eager).join("");
      for (let at = eager, n = 0; at < rows.length; at += CHUNK_ROWS, n++) {
        const part = rows.slice(at, at + CHUNK_ROWS);
        const chunk = `${id}#${n}`;
        pendingChunks.set(chunk, () => part.join(""));
        body += `<div class="rows-later" data-chunk="${esc(chunk)}" style="height:${part.reduce((h, r) => h + rowHeight(r), 0)}px"></div>`;
      }
    }
    const viewed = opts.viewed?.(f);
    const fresh = ui.justOpened === id;
    if (fresh) ui.justOpened = null;
    // createFileHeaderElement.js: the deletions show unless only lines were added, and the reverse.
    const counts = `${f.removed > 0 || f.added === 0 ? `<span class="del">-${f.removed}</span>` : ""}${f.added > 0 || f.removed === 0 ? `<span class="add">+${f.added}</span>` : ""}`;
    return `<section class="file" data-id="${esc(id)}" data-path="${esc(f.path)}" data-status="${f.status}"${collapsed ? " data-collapsed" : ""}${fresh ? " data-fresh" : ""}>
      <header><span class="fold" title="${collapsed ? "Expand diff" : "Collapse diff"}">${icon(collapsed ? "chevron-right" : "chevron-down")}</span>${pierreSymbol(CHANGE_ICON[f.status], "change")}${f.from ? `<span class="from"><bdi>${esc(f.from)}</bdi></span>${pierreSymbol("diffs-icon-arrow-right-short", "moved")}` : ""}<span class="path"><bdi>${esc(f.path)}</bdi></span>
        <button type="button" class="copy" data-copy="${esc(f.path)}" title="Copy path" aria-label="Copy file path">${icon("copy")}</button>${opts.partial === f ? `<span class="partial" title="This file is too large to show in full." aria-label="Partial diff preview">${icon("info")}</span>` : ""}<span class="spacer"></span>
        ${viewed ? `<label class="viewed${viewed.changed ? " changed" : ""}"${viewed.changed ? ' title="This file has been pushed to since you marked it viewed."' : ""}><input type="checkbox" data-viewed="${esc(f.path)}" aria-label="${viewed.changed ? "Changed" : "Viewed"}"${viewed.on ? " checked" : ""}>${viewed.changed ? "Changed" : "Viewed"}</label>` : ""}
        ${counts}</header>
      ${collapsed ? "" : `<div class="hunks">${f.binary ? `<p class="panel-empty">Binary file</p>` : body}</div>`}</section>`;
  }).join("");
}

/** @pierre/trees' icon for a file, in its light or dark color as the theme is. */
function treeIconHtml(path) {
  const icons = window.PIERRE_ICONS;
  if (!icons) return icon("file", "ficon");
  if (!document.getElementById("pierre-sprite")) document.body.insertAdjacentHTML("afterbegin", `<div id="pierre-sprite" aria-hidden="true">${icons.sprite}</div>`);
  const { name, token } = icons.resolve(path);
  const [light, dark] = icons.colors[token ?? "default"] ?? icons.colors.default;
  return `<svg class="i ficon" viewBox="0 0 16 16" style="color: light-dark(${light}, ${dark})" aria-hidden="true"><use href="#${esc(name)}"/></svg>`;
}

const TREE_STATUS = { added: ["A", "added"], deleted: ["D", "deleted"], changed: ["M", "modified"], renamed: ["R", "renamed"] };

/** @pierre/trees as T3's DiffFileTree draws it: compact rows in the diff's order, folders that hold
 * one folder flattened into one row, each file's icon and git-status letter, the chosen file lit. */
function fileTreeHtml(files) {
  const node = () => ({ dirs: new Map(), files: 0, children: [] });
  const root = node();
  for (const f of files) {
    let dir = root;
    for (const part of f.path.split("/").slice(0, -1)) {
      if (!dir.dirs.has(part)) {
        const sub = node();
        dir.dirs.set(part, sub);
        dir.children.push({ name: part, dir: sub });
      }
      dir = dir.dirs.get(part);
    }
    dir.files++;
    dir.children.push({ file: f });
  }
  const rows = [];
  const dirs = [];
  const guides = (depth) => (depth ? `<span class="guides">${"<span></span>".repeat(depth)}</span>` : "");
  const walk = (dir, depth, prefix) => {
    for (const child of dir.children) {
      const f = child.file;
      if (f) {
        const [letter, status] = TREE_STATUS[f.status];
        rows.push(`<button type="button" class="leaf${ui.treePicked === f.path ? " picked" : ""}" data-tree-file="${esc(f.path)}" data-git="${status}" style="--depth:${depth}" title="${esc(f.path)}" aria-selected="${ui.treePicked === f.path}">${guides(depth)}${treeIconHtml(f.path)}<span class="name">${esc(f.path.split("/").pop())}</span><span class="git" title="Git status: ${status}">${letter}</span></button>`);
        continue;
      }
      // A folder holding only one folder shows as one row, like `src/ui`.
      let label = child.name;
      let inner = child.dir;
      while (!inner.files && inner.dirs.size === 1) {
        const [[n, d]] = inner.dirs;
        label += `/${n}`;
        inner = d;
      }
      const path = `${prefix}${label}`;
      dirs.push(path);
      const closed = ui.treeClosed.has(path);
      rows.push(`<button type="button" class="dir" data-tree-dir="${esc(path)}" aria-expanded="${!closed}" style="--depth:${depth}">${guides(depth)}<svg class="i chevron" viewBox="0 0 16 16" aria-hidden="true"><use href="#file-tree-icon-chevron"/></svg><span class="name">${esc(label)}</span><span class="git" title="Contains git status items"><svg viewBox="0 0 6 6" aria-hidden="true"><circle cx="3" cy="3" r="3"/></svg></span></button>`);
      if (!closed) walk(inner, depth + 1, `${path}/`);
    }
  };
  walk(root, 0, "");
  const allOpen = dirs.every((d) => !ui.treeClosed.has(d));
  return `<aside class="file-tree"><header><span class="title">Files</span><span class="count">${files.length}</span>${dirs.length ? `<button type="button" class="icon" data-tree-all="${allOpen ? "close" : "open"}" data-dirs="${esc(JSON.stringify(dirs))}" title="${allOpen ? "Collapse all folders" : "Expand all folders"}" aria-label="${allOpen ? "Collapse all folders" : "Expand all folders"}">${icon(allOpen ? "chevrons-down-up" : "chevrons-up-down")}</button>` : ""}</header>${rows.join("")}</aside>`;
}

/** A file's icon by its kind, colored as the file trees T3 draws do. */
const FILE_ICONS = [
  [/\.(m?[jt]sx?|cjs|cts|mts)$/, "file-code", "#f0db4f"],
  [/\.(rs)$/, "file-code", "#dea584"],
  [/\.(py|rb|go|java|kt|swift|c|cc|cpp|h|hpp|cs|php|lua|dart|ex|exs|zig|scala|hs|sh|bash|zsh)$/, "file-code", "#69b1ff"],
  [/\.(json|jsonc|ya?ml|toml|ini|lock|env)$|(^|\/)\.[^/]*rc$/, "file-cog", "#a3a3a3"],
  [/\.(md|mdx|txt|rst)$/, "file-text", "#68cdf2"],
  [/\.(css|scss|less|html|vue|svelte|xml)$/, "file-code", "#ff855e"],
  [/\.(png|jpe?g|gif|webp|svg|ico|bmp|avif)$/, "file-image", "#d568ea"],
];
function fileIcon(path) {
  const hit = FILE_ICONS.find(([re]) => re.test(path.toLowerCase()));
  return `<span class="ficon" style="color:${hit?.[2] ?? "#a3a3a3"}">${icon(hit?.[1] ?? "file")}</span>`;
}

/** DiffPanel.tsx's header row. The totals, collapse-all and file-tree toggle show only with files;
 * Refresh spins while `refreshing`. */
function diffToolbar(files, key, scope, { refresh = false, refreshing = false } = {}) {
  const on = (b) => (b ? " on" : "");
  const any = files.length > 0;
  const allCollapsed = any && files.every((f) => ui.collapsed.has(`${key}:${f.path}`));
  const tip = (text) => `title="${text}" aria-label="${text}"`;
  return `<div class="diff-bar">${scope}<span class="spacer"></span>${any ? `<span class="add">+${sum(files, "added")}</span><span class="del">-${sum(files, "removed")}</span>` : ""}
    ${refresh ? `<button type="button" data-diff="refresh" class="icon${refreshing ? " refreshing" : ""}" title="${refreshing ? "Refreshing diff…" : "Refresh diff"}" aria-label="${refreshing ? "Refreshing diff" : "Refresh diff"}">${icon("refresh-cw")}</button>` : ""}
    ${any ? `<button type="button" data-diff="collapse" class="icon" ${tip(allCollapsed ? "Expand all files" : "Collapse all files")}>${icon(allCollapsed ? "chevrons-up-down" : "chevrons-down-up")}</button>` : ""}
    <span class="seg"><button type="button" data-diff="unified" class="icon${on(ui.diffView === "unified")}" title="Stacked" aria-label="Stacked diff view">${icon("rows-3")}</button><button type="button" data-diff="split" class="icon${on(ui.diffView === "split")}" title="Split" aria-label="Split diff view">${icon("columns-2")}</button></span>
    <button type="button" data-diff="wrap" class="icon${on(ui.diffWrap)}" title="${ui.diffWrap ? "Disable line wrapping" : "Enable line wrapping"}" aria-label="${ui.diffWrap ? "Disable diff line wrapping" : "Enable diff line wrapping"}">${icon("text-wrap")}</button>
    <button type="button" data-diff="whitespace" class="icon${on(ui.diffHideSpace)}" ${tip(ui.diffHideSpace ? "Show whitespace changes" : "Hide whitespace changes")}>${icon("pilcrow")}</button>
    ${any ? `<button type="button" data-diff="tree" class="icon${on(ui.diffTree)}" ${tip(ui.diffTree ? "Hide file tree" : "Show file tree")}>${icon("folder-tree")}</button>` : ""}</div>`;
}
