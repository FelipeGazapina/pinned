/** The General page: app-wide switches. */
async function showGeneral() {
  const awake = TAURI && (await invoke("get_keep_awake"));
  settingsPage.innerHTML = `<h2>General</h2>
    ${TAURI ? `<p><label class="toggle"><input type="checkbox" id="keep-awake"${awake ? " checked" : ""}> Keep this Mac awake</label></p>
    <p class="meta">While this is on, the Mac and its display don't go to sleep, so sessions keep working when you step away. It turns off when you switch it off or quit pinned.</p>
    <p class="error" id="awake-error"></p>` : ""}${appearanceHtml()}`;
  bindAppearance();
  if (!TAURI) return;
  document.getElementById("keep-awake").onchange = async (e) => {
    try {
      await invoke("set_keep_awake", { on: e.target.checked });
      document.getElementById("awake-error").textContent = "";
    } catch (err) {
      e.target.checked = !e.target.checked;
      document.getElementById("awake-error").textContent = err;
    }
  };
}

let projectsMachine = LOCAL;

async function showProjectsPage() {
  if (!machines.get(projectsMachine)?.view.online) projectsMachine = onlineMachines()[0];
  const projects = projectsMachine ? await callOn(projectsMachine, { cmd: "list_projects" }).catch(() => []) : [];
  const chips = [...machines.entries()].map(([id, m]) =>
    `<button class="chip${id === projectsMachine ? " active" : ""}" data-action="projects-machine" data-machine="${esc(id)}"${m.view.online ? "" : " disabled"}>${esc(machineName(id))}</button>`);
  settingsPage.innerHTML = `<h2>Projects</h2>
    ${machines.size > 1 ? `<div class="chips">${chips.join("")}</div>` : ""}
    <p>Pick a project to see what its sessions changed, turn by turn.</p>
    <div class="projects">${
      projects.map((p) => `<button class="project" data-action="open-project" data-path="${esc(p.path)}">
        <b>${esc(p.name)}</b><span class="path">${esc(tilde(p.path))}</span></button>`).join("") || `<p class="empty">No projects yet.</p>`
    }</div>`;
}

async function showProjectHistory(path) {
  const turns = await callOn(projectsMachine, { cmd: "project_history", path }).catch(() => []);
  const name = path.split("/").filter(Boolean).pop() ?? "/";
  settingsPage.innerHTML = `<button class="secondary" data-action="projects-back">← Projects</button>
    <h2>${esc(name)}</h2><p class="meta">${esc(tilde(path))}</p>
    <div class="history">${historyHtml(turns)}</div>`;
}

actions["projects-machine"] = (button) => {
  projectsMachine = button.dataset.machine;
  showProjectsPage();
};
actions["open-project"] = (button) => showProjectHistory(button.dataset.path);
actions["projects-back"] = showProjectsPage;

const when = (unix) => new Date(unix * 1000).toLocaleString([], { dateStyle: "short", timeStyle: "short" });
const lineClass = (line) =>
  line.startsWith("+++") || line.startsWith("---") || line.startsWith("diff ") || line.startsWith("index ")
    ? "f"
    : line.startsWith("@@") ? "h" : line[0] === "+" ? "a" : line[0] === "-" ? "d" : "";

function historyHtml(turns) {
  if (!turns.length) {
    return `<p class="empty">Nothing recorded yet. Each turn a session finishes in a git project is saved here with the files it changed.</p>`;
  }
  return turns.map((t) => {
    const added = t.files.reduce((n, f) => n + Number(f.added), 0);
    const removed = t.files.reduce((n, f) => n + Number(f.removed), 0);
    const files = t.files.map((f) => `<li><span>${esc(f.path)}</span><span class="add">+${Number(f.added)}</span><span class="del">−${Number(f.removed)}</span></li>`).join("");
    const patch = t.patch.split("\n").map((l) => `<span class="l ${lineClass(l)}">${esc(l)}</span>`).join("");
    return `<details class="turn">
      <summary>
        <span class="what">${esc(t.prompt || t.title || "(no prompt)")}</span>
        <span class="meta">${esc(t.title && t.prompt ? t.title + " · " : "")}${when(t.ended)} · ${t.files.length} file${t.files.length === 1 ? "" : "s"}
          <span class="add">+${added}</span> <span class="del">−${removed}</span></span>
      </summary>
      <ul>${files}</ul>
      <pre>${patch}</pre>${t.truncated ? `<p class="meta">The diff was cut at its size limit.</p>` : ""}
    </details>`;
  }).join("");
}
