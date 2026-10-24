Object.assign(ICON_PATHS, {
  brain: `<path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z"/><path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z"/><path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4"/><path d="M17.599 6.5a3 3 0 0 0 .399-1.375"/><path d="M6.003 5.125A3 3 0 0 0 6.401 6.5"/><path d="M3.477 10.896a4 4 0 0 1 .585-.396"/><path d="M19.938 10.5a4 4 0 0 1 .585.396"/><path d="M6 18a4 4 0 0 1-1.967-.516"/><path d="M19.967 17.484A4 4 0 0 1 18 18"/>`,
  hammer: `<path d="m15 12-8.373 8.373a1 1 0 1 1-3-3L12 9"/><path d="m18 15 4-4"/><path d="m21.5 11.5-1.914-1.914A2 2 0 0 1 19 8.172V7l-2.26-2.26a6 6 0 0 0-4.202-1.756L9 2.96l.92.82A6.18 6.18 0 0 1 12 8.4V10l2 2h1.172a2 2 0 0 1 1.414.586L18.5 14.5"/>`,
  "message-circle": `<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>`,
  download: `<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" x2="12" y1="15" y2="3"/>`,
  "chevrons-down-up": `<path d="m7 20 5-5 5 5"/><path d="m7 4 5 5 5-5"/>`,
  "chevron-up": `<path d="m18 15-6-6-6 6"/>`,
  "folder-closed": `<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/><path d="M2 10h20"/>`,
  file: `<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/>`,
  "redo-2": `<path d="m15 14 5-5-5-5"/><path d="M20 9H9.5A5.5 5.5 0 0 0 4 14.5A5.5 5.5 0 0 0 9.5 20H13"/>`,
  "undo-2": `<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11"/>`,
  "calendar-clock": `<path d="M21 7.5V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h3.5"/><path d="M16 2v4"/><path d="M8 2v4"/><path d="M3 10h5"/><path d="M17.5 17.5 16 16.3V14"/><circle cx="16" cy="16" r="6"/>`,
  "git-fork": `<circle cx="12" cy="18" r="3"/><circle cx="6" cy="6" r="3"/><circle cx="18" cy="6" r="3"/><path d="M18 9v2c0 .6-.4 1-1 1H7c-.6 0-1-.4-1-1V9"/><path d="M12 12v3"/>`,
  play: `<polygon points="6 3 20 12 6 21 6 3"/>`,
  "minimize-2": `<polyline points="4 14 10 14 10 20"/><polyline points="20 10 14 10 14 4"/><line x1="14" x2="21" y1="10" y2="3"/><line x1="3" x2="10" y1="21" y2="14"/>`,
  "corner-left-up": `<polyline points="14 9 9 4 4 9"/><path d="M20 20h-7a4 4 0 0 1-4-4V4"/>`,
});

