/** On a phone a swipe does what a tap in the header does: right opens the sidebar and left closes it, and
 * a pull from the top opens the thread's menu. The UI follows the finger; letting go past the gesture's
 * reach or with a flick runs the same toggle a tap does, and the inline styles go once it settles so the
 * CSS owns the final state. */
const SWIPE = { lock: 10, flick: 0.4, flickWindow: 100, settle: 200, pull: 56 };
/** A touch starting in any of these belongs to it: typing, the terminal, the side panel, a menu. */
const SWIPE_SKIP = "input, textarea, select, [contenteditable], .xterm, #terms, #drawer-resize, #panel, #side-menu, #panel-menu, #panel-submenu, #header-menu, #composer-menu, #composer-dialog, #details-card, #image-zoom, .popup, dialog";
const swipeMs = () => (matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : SWIPE.settle);
/** A rubber band: the first pixels move almost 1:1, then it stiffens toward 24px. */
const rubber = (d) => 24 * (1 - Math.exp(-d / 64));

/** The sidebar at `shown` of its width, the scrim dimmed by the same share. */
function slideSidebar(shown, ms = 0) {
  document.body.classList.add("sidebar-dragging");
  Object.assign(sidebarEl.style, { visibility: "visible", transition: ms ? `transform ${ms}ms ease-out` : "none", transform: `translateX(${(shown - 1) * 100}%)` });
  workspace.style.setProperty("--scrim", shown);
  workspace.style.setProperty("--scrim-ms", `${ms}ms`);
}
function clearSidebarSlide() {
  document.body.classList.remove("sidebar-dragging");
  for (const p of ["visibility", "transition", "transform"]) sidebarEl.style.removeProperty(p);
  for (const p of ["--scrim", "--scrim-ms"]) workspace.style.removeProperty(p);
}
/** Slides to `open` and lands there: the toggle runs when it changes, a spring back when it doesn't. */
function landSidebar(open) {
  const ms = swipeMs();
  if (ms) slideSidebar(open ? 1 : 0, ms);
  if (open !== ui.sidebarOpen) toggleSidebar(open);
  return ms;
}

const titleButton = () => crumbEl.querySelector("button.title");
function pullTitle(d, ms = 0) {
  crumbEl.classList.toggle("pulling", d > 0);
  crumbEl.classList.toggle("armed", d >= SWIPE.pull);
  crumbEl.style.setProperty("--pull", Math.min(1, d / SWIPE.pull));
  Object.assign(crumbEl.style, { transition: ms ? `transform ${ms}ms ease-out` : "none", transform: d ? `translateY(${rubber(d)}px)` : "" });
}
function clearTitlePull() {
  crumbEl.classList.remove("pulling", "armed");
  for (const p of ["--pull", "transition", "transform"]) crumbEl.style.removeProperty(p);
}

/** Every swipe the page answers. `reach` is how far, in pixels along `direction`, a release commits;
 * `commit` and `cancel` return how long they animate before `clear` drops the inline styles. */
const gestures = [
  { name: "open sidebar", direction: "right", accepts: (start) => !ui.sidebarOpen && !!start.target.closest("#chat, #page-view"),
    reach: () => sidebarEl.offsetWidth * 0.4, track: (d) => slideSidebar(Math.min(1, d / sidebarEl.offsetWidth)),
    commit: () => landSidebar(true), cancel: () => landSidebar(false), clear: clearSidebarSlide },
  { name: "close sidebar", direction: "left", accepts: (start) => ui.sidebarOpen && (start.target === workspace || !!start.target.closest("#sidebar")),
    reach: () => sidebarEl.offsetWidth * 0.4, track: (d) => slideSidebar(Math.max(0, 1 - d / sidebarEl.offsetWidth)),
    commit: () => landSidebar(false), cancel: () => landSidebar(true), clear: clearSidebarSlide },
  { name: "title menu", direction: "down", accepts: (start) => !!active && !!titleButton() && !!start.target.closest("#chat-head, #thread"),
    reach: () => SWIPE.pull, track: (d) => pullTitle(d),
    commit: () => (clearTitlePull(), titleButton()?.click(), 0), cancel: () => (pullTitle(0, swipeMs()), swipeMs()), clear: clearTitlePull },
];

