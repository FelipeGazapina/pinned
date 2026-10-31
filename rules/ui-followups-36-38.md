# UI follow-ups for reviews 36 and 38

Completed 2026-10-31. Reviews [36](https://github.com/FelipeGazapina/pinned/pull/36#issuecomment-6544627042) and [38](https://github.com/FelipeGazapina/pinned/pull/38#issuecomment-6544743180).

## Composer

Plain Enter on a touch device still inserts a newline. Cmd or Ctrl+Enter sends, including a Bluetooth keyboard with no trackpad. `enterNeedsMod` is the one shortcut rule used by the send-button title and by `composerEnterIntent`. A stopped claude on touch says to use the send button. A fully typed slash command is picked on touch. On desktop it still sends as typed. The em dash on the phone Enter note is a period.

## Project rows

With no open thread, the first online checkout starts. An open thread still wins, including an offline one. Machines that share a display name get a short id from the machine id in the label. `.where` truncates, the project name keeps a 3rem floor, and the menu gears stay inside a 288px row. The full machine label is the `title`. The screenshots that #38 added under `docs/screenshots/` are removed.

## Verification

`node --test ui/*.test.js`. 63 tests passed and 0 failed.

Playwright 1.64.1 at a 390px viewport, with `#project-menu` set to 288px the way the menu script does. The project label was 62.41px wide. Both gears sat inside the menu (right edges 252 and 284, menu right 296). The machine label client width was 106px against a scroll width of 284px.
