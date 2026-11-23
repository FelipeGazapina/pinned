/** The global orchestrator in the app's chrome: the General page section that reads and writes
 * every machine's settings as one account-wide setting, the account's projects page behind
 * `openGlobalProjects()`, and the global memory panel. Pure logic lives in global-orchestrator.js. */

/** The last read: `{ machines, view }` as `mergeGlobalSettings` shapes them. The name it holds
 * labels the memory panel and the projects page until the next read refreshes it. */
let goSettings = null;
/** The host chosen in the section, held between renders because chips are buttons, not inputs. */
let goHost = null;
/** Whether the background name read has run, so it runs once and never overwrites a real read. */
let goReadOnce = false;

/** The orchestrator's name. Read before any settings read, it asks once in the background so the
 * memory panel and the projects page stop showing the default on the next render. */
const globalOrchestratorName = () => {
  if (!goSettings && !goReadOnce && machines.size) {
    goReadOnce = true;
    readGlobalOrchestratorSettings()
      .then((read) => (goSettings = read))
      .catch(() => {});
  }
  return goSettings?.view?.name ?? "Orchestrator";
};

/** Reads the account's setting: `global_orchestrator_settings` from every online machine, one
 * entry per machine with what that machine answered or why it could not. */
async function readGlobalOrchestratorSettings() {
  const list = await Promise.all(
    [...machines.entries()]
      .filter(([, m]) => !m.view.upstash_box)
      .map(async ([id, m]) => {
        const machine = { id, name: machineName(id), online: m.view.online === true, settings: null, update: false, error: null };
        if (!machine.online) return machine;
        try {
          machine.settings = await callOn(id, { cmd: "global_orchestrator_settings" });
        } catch (e) {
          if (isOlderPinned(e)) machine.update = true;
          else machine.error = String(e?.message ?? e);
        }
        return machine;
      }),
  );
  return { machines: list, view: mergeGlobalSettings(list) };
}

/** The profile's length against the cap, so a paste the textarea cuts short does not pass unnoticed. */
const approvalCount = (text) => `${text.length} / ${GO_APPROVAL_PROFILE_MAX} characters`;

/** The "Global orchestrator" section of the General page: one setting for the whole account, saved
 * to every machine at once. Machines that cannot take it yet say so under the fields, and a machine
 * whose own settings differ from the host's is named at the top, with a way to apply the host's. */