/** The nearest box between `target` and the workspace that can still scroll the way the finger moves,
 * such as a wide code block or a thread scrolled down: the touch is its own. */
function swipeScroller(target, direction) {
  const horizontal = direction !== "down";
  for (let el = target; el && el !== workspace; el = el.parentElement) {
    if (!/auto|scroll/.test(getComputedStyle(el)[horizontal ? "overflowX" : "overflowY"])) continue;
    const room = direction === "right" ? el.scrollLeft > 0 : direction === "left" ? el.scrollLeft < el.scrollWidth - el.clientWidth - 1 : el.scrollTop > 0;
    if (room) return el;
  }
  return null;
}

/** One touch at a time: idle, pending until it moves `SWIPE.lock` pixels, tracking once a gesture
 * claims its axis, settle while the release animates. A touch no gesture claims goes back to idle and
 * the browser keeps it. */
let swipe = { phase: "idle" };
function settleSwipe(gesture, ms) {
  if (!ms) {
    gesture.clear();
    swipe = { phase: "idle" };
    return;
  }
  swipe = { phase: "settle", gesture, timer: setTimeout(() => settleSwipe(gesture, 0), ms) };
}
function releaseSwipe(e) {
  if (swipe.phase === "pending") swipe = { phase: "idle" };
  if (swipe.phase !== "tracking") return;
  const { gesture, d, v, at } = swipe;
  const flick = v > SWIPE.flick && e.timeStamp - at < SWIPE.flickWindow;
  settleSwipe(gesture, e.type === "touchend" && (d >= gesture.reach() || flick) ? gesture.commit() : gesture.cancel());
}
workspace.addEventListener("touchstart", (e) => {
  if (swipe.phase === "settle") {
    clearTimeout(swipe.timer);
    settleSwipe(swipe.gesture, 0);
  }
  if (e.touches.length > 1) return releaseSwipe(e);
  if (!guiOn() || !narrow(768) || !sideMenu.hidden || e.target.closest(SWIPE_SKIP)) return;
  const p = e.touches[0];
  swipe = { phase: "pending", x: p.clientX, y: p.clientY, target: e.target };
}, { passive: true });
workspace.addEventListener("touchmove", (e) => {
  if (swipe.phase !== "pending" && swipe.phase !== "tracking") return;
  const p = e.touches[0];
  const dx = p.clientX - swipe.x;
  const dy = p.clientY - swipe.y;
  if (swipe.phase === "pending") {
    if (Math.max(Math.abs(dx), Math.abs(dy)) < SWIPE.lock) return;
    const direction = Math.abs(dx) > Math.abs(dy) ? (dx > 0 ? "right" : "left") : dy > 0 ? "down" : "up";
    const gesture = gestures.find((g) => g.direction === direction && g.accepts(swipe));
    if (!gesture || swipeScroller(swipe.target, direction)) {
      swipe = { phase: "idle" };
      return;
    }
    swipe = { phase: "tracking", gesture, x: swipe.x, y: swipe.y, d: 0, v: 0, at: e.timeStamp, motion: swipeMs() > 0 };
  }
  e.preventDefault();
  const d = Math.max(0, { right: dx, left: -dx, down: dy }[swipe.gesture.direction]);
  swipe.v = (d - swipe.d) / Math.max(1, e.timeStamp - swipe.at);
  swipe.d = d;
  swipe.at = e.timeStamp;
  if (swipe.motion) swipe.gesture.track(d);
}, { passive: false });
workspace.addEventListener("touchend", releaseSwipe);
workspace.addEventListener("touchcancel", releaseSwipe);
