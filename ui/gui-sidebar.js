function markThreadSeen(tab) {
  if (!tab || !guiOn() || document.hidden) return;
  seenAt[tab.key] = tab.view.updated ?? 0;
  save("gui-seen", JSON.stringify(seenAt));
}

function statusSlot(tab) {
  const phase = phaseOf(tab);
  if (phase === "asking") return { icon: "shield-question", text: "Approval", tone: "warning" };
  if (phase === "working") return { icon: "circle-dashed", text: "Working", tone: "info" };
  if (phase === "input") return { icon: "circle-alert", text: "Input", tone: "input" };
  if (phase === "failed") return tab.view.resume_at ? { icon: "circle-alert", text: "Limited", tone: "warning" } : { icon: "circle-alert", text: "Failed", tone: "error" };
  if (unseenDone(tab)) return { icon: "circle-check", text: "Done", tone: "success" };
  return null;
}

function renderSidebar() {
  const projects = [...new Set(tabs.map(projectName))].sort();
  if (ui.project && !projects.includes(ui.project)) ui.project = "";
  const filter = document.getElementById("project-filter");
  const options = ["", ...projects].map((p) => `<option value="${esc(p)}"${p === ui.project ? " selected" : ""}>${p ? esc(p) : "All projects"}</option>`).join("");
  if (filter.dataset.built !== options) {
    filter.innerHTML = options;
    filter.dataset.built = options;
  }
  const needle = ui.search.trim().toLowerCase();
  const shown = tabs.filter((t) => (!ui.project || projectName(t) === ui.project)
    && (!needle || `${threadTitle(t)} ${projectName(t)} ${threadOf(t).workspace?.branch ?? ""} ${prOf(t)?.number ?? ""}`.toLowerCase().includes(needle)));
  const live = shown.filter((t) => !settled(t));
  const old = shown.filter(settled);
  const rows = live.map(threadRow);
  if (old.length) {
    const head = document.createElement("button");
    head.className = "settled-head";
    head.dataset.open = ui.settledOpen;
    head.innerHTML = `<span>${ui.settledOpen ? "Settled" : `Settled (${old.length})`}</span><span class="line"></span>${icon("chevron-down")}`;
    head.onclick = () => {
      ui.settledOpen = !ui.settledOpen;
      save("gui-settled-open", String(ui.settledOpen));
      renderSidebar();
    };
    rows.push(head, ...(ui.settledOpen ? old.map(threadRow) : []));
  }
  threadList.replaceChildren(...rows);
  if (!shown.length) threadList.innerHTML = `<p class="empty">${tabs.length ? "No threads found" : "No threads yet"}</p>`;
}

function threadRow(tab) {
  const t = threadOf(tab);
  const row = document.createElement("button");
  row.className = "thread-row";
  row.classList.toggle("active", tab === active);
  const slot = statusSlot(tab);
  row.classList.toggle("receded", tab !== active && !slot);
  row.classList.toggle("working", phaseOf(tab) === "working" && tab !== active);
  row.dataset.tab = tab.key;
  const pr = prOf(tab);
  const ws = t.workspace;
  const remote = tab.machine !== LOCAL ? `<span class="machine" title="${esc(machineName(tab.machine))}">${esc(initials(machineName(tab.machine)))}</span>` : "";
  const where = ws?.branch
    ? `${ws.worktree ? icon("folder-git", "dim") : ""}<span class="branch">${esc(ws.branch)}</span>`
    : `<span class="branch">${esc(tilde(tab.view.cwd))}</span>`;
  row.innerHTML = `<span class="l1"><span class="project">${esc(projectName(tab))}</span>${remote}${tab.view.bypass ? `<span class="warn" title="Permissions skipped">⚠</span>` : ""}
      ${slot ? `<span class="status ${slot.tone}">${icon(slot.icon)}${slot.text}</span>` : `<span class="when">${esc(ago(tab.view.updated))}</span>`}</span>
    <span class="l2">${esc(threadTitle(tab))}</span>
    <span class="l3">${where}${pr ? `<span class="pr" data-pr="${esc(pr.url)}" title="Open pull request #${pr.number}">${icon("git-pull-request")}#${pr.number}</span>` : ""}<span class="provider">${icon("claude")}</span></span>`;
  row.onclick = (e) => {
    activate(tab);
    const pr = e.target.closest("[data-pr]");
    if (pr) openPr(tab, pr.dataset.pr);
    if (narrow(768)) toggleSidebar(false);
  };
  return row;
}

document.getElementById("thread-search").oninput = (e) => {
  ui.search = e.target.value;
  renderSidebar();
};
document.getElementById("thread-search").onkeydown = (e) => {
  if (e.key !== "Escape") return;
  e.target.value = ui.search = "";
  renderSidebar();
};
document.getElementById("project-filter").onchange = (e) => {
  ui.project = e.target.value;
  save("gui-project", ui.project);
  renderSidebar();
};
document.getElementById("new-thread").onclick = () => {
  if (narrow(768)) toggleSidebar(false);
  toggleProjects();
};

function toggleSidebar(open = !ui.sidebarOpen) {
  ui.sidebarOpen = open;
  if (!narrow(768)) save("gui-sidebar", String(open));
  document.body.dataset.sidebar = open ? "open" : "closed";
}
for (const id of ["sidebar-toggle", "chat-sidebar-toggle"]) document.getElementById(id).onclick = () => toggleSidebar();
onGuiKey("b", () => toggleSidebar());
onGuiKey("n", () => toggleProjects());
document.getElementById("workspace").addEventListener("click", (e) => {
  if (e.target.id === "workspace" && narrow(768) && ui.sidebarOpen) toggleSidebar(false);
});