async function globalOrchestratorSettingsHtml() {
  goSettings = await readGlobalOrchestratorSettings();
  const { view, machines } = goSettings;
  goHost = view.host;
  const presets = GO_NAME_PRESETS.map((n) => `<button type="button" class="chip go-preset${n === view.name ? " active" : ""}" data-name="${esc(n)}">${esc(n)}</button>`).join("");
  const hosts = machines
    .map((m) => `<button type="button" class="chip go-host${m.id === goHost ? " active" : ""}" data-machine="${esc(m.id)}"${m.online && !m.update ? "" : " disabled"} title="${m.update ? "needs to update pinned" : m.online ? "" : "offline"}">${esc(m.name)}</button>`)
    .join("");
  const busy = GO_BUSY_CHOICES.map(([v, label]) => `<option value="${v}"${v === view.busy ? " selected" : ""}>${label}</option>`).join("");
  const reports = GO_REPORT_CHOICES.map(([v, label]) => `<option value="${v}"${v === view.reports ? " selected" : ""}>${label}</option>`).join("");
  const notes = globalSettingsNotes(machines);
  const profile = view.approval_profile || GO_DEFAULT_APPROVAL_PROFILE;
  const drift = globalSettingsDrift(machines);
  const many = drift.length > 1;
  const driftHtml = drift.length
    ? `<div class="go-drift" id="go-drift" role="alert"><p>${drift.map((d) => esc(`${d.name} does not match ${d.base}: it has ${d.values.join(", ")}.`)).join("<br>")}</p>
      <p class="meta">An agent that starts on ${many ? "those machines" : "that machine"} follows ${many ? "their" : "its"} own settings.</p>
      <button type="button" class="secondary" id="go-reapply">Apply ${esc(drift[0].base)}'s settings to every machine</button></div>`
    : "";
  const toggle = (id, on, title, about) => `<div class="side-setting" data-setting="${id}"><div><label for="go-${id}">${title}</label><p class="meta">${about}</p></div>
    <label class="switch"><input type="checkbox" id="go-${id}"${on ? " checked" : ""}><span></span></label></div>`;
  return `<section id="global-orchestrator-settings"><h2 class="looks-title">Global orchestrator</h2>${driftHtml}
    <div class="side-setting" data-setting="name"><div><label for="go-name">Name</label><p class="meta">How it signs requests to the Captains and reports to you.</p></div>
      <div class="go-name"><div class="chips">${presets}</div><input id="go-name" type="text" maxlength="40" value="${esc(view.name)}" aria-label="Name"></div></div>
    <div class="side-setting" data-setting="host"><div><label>Where it runs</label><p class="meta">One machine hosts the account's orchestrator; the others keep their own Captains.</p></div>
      <div class="chips" id="go-host">${hosts}</div></div>
    <div class="side-setting" data-setting="busy"><div><label for="go-busy">When a Captain is busy</label><p class="meta">What a Captain in the middle of a turn does with a new request.</p></div>
      <select id="go-busy" aria-label="When a Captain is busy">${busy}</select></div>
    <div class="side-setting" data-setting="reports"><div><label for="go-reports">Reports</label><p class="meta">When the Captains tell the orchestrator what they did.</p></div>
      <select id="go-reports" aria-label="Reports">${reports}</select></div>
    ${toggle("group-reports", view.group_reports, "Group consecutive reports", "Reports that arrive one after another become a single row in the chat.")}
    ${toggle("read-aloud", view.read_aloud, "Read summaries aloud on the phone", "The phone reads out the summary of each report that arrives.")}
    ${toggle("auto-merge", view.auto_merge, "Auto-merge", "When on, the Captains and the orchestrator decide by the approval profile whether a finished pull request merges or waits for your review. A change reaches running Captains and the orchestrator at their next turn; a turn already in progress can still merge.")}
    <div class="side-setting" data-setting="approval-profile"><div><label for="go-approval-profile">Approval profile</label><p class="meta">What decides whether a pull request merges by itself. Used when Auto-merge is on. With Auto-merge on, an edit reaches running Captains and the orchestrator at their next turn; a turn already in progress can still merge.</p></div>
      <details class="go-approval"><summary>Show the profile</summary><div class="go-approval-body">
        <textarea id="go-approval-profile" maxlength="${GO_APPROVAL_PROFILE_MAX}" aria-label="Approval profile" aria-describedby="go-approval-count">${esc(profile)}</textarea>
        <div class="go-approval-foot"><span class="meta" id="go-approval-count">${approvalCount(profile)}</span><button type="button" class="secondary" id="go-approval-reset">Reset to default</button></div></div></details></div>
    <div class="side-setting" data-setting="memory"><div><label>Global memory</label><p class="meta">What ${esc(view.name)} remembers about the account, kept on the machine that hosts it.</p></div>
      <button type="button" class="secondary" id="go-memory">${esc(view.name)}'s memory</button></div>
    <div class="actions"><span class="meta" id="go-status" role="status"></span><button type="button" id="go-save">Save</button></div>
    <p class="meta" id="go-notes">${notes.map((n) => esc(n)).join("<br>")}</p>
    <p class="error" id="go-error"></p></section>`;
}

function bindGlobalOrchestratorSettings() {
  const section = document.getElementById("global-orchestrator-settings");
  if (!section || !goSettings) return;
  const name = document.getElementById("go-name");
  const paintName = () => {
    for (const b of section.querySelectorAll(".go-preset")) b.classList.toggle("active", b.dataset.name === name.value);
  };
  for (const b of section.querySelectorAll(".go-preset")) b.onclick = () => {
    name.value = b.dataset.name;
    paintName();
  };
  name.oninput = paintName;
  const paintHost = () => {
    for (const b of section.querySelectorAll(".go-host")) b.classList.toggle("active", b.dataset.machine === goHost);
  };
  for (const b of section.querySelectorAll(".go-host")) b.onclick = () => {
    goHost = b.dataset.machine;
    paintHost();
  };
  const profile = document.getElementById("go-approval-profile");
  const paintCount = () => (document.getElementById("go-approval-count").textContent = approvalCount(profile.value));
  profile.oninput = paintCount;
  document.getElementById("go-approval-reset").onclick = () => {
    profile.value = GO_DEFAULT_APPROVAL_PROFILE;
    paintCount();
  };
  document.getElementById("go-save").onclick = () => saveGlobalOrchestratorSettings();
  const reapply = document.getElementById("go-reapply");
  if (reapply) reapply.onclick = () => saveGlobalOrchestratorSettings({ ...goSettings.view });
  document.getElementById("go-memory").onclick = () => openGlobalMemory();
}

