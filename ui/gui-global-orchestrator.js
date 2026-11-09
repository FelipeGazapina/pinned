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
  return goSettings?.view?.name ?? "Orquestrador";
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

/** The "Orquestrador global" section of the General page: one setting for the whole account, saved
 * to every machine at once. Machines that cannot take it yet say so under the fields. */
async function globalOrchestratorSettingsHtml() {
  goSettings = await readGlobalOrchestratorSettings();
  const { view, machines } = goSettings;
  goHost = view.host;
  const presets = GO_NAME_PRESETS.map((n) => `<button type="button" class="chip go-preset${n === view.name ? " active" : ""}" data-name="${esc(n)}">${esc(n)}</button>`).join("");
  const hosts = machines
    .map((m) => `<button type="button" class="chip go-host${m.id === goHost ? " active" : ""}" data-machine="${esc(m.id)}"${m.online && !m.update ? "" : " disabled"} title="${m.update ? "precisa atualizar o pinned" : m.online ? "" : "offline"}">${esc(m.name)}</button>`)
    .join("");
  const busy = GO_BUSY_CHOICES.map(([v, label]) => `<option value="${v}"${v === view.busy ? " selected" : ""}>${label}</option>`).join("");
  const reports = GO_REPORT_CHOICES.map(([v, label]) => `<option value="${v}"${v === view.reports ? " selected" : ""}>${label}</option>`).join("");
  const notes = globalSettingsNotes(machines);
  const toggle = (id, on, title, about) => `<div class="side-setting" data-setting="${id}"><div><label for="go-${id}">${title}</label><p class="meta">${about}</p></div>
    <label class="switch"><input type="checkbox" id="go-${id}"${on ? " checked" : ""}><span></span></label></div>`;
  return `<section id="global-orchestrator-settings"><h2 class="looks-title">Orquestrador global</h2>
    <div class="side-setting" data-setting="name"><div><label for="go-name">Nome</label><p class="meta">Como ele assina os pedidos aos Capitães e os relatórios para você.</p></div>
      <div class="go-name"><div class="chips">${presets}</div><input id="go-name" type="text" maxlength="40" value="${esc(view.name)}" aria-label="Nome"></div></div>
    <div class="side-setting" data-setting="host"><div><label>Onde roda</label><p class="meta">Uma máquina hospeda o orquestrador da conta; as outras continuam com seus Capitães.</p></div>
      <div class="chips" id="go-host">${hosts}</div></div>
    <div class="side-setting" data-setting="busy"><div><label for="go-busy">Capitão ocupado</label><p class="meta">O que um Capitão no meio de um turno faz com um pedido novo.</p></div>
      <select id="go-busy" aria-label="Capitão ocupado">${busy}</select></div>
    <div class="side-setting" data-setting="reports"><div><label for="go-reports">Relatórios</label><p class="meta">Quando os Capitães contam ao orquestrador o que fizeram.</p></div>
      <select id="go-reports" aria-label="Relatórios">${reports}</select></div>
    ${toggle("group-reports", view.group_reports, "Agrupar relatórios seguidos", "Relatórios um atrás do outro viram uma linha só no chat.")}
    ${toggle("read-aloud", view.read_aloud, "Ler resumos em voz alta no celular", "O telefone lê o resumo de cada relatório que chega.")}
    <div class="side-setting" data-setting="memory"><div><label>Memória global</label><p class="meta">O que o ${esc(view.name)} lembra da conta, guardado na máquina que o hospeda.</p></div>
      <button type="button" class="secondary" id="go-memory">Memória do ${esc(view.name)}</button></div>
    <div class="actions"><span class="meta" id="go-status" role="status"></span><button type="button" id="go-save">Salvar</button></div>
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
  document.getElementById("go-save").onclick = () => saveGlobalOrchestratorSettings();
  document.getElementById("go-memory").onclick = () => openGlobalMemory();
}

/** Saves the account-wide setting: the shared fields to every online machine, `host` true only on
 * the chosen one. A machine that cannot take it says so below the fields; the save itself stands
 * when the chosen host took it. Then the page tells the rest of the UI through the
 * "global-orchestrator-settings" event, whose name is a contract. */
async function saveGlobalOrchestratorSettings() {
  const error = document.getElementById("go-error");
  const save = document.getElementById("go-save");
  const view = {
    name: document.getElementById("go-name").value,
    host: goHost,
    busy: document.getElementById("go-busy").value,
    reports: document.getElementById("go-reports").value,
    group_reports: document.getElementById("go-group-reports").checked,
    read_aloud: document.getElementById("go-read-aloud").checked,
  };
  // Machines can drop or answer between the read and the save, so the live map decides.
  const live = (goSettings?.machines ?? []).map((m) => ({ ...m, online: machines.get(m.id)?.view.online === true }));
  const host = live.find((m) => m.id === view.host);
  if (!host || !host.online || host.update) {
    error.textContent = "Escolha em qual máquina online o orquestrador roda.";
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
    error.textContent = `Não foi possível salvar em ${host.name}: ${failed?.error ?? "sem resposta"}`;
    return;
  }
  document.dispatchEvent(new CustomEvent("global-orchestrator-settings", { detail: saved.settings }));
  const section = document.getElementById("global-orchestrator-settings");
  if (section) {
    section.outerHTML = await globalOrchestratorSettingsHtml();
    bindGlobalOrchestratorSettings();
    const status = document.getElementById("go-status");
    if (status) status.textContent = "Salvo";
  }
}

/** The account's projects as the projects page sees them: the tag catalog with each host's running
 * worker count, named by the orchestrator Felipe chose. */
function globalProjectsCatalog() {
  const directory = machineDirectory();
  directory.localName = machines.get(LOCAL)?.hubView?.name ?? directory.localName;
  return tagCatalog({
    groups: projectGroups(true),
    directory,
    phase: phaseOf,
    repoAt: (machine, path) => projectRepos.get(`${machine}:${path}`) ?? null,
    name: globalOrchestratorName(),
    workers: (tab) => runningAgents(tab).length,
  });
}

/** Opens the account's projects page. The name is a contract: the chat links to it. */
function openGlobalProjects() {
  openPage("global-projects");
}

SHELL_PAGES["global-projects"] = { show() { renderGlobalProjectsPage(); } };

function renderGlobalProjectsPage() {
  const catalog = globalProjectsCatalog();
  const rows = catalog.projects.map((p) => {
    const state = captainState(p.machines.map((m) => m.captain));
    const machinesHtml = p.machines
      .map((m) => {
        const captain = m.captain === "nenhum" ? "" : `<span class="gp-captain ${m.captain}">${m.captain}</span>`;
        const title = m.captain === "nenhum" ? ' title="Sem Capitão nesta máquina"' : "";
        return `<button type="button" class="gp-machine${m.online ? "" : " off"}" data-project="${esc(p.slug)}" data-machine="${esc(m.id)}"${m.captain === "nenhum" || !m.online ? " disabled" : ""}${title}>${esc(m.name)}${captain}</button>`;
      })
      .join("");
    const running = p.machines.reduce((n, m) => n + (m.workers || 0), 0);
    return `<div class="gp-row">
      <div class="gp-main"><span class="gp-name">${esc(p.name)}</span><span class="gp-state ${state}">${state}</span><span class="gp-repo">${esc(p.repo || "sem remote")}</span></div>
      <div class="gp-side">${machinesHtml}<span class="gp-workers">${running ? `${running} worker${running > 1 ? "s" : ""}` : "—"}</span></div>
    </div>`;
  });
  setPage(
    `${headHtml("Projetos", `<span class="range">${catalog.projects.length} projeto${catalog.projects.length === 1 ? "" : "s"} · ${catalog.machines.length} máquina${catalog.machines.length === 1 ? "" : "s"}</span>`)}
    <div class="page-body"><div class="page-wrap gp-list">${rows.join("") || '<p class="gp-empty">Nenhum projeto nas máquinas da conta ainda.</p>'}</div></div>`,
  );
}

/** A project's Capitão tab on one machine, when one is open; the chips that have one open it. */
function captainTabFor(projectSlug, machineId) {
  const group = projectGroups(true).find((g) => tagSlug(g.name) === projectSlug);
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
setInterval(() => !document.hidden && shellPage() === "global-projects" && renderGlobalProjectsPage(), 30000);

/** Opens the global memory: the host machine's orchestrator folder, as the memory panel of its
 * global tab. The panel reads and writes through the tab, so it lands there first. */
async function openGlobalMemory() {
  const name = globalOrchestratorName();
  const host = goSettings?.machines.find((m) => m.settings?.host === true && m.online && !m.update);
  if (!host) return tell(`Escolha primeiro em qual máquina o ${name} roda, nas configurações.`);
  let tab;
  try {
    tab = await revealTab(host.id, await callOn(host.id, { cmd: "open_global_orchestrator" }));
  } catch (e) {
    return tell(`Não deu para abrir a memória do ${name}: ${esc(String(e?.message ?? e))}`, 6000);
  }
  addSurface(tab, { id: "memory", kind: "memory" });
}
