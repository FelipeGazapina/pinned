# Composer chord guard

Completed 2026-10-31. Follow-up to the independent review on https://github.com/FelipeGazapina/pinned/pull/39#issuecomment-6545960043.

## Problem

Alt+Cmd+Enter during IME composition called `sendAndNewThread` and left the slash menu open. The next plain Enter then picked the highlighted row. `keyCode` 229 still picked a row. Option+Enter, Mac Ctrl+Enter, and Windows Meta+Enter closed the menu. On desktop those chords sent the typed fragment. On touch they inserted a newline. The project filter name and each project menu name truncate, and neither span had a `title`.

## Decision

The prompt keydown listener returns at entry when `isComposing` is set or `keyCode` is 229. That matches the shell, sidebar search, timeline, and theme-import handlers. `triggerKey` leaves the menu only when `composerEnterIntent` returns `new-thread`, or `send` together with `platformEnterMod`. Plain Enter, Option+Enter, and the modifier that is not this platform's send key keep the previous pick. `projectNameMark` puts the escaped full name on the filter and on each menu row. `projectWhere` already titles the machine label, and the composer `.where` spans already title the path or the machine label.

## Fix

`ui/gui-composer.js` returns at the top of the prompt listener. `composerEnterIntent` returns `new-thread` for Alt plus the platform modifier, and both the listener and `triggerKey` use that result. `ui/gui-sidebar.js` renders the filter and the menu name through `projectNameMark`.

## Tests

`ui/composer-enter.test.js` dispatches a cancelable keydown on the listener from `gui-composer.js`. A composing Alt+Cmd+Enter sends nothing and picks nothing. `keyCode` 229 with Enter, Cmd+Enter, Alt+Cmd+Enter, and Tab does the same. Option+Enter on desktop and on touch picks `/help` and sends nothing. Mac Ctrl+Enter and Windows Meta+Enter pick too. Cmd+Enter and Alt+Cmd+Enter still send, including when the shortcut is `mod-enter`. `ui/project-filter.test.js` runs `renderSidebar` and `openProjectMenu` and reads the `title` on the name and on the machine.

Removing the listener return made a composing Enter and `keyCode` 229 pick `/help`. Removing the narrow fall-through made Option+Enter pick nothing. Removing the `title` restored the bare name span in the filter and in the menu.

## Verification

`node --test ui/*.test.js`. 77 passed and 0 failed. No browser run. The listener is a real `Event` on an `EventTarget`. The filter test runs the sidebar functions that write the HTML.
