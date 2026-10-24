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

const LANG_BY_EXT = { js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "jsx", ts: "typescript", tsx: "tsx", mts: "typescript", cts: "typescript", py: "python", rs: "rust", rb: "ruby", go: "go", java: "java", kt: "kotlin", swift: "swift", c: "c", h: "c", cc: "cpp", cpp: "cpp", hpp: "cpp", cs: "csharp", php: "php", sh: "bash", zsh: "bash", bash: "bash", json: "json", jsonc: "jsonc", yml: "yaml", yaml: "yaml", toml: "toml", ini: "ini", md: "markdown", mdx: "mdx", css: "css", scss: "scss", less: "less", html: "html", xml: "xml", svg: "xml", vue: "vue", svelte: "svelte", sql: "sql", lua: "lua", r: "r", graphql: "graphql", gql: "graphql", m: "objective-c", pl: "perl", dart: "dart", ex: "elixir", exs: "elixir", zig: "zig", scala: "scala", hs: "haskell", tf: "hcl", proto: "proto", diff: "diff" };
const LANG_BY_NAME = { Makefile: "makefile", Dockerfile: "docker", "Cargo.lock": "toml", ".gitignore": "ini", ".env": "ini" };
const HLJS_NAMES = { jsx: "javascript", tsx: "typescript", jsonc: "json", toml: "ini", html: "xml", vue: "xml", svelte: "xml", mdx: "markdown", "objective-c": "objectivec", docker: "dockerfile", hcl: "ini" };
const langOf = (path) => {
  const name = path.split("/").pop();
  return LANG_BY_NAME[name] ?? LANG_BY_EXT[name.split(".").pop().toLowerCase()] ?? null;
};

/** Shiki with T3's pierre-dark theme, loaded on first use; hljs colors lines until it is ready. */
let shiki = null;
let shikiLoading = null;
const shikiLangs = new Set();
const shikiWaiting = new Set();
function loadShiki() {
  shikiLoading ??= import("./vendor/shiki/shiki.js").then(async (mod) => {
    shiki = await mod.highlighter();
    for (const lang of shikiWaiting) await shiki.loadLanguage(lang).then(() => shikiLangs.add(lang), () => {});
    shikiWaiting.clear();
    highlighted.clear();
    document.dispatchEvent(new Event("highlighter"));
  }, (e) => console.warn("shiki didn't load", e));
  return shikiLoading;
}
function wantLang(lang) {
  if (!lang || shikiLangs.has(lang)) return;
  if (!shiki) {
    shikiWaiting.add(lang);
    loadShiki();
    return;
  }
  if (shikiWaiting.has(lang)) return;
  shikiWaiting.add(lang);
  shiki.loadLanguage(lang).then(() => {
    shikiLangs.add(lang);
    shikiWaiting.delete(lang);
    highlighted.clear();
    document.dispatchEvent(new Event("highlighter"));
  }, () => shikiWaiting.delete(lang));
}

const highlighted = new Map();
/** Each of `texts` as highlighted HTML, lexed together so a string or comment that spans lines keeps
 * its color. */
function highlightLines(texts, lang) {
  if (!lang) return texts.map((t) => esc(t));
  wantLang(lang);
  const source = texts.join("\n");
  const key = `${lang}\n${source}`;
  if (highlighted.has(key)) return highlighted.get(key);
  if (highlighted.size > 2000) highlighted.clear();
  const lines = shikiLangs.has(lang) ? shikiLines(source, lang) : hljsLines(source, HLJS_NAMES[lang] ?? lang);
  if (!lines) return texts.map((t) => esc(t));
  highlighted.set(key, lines);
  return lines;
}

function shikiLines(source, lang) {
  try {
    const { tokens } = shiki.codeToTokens(source, { lang, theme: "pierre-dark" });
    return tokens.map((line) => line.map((t) => {
      const style = `${t.color ? `color:${t.color}` : ""}${t.fontStyle & 1 ? ";font-style:italic" : ""}${t.fontStyle & 2 ? ";font-weight:600" : ""}`;
      return style ? `<span style="${style}">${esc(t.content)}</span>` : esc(t.content);
    }).join(""));
  } catch {
    return null;
  }
}

