/** On a phone a swipe does what a tap does: on a thread, right from the left edge goes back to Threads, a
 * pull from the top opens the thread's menu and left over the chat opens the header's options; on the
 * Threads screen, left on a row settles it and right snoozes it, and a long press opens its menu. The UI
 * follows the finger; letting go past the gesture's reach or with a flick runs the same action a tap
 * does, and the inline styles go once it settles so the CSS owns the final state. */
const SWIPE = { lock: 10, flick: 0.4, flickWindow: 100, settle: 200, pull: 56, options: 72, edge: 24, back: 0.3, row: 0.35, press: 500 };
/** A touch starting in any of these belongs to it: typing, the terminal, the side panel, a menu. */
const SWIPE_SKIP = "input, textarea, select, [contenteditable], .xterm, #terms, #drawer-resize, #panel, #side-menu, #panel-menu, #panel-submenu, #header-menu, #composer-menu, #composer-dialog, #details-card, #image-zoom, .popup, dialog";
const headMore = document.getElementById("head-more");
const swipeChat = document.getElementById("chat");
const swipeMs = () => (matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : SWIPE.settle);
/** A rubber band: the first pixels move almost 1:1, then it stiffens toward 24px. */
const rubber = (d) => 24 * (1 - Math.exp(-d / 64));

