function openPanel(kind) {
  ui.panel = kind;
  ui.panelOpen = true;
  save("gui-panel", kind);
  save("gui-panel-open", "true");
  renderPanel();
}
function closePanel() {
  ui.panelOpen = false;
  save("gui-panel-open", "false");
  renderPanel();
}
const togglePanel = () => (ui.panelOpen ? closePanel() : openPanel(ui.panel));
document.getElementById("toggle-panel").onclick = togglePanel;
document.getElementById("panel-close").onclick = closePanel;
onGuiKey("b", togglePanel, { alt: true });
onGuiKey("d", () => (ui.panelOpen && ui.panel === "diff" ? closePanel() : openPanel("diff")));
document.getElementById("pr-chip").onclick = (e) => (e.metaKey || e.ctrlKey ? openOutside(prOf(active).url) : openPr(active, prOf(active).url));
for (const b of document.querySelectorAll("#panel-tabs [data-panel]")) b.onclick = () => openPanel(b.dataset.panel);
document.getElementById("panel-expand").onclick = () => {
  document.body.dataset.panelMax = document.body.dataset.panelMax === "true" ? "false" : "true";
};

const PANEL_MIN = 360;
const setPanelWidth = (px) => {
  const max = Math.max(PANEL_MIN, Math.min(Math.floor(innerWidth * 0.7), innerWidth - 360 - (ui.sidebarOpen ? 256 : 0)));
  const width = Math.round(Math.min(max, Math.max(PANEL_MIN, px)));
  document.documentElement.style.setProperty("--panel-width", `${width}px`);
  return width;
};
setPanelWidth(Number(load("gui-panel-width")) || 540);
document.getElementById("panel-resize").onmousedown = (down) => {
  down.preventDefault();
  const right = document.getElementById("panel").getBoundingClientRect().right;
  document.body.classList.add("resizing");
  drag(down, {
    move: (e) => setPanelWidth(right - e.clientX),
    drop: (e) => save("gui-panel-width", String(setPanelWidth(right - e.clientX))),
  });
  addEventListener("mouseup", () => document.body.classList.remove("resizing"), { once: true });
};

let shownPanel = "";
/** Sets the panel's HTML only when it changed, so a render that changes nothing keeps its scroll. */
function showPanel(html) {
  if (html === shownPanel && panelBody.isConnected) return;
  shownPanel = html;
  panelBody.innerHTML = html;
}

function renderPanel() {
  document.body.dataset.panel = ui.panelOpen ? "open" : "closed";
  if (!guiOn() || !ui.panelOpen) return;
  const pr = active && shownPr(active);
  for (const b of document.querySelectorAll("#panel-tabs [data-panel]")) {
    b.classList.toggle("active", b.dataset.panel === ui.panel);
    if (b.dataset.panel === "pr") b.querySelector("span").textContent = pr ? `#${pr.number}` : "Pull request";
  }
  panelBody.dataset.kind = ui.panel;
  if (ui.panel === "diff") renderDiffPanel();
  else if (ui.panel === "browser") renderBrowser();
  else renderPrPanel();
}

function renderDiffPanel() {
  const tab = active;
  const history = tab && threadOf(tab).history;
  if (!tab) return showPanel(`<p class="panel-empty">Select a thread to inspect turn diffs.</p>`);
  if (threadOf(tab).workspace?.repo === false) {
    return showPanel(`<p class="panel-empty">Turn diffs are unavailable because this folder is not a git repository.</p>`);
  }
  if (!history) {
    showPanel(`<p class="panel-empty">Loading changes…</p>`);
    return fetchHistory(tab);
  }
  if (!history.length) return showPanel(`<p class="panel-empty">No completed turns with changes yet.</p>`);
  const chosen = history.find((h) => h.ended === ui.diffTurn) ?? history.at(-1);
  const scope = `<label class="scope"><select id="diff-turn">${history.map((h, i) => `<option value="${h.ended}"${h === chosen ? " selected" : ""}>Turn ${i + 1}</option>`).join("")}</select>${icon("chevron-down")}</label>`;
  const files = parseDiff(chosen.patch);
  showPanel(`${diffToolbar(sum(files, "added"), sum(files, "removed"), scope)}
    <div class="diff ${ui.diffView}${ui.diffWrap ? " wrap" : ""}">${filesHtml(files, chosen.ended)}
    ${chosen.truncated ? `<p class="panel-empty">This diff exceeds the size limit, so its end is cut.</p>` : ""}</div>`);
  document.getElementById("diff-turn").onchange = (e) => {
    ui.diffTurn = Number(e.target.value);
    renderDiffPanel();
  };
  if (ui.diffFile) {
    panelBody.querySelector(`.file[data-id="${CSS.escape(`${chosen.ended}:${ui.diffFile}`)}"]`)?.scrollIntoView({ block: "start" });
    ui.diffFile = null;
  }
}