const PLAN_TOOL = "ExitPlanMode";
const QUESTION_TOOL = "AskUserQuestion";
const interruption = (step) => /^\[Request interrupted/.test(step.result?.text ?? "");

function turnsOf(items) {
  const turns = [];
  const results = new Map(items.filter((i) => i.kind === "result").map((r) => [r.tool, r]));
  let turn = null;
  for (const item of items) {
    if (item.kind === "result" || item.kind === "model") continue;
    if (item.kind === "user" || item.kind === "steer" || !turn) {
      const user = item.kind === "user" ? item : item.kind === "steer" ? { ...item, kind: "user", images: 0, steer: true } : null;
      turn = { id: item.id ?? `t${turns.length}`, user, steps: [], prs: [], notes: [] };
      turns.push(turn);
      if (user) continue;
    }
    if (item.kind === "pr") turn.prs.push(item);
    else if (item.kind === "note") turn.notes.push(item);
    else turn.steps.push(item.kind === "tool" ? { ...item, result: results.get(item.id) } : item);
  }
  for (const t of turns) {
    const texts = t.steps.filter((s) => s.kind === "text");
    t.answer = t.steps.at(-1)?.kind === "text" ? t.steps.at(-1) : null;
    t.lead = texts[0] && texts[0] !== t.answer ? texts[0] : null;
    const lastText = t.steps.lastIndexOf(texts.at(-1));
    // Errors, answered questions and plans stay in sight, and once the turn settles so does a tool that
    // failed after the last thing claude said.
    const shown = (s) => s === t.lead || s === t.answer || s.kind === "error" || (s.kind === "tool" && (s.name === PLAN_TOOL || (s.name === QUESTION_TOOL && s.result)));
    t.failed = new Set(t.steps.filter((s, i) => i > lastText && s.kind === "tool" && s.result?.error && !shown(s) && !interruption(s)));
    t.shown = t.steps.filter(shown);
    t.work = t.steps.filter((s) => !shown(s));
    t.interrupted = t.notes.some((n) => n.text === "Interrupted.");
    const times = [t.user?.at, ...t.steps.flatMap((s) => [s.at, s.result?.at])].filter(Boolean).map(Date.parse).filter(Number.isFinite);
    t.start = times.length ? Math.min(...times) : 0;
    t.end = times.length ? Math.max(...times) : 0;
  }
  // A turn's own fold counts the work its steers continued, as T3 Code's top row does.
  let runEnd = 0;
  for (const t of [...turns].reverse()) {
    runEnd = Math.max(runEnd, t.end);
    t.runEnd = runEnd;
    if (!t.user?.steer) runEnd = 0;
  }
  return turns;
}

function duration(ms) {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`;
  const total = Math.round(ms / 1000);
  if (total < 60) return `${total}s`;
  const [h, m, s] = [Math.floor(total / 3600), Math.floor((total % 3600) / 60), total % 60];
  return [h && `${h}h`, m && `${m}m`, s && `${s}s`].filter(Boolean).join(" ");
}
/** A running turn's clock counts whole seconds, as T3 Code's does. */
const ticking = (ms) => (ms < 60_000 ? `${Math.max(0, Math.floor(ms / 1000))}s` : duration(Math.floor(ms / 1000) * 1000));

const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const shortDate = new Intl.DateTimeFormat(undefined, { month: "numeric", day: "numeric" });
const longDate = new Intl.DateTimeFormat(undefined, { month: "numeric", day: "numeric", year: "numeric" });
const monthName = new Intl.DateTimeFormat("en-US", { month: "long" });
const dateOf = (at) => {
  const d = new Date(at || NaN);
  return Number.isNaN(d.getTime()) ? null : d;
};
function dayAware(at) {
  const d = dateOf(at);
  if (!d) return "";
  const now = new Date();
  const days = Math.round((new Date(now.getFullYear(), now.getMonth(), now.getDate()) - new Date(d.getFullYear(), d.getMonth(), d.getDate())) / 86_400_000);
  if (days <= 0) return clock.format(d);
  if (days === 1) return `yesterday at ${clock.format(d)}`;
  return `${(d.getFullYear() === now.getFullYear() ? shortDate : longDate).format(d)} ${clock.format(d)}`;
}
function fullTime(at) {
  const d = dateOf(at);
  if (!d) return "";
  const day = d.getDate();
  const suffix = [11, 12, 13].includes(day % 100) ? "th" : ["th", "st", "nd", "rd"][day % 10] ?? "th";
  return `${clock.format(d)}, ${day}${suffix} ${monthName.format(d)} ${d.getFullYear()}`;
}
const whenHtml = (at, cls = "when") => (dateOf(at) ? `<span class="${cls}" title="${esc(fullTime(at))}">${esc(dayAware(at))}</span>` : "");

/** 1234 reads 1.2k, as T3 Code's diff counts do. */
function compact(n) {
  for (const [size, unit] of [[1e9, "b"], [1e6, "m"], [1e3, "k"]]) {
    if (n < size) continue;
    const v = n / size;
    return `${v < 10 ? v.toFixed(1).replace(/\.0$/, "") : Math.round(v)}${unit}`;
  }
  return String(n);
}

const WEB_URL = /^https?:\/\//i;
const GITHUB_URL = /^https?:\/\/(www\.)?github\.com\//i;
const PR_URL = /^https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/pull\/\d+\/?$/i;
const codeKey = (text) => {
  let h = 0;
  for (let i = 0; i < text.length; i++) h = (h * 31 + text.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
};
/** Code blocks and tables the user wrapped or unwrapped against the Word wrap setting, by content. */
const flipped = new Set();
const wrapsByDefault = () => load("look-word-wrap") !== "false";
const wraps = (key) => wrapsByDefault() !== flipped.has(key);

/** Hosts whose names may go to Google's favicon service, as T3 Code decides: never a private or internal one. */
const PRIVATE_TLDS = /\.(alt|corp|example|home|internal|intranet|invalid|lan|onion|private|test|local|localhost|arpa)$/;
function publicHost(host) {
  const name = host.toLowerCase().replace(/\.$/, "");
  if (!name.includes(".") || name.includes(":") || PRIVATE_TLDS.test(name) || name.split(".").some((l) => ["corp", "internal", "intranet"].includes(l))) return false;
  const ip = name.split(".").map(Number);
  if (ip.length !== 4 || ip.some((n) => !Number.isInteger(n) || n < 0 || n > 255)) return true;
  const [a, b, c] = ip;
  return !(a === 0 || a === 10 || a === 127 || a >= 224 || (a === 100 && b >= 64 && b <= 127) || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254) || (a === 198 && b >= 18 && b <= 19) || (a === 192 && b === 0 && [0, 2].includes(c)) || (a === 198 && b === 51 && c === 100) || (a === 203 && b === 0 && c === 113));
}
const failedFavicons = new Set();
function linkIcon(href) {
  if (GITHUB_URL.test(href)) return `<span class="link-icon">${icon("github")}</span>`;
  const host = new URL(href).hostname;
  if (!publicHost(host) || failedFavicons.has(host)) return `<span class="link-icon">${icon("globe")}</span>`;
  return `<span class="link-icon"><img class="favicon" alt="" loading="lazy" draggable="false" data-host="${esc(host)}" src="https://www.google.com/s2/favicons?domain=${encodeURIComponent(host)}&amp;sz=32"></span>`;
}

/** Code is highlighted as T3 Code does it, by Shiki with the Pierre Dark theme. Until a language's grammar
 * has loaded its blocks show plain, and the thread draws again once it has. */
const wantedGrammars = new Set();
const loadingGrammars = new Set();
function loadGrammar(language) {
  if (loadingGrammars.has(language)) return;
  loadingGrammars.add(language);
  SHIKI.load(language).then((id) => id && grammarLoaded(), console.warn);
}
function shikiLines(text, language) {
  if (!window.SHIKI) {
    wantedGrammars.add(language);
    return null;
  }
  try {
    const lines = SHIKI.tokens(text, language);
    if (lines) return lines;
  } catch (e) {
    console.warn(`highlighting ${language} failed`, e);
    return null;
  }
  loadGrammar(language);
  return null;
}
const tokenHtml = (t) => `<span style="color:${esc(t.color ?? "")}${t.fontStyle & 1 ? ";font-style:italic" : ""}${t.fontStyle & 2 ? ";font-weight:600" : ""}${t.fontStyle & 4 ? ";text-decoration:underline" : ""}">${esc(t.content)}</span>`;
let redrawTimer = 0;
function grammarLoaded() {
  clearTimeout(redrawTimer);
  redrawTimer = setTimeout(() => {
    rendered.clear();
    if (!guiOn()) return;
    rerender();
    if (ui.panelOpen && ui.panel === "pr") renderPanel();
  }, 30);
}
window.addEventListener("shiki", () => wantedGrammars.forEach(loadGrammar));

const SHELL_LANGS = /^(?:sh|bash|zsh|fish|shell|powershell|pwsh|console)$/;
/** Assistant text and pull request comments come from others, so only web links render, and an image
 * is a link to it rather than a request made on sight. */
const RENDERER = {
  html: ({ text }) => esc(text),
  link({ href, tokens }) {
    const text = this.parser.parseInline(tokens);
    if (!WEB_URL.test(href)) return text;
    return `<a href="${esc(href)}" title="${esc(href)}"${PR_URL.test(href) ? ` data-pr="${esc(href)}"` : ""}>${linkIcon(href)}${text}</a>`;
  },
  table(token) {
    const key = codeKey(token.raw);
    return `<div class="md-table" data-table="${key}" data-expanded="${wraps(key)}"><div class="table-scroll">${marked.Renderer.prototype.table.call(this, token)}</div>
      <div class="table-foot"><button type="button" class="xs" data-table-expand title="${wraps(key) ? "Collapse table cells" : "Expand table cells"}">${icon(wraps(key) ? "minimize-2" : "maximize-2")}</button>
      <span class="table-copy"><button type="button" class="xs" data-table-menu title="Copy table">${icon("copy")}</button><span class="table-menu" hidden><button type="button" data-copy-table="markdown">Copy as Markdown</button><button type="button" data-copy-table="csv">Copy as CSV</button></span></span></div></div>`;
  },
  image: ({ href, text }) => (WEB_URL.test(href) ? `<a href="${esc(href)}" title="${esc(href)}">${esc(text || href)}</a>` : esc(text)),
  code({ text, lang }) {
    const language = (lang ?? "").trim().split(/\s+/)[0].toLowerCase();
    const lines = language ? shikiLines(text, language) : null;
    const body = lines ? lines.map((line) => line.map(tokenHtml).join("")).join("\n") : esc(text);
    const key = codeKey(text);
    const runs = SHELL_LANGS.test(language) && !!text.trim() && !/\\$/.test(text.trim()) && !/[\p{Cc}\p{Cf}]/u.test(text);
    return `<div class="codeblock" data-code="${key}" data-wrap="${wraps(key)}"><div class="code-head"><span class="lang">${esc(language || "text")}</span>
      <span class="code-actions"><button type="button" class="xs" data-code-wrap title="${wraps(key) ? "Disable line wrap" : "Wrap lines"}">${icon("text-wrap")}</button>${runs ? `<button type="button" class="xs" data-run-code title="Run in terminal">${icon("play")}</button>` : ""}<button type="button" class="xs" data-copy-code title="Copy code">${icon("copy")}</button></span></div>
      <pre><code${lines ? ` class="shiki language-${esc(language)}"` : ""}>${body}</code></pre></div>`;
  },
};
marked.use({ gfm: true, renderer: RENDERER });

/** `@src/app.ts` in a prompt is a file Claude Code was pointed at, so it reads as a chip. */
const MENTION = {
  name: "mention",
  level: "inline",
  start: (src) => {
    const m = /(^|\s)@[\w.~/]/.exec(src);
    return m ? m.index + m[1].length : undefined;
  },
  tokenizer(src) {
    const m = /^@((?:[\w.~-]+\/)*[\w.-]*[\w/])/.exec(src);
    if (m && /[/.]/.test(m[1])) return { type: "mention", raw: m[0], path: m[1] };
  },
  renderer: ({ path }) => `<span class="mention" title="${esc(path)}">${path.endsWith("/") ? icon("folder") : fileIconHtml(path)}${esc(path.replace(/\/$/, "").split("/").pop() || path)}</span>`,
};
const promptMarked = new marked.Marked({ gfm: true, breaks: true, renderer: RENDERER }, { extensions: [MENTION] });

const rendered = new Map();
function markdown(text, parser = marked) {
  const key = parser === marked ? text : `\0${text}`;
  let html = rendered.get(key);
  if (html === undefined) {
    if (rendered.size > 4000) rendered.clear();
    html = parser.parse(text).trim();
    rendered.set(key, html);
  }
  return html;
}

const TOOL_ICONS = { Bash: "terminal", Read: "eye", Edit: "square-pen", MultiEdit: "square-pen", Write: "square-pen", NotebookEdit: "square-pen", Grep: "search", Glob: "search", WebFetch: "globe", WebSearch: "globe", Agent: "bot", Task: "bot", [QUESTION_TOOL]: "message-circle" };
const shortPath = (path, cwd) => (cwd && path.startsWith(`${cwd}/`) ? path.slice(cwd.length + 1) : tilde(path));
const toolIcon = (step) => (step.kind === "thinking" ? "brain" : /^mcp__/.test(step.name) ? "wrench" : TOOL_ICONS[step.name] ?? "zap");

function toolLabel(step, cwd) {
  const what = /^(Read|Write|Edit|MultiEdit|NotebookEdit)$/.test(step.name) ? shortPath(step.summary, cwd) : step.summary;
  const verb = { Read: "Read", Edit: "Edited", MultiEdit: "Edited", Write: "Wrote", Grep: "Searched", Glob: "Listed", WebFetch: "Fetched", WebSearch: "Searched the web for", Agent: "Delegated", Task: "Delegated", Skill: "Used skill" }[step.name];
  if (step.name === "Bash") return what || step.detail.split("\n")[0];
  return verb ? `${verb} ${what}` : `${step.name.replace(/^mcp__([^_]+)__/, "$1 · ")}${what ? ` ${what}` : ""}`;
}

/** The thought's first sentence, or its bold heading, as one plain line. */
function thoughtLine(text) {
  const heading = /^\s*\*\*([^*\r\n]+)\*\*[ \t]*\r?(?:\n|$)/.exec(text)?.[1];
  const line = (heading ?? text)
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/^[ \t]*(?:#{1,6}|[-*+]|\d+\.)[ \t]+/gm, "")
    .replace(/`+|\*\*|~~/g, "")
    .replace(/(^|[^\w*])[*_]([^*_\n]+)[*_](?![\w*])/g, "$1$2")
    .replace(/\s+/g, " ")
    .trim();
  if (heading !== undefined) return line;
  const end = /[.?!]["'”’)]?(?=\s)/.exec(line);
  return end ? line.slice(0, end.index + end[0].length) : line;
}

const programOf = (command) => command.trim().split(/\s+/).find((w) => !/^\w+=/.test(w))?.split("/").pop() || "command";

/** T3 Code's summary of a run of tools, such as "Read 3 files and ran 2 commands". */
const ACTIONS = {
  command: [0, (n) => `Ran ${n} ${n === 1 ? "command" : "commands"}`],
  edit: [0, (n) => `Changed ${n} ${n === 1 ? "file" : "files"}`],
  delegate: [0, (n) => `Delegated ${n} ${n === 1 ? "task" : "tasks"}`],
  read: [1, (n) => `Read ${n} ${n === 1 ? "file" : "files"}`],
  "code-search": [1, (n) => `Searched code ${n} ${n === 1 ? "time" : "times"}`],
  search: [1, (n) => `Searched the web ${n} ${n === 1 ? "time" : "times"}`],
  other: [2, (n) => `Used ${n} ${n === 1 ? "tool" : "tools"}`],
};
const actionOf = (step) =>
  ({ Bash: "command", Read: "read", Edit: "edit", MultiEdit: "edit", Write: "edit", NotebookEdit: "edit", Grep: "code-search", Glob: "code-search", WebFetch: "search", WebSearch: "search", Agent: "delegate", Task: "delegate" })[step.name] ?? "other";
const ACTION_ICONS = { command: "terminal", edit: "square-pen", delegate: "bot", read: "eye", "code-search": "search", search: "globe", other: "wrench" };

function groupSummary(steps) {
  const tools = steps.filter((s) => s.kind === "tool");
  if (!tools.length) return { label: steps.length === 1 ? "Thought" : `Thought (×${steps.length})`, icon: "brain" };
  const groups = new Map();
  for (const s of tools) {
    const action = actionOf(s);
    const key = action === "edit" ? s.summary || s.id : s.id;
    if (!groups.has(action)) groups.set(action, new Set());
    groups.get(action).add(key);
  }
  const summaries = [...groups].map(([action, keys], index) => ({ index, count: keys.size, calls: tools.filter((s) => actionOf(s) === action).length, priority: ACTIONS[action][0], label: ACTIONS[action][1](keys.size) }));
  const picked = [...summaries].sort((a, b) => a.priority - b.priority || a.index - b.index).slice(0, 2).sort((a, b) => a.index - b.index);
  const labels = picked.map((s) => s.label);
  const rest = tools.length - picked.reduce((n, s) => n + s.calls, 0);
  if (rest > 0) labels.push(`Performed ${rest} other ${rest === 1 ? "action" : "actions"}`);
  const sentence = labels.map((l, i) => (i ? l[0].toLowerCase() + l.slice(1) : l));
  const label = sentence.length < 3 ? sentence.join(" and ") : `${sentence.slice(0, -1).join(", ")}, and ${sentence.at(-1)}`;
  return { label, icon: groups.size === 1 ? ACTION_ICONS[groups.keys().next().value] : "hammer" };
}

function workRowHtml(step, cwd, open) {
  if (step.kind === "thinking") {
    return `<details class="tool thought" data-fold="${esc(step.id)}"${open.has(step.id) ? " open" : ""}><summary>${icon("brain", "kind")}<span class="label">${esc(thoughtLine(step.text) || "Thought")}</span>
      ${whenHtml(step.at)}${icon("chevron-right", "chev")}</summary><div class="md thought-body">${markdown(step.text)}</div></details>`;
  }
  const state = !step.result ? "running" : step.result.error ? "error" : "ok";
  const body = [step.detail && `<pre class="input">${esc(step.detail)}</pre>`, step.result?.text && `<pre>${esc(step.result.text)}</pre>`].filter(Boolean).join("");
  return `<details class="tool" data-fold="${esc(step.id)}" data-state="${state}"${body ? "" : " data-empty"}${open.has(step.id) ? " open" : ""}><summary>${icon(toolIcon(step), "kind")}<span class="label">${esc(toolLabel(step, cwd))}</span>
    ${state === "error" ? icon("x", "failed") : ""}${whenHtml(step.result?.at ?? step.at)}${body ? icon("chevron-right", "chev") : ""}</summary>${body}</details>`;
}

/** Consecutive tools fold under one summary row. Anything else in between breaks the run, though only
 * the steps in `work` show here. */
function workHtml(steps, work, cwd, t, live) {
  const out = [];
  let run = [];
  const flush = (last) => {
    if (run.length === 1) out.push(workRowHtml(run[0], cwd, t.open));
    else if (run.length > 1) {
      const key = `g:${run[0].id}`;
      const auto = live && last;
      const open = auto ? !t.open.has(`shut:${key}`) : t.open.has(key);
      const { label, icon: name } = groupSummary(run);
      const failed = run.some((s) => s.result?.error);
      out.push(`<details class="group" data-fold="${esc(key)}"${auto ? " data-auto" : ""}${open ? " open" : ""}><summary>${icon(name, "kind")}<span class="label">${esc(label)}</span>
        ${failed ? icon("x", "failed") : ""}${whenHtml(run.at(-1).result?.at ?? run.at(-1).at)}${icon("chevron-right", "chev")}</summary>
        <div class="rows">${run.map((s) => workRowHtml(s, cwd, t.open)).join("")}</div></details>`);
    }
    run = [];
  };
  for (const s of steps) {
    if (!work.includes(s)) flush(false);
    else if (s.kind === "tool" || s.kind === "thinking") run.push(s);
    else {
      flush(false);
      out.push(`<div class="narration md">${markdown(s.text)}</div>`);
    }
  }
  flush(true);
  return out.join("");
}

/** The running turn's latest work: its last thought above its last tool, and "Thinking" while no tool runs,
 * since claude writes a thought to the transcript only once it is done. */
function liveHtml(turn, active, cwd) {
  const thought = active.findLast((s) => s.kind === "thinking");
  const tool = active.findLast((s) => s.kind === "tool");
  const open = `data-open-turn="${esc(turn.id)}"`;
  const thinking = (above = "") => `<button type="button" class="live-tool" ${open}>${above}<span class="line">${icon("brain", "kind")}<span class="label shimmer">Thinking</span></span></button>`;
  const line = thought ? `<span class="line thought-line">${icon("brain", "kind")}<span class="label">${esc(thoughtLine(thought.text) || "Thought")}</span></span>` : "";
  if (!tool || active.at(-1) === thought) return thinking(line);
  const label =
    tool.name === "Bash" ? `${!tool.result ? "Running" : interruption(tool) ? "Stopped" : tool.result.error ? "Failed" : "Ran"} ${programOf(tool.detail || tool.summary)}`
    : toolLabel(tool, cwd);
  const row = `<button type="button" class="live-tool" ${open}>${line}<span class="line${tool.result ? " done" : ""}">${icon(toolIcon(tool), "kind")}<span class="label">${esc(label)}</span>${tool.result?.error ? icon("x", "failed") : ""}</span></button>`;
  return tool.result ? row + thinking() : row;
}

const ANSWER = /"([^"]*)"="([^"]*)"/g;
function questionHtml(step) {
  const answers = [...(step.result?.text ?? "").matchAll(ANSWER)].map((m) => m[2]);
  return `<div class="tool answered"><div class="row">${icon("message-circle", "kind")}<span class="label">${esc(step.summary || "Question")}</span>${whenHtml(step.result?.at)}</div>
    <p class="message">${esc(answers.join(", ") || step.result?.text || "")}</p></div>`;
}

/** Claude Code's words when a plan's usage or spend limit stops the turn, such as "You've hit your limit · resets 5pm". */
const USAGE_LIMIT = /^(You've hit your\b.*\blimit|Claude AI usage limit reached)/i;

function errorHtml(step, cwd) {
  const label = step.kind === "error" ? step.text.split("\n")[0] : toolLabel(step, cwd);
  const limit = step.kind === "error" && USAGE_LIMIT.test(label);
  const message = limit ? "" : step.kind === "error" ? step.text.split("\n").slice(1).join("\n") : step.result.text;
  return `<div class="tool error-row${limit ? " limit" : ""}"><div class="row">${icon("circle-alert", "kind")}<span class="label">${esc(label)}</span>${whenHtml(step.result?.at ?? step.at, "when on")}</div>
    ${message.trim() ? `<p class="message">${esc(message.trim())}</p>` : ""}</div>`;
}

function noteHtml(note) {
  const [label, ...rest] = note.text.split("\n");
  return `<div class="tool info note"><div class="row">${icon("check", "kind")}<span class="label">${esc(label)}</span>${whenHtml(note.at)}</div>
    ${rest.join("\n").trim() ? `<p class="message">${esc(rest.join("\n").trim())}</p>` : ""}</div>`;
}

const planTitle = (plan) => plan.match(/^\s{0,3}#{1,6}\s+(.+)$/m)?.[1]?.trim() || null;
function planText(step) {
  if (step.detail) return step.detail;
  const approved = step.result?.text.split(/^##\s*Approved Plan:?\s*$/m)[1];
  return (approved ?? step.summary ?? "").trim();
}
function stripPlan(plan) {
  const lines = plan.trimEnd().split(/\r?\n/);
  const rest = /^\s{0,3}#{1,6}\s+/.test(lines[0] ?? "") ? lines.slice(1) : lines;
  while (rest[0]?.trim() === "") rest.shift();
  if (/^\s{0,3}#{1,6}\s+summary\s*$/i.test(rest[0] ?? "")) {
    rest.shift();
    while (rest[0]?.trim() === "") rest.shift();
  }
  return rest.join("\n");
}
const planFile = (plan) => `${(planTitle(plan) ?? "plan").toLowerCase().replace(/[`'".,!?()[\]{}]+/g, "").replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "plan"}.md`;

function planHtml(step, t) {
  const plan = planText(step);
  const long = plan.length > 900 || plan.split("\n").length > 20;
  const key = `plan:${step.id}`;
  const clamped = long && !t.open.has(key);
  return `<div class="plan-card" data-plan="${esc(step.id)}"><div class="plan-head"><span class="badge">Plan</span><h3>${esc(planTitle(plan) ?? "Proposed plan")}</h3>
      <span class="plan-actions"><button type="button" class="xs outline" data-plan-menu title="Plan actions">${icon("ellipsis")}</button>
      <span class="plan-menu" hidden><button type="button" data-plan-copy>Copy to clipboard</button><button type="button" data-plan-download>Download as markdown</button></span></span></div>
    <div class="plan-body md${clamped ? " clamped" : ""}">${markdown(stripPlan(plan))}</div>
    ${long ? `<div class="plan-foot"><button type="button" class="outline" data-toggle="${esc(key)}">${clamped ? "Expand plan" : "Collapse plan"}</button></div>` : ""}</div>`;
}

function shownHtml(step, cwd, t, answer) {
  if (step.kind === "error" || (step.kind === "tool" && step.result?.error && step.name !== QUESTION_TOOL && step.name !== PLAN_TOOL)) return errorHtml(step, cwd);
  if (step.name === PLAN_TOOL) return planHtml(step, t);
  if (step.name === QUESTION_TOOL) return questionHtml(step);
  if (step !== answer) return `<div class="assistant md">${markdown(step.text)}</div>`;
  return `<div class="answer"><div class="assistant md">${markdown(step.text)}</div>
    <div class="meta"><button type="button" class="xs" data-copy-item="${esc(step.id)}" title="Copy">${icon("copy")}</button>${whenHtml(step.at)}</div></div>`;
}

const endOfSecond = (seconds) => seconds * 1000 + 999;

function changesByTurn(history, turns) {
  const found = new Map();
  for (const h of history ?? []) {
    const turn = turns.findLast((t) => t.user && t.start <= endOfSecond(h.ended));
    if (turn && !found.has(turn)) found.set(turn, h);
  }
  return found;
}

/** Changed files as folders, a folder holding only one folder reading as one `a/b` row. */
function fileTree(files) {
  const root = { dirs: new Map(), files: [] };
  for (const f of files) {
    const parts = f.path.split("/").filter(Boolean);
    let dir = root;
    for (const [i, name] of parts.slice(0, -1).entries()) {
      if (!dir.dirs.has(name)) dir.dirs.set(name, { name, path: parts.slice(0, i + 1).join("/"), dirs: new Map(), files: [], added: 0, removed: 0 });
      dir = dir.dirs.get(name);
      dir.added += Number(f.added);
      dir.removed += Number(f.removed);
    }
    dir.files.push({ ...f, name: parts.at(-1) ?? f.path });
  }
  const nodes = (dir) => [
    ...[...dir.dirs.values()].sort((a, b) => a.name.localeCompare(b.name)).map((d) => {
      while (d.dirs.size === 1 && !d.files.length) {
        const only = d.dirs.values().next().value;
        d = { ...only, name: `${d.name}/${only.name}` };
      }
      return { ...d, children: nodes(d) };
    }),
    ...dir.files.sort((a, b) => a.name.localeCompare(b.name)),
  ];
  return nodes(root);
}

/** A path's file-type icon from T3 Code's Pierre set, its sprite added to the page on first use. */
function fileIconHtml(path, cls = "") {
  const icons = window.PIERRE_ICONS;
  if (!icons) return icon("file", cls);
  if (!document.getElementById("pierre-sprite")) document.body.insertAdjacentHTML("afterbegin", `<div id="pierre-sprite" aria-hidden="true">${icons.sprite}</div>`);
  const { name, token } = icons.resolve(path);
  const color = (icons.colors[token ?? "default"] ?? icons.colors.default)[1];
  return `<svg class="i pierre ${cls}" viewBox="0 0 16 16" style="color: ${color}" aria-hidden="true"><use href="#${esc(name)}"/></svg>`;
}

const stat = (added, removed) => `<span class="add">+${compact(Number(added))}</span><span class="del">−${compact(Number(removed))}</span>`;

function changesHtml(change, open) {
  const n = change.files.length;
  const fold = `changes:${change.ended}`;
  const tree = fileTree(change.files);
  const hasDirs = tree.some((node) => node.children);
  const allOpen = !open.has(`dirs-shut:${change.ended}`);
  const rows = [];
  const walk = (nodes, depth) => {
    for (const node of nodes) {
      const pad = `style="padding-left:${8 + depth * 14}px"`;
      if (!node.children) {
        rows.push(`<li data-ended="${change.ended}" data-file="${esc(node.path)}" title="${esc(node.path)}" ${pad}>${hasDirs || depth ? `<span class="indent"></span>` : ""}${fileIconHtml(node.path, "file-icon")}<span class="path">${middle(node.name, 18)}</span>${stat(node.added, node.removed)}</li>`);
        continue;
      }
      const isOpen = open.has(`dir:${change.ended}:${node.path}`) ? !allOpen : allOpen;
      rows.push(`<li class="dir" data-dir="${esc(node.path)}" data-ended="${change.ended}" ${pad} aria-expanded="${isOpen}">${icon("chevron-right", "chev")}${icon(isOpen ? "folder" : "folder-closed", "file-icon")}<span class="path">${esc(node.name)}</span>${stat(node.added, node.removed)}</li>`);
      if (isOpen) walk(node.children, depth + 1);
    }
  };
  walk(tree, 0);
  return `<details class="changes" data-fold="${fold}"${open.has(fold) ? " open" : ""}><summary>${icon("chevron-right", "chev")}<span class="count">${n} changed file${n === 1 ? "" : "s"}</span>
      ${stat(sum(change.files, "added"), sum(change.files, "removed"))}<span class="show">Show files</span><span class="spacer"></span>
      ${hasDirs ? `<button type="button" class="xs all-dirs" data-all-dirs="${change.ended}" title="${allOpen ? "Collapse all folders" : "Expand all folders"}">${icon(allOpen ? "chevrons-down-up" : "chevrons-up-down")}</button>` : ""}
      <button class="open-diff" data-ended="${change.ended}" title="Open the full diff">${icon("file-diff")}<span>Open diff</span></button></summary>
    <ul>${rows.join("")}</ul></details>`;
}

/** A prompt pinned typed rather than the user: a workflow's opening task, a Slack reply it relayed, or
 * the nudge that carries on after a usage limit lifts. */
function sentBy(tab, user, first) {
  if (first && tab.view.connector) return "automation";
  return user.text === "Continue from where you left off." || user.text.startsWith("[Slack reply from ") ? "pinned" : null;
}

function automationHtml(user, t) {
  const key = `auto:${user.id}`;
  const open = t.open.has(key);
  return `<div class="user automation"><div class="bubble folded${open ? " open" : ""}"><button type="button" class="fold-head" data-toggle="${esc(key)}" aria-expanded="${open}">${icon("calendar-clock")}
      <span class="who">Automation</span><span class="summary">${esc(user.text.trim().split("\n")[0])}</span>${icon("chevron-down", "chev")}</button>
      ${open ? `<div class="body md">${markdown(user.text, promptMarked)}</div>` : ""}</div>
    ${open ? `<div class="meta"><button type="button" class="link" data-open-automation>Open automation</button>${whenHtml(user.at)}<button type="button" class="xs" data-copy-item="${esc(user.id)}" title="Copy">${icon("copy")}</button></div>` : ""}</div>`;
}

function userHtml(user, t, tab, first) {
  const by = sentBy(tab, user, first);
  if (by === "automation") return automationHtml(user, t);
  const key = `full:${user.id}`;
  const long = user.text.length > 600 || user.text.split("\n").length > 8;
  const clamped = long && !t.open.has(key);
  const pictures = user.pictures ?? [];
  const missing = user.images - pictures.length;
  const bubble = [
    pictures.length && `<div class="pictures">${pictures.map((src, i) => `<button type="button" class="picture" data-zoom="${esc(user.id)}:${i}"><img src="${esc(src)}" alt="Image ${i + 1}"></button>`).join("")}</div>`,
    user.text && `<div class="body md${clamped ? " clamped" : ""}">${markdown(user.text, promptMarked)}</div>`,
    missing > 0 && `<span class="images">${missing} image${missing > 1 ? "s" : ""}</span>`,
    long && `<button type="button" class="show-full" data-toggle="${esc(key)}">${clamped ? "Show full message" : "Show less"}</button>`,
  ];
  const above = [
    by === "pinned" && `<p class="sent-by">Sent by Pinned</p>`,
    user.steer && `<span class="steer" title="Steered the active turn">${icon("redo-2")}Steer</span>`,
  ];
  return `<div class="user">${above.filter(Boolean).join("")}<div class="bubble">${bubble.filter(Boolean).join("")}</div>
    <div class="meta">${whenHtml(user.at)}${user.text ? `<button type="button" class="xs" data-copy-item="${esc(user.id)}" title="Copy">${icon("copy")}</button>` : ""}</div></div>`;
}

function turnHtml(tab, turn, change, live, first) {
  const t = threadOf(tab);
  const cwd = tab.view.cwd;
  const work = live ? turn.work : turn.work.filter((s) => !turn.failed.has(s));
  const shown = live ? turn.shown : turn.steps.filter((s) => turn.shown.includes(s) || turn.failed.has(s));
  const steps = workHtml(turn.steps, work, cwd, t, live);
  let fold = "";
  if (live) {
    const compacting = /^\/compact\b/.test(turn.user?.text ?? "") && !turn.notes.some((n) => /compacted/.test(n.text));
    const active = turn.steps.slice(turn.steps.findLastIndex((s) => s.kind === "text") + 1).filter((s) => (s.kind === "tool" || s.kind === "thinking") && !shown.includes(s));
    const label = compacting ? `<span class="shimmer">Compacting…</span>` : `Working for <span class="ticker" data-since="${turn.start || Date.now()}">${ticking(turn.start ? Date.now() - turn.start : 0)}</span>`;
    fold = `<details class="work live" data-turn="${esc(turn.id)}"${t.open.has(turn.id) ? " open" : ""}>
        <summary><span class="label">${label}</span></summary>
        <div class="steps">${steps}</div></details>
      ${!t.open.has(turn.id) && !compacting ? liveHtml(turn, active, cwd) : ""}`;
  } else if (work.length) {
    const end = turn.user?.steer ? turn.end : turn.runEnd;
    const took = end > turn.start ? duration(end - turn.start) : "";
    const label =
      turn.interrupted ? (took ? `You stopped after ${took}` : "You stopped this response")
      : turn.user?.steer ? (took ? `Continued for ${took}` : "Continued")
      : took ? `Worked for ${took}` : "Worked";
    fold = `<details class="work" data-turn="${esc(turn.id)}"${t.open.has(turn.id) ? " open" : ""}>
        <summary><span class="label">${label}</span>${icon("chevron-right", "chev")}${whenHtml(turn.end && new Date(turn.end).toISOString())}</summary>
        <div class="steps">${steps}</div></details>`;
  }
  const notes = turn.notes.filter((n) => !(fold && !live && turn.interrupted && n.text === "Interrupted."));
  return `<section class="turn" data-turn-id="${esc(turn.id)}">${turn.user ? userHtml(turn.user, t, tab, first) : ""}${notes.map(noteHtml).join("")}${fold}
    ${shown.map((s) => shownHtml(s, cwd, t, live ? null : turn.answer)).join("")}
    ${turn.prs.map((p) => `<p class="pr-line"><button class="pr-link" data-pr="${esc(p.url)}" title="${esc(p.url)}">${icon("github")}PR #${p.number}</button><span class="muted">${esc(p.repo)}</span></p>`).join("")}
    ${change ? changesHtml(change, t.open) : ""}</section>`;
}

const TURNS_SHOWN = 40;

function renderThread() {
  const tab = active;
  const crumb = document.querySelector("#chat-head .crumb");
  if (!tab) {
    crumb.textContent = "";
    threadView.dataset.key = "";
    threadView.innerHTML = `<div class="blank"><p>No threads open.</p><button class="primary" data-new-thread>New thread</button></div>`;
    renderComposer(null);
    return;
  }
  const t = threadOf(tab);
  if (!crumb.querySelector("input.rename:focus")) crumb.innerHTML = crumbHtml(tab);
  const pr = prOf(tab);
  const chip = document.getElementById("pr-chip");
  chip.hidden = !pr;
  if (pr) chip.querySelector("span").textContent = `#${pr.number}`;

  const working = turnLive(tab);
  const limit = finder.open ? Infinity : t.limit ?? TURNS_SHOWN;
  const key = JSON.stringify([tab.key, tab.view.cwd, t.loaded, t.error, t.session, working, t.items.length, !!t.history, [...t.open], limit]);
  if (threadView.dataset.key === key) return renderComposer(tab);
  const turns = turnsOf(t.items);
  const changes = changesByTurn(t.history, turns);
  const opening = turns.find((x) => x.user);
  const first = Math.max(0, turns.length - limit);
  const body = turns.length
    ? `${first ? `<button type="button" class="earlier" data-earlier><span>Load earlier turns</span></button>` : ""}${turns.slice(first).map((turn, i) => turnHtml(tab, turn, changes.get(turn), working && first + i === turns.length - 1, turn === opening)).join("")}`
    : `<div class="blank"><p>${t.loaded ? "Send a message to start the conversation." : t.error ? `This thread couldn't be read: ${esc(t.error)}` : "Loading…"}</p><p class="hint">${esc(tilde(tab.view.cwd))}</p></div>`;
  const stick = threadView.dataset.tab !== tab.key || threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight < 40;
  const liveRows = threadView.querySelector(".work.live .group[open] > .rows");
  const follow = !liveRows || liveRows.scrollHeight - liveRows.scrollTop - liveRows.clientHeight < 24;
  threadView.innerHTML = `<div class="lane">${body}</div>`;
  threadView.dataset.key = key;
  threadView.dataset.tab = tab.key;
  markClippedTables(threadView);
  for (const rows of threadView.querySelectorAll(".group > .rows")) {
    rows.classList.toggle("scrolls", rows.scrollHeight > rows.clientHeight + 1);
    if (follow && rows.closest(".work.live")) rows.scrollTop = rows.scrollHeight;
  }
  if (stick) threadView.scrollTop = threadView.scrollHeight;
  if (finder.open) findMatches();
  showScrollEnd();
  renderComposer(tab);
}

const rerender = () => {
  threadView.dataset.key = "";
  renderThread();
};

setInterval(() => {
  for (const ticker of threadView.querySelectorAll(".ticker")) ticker.textContent = ticking(Date.now() - Number(ticker.dataset.since));
}, 1000);

const itemById = (id) => threadOf(active).items.find((i) => i.id === id);

/** A pull request link someone wrote in the thread, shaped like the session's own `pr` items. */
function prOfUrl(url) {
  const m = /^https:\/\/github\.com\/([\w.-]+\/[\w.-]+)\/pull\/(\d+)/i.exec(url ?? "");
  return m && { kind: "pr", number: Number(m[2]), url: m[0], repo: m[1] };
}

async function copied(button, text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    return;
  }
  const before = button.innerHTML;
  button.innerHTML = icon("check");
  button.classList.add("copied");
  setTimeout(() => {
    button.innerHTML = before;
    button.classList.remove("copied");
  }, 1200);
}

function downloadText(name, text) {
  const url = URL.createObjectURL(new Blob([text.trimEnd() + "\n"], { type: "text/markdown;charset=utf-8" }));
  const a = Object.assign(document.createElement("a"), { href: url, download: name });
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

const flip = (key) => flipped.delete(key) || flipped.add(key);

function tableText(table, format) {
  const rows = [...table.rows].map((row) => [...row.cells].map((cell) => cell.textContent.replace(/\s+/g, " ").trim()));
  if (format === "csv") return rows.map((r) => r.map((v) => (/[",\n]/.test(v) ? `"${v.replaceAll('"', '""')}"` : v)).join(",")).join("\n");
  const line = (r) => `| ${r.map((v) => v.replaceAll("|", "\\|")).join(" | ")} |`;
  return [line(rows[0] ?? []), line((rows[0] ?? []).map(() => "---")), ...rows.slice(1).map(line)].join("\n");
}

/** A table offers to expand its cells only when one is cut short in the collapsed layout. */
function markClippedTables(root) {
  for (const box of root.querySelectorAll(".md-table")) {
    const was = box.dataset.expanded;
    box.dataset.expanded = "false";
    box.dataset.clipped = [...box.querySelectorAll("th, td")].some((cell) => cell.scrollWidth > cell.clientWidth);
    box.dataset.expanded = was;
  }
}

// Code blocks and tables also render in the pull request panel.
document.addEventListener("click", (e) => {
  const block = e.target.closest?.(".codeblock");
  const table = e.target.closest?.(".md-table");
  if (block) {
    if (e.target.closest("[data-copy-code]")) copied(e.target.closest("[data-copy-code]"), block.querySelector("pre").textContent);
    else if (e.target.closest("[data-run-code]")) runInTerminal(block.querySelector("pre").textContent.trim());
    else if (e.target.closest("[data-code-wrap]")) {
      flip(block.dataset.code);
      const wrap = wraps(block.dataset.code);
      block.dataset.wrap = wrap;
      e.target.closest("[data-code-wrap]").title = wrap ? "Disable line wrap" : "Wrap lines";
    }
  } else if (table) {
    const menu = table.querySelector(".table-menu");
    if (e.target.closest("[data-table-expand]")) {
      flip(table.dataset.table);
      const open = wraps(table.dataset.table);
      table.dataset.expanded = open;
      Object.assign(e.target.closest("[data-table-expand]"), { title: open ? "Collapse table cells" : "Expand table cells", innerHTML: icon(open ? "minimize-2" : "maximize-2") });
    } else if (e.target.closest("[data-table-menu]")) menu.hidden = !menu.hidden;
    else if (e.target.closest("[data-copy-table]")) {
      copied(table.querySelector("[data-table-menu]"), tableText(table.querySelector("table"), e.target.closest("[data-copy-table]").dataset.copyTable));
      menu.hidden = true;
    }
  }
});

/** T3 Code runs a one-line shell block in its terminal; here the terminal is claude's, which runs a
 * line starting with `!` as a shell command. */
function runInTerminal(command) {
  if (!active || !sendSlash(active, `! ${command}`)) return;
  toggleDrawer(true);
}

document.addEventListener("error", (e) => {
  if (!e.target.matches?.("img.favicon")) return;
  failedFavicons.add(e.target.dataset.host);
  e.target.outerHTML = icon("globe");
}, true);

threadView.addEventListener("click", (e) => {
  if (e.target.closest("[data-new-thread]")) return toggleProjects();
  if (e.target.closest("[data-mode-terminal]")) return setMode("terminal");
  if (!active) return;
  const t = threadOf(active);
  const toggle = e.target.closest("[data-toggle]");
  if (toggle) {
    const key = toggle.dataset.toggle;
    if (!t.open.delete(key)) t.open.add(key);
    return rerender();
  }
  if (e.target.closest("[data-earlier]")) {
    const height = threadView.scrollHeight;
    t.limit = (t.limit ?? TURNS_SHOWN) + TURNS_SHOWN;
    rerender();
    threadView.scrollTop += threadView.scrollHeight - height;
    return;
  }
  const live = e.target.closest("[data-open-turn]");
  if (live) {
    t.open.add(live.dataset.openTurn);
    return rerender();
  }
  if (e.target.closest("[data-open-automation]")) {
    page = "connectors";
    return toggleSettings();
  }
  const copy = e.target.closest("[data-copy-item]");
  if (copy) return copied(copy, itemById(copy.dataset.copyItem)?.text ?? "");
  const zoom = e.target.closest("[data-zoom]");
  if (zoom) return zoomImage(zoom.querySelector("img").src);
  const allDirs = e.target.closest("[data-all-dirs]");
  if (allDirs) {
    e.preventDefault();
    const ended = allDirs.dataset.allDirs;
    for (const k of [...t.open]) if (k.startsWith(`dir:${ended}:`)) t.open.delete(k);
    if (!t.open.delete(`dirs-shut:${ended}`)) t.open.add(`dirs-shut:${ended}`);
    return rerender();
  }
  const dir = e.target.closest("[data-dir]");
  if (dir) {
    const k = `dir:${dir.dataset.ended}:${dir.dataset.dir}`;
    if (!t.open.delete(k)) t.open.add(k);
    return rerender();
  }
  const plan = e.target.closest("[data-plan]");
  if (plan) {
    const menu = plan.querySelector(".plan-menu");
    if (e.target.closest("[data-plan-menu]")) return (menu.hidden = !menu.hidden);
    const result = t.items.find((i) => i.kind === "result" && i.tool === plan.dataset.plan);
    const text = planText({ ...itemById(plan.dataset.plan), result });
    if (e.target.closest("[data-plan-copy]")) copied(e.target.closest("[data-plan-copy]"), text);
    if (e.target.closest("[data-plan-download]")) downloadText(planFile(text), text);
    menu.hidden = true;
  }
  const diff = e.target.closest(".open-diff, .changes li[data-file]");
  if (diff) {
    e.preventDefault();
    ui.diffTurn = Number(diff.dataset.ended);
    ui.diffFile = diff.dataset.file ?? null;
    return openPanel("diff");
  }
  const pr = e.target.closest("[data-pr]");
  if (pr) {
    e.preventDefault();
    return e.metaKey || e.ctrlKey ? openOutside(pr.dataset.pr) : openPr(active, pr.dataset.pr);
  }
  const link = e.target.closest("a[href]");
  if (link) {
    e.preventDefault();
    openLink(link.href, e.metaKey || e.ctrlKey);
  }
});
threadView.addEventListener("toggle", (e) => {
  const fold = e.target.dataset?.turn ?? e.target.dataset?.fold;
  if (!fold || !active) return;
  const open = threadOf(active).open;
  if ("auto" in e.target.dataset) {
    if (e.target.open) open.delete(`shut:${fold}`);
    else open.add(`shut:${fold}`);
  } else if (e.target.open) open.add(fold);
  else open.delete(fold);
  if (e.target.dataset.turn || "auto" in e.target.dataset) threadView.dataset.key = "";
  const rows = e.target.open && e.target.querySelector(":scope > .rows");
  if (rows) rows.classList.toggle("scrolls", rows.scrollHeight > rows.clientHeight + 1);
}, true);

const openOutside = (url) => WEB_URL.test(url) && (TAURI ? invoke("open_url", { url }).catch(console.warn) : window.open(url, "_blank", "noopener"));

function openLink(url, outside = false) {
  if (!outside && /^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0)(:\d+)?([/?#]|$)/.test(url)) {
    setBrowserUrl(url);
    return openPanel("browser");
  }
  openOutside(url);
}

function renderUnsupported() {
  threadView.dataset.key = "";
  threadView.innerHTML = `<div class="blank"><p>This thread can't be shown here yet: ${esc(machineName(active.machine))} or the hub runs an older pinned.</p>
    <p class="hint">Update pinned there, or use the terminal view.</p><button class="primary" data-mode-terminal>Terminal view</button></div>`;
}

const imageZoom = document.getElementById("image-zoom");
function zoomImage(src) {
  imageZoom.querySelector("img").src = src;
  imageZoom.hidden = false;
}
imageZoom.onclick = () => (imageZoom.hidden = true);
window.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || imageZoom.hidden) return;
  imageZoom.hidden = true;
  e.preventDefault();
  e.stopPropagation();
}, true);

const scrollEnd = document.getElementById("scroll-end");
let scrollTimer = 0;
function showScrollEnd() {
  const away = guiOn() && threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight > 40;
  scrollEnd.style.bottom = `${composer.offsetHeight + 4}px`;
  scrollEnd.hidden = !away;
}
threadView.addEventListener("scroll", () => {
  clearTimeout(scrollTimer);
  if (threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight <= 40) return showScrollEnd();
  scrollTimer = setTimeout(showScrollEnd, 150);
}, { passive: true });
scrollEnd.addEventListener("pointerdown", (e) => e.preventDefault());
scrollEnd.onclick = () => {
  threadView.scrollTo({ top: threadView.scrollHeight, behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  scrollEnd.hidden = true;
};

/** ⌘F: find in the thread. Matches are painted with the CSS highlight API, so the markdown and copied
 * text stay as they are; going to one opens the folds around it. */
const finder = { open: false, ranges: [], index: 0, bar: document.getElementById("thread-find") };
const findInput = finder.bar.querySelector("input");
finder.bar.querySelector('[data-find-step="-1"]').innerHTML = icon("chevron-up");

function findMatches() {
  const query = findInput.value.trim().toLowerCase();
  finder.ranges = [];
  if (query) {
    const walker = document.createTreeWalker(threadView, NodeFilter.SHOW_TEXT, {
      acceptNode: (n) => (n.parentElement.closest(".code-head, .meta, svg, button.xs, .plan-menu, .live-tool") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
    });
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const text = node.data.toLowerCase();
      for (let i = text.indexOf(query); i >= 0; i = text.indexOf(query, i + query.length)) {
        const range = new Range();
        range.setStart(node, i);
        range.setEnd(node, i + query.length);
        finder.ranges.push(range);
      }
    }
  }
  finder.index = Math.min(finder.index, Math.max(0, finder.ranges.length - 1));
  paintMatches();
}

function paintMatches() {
  const n = finder.ranges.length;
  const count = finder.bar.querySelector(".count");
  count.textContent = findInput.value.trim() ? (n ? `${finder.index + 1}/${n}` : "0/0") : "";
  count.classList.toggle("none", !!findInput.value.trim() && !n);
  for (const b of finder.bar.querySelectorAll("[data-find-step]")) b.disabled = !n;
  if (!window.CSS?.highlights || typeof Highlight === "undefined") return;
  CSS.highlights.set("thread-find", new Highlight(...finder.ranges.filter((_, i) => i !== finder.index)));
  CSS.highlights.set("thread-find-active", new Highlight(...(n ? [finder.ranges[finder.index]] : [])));
}

function goToMatch(step) {
  const n = finder.ranges.length;
  if (!n) return;
  finder.index = (((finder.index + step) % n) + n) % n;
  const t = threadOf(active);
  const node = finder.ranges[finder.index].startContainer.parentElement;
  let reopen = false;
  for (let el = node.closest("details:not([open])"); el; el = el.parentElement?.closest("details:not([open])")) {
    t.open.add(el.dataset.turn ?? el.dataset.fold);
    reopen = true;
  }
  const clamp = node.closest(".clamped");
  if (clamp) {
    const toggle = clamp.parentElement.querySelector("[data-toggle]");
    if (toggle) t.open.add(toggle.dataset.toggle);
    reopen = true;
  }
  if (reopen) {
    const at = finder.index;
    rerender();
    finder.index = at;
    paintMatches();
  }
  const rect = finder.ranges[finder.index]?.getBoundingClientRect();
  if (rect) {
    const box = threadView.getBoundingClientRect();
    if (rect.top < box.top + 48 || rect.bottom > box.bottom - composer.offsetHeight) threadView.scrollTop += rect.top - box.top - box.height / 3;
  }
  paintMatches();
}

function openFind() {
  if (!active) return;
  const reveal = !finder.open;
  finder.open = true;
  finder.bar.hidden = false;
  if (reveal) rerender();
  findInput.focus();
  findInput.select();
}

function closeFind() {
  finder.open = false;
  finder.bar.hidden = true;
  finder.ranges = [];
  if (window.CSS?.highlights) {
    CSS.highlights.delete("thread-find");
    CSS.highlights.delete("thread-find-active");
  }
  rerender();
}

findInput.addEventListener("input", () => {
  finder.index = 0;
  findMatches();
  if (finder.ranges.length) goToMatch(0);
});
findInput.addEventListener("keydown", (e) => {
  if (e.isComposing) return;
  if (e.key === "Enter") goToMatch(e.shiftKey ? -1 : 1);
  else if (e.key === "Escape") closeFind();
  else return;
  e.preventDefault();
  e.stopPropagation();
});
finder.bar.addEventListener("click", (e) => {
  const step = e.target.closest("[data-find-step]");
  if (step) goToMatch(Number(step.dataset.findStep));
  if (e.target.closest("[data-find-close]")) closeFind();
});
onGuiKey("f", openFind);

/** Appearance settings: how wide the thread grows, diff colors, and T3 Code's typography: font families
 * and sizes for the interface, the prompt, code and the terminal. */
const SANS = '-apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif';
const MONO = '"SF Mono", "SFMono-Regular", Menlo, Consolas, "Liberation Mono", monospace';
const range = (from, to) => Array.from({ length: to - from + 1 }, (_, i) => from + i);
const LOOKS = {
  "chat-width": { label: "Chat width", about: "Set how wide messages and the composer can grow on large screens.", options: [["", "Comfortable"], ["wide", "Wide"], ["full", "Full"]] },
  "diff-colors": { label: "Diff colors", about: "Choose colors for additions and deletions, including change counts.", options: [["", "Red & green"], ["blue-orange", "Blue & orange"]] },
};
/** Each font row: its family and size settings, the sizes it offers and its default size. */
const FONTS = [
  { name: "interface", label: "Interface font", about: "Everything outside code blocks and the terminal.", family: "sans-font", size: "ui-size", sizes: range(12, 20), base: 16, placeholder: "System default" },
  { name: "prompt", label: "Prompt font", about: "Only the box you write prompts in. Mono works well here.", family: "prompt-font", size: "prompt-size", sizes: range(12, 20), base: 14, placeholder: "Interface font", advanced: true },
  { name: "code", label: "Code font", simpleLabel: "Monospace font", about: "Code blocks, diffs, and file previews.", simpleAbout: "Code blocks, diffs, file previews, and the terminal.", family: "mono-font", size: "code-size", sizes: range(10, 18), base: 13, placeholder: "System monospace" },
  { name: "terminal", label: "Terminal font", about: "Terminal output, independent from code blocks and diffs.", family: "terminal-font", size: "terminal-size", sizes: range(8, 20), base: 12, placeholder: "System monospace", advanced: true },
];
const look = (name) => load(`look-${name}`) ?? "";
const advancedType = () => look("advanced") === "true";
const sizeOf = (font) => Number.parseInt(look(font.size)) || font.base;
const fontByName = (name) => FONTS.find((f) => f.name === name);

/** A typed font list as CSS: each name quoted unless it is a plain word. */
function fontStack(custom, fallback) {
  const names = custom.split(",").map((n) => n.trim().replace(/^(['"])(.*)\1$/, "$2").replaceAll('"', "")).filter(Boolean);
  return names.length ? `${names.map((n) => (/^[a-zA-Z][a-zA-Z0-9-]*$/.test(n) ? n : `"${n}"`)).join(", ")}, ${fallback}` : null;
}

/** The terminal's font: the code font unless Advanced gives it its own, as in T3 Code. Until one is set
 * it keeps pinned's own 12px terminal. */
function terminalLook() {
  const font = fontByName(advancedType() ? "terminal" : "code");
  return { fontFamily: fontStack(look(font.family), MONO) ?? "ui-monospace, Menlo, monospace", fontSize: look(font.size) ? sizeOf(font) : 12 };
}

function applyLooks() {
  const root = document.documentElement;
  for (const name of Object.keys(LOOKS)) {
    const key = name.replace(/-(\w)/g, (_, c) => c.toUpperCase());
    if (look(name)) root.dataset[key] = look(name);
    else delete root.dataset[key];
  }
  const vars = {
    "--g-sans": fontStack(look("sans-font"), SANS),
    "--g-mono": fontStack(look("mono-font"), MONO),
    "--prompt-font": fontStack(look("prompt-font"), "var(--g-sans)"),
    "--prompt-size": look("prompt-size") ? `${sizeOf(fontByName("prompt"))}px` : null,
    "--code-size": look("code-size") ? `${sizeOf(fontByName("code"))}px` : null,
    "--ui-zoom": sizeOf(fontByName("interface")) === 16 ? null : String(sizeOf(fontByName("interface")) / 16),
  };
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value);
  root.dataset.smoothing = look("smoothing") !== "false";
  if (vars["--ui-zoom"]) root.dataset.zoom = "";
  else delete root.dataset.zoom;
  for (const tab of typeof tabs === "undefined" ? [] : tabs) {
    Object.assign(tab.term.options, terminalLook());
    refit(tab);
  }
}

const sizeSelect = (font) =>
  `<select id="look-${font.size}" data-look="${font.size}" aria-label="${font.label} size">${font.sizes.map((n) => `<option value="${n === font.base ? "" : `${n}px`}"${sizeOf(font) === n ? " selected" : ""}>${n} px</option>`).join("")}</select>`;
const switchRow = (name, label, about, on) =>
  `<div class="look switch-row"><div><label for="look-${name}">${label}</label><p class="meta">${about}</p></div><span class="switch"><input type="checkbox" id="look-${name}" data-look-switch="${name}"${on ? " checked" : ""}><span></span></span></div>`;

function appearanceHtml() {
  const advanced = advancedType();
  const fonts = FONTS.filter((f) => advanced || !f.advanced)
    .map((f) => `<div class="look font-row"><label for="look-${f.family}">${advanced ? f.label : f.simpleLabel ?? f.label}</label><p class="meta">${advanced ? f.about : f.simpleAbout ?? f.about}</p>
      <div class="pair"><input id="look-${f.family}" data-look-font="${f.family}" value="${esc(look(f.family))}" placeholder="${f.placeholder}" maxlength="200" spellcheck="false" autocomplete="off" autocapitalize="off">${sizeSelect(f)}</div></div>`)
    .join("");
  return `<h2 class="looks-title">Appearance</h2>${Object.entries(LOOKS)
    .map(([name, l]) => `<div class="look"><label for="look-${name}">${l.label}</label><p class="meta">${l.about}</p>
      <select id="look-${name}" data-look="${name}">${l.options.map(([v, label]) => `<option value="${v}"${look(name) === v ? " selected" : ""}>${label}</option>`).join("")}</select></div>`)
    .join("")}
    <div class="looks-head"><h3>Typography</h3><label class="advanced">Advanced <span class="switch"><input type="checkbox" id="look-advanced" data-look-switch="advanced"${advanced ? " checked" : ""} aria-label="Show advanced typography settings"><span></span></span></label></div>
    ${fonts}
    ${advanced && IS_MAC ? switchRow("smoothing", "Font smoothing", "Use thinner grayscale text smoothing instead of the macOS default.", look("smoothing") !== "false") : ""}
    ${switchRow("word-wrap", "Word wrap", "Wrap long lines in code blocks, tables, diffs, and file previews by default.", wrapsByDefault())}`;
}

function bindAppearance() {
  const changed = () => {
    applyLooks();
    rendered.clear();
    if (guiOn()) rerender();
  };
  for (const select of settingsPage.querySelectorAll("[data-look]")) {
    select.onchange = () => {
      save(`look-${select.dataset.look}`, select.value);
      changed();
    };
  }
  for (const input of settingsPage.querySelectorAll("[data-look-switch]")) {
    input.onchange = () => {
      save(`look-${input.dataset.lookSwitch}`, String(input.checked));
      changed();
      if (input.dataset.lookSwitch === "advanced") showGeneral();
    };
  }
  for (const input of settingsPage.querySelectorAll("[data-look-font]")) {
    let timer = 0;
    const commit = () => {
      clearTimeout(timer);
      save(`look-${input.dataset.lookFont}`, input.value.trim());
      changed();
    };
    input.oninput = () => {
      clearTimeout(timer);
      timer = setTimeout(commit, 400);
    };
    input.onchange = commit;
    input.onkeydown = (e) => {
      if (e.key === "Enter") commit();
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      clearTimeout(timer);
      input.value = look(input.dataset.lookFont);
    };
  }
}
applyLooks();
