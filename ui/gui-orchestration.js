/** The Captain and workers section of the General page: the way into the Way of working page, and
 * in the app, how many workers may run at once on this machine. */
async function orchestrationSettingsHtml() {
  let maxWorkers = "";
  if (TAURI) {
    let current = { max_workers: 100 };
    try {
      current = await invoke("get_orchestration");
    } catch (e) {
      console.warn("orchestration", e);
    }
    maxWorkers = `<div class="side-setting" data-setting="maxWorkers"><div><label for="orchestration-max">Max workers</label><p class="meta">How many workers run at once on this ${COMPUTER}. Each project's pace is set in Way of working.</p></div>
      <input type="number" id="orchestration-max" min="1" max="10000" value="${Number(current.max_workers) || 100}" aria-label="Max workers"></div>`;
  }
  return `<section id="orchestration-settings"><h2 class="looks-title">Captain and workers</h2>
    <div class="side-setting" data-setting="wayOfWorking"><div><label for="orchestration-taste">Way of working</label><p class="meta">Profiles, models and pace for the orchestrator and the Captains.</p></div>
      <button type="button" class="secondary" id="orchestration-taste">Open</button></div>
    ${maxWorkers}
    <p class="error" id="orchestration-error"></p></section>`;
}
function bindOrchestrationSettings() {
  const open = document.getElementById("orchestration-taste");
  if (open) open.onclick = () => openPage("taste");
  const max = document.getElementById("orchestration-max");
  if (!max) return;
  max.onchange = async () => {
    const error = document.getElementById("orchestration-error");
    const max_workers = Math.max(1, Math.min(10000, Math.round(Number(max.value) || 100)));
    max.value = max_workers;
    try {
      await invoke("set_orchestration", { maxWorkers: max_workers });
      error.textContent = "";
    } catch (err) {
      error.textContent = String(err?.message ?? err);
    }
  };
}
