# Touch send chord with a menu open

Completed 2026-10-31. Review on https://github.com/FelipeGazapina/pinned/pull/39#issuecomment-6545329640.

## Problem

On a touch device, `triggerKey` treats Enter as a pick whenever a slash, mention, skill, or pull-request menu is open. It returns true. The prompt keydown listener then calls `preventDefault` and never reaches `composerEnterIntent` or `sendAndNewThread`. Cmd+Enter on a fully typed `/help` picked `/help` and sent nothing. Alt+Cmd+Enter did the same, so send-and-new-thread never ran. A composing Enter picked too. Desktop already sent an exact `/help` on that chord, because the exact-match path returned false.

## Decision

A modified Enter falls through on every platform. There is no extra `touchComposer()` check. Plain Enter on desktop still picks a shorter highlight and still sends an exact command. Cmd+Enter with a shorter highlight now sends the typed text on desktop as well. That chord means send. Shift+Enter stays a newline and leaves the menu open. A composing Enter does not pick and does not close the menu.

The review also suggested keeping the highlight after a plain Enter pick on touch, or telling the user to tap Send. That two-step is the product this branch already stated. It is left as it is.

## Fix

`triggerKey` returns false for Enter with Cmd, Ctrl, Alt, or a composition, unless Shift is held. It closes the menu only when the key is not composing, so the list does not stay up after a send. `composerEnterIntent` returns `none` for `keyCode` 229, matching the other GUI handlers. On touch the send button title is `Send (⌘Enter)` or `Send (CtrlEnter)`. The settings sentence says Enter adds a line, and that the same chord on a hardware keyboard sends.

## Tests

`ui/composer-enter.test.js` dispatches a cancelable keydown on the listener taken from `gui-composer.js`. Touch plus an open slash menu plus Cmd+Enter sends `/help` and does not pick. Alt+Cmd+Enter records `new-thread /help`. A composing Enter does not pick. Desktop plain Enter and an exact Cmd+Enter stay as they were. `keyCode` 229 does not send. The source-slice asserts in the shared-helper test are gone. The phone Enter note no longer states a test count.

Removing the fall-through made the touch send, the new-thread chord, the composing case, and the shorter highlight fail. Removing the menu-close assignment left the menu open after Cmd+Enter. Removing `keyCode === 229` sent `hello`. Removing the title expression restored `Send`. Removing the settings sentence restored the old button-only copy. Removing the Shift exclusion closed the menu on Shift+Cmd+Enter.

## Verification

`node --test ui/*.test.js`. 71 passed and 0 failed. No browser run. The listener is exercised in Node with a real `Event` on an `EventTarget`.