panelBody.addEventListener("click", (e) => {
  const action = e.target.closest("[data-diff]")?.dataset.diff;
  if (action === "unified" || action === "split") {
    ui.diffView = action;
    save("gui-diff-view", action);
  } else if (action === "wrap") {
    ui.diffWrap = !ui.diffWrap;
    save("gui-diff-wrap", String(ui.diffWrap));
  } else if (action === "collapse") {
    const ids = [...panelBody.querySelectorAll(".file")].map((f) => f.dataset.id);
    const all = ids.every((id) => ui.collapsed.has(id));
    for (const id of ids) all ? ui.collapsed.delete(id) : ui.collapsed.add(id);
  } else if (e.target.closest("[data-copy]")) {
    navigator.clipboard?.writeText(e.target.closest("[data-copy]").dataset.copy);
    return;
  } else if (e.target.closest(".file > header")) {
    const id = e.target.closest(".file").dataset.id;
    ui.collapsed.has(id) ? ui.collapsed.delete(id) : ui.collapsed.add(id);
  } else if (e.target.closest("[data-pr-tab]")) {
    ui.prTab = e.target.closest("[data-pr-tab]").dataset.prTab;
  } else if (e.target.closest("[data-refresh-pr]")) {
    ui.pr = null;
  } else if (e.target.closest("[data-open-url]")) {
    return openOutside(e.target.closest("[data-open-url]").dataset.openUrl);
  } else if (e.target.closest("a[href]")) {
    e.preventDefault();
    return openLink(e.target.closest("a[href]").href, e.metaKey || e.ctrlKey);
  } else return;
  renderPanel();
});

