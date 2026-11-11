/** The chat header's own parts: the Captain or Worker chip, the time and cost (without the worker
 * count, which the thread's Workers block carries), a worker's Open diff, the layout group's pressed
 * states, and how the header folds as it narrows. */

const roleChipHtml = (tab) =>
  isOrchestrator(tab) && !isGlobalOrchestratorTab(tab) ? `<span class="role-chip">Captain</span>` : tab.view.role === "worker" ? `<span class="role-chip worker">Worker</span>` : "";

/** `statsLabel` without its trailing "N workers", which it adds last when the thread delegated. */
function headStatsLabel(tab) {
  const parts = statsLabel(tab).split(" · ").filter(Boolean);
  return (tab.view.work ? parts.slice(0, -1) : parts).join(" · ");
}
/** The header's time and cost. A phone shows the cost only, so the cost comes twice and the style shows one. */
function headStatsHtml(tab) {
  const label = headStatsLabel(tab);
  const work = tab.view.work;
  const title = work ? `Cost of this thread and the ${nWorkers(work.workers)} it started${work.unknown ? `. ${unknownTitle(work.unknown)}` : ""}` : "";
  return label ? `<span class="stats" data-head-stats="${esc(tab.key)}"${title ? ` title="${esc(title)}"` : ""}><span class="long">${esc(label)}</span><span class="usd">${esc(statsLabel(tab, true))}</span></span>` : "";
}
visibleInterval(() => {
  if (!guiOn()) return;
  for (const el of crumbEl.querySelectorAll("[data-head-stats]")) {
    const tab = tabByKey(el.dataset.headStats);
    if (!tab) continue;
    for (const [part, text] of [[".long", headStatsLabel(tab)], [".usd", statsLabel(tab, true)]]) {
      const at = el.querySelector(part);
      if (at && at.textContent !== text) at.textContent = text;
    }
  }
}, 1000);

crumbEl.addEventListener("click", (e) => {
  const parent = tabByKey(e.target.closest("[data-parent-thread]")?.dataset.parentThread);
  if (parent) activate(parent);
});

/** A worker's header opens its latest turn's diff, with that turn's line counts once its history is read. */
const openDiffButton = document.getElementById("open-diff");
function renderOpenDiff(tab) {
  const t = tab && threadOf(tab);
  openDiffButton.hidden = !t?.workspace || tab.view.role !== "worker";
  if (openDiffButton.hidden) return;
  const files = t.history?.at(-1)?.files;
  setHtml(openDiffButton, `<span class="label">Open diff</span>${files?.length ? `<span class="add">+${sum(files, "added")}</span><span class="del">-${sum(files, "removed")}</span>` : ""}`);
}
openDiffButton.onclick = () => {
  const tab = active;
  if (!tab) return;
  const turn = threadOf(tab).history?.at(-1);
  if (turn) setDiffState(tab, { scope: "turn", turn: turn.ended });
  openSurface("diff");
};
// The crumb is redrawn when the thread's history arrives, which brings the counts.
new MutationObserver(() => renderOpenDiff(active)).observe(crumbEl, { childList: true });

/** A layout button says whether its pane shows, in `aria-pressed` and its name. */
function setPressed(button, on) {
  const name = button.getAttribute("aria-label")?.replace(/, shown$/, "") ?? "";
  button.setAttribute("aria-pressed", String(on));
  button.setAttribute("aria-label", on ? `${name}, shown` : name);
}

/** Above phone width the header folds in steps while the crumb would lose its title: Open in drops
 * its label, then goes (Thread details and ⌘O keep it), with the scripts menu's chevron; then the
 * other labels drop; then the actions fold into the ⋯ menu; last, the crumb drops its chip and stats. */
const HEAD_FOLDS = 5;
const CRUMB_ROOM = 200;
const chatHeadEl = document.getElementById("chat-head");
function crumbNeed() {
  const items = [...crumbEl.children].filter((el) => !el.classList.contains("parent"));
  const gap = parseFloat(getComputedStyle(crumbEl).columnGap) || 0;
  return items.reduce((n, el) => n + (el.classList.contains("title") ? el.querySelector(".text").scrollWidth + 18 : el.getBoundingClientRect().width), gap * (items.length - 1));
}
let fittingHead = false;
function fitHead() {
  if (fittingHead || narrow(768) || !chatHeadEl.offsetWidth) return;
  fittingHead = true;
  const need = Math.min(crumbNeed(), CRUMB_ROOM);
  let fold = 0;
  chatHeadEl.dataset.fold = "0";
  while (fold < HEAD_FOLDS && crumbEl.clientWidth < need) chatHeadEl.dataset.fold = String(++fold);
  fittingHead = false;
}
new ResizeObserver(fitHead).observe(chatHeadEl);
// Where the panel turns into an overlay the layout group moves into its tab row, and on a phone the
// drawer's tab row shows only beside a shell.
matchMedia("(max-width: 980px)").addEventListener("change", () => guiOn() && renderPanel());
matchMedia("(max-width: 768px)").addEventListener("change", () => guiOn() && renderDrawer());
new MutationObserver(fitHead).observe(chatHeadEl, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ["hidden"] });
