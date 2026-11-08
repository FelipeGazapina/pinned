/** The Capitão and workers section of the General page: the global cost style the Capitão
 * routes by, and how many workers may run at once on this machine. */
const ORCHESTRATION_STYLES = [
  ["economical", "Economical", "The cheapest model that can do the job, one worker."],
  ["balanced", "Balanced", "Mid models first, a second opinion only when it matters."],
  ["quality", "Max quality", "The strongest models in parallel, exploring every angle."],
];
async function orchestrationSettingsHtml() {
  if (!TAURI) return "";
  let current = { style: "balanced", max_workers: 100 };
  try {
    current = await invoke("get_orchestration");
  } catch (e) {
    console.warn("orchestration", e);
  }
  const styles = ORCHESTRATION_STYLES.map(([v, label]) => `<option value="${v}"${v === current.style ? " selected" : ""}>${label}</option>`).join("");
  return `<section id="orchestration-settings"><h2 class="looks-title">Capitão and workers</h2>
    <div class="side-setting" data-setting="style"><div><label for="orchestration-style">Cost style</label><p class="meta" id="orchestration-style-about">${esc((ORCHESTRATION_STYLES.find(([v]) => v === current.style) ?? [])[2] ?? "")}</p></div>
      <select id="orchestration-style" aria-label="Cost style">${styles}</select></div>
    <div class="side-setting" data-setting="maxWorkers"><div><label for="orchestration-max">Max workers</label><p class="meta">How many workers run at once on this ${COMPUTER}. Per-project habits stay in that project's memory.</p></div>
      <input type="number" id="orchestration-max" min="1" max="10000" value="${Number(current.max_workers) || 100}" aria-label="Max workers"></div>
    <p class="error" id="orchestration-error"></p></section>`;
}
function bindOrchestrationSettings() {
  const style = document.getElementById("orchestration-style");
  const max = document.getElementById("orchestration-max");
  if (!style || !max) return;
  const about = document.getElementById("orchestration-style-about");
  style.onchange = () => {
    if (about) about.textContent = (ORCHESTRATION_STYLES.find(([v]) => v === style.value) ?? [])[2] ?? "";
    saveOrchestration();
  };
  max.onchange = () => saveOrchestration();
  async function saveOrchestration() {
    const error = document.getElementById("orchestration-error");
    const max_workers = Math.max(1, Math.min(10000, Math.round(Number(max.value) || 100)));
    max.value = max_workers;
    try {
      await invoke("set_orchestration", { style: style.value, max_workers });
      error.textContent = "";
    } catch (err) {
      error.textContent = String(err?.message ?? err);
    }
  }
}
