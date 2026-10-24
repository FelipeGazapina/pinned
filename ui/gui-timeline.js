function turnsOf(items) {
  const turns = [];
  const results = new Map(items.filter((i) => i.kind === "result").map((r) => [r.tool, r]));
  let turn = null;
  for (const item of items) {
    if (item.kind === "result" || item.kind === "model") continue;
    if (item.kind === "user" || !turn) {
      turn = { id: item.id ?? `t${turns.length}`, user: item.kind === "user" ? item : null, steps: [], prs: [], notes: [] };
      turns.push(turn);
      if (item.kind === "user") continue;
    }
    if (item.kind === "pr") turn.prs.push(item);
    else if (item.kind === "note") turn.notes.push(item);
    else turn.steps.push(item.kind === "tool" ? { ...item, result: results.get(item.id) } : item);
  }
  for (const t of turns) {
    const texts = t.steps.filter((s) => s.kind === "text");
    t.answer = t.steps.at(-1)?.kind === "text" ? t.steps.at(-1) : null;
    t.lead = texts[0] && texts[0] !== t.answer ? texts[0] : null;
    t.work = t.steps.filter((s) => s !== t.lead && s !== t.answer);
    const times = [t.user?.at, ...t.steps.flatMap((s) => [s.at, s.result?.at])].filter(Boolean).map(Date.parse).filter(Number.isFinite);
    t.start = times.length ? Math.min(...times) : 0;
    t.end = times.length ? Math.max(...times) : 0;
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

const WEB_URL = /^https?:\/\//i;
/** Assistant text and pull request comments come from others, so only web links render, and an image
 * is a link to it rather than a request made on sight. */
marked.use({
  gfm: true,
  renderer: {
    html: ({ text }) => esc(text),
    link({ href, tokens }) {
      const text = this.parser.parseInline(tokens);
      return WEB_URL.test(href) ? `<a href="${esc(href)}">${text}</a>` : text;
    },
    image: ({ href, text }) => (WEB_URL.test(href) ? `<a href="${esc(href)}">${esc(text || href)}</a>` : esc(text)),
  },
});
const rendered = new Map();
function markdown(text) {
  let html = rendered.get(text);
  if (html === undefined) {
    if (rendered.size > 4000) rendered.clear();
    html = marked.parse(text);
    rendered.set(text, html);
  }
  return html;
}

const TOOL_ICONS = { Bash: "terminal", Read: "eye", Edit: "square-pen", MultiEdit: "square-pen", Write: "square-pen", NotebookEdit: "square-pen", Grep: "search", Glob: "search", WebFetch: "globe", WebSearch: "globe", Agent: "bot", Task: "bot" };
const shortPath = (path, cwd) => (cwd && path.startsWith(`${cwd}/`) ? path.slice(cwd.length + 1) : tilde(path));

function toolLabel(step, cwd) {
  const what = /^(Read|Write|Edit|MultiEdit|NotebookEdit)$/.test(step.name) ? shortPath(step.summary, cwd) : step.summary;
  const verb = { Read: "Read", Edit: "Edited", MultiEdit: "Edited", Write: "Wrote", Grep: "Searched", Glob: "Listed", WebFetch: "Fetched", WebSearch: "Searched the web for", Agent: "Delegated", Task: "Delegated", Skill: "Used skill" }[step.name];
  if (step.name === "Bash") return what || step.detail.split("\n")[0];
  return verb ? `${verb} ${what}` : `${step.name.replace(/^mcp__([^_]+)__/, "$1 · ")}${what ? ` ${what}` : ""}`;
}

function toolHtml(step, cwd, open) {
  const name = /^mcp__/.test(step.name) ? "wrench" : TOOL_ICONS[step.name] ?? "zap";
  const state = !step.result ? "running" : step.result.error ? "error" : "ok";
  const body = [step.detail && `<pre class="input">${esc(step.detail)}</pre>`, step.result?.text && `<pre>${esc(step.result.text)}</pre>`].filter(Boolean).join("");
  return `<details class="tool" data-fold="${esc(step.id)}" data-state="${state}"${body ? "" : " data-empty"}${open.has(step.id) ? " open" : ""}><summary>${icon(name, "kind")}<span class="label">${esc(toolLabel(step, cwd))}</span>
    ${state === "error" ? icon("x", "failed") : ""}${body ? icon("chevron-right", "chev") : ""}</summary>${body}</details>`;
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

function changesHtml(change, open) {
  const n = change.files.length;
  const fold = `changes:${change.ended}`;
  return `<details class="changes" data-fold="${fold}"${open.has(fold) ? " open" : ""}><summary>${icon("chevron-right", "chev")}<span class="count">${n} changed file${n === 1 ? "" : "s"}</span>
      <span class="add">+${sum(change.files, "added")}</span><span class="del">−${sum(change.files, "removed")}</span><span class="show">Show files</span>
      <button class="open-diff" data-ended="${change.ended}">${icon("file-diff")}Open diff</button></summary>
    <ul>${change.files.map((f) => `<li data-ended="${change.ended}" data-file="${esc(f.path)}"><span class="path">${esc(f.path)}</span><span class="add">+${f.added}</span><span class="del">−${f.removed}</span></li>`).join("")}</ul></details>`;
}

function turnHtml(tab, turn, change, live) {
  const t = threadOf(tab);
  const user = turn.user
    ? `<div class="user"><div class="bubble">${esc(turn.user.text)}${turn.user.images ? `<span class="images">${turn.user.images} image${turn.user.images > 1 ? "s" : ""}</span>` : ""}</div></div>`
    : "";
  const steps = turn.work.map((s) => (s.kind === "tool" ? toolHtml(s, tab.view.cwd, t.open) : `<div class="narration md">${markdown(s.text)}</div>`)).join("");
  let fold = "";
  if (live) {
    const running = [...turn.steps].reverse().find((s) => s.kind === "tool");
    fold = `<details class="work live" data-turn="${esc(turn.id)}"${t.open.has(turn.id) ? " open" : ""}>
        <summary><span class="label">Working for <span class="ticker" data-since="${turn.start || Date.now()}">${duration(turn.start ? Date.now() - turn.start : 0)}</span></span></summary>
        <div class="steps">${steps}</div></details>
      ${running && !t.open.has(turn.id) ? `<div class="live-tool">${toolHtml(running, tab.view.cwd, t.open)}</div>` : ""}`;
  } else if (turn.work.length) {
    fold = `<details class="work" data-turn="${esc(turn.id)}"${t.open.has(turn.id) ? " open" : ""}>
        <summary><span class="label">Worked for ${duration(turn.end - turn.start)}</span>${icon("chevron-right", "chev")}</summary>
        <div class="steps">${steps}</div></details>`;
  }
  const said = (s) => (s ? `<div class="assistant md">${markdown(s.text)}</div>` : "");
  return `<section class="turn">${user}${turn.notes.map((n) => `<p class="note">${esc(n.text)}</p>`).join("")}${fold}${said(turn.lead)}${said(turn.answer)}
    ${turn.prs.map((p) => `<p class="pr-line"><button class="pr-link" data-pr="${esc(p.url)}">${icon("github")}PR #${p.number}</button><span class="muted">${esc(p.repo)}</span></p>`).join("")}
    ${change ? changesHtml(change, t.open) : ""}</section>`;
}

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
  crumb.innerHTML = `<span class="project">${esc(projectName(tab))}</span><span class="sep">/</span><span class="title" title="${esc(threadTitle(tab))}">${esc(threadTitle(tab))}</span>`;
  const pr = prOf(tab);
  const chip = document.getElementById("pr-chip");
  chip.hidden = !pr;
  if (pr) chip.querySelector("span").textContent = `#${pr.number}`;

  const turns = turnsOf(t.items);
  const working = turnLive(tab);
  const changes = changesByTurn(t.history, turns);
  const body = turns.length
    ? turns.map((turn, i) => turnHtml(tab, turn, changes.get(turn), working && i === turns.length - 1)).join("")
    : `<div class="blank"><p>${t.loaded ? "Send a message to start the conversation." : t.error ? `This thread couldn't be read: ${esc(t.error)}` : "Loading…"}</p><p class="hint">${esc(tilde(tab.view.cwd))}</p></div>`;
  const key = JSON.stringify([tab.key, tab.view.cwd, t.loaded, t.error, t.session, working, t.items.length, !!t.history, [...t.open]]);
  if (threadView.dataset.key === key) return renderComposer(tab);
  const stick = threadView.dataset.tab !== tab.key || threadView.scrollHeight - threadView.scrollTop - threadView.clientHeight < 40;
  threadView.innerHTML = `<div class="lane">${body}</div>`;
  threadView.dataset.key = key;
  threadView.dataset.tab = tab.key;
  if (stick) threadView.scrollTop = threadView.scrollHeight;
  renderComposer(tab);
}

setInterval(() => {
  for (const ticker of threadView.querySelectorAll(".ticker")) ticker.textContent = duration(Date.now() - Number(ticker.dataset.since));
}, 1000);

threadView.addEventListener("click", (e) => {
  if (e.target.closest("[data-new-thread]")) return toggleProjects();
  if (e.target.closest("[data-mode-terminal]")) return setMode("terminal");
  const diff = e.target.closest(".open-diff, .changes li");
  if (diff) {
    e.preventDefault();
    ui.diffTurn = Number(diff.dataset.ended);
    ui.diffFile = diff.dataset.file ?? null;
    return openPanel("diff");
  }
  const pr = e.target.closest("[data-pr]");
  if (pr) return e.metaKey || e.ctrlKey ? openOutside(pr.dataset.pr) : openPr(active, pr.dataset.pr);
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
  if (e.target.open) open.add(fold);
  else open.delete(fold);
  if (e.target.dataset.turn) threadView.dataset.key = "";
}, true);

const openOutside = (url) => WEB_URL.test(url) && (TAURI ? invoke("open_url", { url }).catch(console.warn) : window.open(url, "_blank", "noopener"));

function openLink(url, outside = false) {
  if (!outside && /^https?:\/\/(localhost|127\.0\.0\.1|0\.0\.0\.0)(:\d+)?/.test(url)) {
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
