/** T3 Code's tooltip (components/ui/tooltip.tsx): in the app view a control's title shows in a styled
 * popup above it, or below when there is no room, once the pointer rests 600 ms, and at once when
 * another closed less than 400 ms ago. The title moves to data-tip while it shows, so the browser's
 * own tooltip stays away, and comes back when it hides. */
const tipEl = Object.assign(document.createElement("div"), { id: "g-tooltip", role: "tooltip", hidden: true });
document.body.append(tipEl);
const tip = { target: null, timer: 0, closedAt: 0 };

function showTip() {
  const el = tip.target;
  if (!el?.isConnected) return hideTip();
  tipEl.textContent = el.dataset.tip;
  tipEl.dataset.instant = performance.now() - tip.closedAt < 400;
  tipEl.hidden = false;
  const r = el.getBoundingClientRect();
  const w = tipEl.offsetWidth;
  const h = tipEl.offsetHeight;
  const below = el.dataset.tipSide === "bottom" ? r.bottom + 4 + h <= innerHeight - 5 : r.top - 4 - h < 5;
  tipEl.dataset.side = below ? "bottom" : "top";
  tipEl.style.left = `${Math.max(5, Math.min(r.left + r.width / 2 - w / 2, innerWidth - w - 5))}px`;
  tipEl.style.top = `${below ? r.bottom + 4 : r.top - 4 - h}px`;
}
function hideTip() {
  clearTimeout(tip.timer);
  const el = tip.target;
  if (el) {
    if (!el.hasAttribute("title")) el.title = el.dataset.tip;
    delete el.dataset.tip;
  }
  if (!tipEl.hidden) tip.closedAt = performance.now();
  tipEl.hidden = true;
  tip.target = null;
}

document.addEventListener("pointerover", (e) => {
  if (e.pointerType !== "mouse") return;
  const el = guiOn() ? e.target.closest?.("[title], [data-tip]") : null;
  if (el && el === tip.target) return;
  hideTip();
  if (!el?.title.trim()) return;
  tip.target = el;
  el.dataset.tip = el.title;
  el.removeAttribute("title");
  tip.timer = setTimeout(showTip, performance.now() - tip.closedAt < 400 ? 0 : 600);
});
document.addEventListener("pointerout", (e) => e.relatedTarget || hideTip());
for (const type of ["pointerdown", "keydown", "wheel", "scroll"]) addEventListener(type, () => tip.target && hideTip(), true);