/** What the section's fields hold now, as a settings view. */
function globalOrchestratorForm() {
  return {
    name: document.getElementById("go-name").value,
    host: goHost,
    busy: document.getElementById("go-busy").value,
    reports: document.getElementById("go-reports").value,
    group_reports: document.getElementById("go-group-reports").checked,
    read_aloud: document.getElementById("go-read-aloud").checked,
    auto_merge: document.getElementById("go-auto-merge").checked,
    approval_profile: document.getElementById("go-approval-profile").value,
  };
}

/** Saves the account-wide setting: the shared fields to every online machine, `host` true only on
 * the chosen one. A machine that cannot take it says so below the fields; the save itself stands
 * when the chosen host took it, and any other machine whose write failed is named, as it keeps its
 * old settings. Then the page tells the rest of the UI through the "global-orchestrator-settings"
 * event, whose name is a contract. `view` is the form's by default; the drift note passes the
 * host's saved one. */
async function saveGlobalOrchestratorSettings(view = globalOrchestratorForm()) {
  const error = document.getElementById("go-error");
  const save = document.getElementById("go-save");
  // Machines can drop or answer between the read and the save, so the live map decides.
  const live = (goSettings?.machines ?? []).map((m) => ({ ...m, online: machines.get(m.id)?.view.online === true }));
  const host = live.find((m) => m.id === view.host);
  if (!host || !host.online || host.update) {
    error.textContent = "Choose which online machine the orchestrator runs on.";
    return;
  }
  error.textContent = "";
  save.disabled = true;
  const outcomes = await Promise.all(
    globalSettingsWrites(view, live).map(async (w) => {
      try {
        return { machine: w.machine, settings: await callOn(w.machine, { cmd: "set_global_orchestrator_settings", settings: w.settings }) };
      } catch (e) {
        return { machine: w.machine, error: String(e?.message ?? e) };
      }
    }),
  );
  save.disabled = false;
  const saved = outcomes.find((o) => o.machine === view.host && !o.error);
  if (!saved) {
    const failed = outcomes.find((o) => o.machine === view.host);
    error.textContent = `Could not save on ${host.name}: ${failed?.error ?? "no answer"}`;
    return;
  }
  document.dispatchEvent(new CustomEvent("global-orchestrator-settings", { detail: saved.settings }));
  // What the section read before the save, which the re-render replaces.
  const told = goSettings?.view && approvalChangeTold(goSettings.view, saved.settings);
  const section = document.getElementById("global-orchestrator-settings");
  if (section) {
    section.outerHTML = await globalOrchestratorSettingsHtml();
    bindGlobalOrchestratorSettings();
    const missed = outcomes.filter((o) => o.error).map((o) => `Not saved on ${live.find((m) => m.id === o.machine)?.name ?? o.machine}: ${o.error.replace(/\.$/, "")}. It keeps its old settings.`);
    const status = document.getElementById("go-status");
    const savedText = missed.length ? `Saved on ${host.name}` : "Saved";
    if (status) status.textContent = told ? `${savedText}. Running Captains and the orchestrator were told.` : savedText;
    const failed = document.getElementById("go-error");
    if (failed) failed.textContent = missed.join(" ");
  }
}

/** The account's projects as the projects page sees them: the tag catalog, named by the
 * orchestrator Felipe chose. */
function globalProjectsCatalog() {
  const directory = machineDirectory();
  directory.localName = machines.get(LOCAL)?.hubView?.name ?? directory.localName;
  return tagCatalog({
    // Threads started without a project share a "No project" group, which is not a project.
    groups: projectGroups(true).filter((g) => !g.tabs.length || g.tabs.some(hasProject)),
    directory,
    phase: phaseOf,
    repoAt: (machine, path) => projectRepos.get(`${machine}:${path}`) ?? null,
    name: globalOrchestratorName(),
  });
}

/** Opens the account's projects page. The name is a contract: the chat links to it. */
function openGlobalProjects() {
  openPage("global-projects");
}

SHELL_PAGES["global-projects"] = { show() { renderGlobalProjectsPage(); } };

/** A project group's name and slug as the tag catalog names it, so counts and Captains find their row. */
const groupName = (group) => projectTagName(group, group.checkouts.map((c) => projectRepos.get(`${c.machine}:${c.path}`)).find(Boolean));
const groupSlug = (group) => tagSlug(groupName(group));

/** Each project's running workers by machine, by project slug and machine id, for the projects page and
 * the Captains rail: worker tabs `atWork`, with a turn running or starting, or one that waits on the user,
 * a finished, idle or sleeping one left out. Core's `projects()` counts any live worker not finished, so a
 * worker resumed at its prompt counts there and not here. */