/** hljs escapes the text; a span left open at a line's end is closed and opened again. */
function hljsLines(source, lang) {
  if (!window.hljs?.getLanguage(lang)) return null;
  let html;
  try {
    html = hljs.highlight(source, { language: lang, ignoreIllegals: true }).value;
  } catch {
    return null;
  }
  const lines = [];
  const open = [];
  let line = "";
  for (const part of html.split(/(<span[^>]*>|<\/span>|\n)/)) {
    if (part === "\n") {
      lines.push(line + "</span>".repeat(open.length));
      line = open.join("");
    } else if (part.startsWith("<span")) {
      open.push(part);
      line += part;
    } else if (part === "</span>") {
      open.pop();
      line += part;
    } else line += part;
  }
  lines.push(line + "</span>".repeat(open.length));
  return lines;
}

function lineHtml(hunk, lang) {
  const sides = (type) => hunk.lines.filter((l) => l.type !== type);
  const newSide = sides("del");
  const oldSide = sides("add");
  const newHtml = highlightLines(newSide.map((l) => l.text), lang);
  const oldHtml = highlightLines(oldSide.map((l) => l.text), lang);
  const html = new Map();
  newSide.forEach((l, i) => html.set(l, newHtml[i]));
  oldSide.forEach((l, i) => l.type === "del" && html.set(l, oldHtml[i]));
  return (l) => html.get(l) || " ";
}

