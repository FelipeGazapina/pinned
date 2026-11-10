/** The Captain and workers section of the General page: the way into the Way of working page,
 * where the delegation limits live with the rest of the account's tastes. */
async function orchestrationSettingsHtml() {
  return `<section id="orchestration-settings"><h2 class="looks-title">Captain and workers</h2>
    <div class="side-setting" data-setting="wayOfWorking"><div><label for="orchestration-taste">Way of working</label><p class="meta">Profiles, models, pace and delegation limits for the orchestrator and the Captains.</p></div>
      <button type="button" class="secondary" id="orchestration-taste">Open</button></div></section>`;
}
function bindOrchestrationSettings() {
  const open = document.getElementById("orchestration-taste");
  if (open) open.onclick = () => openPage("taste");
}