function runningWorkers() {
  const by = new Map();
  for (const group of projectGroups(true)) {
    const counts = new Map();
    for (const tab of group.tabs) if (tab.view.role === "worker" && atWork(tab)) counts.set(tab.machine, (counts.get(tab.machine) ?? 0) + 1);
    by.set(groupSlug(group), counts);
  }
  return by;
}

/** What each project's threads and workers cost, as `$1.26 · 2 workers`, by project slug. */
function projectCosts() {
  const by = new Map();
  for (const group of projectGroups(true)) by.set(groupSlug(group), projectCostLabel(spawnTree(group.tabs).roots));
  return by;
}

function renderGlobalProjectsPage() {
  const catalog = globalProjectsCatalog();
  const workers = runningWorkers();
  const costs = projectCosts();
  const rows = catalog.projects.map((p) => {
    const state = captainState(p.machines.map((m) => m.captain));
    const counts = workers.get(p.slug) ?? new Map();
    const machinesHtml = p.machines
      .map((m) => {
        const captain = m.captain === "none" ? "" : `<span class="gp-captain ${m.captain}">${CAPTAIN_STATE_LABELS[m.captain]}</span>`;
        const running = counts.get(m.id) ? `<span class="gp-count">${workersLabel(counts.get(m.id))}</span>` : "";
        const title = m.captain === "none" ? ' title="No Captain on this machine"' : "";
        return `<button type="button" class="gp-machine${m.online ? "" : " off"}" data-project="${esc(p.slug)}" data-machine="${esc(m.id)}"${m.captain === "none" || !m.online ? " disabled" : ""}${title}>${esc(m.name)}${captain}${running}</button>`;
      })
      .join("");
    const running = [...counts.values()].reduce((n, c) => n + c, 0);
    return `<div class="gp-row">
      <div class="gp-main"><span class="gp-name">${esc(p.name)}</span><span class="gp-state ${state}">${CAPTAIN_STATE_LABELS[state]}</span><span class="gp-repo">${esc(p.repo || "no remote")}</span></div>
      <div class="gp-side">${machinesHtml}${costs.get(p.slug) ? `<span class="gp-cost" title="What this project's threads and workers cost">${esc(costs.get(p.slug))}</span>` : ""}<span class="gp-workers">${running ? workersLabel(running) : "—"}</span></div>
    </div>`;
  });
  setPage(
    `${headHtml("Projects", `<span class="range">${catalog.projects.length} project${catalog.projects.length === 1 ? "" : "s"} · ${catalog.machines.length} machine${catalog.machines.length === 1 ? "" : "s"}</span>`)}
    <div class="page-body"><div class="page-wrap gp-list">${rows.join("") || '<p class="gp-empty">No projects on the account\'s machines yet.</p>'}</div></div>`,
  );
}

/** A project's Captain tab on one machine, when one is open; the chips that have one open it. */
function captainTabFor(projectSlug, machineId) {
  const group = projectGroups(true).find((g) => groupSlug(g) === projectSlug);
  return group?.tabs.find((t) => t.machine === machineId && t.view.role === "orchestrator") ?? null;
}

pageView.addEventListener("click", (e) => {
  if (shellPage() !== "global-projects") return;
  const chip = e.target.closest(".gp-machine");
  if (!chip || chip.disabled) return;
  const captain = captainTabFor(chip.dataset.project, chip.dataset.machine);
  if (captain) activate(captain);
});

// The page reads tabs and machines, which change without a page event of their own: machine views
// notify through viewListeners, and a captain's turn moves on a timer while the page is shown.
viewListeners.push(() => shellPage() === "global-projects" && renderGlobalProjectsPage());
visibleInterval(() => shellPage() === "global-projects" && renderGlobalProjectsPage(), 30000);

/** Opens the global memory: the host machine's orchestrator folder, as the memory panel of its
 * global tab. The panel reads and writes through the tab, so it lands there first. */
async function openGlobalMemory() {
  const name = globalOrchestratorName();
  const host = goSettings?.machines.find((m) => m.settings?.host === true && m.online && !m.update);
  if (!host) return tell(`Choose which machine ${name} runs on in the settings first.`);
  let tab;
  try {
    tab = await revealTab(host.id, await callOn(host.id, { cmd: "open_global_orchestrator" }));
  } catch (e) {
    return tell(`Couldn't open ${name}'s memory: ${esc(String(e?.message ?? e))}`, 6000);
  }
  addSurface(tab, { id: "memory", kind: "memory" });
}