function seenUrls(tab) {
  const found = new Set();
  const pattern = /https?:\/\/(?:localhost|127\.0\.0\.1|0\.0\.0\.0)(?::\d+)?(?:\/[^\s"'`)<>\]]*)?/g;
  for (const item of [...threadOf(tab).items].reverse()) {
    for (const m of `${item.text ?? ""} ${item.detail ?? ""}`.matchAll(pattern)) found.add(m[0].replace(/[.,;:]+$/, ""));
    if (found.size >= 8) break;
  }
  return [...found];
}

function setBrowserUrl(url) {
  ui.browserUrl = url;
  save("gui-browser-url", url);
}

function renderBrowser() {
  if (panelBody.querySelector(".browser") && panelBody.dataset.url === ui.browserUrl) return;
  panelBody.dataset.url = ui.browserUrl;
  const urls = active ? seenUrls(active) : [];
  const remote = active && active.machine !== LOCAL;
  showPanel(`<div class="browser"><form class="url-bar">
      <button type="button" class="icon" data-nav="back" title="Back">${icon("arrow-left")}</button><button type="button" class="icon" data-nav="forward" title="Forward">${icon("arrow-right")}</button>
      <button type="button" class="icon" data-nav="reload" title="Reload">${icon("rotate-cw")}</button>
      <input name="url" value="${esc(ui.browserUrl)}" placeholder="Search or enter URL" spellcheck="false" autocomplete="off">
      <button type="button" class="icon" data-nav="external" title="Open in system browser">${icon("external-link")}</button></form>
    ${ui.browserUrl
      ? `<iframe src="${esc(ui.browserUrl)}" title="Preview" sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads"></iframe>`
      : `<div class="browser-empty">${icon("globe", "big")}<p class="title">No preview yet</p><p>Type a URL above, or start a dev server in this thread.</p>
        ${urls.length ? `<p class="group">Local servers</p><div class="servers">${urls.map((u) => `<button type="button" data-url="${esc(u)}">${icon("radio-tower")}<span><b>Listening</b><span>${esc(u.replace(/^https?:\/\//, ""))}</span></span></button>`).join("")}</div>` : ""}
        ${remote ? `<p class="hint">This thread runs on ${esc(machineName(active.machine))}, so its local servers are that machine's, not this device's.</p>` : ""}</div>`}</div>`);
  const form = panelBody.querySelector(".url-bar");
  form.onsubmit = (e) => {
    e.preventDefault();
    let url = form.url.value.trim();
    if (url && !/^https?:\/\//.test(url)) {
      url = /^(localhost|127\.|0\.0\.0\.0|\[::1\])/.test(url) ? `http://${url}` : /\.\w{2,}($|\/)/.test(url) ? `https://${url}` : `https://duckduckgo.com/?q=${encodeURIComponent(url)}`;
    }
    setBrowserUrl(url);
    renderBrowser();
  };
  form.onclick = (e) => {
    const nav = e.target.closest("[data-nav]")?.dataset.nav;
    const frame = panelBody.querySelector("iframe");
    if (nav === "external" && ui.browserUrl) openOutside(ui.browserUrl);
    else if (nav === "reload" && frame) frame.src = frame.src;
    else if ((nav === "back" || nav === "forward") && frame) {
      try {
        frame.contentWindow.history[nav]();
      } catch {}
    }
  };
  for (const b of panelBody.querySelectorAll("[data-url]")) {
    b.onclick = () => {
      setBrowserUrl(b.dataset.url);
      renderBrowser();
    };
  }
}

const checkState = (c) => {
  const v = (c.conclusion || c.state || c.status || "").toUpperCase();
  if (["SUCCESS", "NEUTRAL", "SKIPPED"].includes(v)) return "ok";
  if (["FAILURE", "ERROR", "CANCELLED", "TIMED_OUT", "ACTION_REQUIRED", "STARTUP_FAILURE"].includes(v)) return "fail";
  return "pending";
};
const CHECK_ICONS = { ok: "circle-check", fail: "circle-alert", pending: "circle-dashed" };

async function loadPr(tab, pr) {
  const key = `${tab.key}:${pr.url}`;
  ui.pr = { key, loading: true };
  renderPrPanel();
  try {
    const data = await callOn(tab.machine, { cmd: "pull_request", tab: tab.view.id, pr: pr.url });
    ui.pr = { key, data, at: Date.now() };
  } catch (e) {
    ui.pr = { key, error: String(e) };
  }
  if (ui.panel === "pr" && ui.panelOpen) renderPrPanel();
}

/** The pull request the thread last clicked, or else its latest. */
const shownPr = (tab) => {
  const t = threadOf(tab);
  return t.items.find((i) => i.kind === "pr" && i.url === t.prUrl) ?? prOf(tab);
};
function openPr(tab, url) {
  threadOf(tab).prUrl = url;
  openPanel("pr");
}

