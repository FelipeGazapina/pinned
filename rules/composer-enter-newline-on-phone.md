# Composer Enter newline on phone

Completed 2026-10-31. PR: https://github.com/FelipeGazapina/pinned/pull/36

## Problem
Mobile web UI sent the prompt on Enter because desktop uses Enter-to-send when the send-shortcut setting is `enter`. Touch keyboards have no Shift+Enter for multiline input.

## Solution
- `touchComposer()` detects coarse pointer without a fine pointer (`matchMedia`).
- `composerEnterIntent()` returns `send` vs `newline` vs `none`. Plain Enter newlines on touch. Cmd or Ctrl+Enter still sends.
- `prompt.enterKeyHint = "enter"` on touch. The send button title names ⌘Enter or CtrlEnter. The settings sentence names that chord on a hardware keyboard.
- Settings General page shows static copy instead of the send-shortcut `<select>` on touch.
- `ui/gui-composer.css`: 40px send button on `(pointer: coarse)`.

## Tests
`ui/composer-enter.test.js` exercises `composerEnterIntent`, `touchComposer`, and `triggerKey` Enter picking via vm slices of `gui-composer.js`.

## Verification
`node --test ui/*.test.js`. No browser screenshot (GUI boot too heavy for this run).