/** `comments` holds a file's review comments by `side:line`; `draft` is the one being written. */
function diffRows(hunk, { lang, comments, draft }) {
  const code = lineHtml(hunk, lang);
  const notes = (side, n) => [...(comments?.get(`${side}:${n}`) ?? []).map(commentHtml), draft?.side === side && draft.line === n ? draftHtml(draft) : ""].join("");
  const no = (side, n, cls = "") => `<span class="no${cls}"${n == null ? "" : ` data-side="${side}" data-line="${n}"`}>${n ?? ""}</span>`;
  if (ui.diffView === "unified") {
    return hunk.lines.map((l) => {
      const [side, n] = l.type === "del" ? ["old", l.old] : ["new", l.new];
      return `<div class="dl ${l.type}">${no(side, n)}<span class="code">${code(l)}</span></div>${notes(side, n)}`;
    }).join("");
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
  const side = (l, which, n) => (l ? `${no(which, n, ` ${l.type}`)}<span class="code ${l.type}">${code(l)}</span>` : `<span class="no none"></span><span class="code none"></span>`);
  return rows.map(([a, b]) => `<div class="dl split">${side(a, "old", a?.old)}${side(b, "new", b?.new)}</div>${a ? notes("old", a.old) : ""}${b ? notes("new", b.new) : ""}`).join("");
}

const commentHtml = (c) => `<div class="dcomment">${icon("message-circle")}<p>${esc(c.text)}</p><button type="button" class="icon" data-comment-delete="${esc(c.id)}" title="Delete comment">${icon("trash-2")}</button></div>`;
const draftHtml = (d) => `<div class="dcomment draft"><textarea rows="3" placeholder="Add a comment…" data-comment-text>${esc(d.text ?? "")}</textarea>
  <div class="row"><span class="hint">${IS_MAC ? "⌘" : "Ctrl"} Enter to send</span><span class="spacer"></span><button type="button" class="outline" data-comment-cancel>Cancel</button><button type="button" class="primary" data-comment-save>Comment</button></div></div>`;

/** `opts.comments` (Map path → Map `side:line` → comments) turns line numbers into comment buttons;
 * `opts.viewed(path, file)` answers a Code tab's "Viewed" state. */
function filesHtml(files, key, opts = {}) {
  if (!files.length) return `<p class="panel-empty">No net changes in this selection.</p>`;
  return files.map((f) => {
    const id = `${key}:${f.path}`;
    const collapsed = ui.collapsed.has(id);
    const lang = collapsed ? null : langOf(f.path);
    const rowOpts = { lang, comments: opts.comments?.get(f.path), draft: opts.draft?.path === f.path ? opts.draft : null };
    let next = 1;
    const hunks = collapsed ? "" : f.hunks.map((h) => {
      const gap = h.oldStart - next;
      next = h.oldStart + h.lines.filter((l) => l.type !== "add").length;
      return `${gap > 0 ? `<div class="gap">${gap} unmodified line${gap === 1 ? "" : "s"}</div>` : ""}${diffRows(h, rowOpts)}`;
    }).join("");
    const viewed = opts.viewed?.(f);
    return `<section class="file" data-id="${esc(id)}" data-path="${esc(f.path)}" data-status="${f.status}"${collapsed ? " data-collapsed" : ""}>
      <header>${icon(collapsed ? "chevron-right" : "chevron-down", "fold")}<span class="change"></span><span class="path" title="${esc(opts.openTitle ?? f.path)}"><bdi>${esc(f.path)}</bdi></span>
        <button type="button" class="copy" data-copy="${esc(f.path)}" title="Copy path">${icon("copy")}</button><span class="spacer"></span>
        ${viewed ? `<label class="viewed${viewed.changed ? " changed" : ""}"><input type="checkbox" data-viewed="${esc(f.path)}"${viewed.on ? " checked" : ""}>${viewed.changed ? "Changed" : "Viewed"}</label>` : ""}
        <span class="del">-${f.removed}</span><span class="add">+${f.added}</span></header>
      ${collapsed ? "" : `<div class="hunks">${f.binary ? `<p class="panel-empty">Binary file</p>` : hunks}</div>`}</section>`;
  }).join("");
}

function fileTreeHtml(files) {
  const root = { dirs: new Map(), files: [] };
  for (const f of files) {
    const parts = f.path.split("/");
    let dir = root;
    for (const part of parts.slice(0, -1)) {
      if (!dir.dirs.has(part)) dir.dirs.set(part, { dirs: new Map(), files: [] });
      dir = dir.dirs.get(part);
    }
    dir.files.push(f);
  }
  const rows = [];
  const walk = (dir, depth, prefix) => {
    for (const [name, sub] of [...dir.dirs].sort(([a], [b]) => a.localeCompare(b))) {
      // A folder holding only one folder shows as one row, like `src/ui`.
      let label = name;
      let inner = sub;
      while (!inner.files.length && inner.dirs.size === 1) {
        const [[n, d]] = inner.dirs;
        label += `/${n}`;
        inner = d;
      }
      const path = `${prefix}${label}`;
      const closed = ui.treeClosed.has(path);
      rows.push(`<button type="button" class="dir" data-tree-dir="${esc(path)}" style="--depth:${depth}">${icon(closed ? "chevron-right" : "chevron-down")}${icon("folder")}<span>${esc(label)}</span></button>`);
      if (!closed) walk(inner, depth + 1, `${path}/`);
    }
    for (const f of dir.files.sort((a, b) => a.path.localeCompare(b.path))) {
      rows.push(`<button type="button" class="leaf" data-tree-file="${esc(f.path)}" data-status="${f.status}" style="--depth:${depth}" title="${esc(f.path)}"><span class="change"></span><span class="name">${esc(f.path.split("/").pop())}</span><span class="del">-${f.removed}</span><span class="add">+${f.added}</span></button>`);
    }
  };
  walk(root, 0, "");
  return `<aside class="file-tree"><header><span>Files</span><span class="muted">${files.length}</span></header>${rows.join("")}</aside>`;
}

function diffToolbar(added, removed, scope, { refresh = false } = {}) {
  const on = (b) => (b ? " on" : "");
  return `<div class="diff-bar">${scope}<span class="spacer"></span><span class="add">+${added}</span><span class="del">-${removed}</span>
    ${refresh ? `<button type="button" data-diff="refresh" class="icon" title="Refresh">${icon("rotate-cw")}</button>` : ""}
    <button type="button" data-diff="collapse" class="icon" title="Collapse or expand all files">${icon("chevrons-up-down")}</button>
    <span class="seg"><button type="button" data-diff="unified" class="icon${on(ui.diffView === "unified")}" title="Stacked">${icon("rows-3")}</button><button type="button" data-diff="split" class="icon${on(ui.diffView === "split")}" title="Split">${icon("columns-2")}</button></span>
    <button type="button" data-diff="wrap" class="icon${on(ui.diffWrap)}" title="Wrap lines">${icon("text-wrap")}</button>
    <button type="button" data-diff="whitespace" class="icon${on(ui.diffHideSpace)}" title="${ui.diffHideSpace ? "Show whitespace changes" : "Hide whitespace changes"}">${icon("pilcrow")}</button>
    <button type="button" data-diff="tree" class="icon${on(ui.diffTree)}" title="File tree">${icon("folder-tree")}</button></div>`;
}