/** The thread slid `d` pixels right, toward the list it goes back to. */
function slideBack(d, ms = 0) {
  Object.assign(swipeChat.style, { transition: ms ? `transform ${ms}ms ease-out` : "none", transform: d ? `translateX(${d}px)` : "" });
}
function clearBack() {
  for (const p of ["transition", "transform"]) swipeChat.style.removeProperty(p);
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

function nudgeOptions(d, ms = 0) {
  headMore.classList.toggle("on", d >= SWIPE.options);
  Object.assign(headMore.style, { transition: ms ? `transform ${ms}ms ease-out` : "none",
    transform: d ? `translateX(${-rubber(d) / 2}px) scale(${1 + 0.25 * Math.min(1, d / SWIPE.options)})` : "" });
}
function clearOptionsNudge() {
  headMore.classList.remove("on");
  for (const p of ["transition", "transform"]) headMore.style.removeProperty(p);
}

/** What a swipe on a thread row does: its color and name under the row, and the action. Settle
 * leaves the row, so it runs once the row has slid out; Snooze asks until when, as its button does. */
const ROW_SWIPES = {
  left: { act: "settle", label: "Settle", icon: "check", can: (tab) => !settled(tab), run: (tab) => settle(tab), out: true },
  right: { act: "snooze", label: "Snooze", icon: "clock", can: (tab) => canSnooze(tab) && !snoozedNow(tab), run: (tab, row) => openSideMenu(snoozeItems([tab]), row), out: false },
};
/** The row of the thread `key` as the list draws it now: a render while the finger moves replaces it. */
const rowOf = (key) => [...threadList.querySelectorAll(".thread-row")].find((r) => r.dataset.tab === key) ?? null;
const swipeUnder = document.createElement("div");
swipeUnder.className = "swipe-under";
function slideRow(key, kind, d, ms = 0) {
  const row = rowOf(key);
  if (!row) return;
  const look = ROW_SWIPES[kind];
  row.classList.add("swiping");
  Object.assign(row.style, { transition: ms ? `transform ${ms}ms ease-out` : "none", transform: d ? `translateX(${kind === "left" ? -d : d}px)` : "" });
  if (swipeUnder.parentElement !== threadList) threadList.append(swipeUnder);
  if (swipeUnder.dataset.act !== look.act) {
    swipeUnder.dataset.act = look.act;
    swipeUnder.innerHTML = `${icon(look.icon)}<span>${look.label}</span>`;
  }
  swipeUnder.classList.toggle("armed", d >= row.offsetWidth * SWIPE.row);
  Object.assign(swipeUnder.style, { top: `${row.offsetTop}px`, height: `${row.offsetHeight}px` });
}
function clearRow(key) {
  const row = rowOf(key);
  if (row) {
    row.classList.remove("swiping");
    for (const p of ["transition", "transform"]) row.style.removeProperty(p);
  }
  swipeUnder.remove();
}
/** A thread row on the Threads screen that `kind` can act on, as { key, tab }. */
function swipedRow(start, kind) {
  if (phoneScreen() !== "threads") return null;
  const row = start.target.closest("#thread-list .thread-row:is(.tall, .line)");
  const tab = row && tabByKey(row.dataset.tab);
  return tab && ROW_SWIPES[kind].can(tab) ? { key: tab.key, tab } : null;
}
function rowGesture(kind) {
  return {
    name: ROW_SWIPES[kind].act, direction: kind,
    accepts: (start) => !!(start.row = swipedRow(start, kind)),
    reach: () => (rowOf(swipe.row.key)?.offsetWidth ?? 0) * SWIPE.row,
    track: (d) => slideRow(swipe.row.key, kind, d),
    commit: () => {
      const { key, tab } = swipe.row;
      const row = rowOf(key);
      const ms = swipeMs();
      if (!ROW_SWIPES[kind].out) {
        slideRow(key, kind, 0, ms);
        ROW_SWIPES[kind].run(tab, row);
        return ms;
      }
      slideRow(key, kind, row?.offsetWidth ?? 0, ms);
      setTimeout(() => ROW_SWIPES[kind].run(tab, row), ms);
      return ms;
    },
    cancel: () => (slideRow(swipe.row.key, kind, 0, swipeMs()), swipeMs()),
    clear: () => clearRow(swipe.row.key),
  };
}

/** Every swipe the page answers. `reach` is how far, in pixels along `direction`, a release commits;
 * `commit` and `cancel` return how long they animate before `clear` drops the inline styles. */
const gestures = [
  { name: "back", direction: "right", accepts: (start) => phoneScreen() === "thread" && start.x <= SWIPE.edge && !!start.target.closest("#chat"),
    reach: () => swipeChat.offsetWidth * SWIPE.back, track: (d) => slideBack(d),
    commit: () => (clearBack(), phoneGo("threads"), 0), cancel: () => (slideBack(0, swipeMs()), swipeMs()), clear: clearBack },
  { name: "title menu", direction: "down", accepts: (start) => !!active && !!titleButton() && !!start.target.closest("#chat-head, #thread"),
    reach: () => SWIPE.pull, track: (d) => pullTitle(d),
    commit: () => (clearTitlePull(), titleButton()?.click(), 0), cancel: () => (pullTitle(0, swipeMs()), swipeMs()), clear: clearTitlePull },
  { name: "options", direction: "left", accepts: (start) => !!active && !!headMore.offsetParent && !!start.target.closest("#chat"),
    reach: () => SWIPE.options, track: (d) => nudgeOptions(d),
    commit: () => (clearOptionsNudge(), headMore.click(), 0), cancel: () => (nudgeOptions(0, swipeMs()), swipeMs()), clear: clearOptionsNudge },
  rowGesture("left"),
  rowGesture("right"),
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

/** A finger held still on a row or a project's head opens its menu, as a right-click does. A phone
 * that sends its own contextmenu for the press opens it once. */
let press = null;
function startPress(e, p) {
  const target = phoneScreen() === "threads" && e.target.closest("#thread-list :is(.thread-row, .project-head)");
  if (!target) return;
  const held = (press = { fired: false, timer: 0 });
  held.timer = setTimeout(() => {
    held.fired = true;
    target.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: p.clientX, clientY: p.clientY }));
  }, SWIPE.press);
}
function endPress() {
  if (press) clearTimeout(press.timer);
  const fired = !!press?.fired;
  press = null;
  return fired;
}
threadList.addEventListener("contextmenu", (e) => {
  if (!e.isTrusted || !press) return;
  if (press.fired) {
    e.preventDefault();
    e.stopImmediatePropagation();
  } else {
    clearTimeout(press.timer);
    press.fired = true;
  }
}, true);

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
  swipe = { ...swipe, phase: "settle", gesture, timer: setTimeout(() => settleSwipe(gesture, 0), ms) };
}
function releaseSwipe(e) {
  // The press opened a menu: no tap follows it, so the menu stays up.
  if (endPress() && e.type === "touchend") e.preventDefault();
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
  endPress();
  if (e.touches.length > 1) return releaseSwipe(e);
  if (!guiOn() || !narrow(768) || !sideMenu.hidden || e.target.closest(SWIPE_SKIP)) return;
  const p = e.touches[0];
  swipe = { phase: "pending", x: p.clientX, y: p.clientY, target: e.target };
  startPress(e, p);
}, { passive: true });
workspace.addEventListener("touchmove", (e) => {
  if (swipe.phase !== "pending" && swipe.phase !== "tracking") return;
  const p = e.touches[0];
  const dx = p.clientX - swipe.x;
  const dy = p.clientY - swipe.y;
  if (swipe.phase === "pending") {
    if (Math.max(Math.abs(dx), Math.abs(dy)) < SWIPE.lock) return;
    endPress();
    const direction = Math.abs(dx) > Math.abs(dy) ? (dx > 0 ? "right" : "left") : dy > 0 ? "down" : "up";
    const gesture = gestures.find((g) => g.direction === direction && g.accepts(swipe));
    if (!gesture || swipeScroller(swipe.target, direction)) {
      swipe = { phase: "idle" };
      return;
    }
    swipe = { phase: "tracking", gesture, row: swipe.row, x: swipe.x, y: swipe.y, d: 0, v: 0, at: e.timeStamp, motion: swipeMs() > 0 };
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
