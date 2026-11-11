// The phone's bottom sheet (Phone-Workers, Phone-Approval, Phone-Actions): a scrim and a panel
// pinned to the bottom with a grabber on top. One sheet shows at a time.
//
// openSheet({ label, content, tone, className, onClose }) -> { el, close }
//   `label` names the dialog for screen readers. `content` is an HTML string or a Node put under
//   the grabber. `tone: "approval"` draws the amber top border. `className` is added to the panel,
//   whose padding and gap a caller sets through --sheet-pad-x and --sheet-gap. `el` is the panel.
//   A tap on the scrim, Escape, a swipe down on the panel and close() all close it; `onClose` runs
//   once, after it left the page. Focus moves into the panel and goes back where it was on close.
//   Opening a sheet closes the one before.

let openedSheet = null;
const SHEET_FOCUSABLE = "button:not([disabled]), [href], input:not([disabled]), select, textarea, [tabindex]:not([tabindex='-1'])";
/** How far down a drag must carry the panel to close it, in pixels, or how fast in px/ms once it
 * moved `flick` pixels, so a tap's jitter never counts. */
const SHEET_SWIPE = { distance: 80, flick: 24, speed: 0.5 };

function openSheet({ label, content, tone = null, className = "", onClose = null }) {
  openedSheet?.close();
  const back = document.activeElement;
  const root = document.createElement("div");
  root.className = "sheet-root";
  root.innerHTML = `<div class="sheet-scrim"></div><section class="sheet-panel" role="dialog" aria-modal="true" tabindex="-1"><span class="sheet-grabber" aria-hidden="true"></span></section>`;
  const panel = root.querySelector(".sheet-panel");
  panel.setAttribute("aria-label", label);
  if (tone) panel.dataset.tone = tone;
  if (className) panel.classList.add(...className.split(/\s+/).filter(Boolean));
  if (typeof content === "string") panel.insertAdjacentHTML("beforeend", content);
  else if (content) panel.append(content);

  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    document.removeEventListener("keydown", onKey, true);
    root.remove();
    if (openedSheet?.el === panel) openedSheet = null;
    if (back?.isConnected) back.focus({ preventScroll: true });
    onClose?.();
  };
  const onKey = (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      return close();
    }
    if (e.key !== "Tab") return;
    const stops = [...panel.querySelectorAll(SHEET_FOCUSABLE)].filter((el) => el.offsetParent !== null);
    if (!stops.length) return e.preventDefault();
    const [first, last] = [stops[0], stops.at(-1)];
    if (e.shiftKey && (document.activeElement === first || !panel.contains(document.activeElement))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };
  document.addEventListener("keydown", onKey, true);
  root.querySelector(".sheet-scrim").addEventListener("click", close);
  swipeToClose(panel, close);

  document.body.append(root);
  (panel.querySelector(SHEET_FOCUSABLE) ?? panel).focus({ preventScroll: true });
  openedSheet = { el: panel, close };
  return openedSheet;
}

/** A drag that starts while the panel is scrolled to its top follows the finger down, and closes
 * the sheet past SHEET_SWIPE; a shorter one springs back. */
function swipeToClose(panel, close) {
  let start = null;
  panel.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || panel.scrollTop > 0 || e.target.closest("input, textarea, select")) return;
    start = { y: e.clientY, at: performance.now(), id: e.pointerId, moved: 0 };
  });
  panel.addEventListener("pointermove", (e) => {
    if (!start || e.pointerId !== start.id) return;
    start.moved = Math.max(0, e.clientY - start.y);
    if (start.moved < 6) return;
    if (!panel.hasPointerCapture(e.pointerId)) panel.setPointerCapture(e.pointerId);
    panel.style.transition = "none";
    panel.style.transform = `translateY(${start.moved}px)`;
  });
  const end = (e) => {
    if (!start || e.pointerId !== start.id) return;
    const { moved, at } = start;
    start = null;
    panel.style.transition = "";
    panel.style.transform = "";
    if (moved >= SHEET_SWIPE.distance || (moved >= SHEET_SWIPE.flick && moved / Math.max(1, performance.now() - at) >= SHEET_SWIPE.speed)) close();
  };
  panel.addEventListener("pointerup", end);
  panel.addEventListener("pointercancel", end);
}