function renderPrPanel() {
  const tab = active;
  const pr = tab && shownPr(tab);
  if (!pr) {
    showPanel(`<p class="panel-empty">No pull request in this thread yet. When the session opens one with <code>gh pr create</code>, it shows here.</p>`);
    return;
  }
  const key = `${tab.key}:${pr.url}`;
  if (ui.pr?.key !== key) return loadPr(tab, pr);
  if (ui.pr.loading) return showPanel(`<p class="panel-empty">Loading #${pr.number}…</p>`);
  if (ui.pr.error) {
    showPanel(`<div class="panel-empty"><p>${esc(ui.pr.error)}</p><button class="outline" data-open-url="${esc(pr.url)}">${icon("external-link")}Open #${pr.number} on GitHub</button></div>`);
    return;
  }
  const d = ui.pr.data;
  const state = d.isDraft ? "draft" : (d.state ?? "open").toLowerCase();
  const checks = d.statusCheckRollup ?? [];
  const failed = checks.filter((c) => checkState(c) === "fail").length;
  const passed = checks.filter((c) => checkState(c) === "ok").length;
  const files = ui.prTab === "code" ? parseDiff(d.diff ?? "") : [];
  const head = `<div class="pr-head">
      <div class="crumbs"><span class="muted">${esc(pr.repo)}</span><button class="num ${state}" data-open-url="${esc(d.url)}" title="Open on GitHub">#${d.number}${icon("external-link")}</button>
        <span class="spacer"></span><button class="icon" data-refresh-pr title="Refresh">${icon("rotate-cw")}</button><span class="state ${state}">${icon("git-pull-request")}${esc(state[0].toUpperCase() + state.slice(1))}</span></div>
      <h3>${esc(d.title)}</h3>
      <p class="muted">${esc(d.author?.login ?? "")}${d.createdAt ? ` · opened ${esc(ago(Date.parse(d.createdAt) / 1000))} ago` : ""}</p>
      <p class="refs"><code>${esc(d.baseRefName)}</code>${icon("arrow-left")}<code>${esc(d.headRefName)}</code><span class="spacer"></span>${icon("file-diff")}${d.changedFiles}<span class="add">+${d.additions}</span><span class="del">-${d.deletions}</span></p>
      <div class="tabs"><span class="seg"><button data-pr-tab="summary" class="${ui.prTab === "summary" ? "on" : ""}">Summary</button><button data-pr-tab="code" class="${ui.prTab === "code" ? "on" : ""}">Code</button></span>
        <span class="checks-line ${failed ? "fail" : passed === checks.length ? "ok" : "pending"}">${checks.length ? `${icon(CHECK_ICONS[failed ? "fail" : passed === checks.length ? "ok" : "pending"])}${passed} passed${failed ? ` / ${failed} failed` : ""}` : "No checks"}</span>
        ${d.reviewDecision ? `<span class="muted">${esc(d.reviewDecision.toLowerCase().replace(/_/g, " "))}</span>` : ""}</div></div>`;
  const summary = `<div class="pr-body">
      <h4>Description</h4><div class="md">${d.body ? markdown(d.body) : `<p class="muted"><em>No description provided.</em></p>`}</div>
      ${checks.length ? `<h4>Checks</h4><ul class="checks">${checks.map((c) => `<li class="${checkState(c)}">${icon(CHECK_ICONS[checkState(c)])}<span>${esc(c.name || c.context || "check")}</span><span class="muted">${esc((c.conclusion || c.status || c.state || "").toLowerCase())}</span>${c.detailsUrl || c.targetUrl ? `<a href="${esc(c.detailsUrl || c.targetUrl)}">Details</a>` : ""}</li>`).join("")}</ul>` : ""}
      ${(d.reviews ?? []).length ? `<h4>Reviews</h4><ul class="reviews">${d.reviews.map((r) => `<li><b>${esc(r.author?.login ?? "")}</b> <span class="muted">${esc((r.state ?? "").toLowerCase().replace(/_/g, " "))}</span>${r.body ? `<div class="md">${markdown(r.body)}</div>` : ""}</li>`).join("")}</ul>` : ""}
      ${(d.comments ?? []).length ? `<h4>Comments (${d.comments.length})</h4><ul class="reviews">${d.comments.map((c) => `<li><b>${esc(c.author?.login ?? "")}</b><div class="md">${markdown(c.body ?? "")}</div></li>`).join("")}</ul>` : ""}</div>`;
  const code = `${diffToolbar(d.additions, d.deletions, `<span class="muted">${d.changedFiles} file${d.changedFiles === 1 ? "" : "s"}</span>`)}
    <div class="diff ${ui.diffView}${ui.diffWrap ? " wrap" : ""}">${filesHtml(files, `pr${d.number}`)}${d.truncated ? `<p class="panel-empty">This diff exceeds the size limit, so its end is cut.</p>` : ""}</div>`;
  showPanel(`<div class="pr">${head}${ui.prTab === "code" ? code : summary}</div>`);
}
