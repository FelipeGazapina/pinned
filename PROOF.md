# PR #207 dictation permission: UI proof

The real `ui/` of `fix/dictation-permission` at `d7744ffc`, served as static files and driven in headless Google Chrome 1440x900, dark theme, composer in view. Invented data only (project `acme-web`, machine `Demo Mac`). No code in the PR was changed.

| Shot | What it shows |
| --- | --- |
| `shots/1-microphone-denied.png` | Mic clicked with Microphone `denied`: the message "pinned isn't allowed to use Microphone. Turn on pinned under Privacy & Security > Microphone in System Settings, then click the mic again." and the **Open System Settings** button, under the composer. |
| `shots/2-speech-recognition-denied.png` | Microphone `granted`, Speech Recognition `denied`: the same message naming Speech Recognition, button opens the `speechRecognition` pane. |
| `shots/3-second-click-still-denied.png` | A second mic click while still denied: the same Microphone message again. The UI asked macOS again (see counts). |
| `shots/4-granted-after-focus.png` | Both granted in the stub, then a window `focus` event while the message was up: the message is gone and "Microphone and Speech Recognition are on. Click the mic to dictate." shows. |
| `shots/5-granted-recording.png` | Both granted, mic clicked: the mic button is in the recording state (`aria-pressed=true`, title "Stop dictation (fn)", composer `data-dictating=true`). Taken with reduced motion so the pulse is not caught mid-fade. |
| `shots/6-browser-microphone-blocked.png` | Browser mode (no Tauri), `getUserMedia` rejected with `NotAllowedError`: "Your browser blocked the microphone for this site. Click the lock or site settings icon in the address bar, allow the microphone, then click the mic again." and no button. |

## How it was faked

- Shots 1 to 5 (app mode): a `window.__TAURI__` stub added with `addInitScript` before the page loads. Its `invoke` answers `dictation_permissions` from a variable (`{microphone, speechRecognition}`) that the script changes between shots, `dictation_start` and `dictation_stop` succeed, `open_privacy_settings` is recorded, `account` returns `{signed_in:false}` so no hub is needed, and `machine_call` serves one `acme-web` thread with two messages. `navigator.platform` is already `MacIntel` on this Mac, so `IS_MAC` is true.
- Shot 6 (browser mode): no stub. `localStorage hub-token` is set and `page.routeWebSocket(/\/ws\/client/)` plays the hub (`hello` with one online machine and the same thread, `reply` frames for `call` and `hub`). `navigator.mediaDevices.getUserMedia` is replaced with a function that rejects with a `NotAllowedError`.

## Stub call counts (one run, shots 1 to 5)

| Command | Calls | When |
| --- | --- | --- |
| `dictation_permissions` | 5 | click (shot 1), click (shot 3), click (shot 2), `focus` recheck (shot 4), click (shot 5) |
| `open_privacy_settings` | 1 | `{permission:"microphone"}` from the button under shot 3 |
| `dictation_start` | 1 | only the granted click (shot 5); none of the three denied clicks started it |

Shot 6: `getUserMedia` was called once.

## Seen while shooting (not changed)

- In shot 5 the mic stays gray while recording. `#composer-dictate[data-active="true"]` sets the red color, but `#composer .tools > :is(#composer-attach, #composer-dictate)` (specificity higher, in `gui-composer.css`) sets `color: var(--g-side-muted)` and wins; only the icon pulse marks recording. This rule is not in the PR's diff.
- "Microphone and Speech Recognition are on. Click the mic to dictate." (shot 4) is still on screen after the mic is clicked and recording starts (shot 5).
