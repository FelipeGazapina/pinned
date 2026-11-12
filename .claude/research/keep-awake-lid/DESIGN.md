# Keep awake with the lid closed while agents run (0.18.2 design)

Status: draft for review. Design only: no code changed. Base: `main` at v0.18.1 (`5af27835`).

## What Felipe asked, and what this design assumes

Felipe: "add a switch where user on the general settings can set as true for pinned to work when the
notebook is closed or on sleep mode … I want basically a way to close my laptop and the workers and
orchestrator to still be alive while any agent is running".

Confirmed with Felipe:

- One switch in Settings → General, **off by default**.
- When it is on, the computer stays awake, **lid closed included**, **while any agent runs on that
  machine** (an Orchestrator turn, a Captain, a worker, a session, a scheduled workflow or task run).
- When nothing runs, the computer sleeps as it normally would.

Assumed by the Orchestrator, not said by Felipe (each is a default the user can change):

- It works on charger and on battery. On battery it stops below a minimum charge, **20%** by
  default, shown under the switch. Then it lets the computer sleep and says so in the app and the hub.
- **Where I'd default differently:** I keep 20%, but I add two guards Felipe did not mention, both
  on by default, because with the lid closed nobody sees the machine: a **heat guard** (section 4)
  and a **time cap** of 8 hours per stretch of keeping awake on battery (none on charger). Reason:
  on macOS the only way to stay awake with the lid closed also turns off the system's own
  low-battery and thermal sleep (section 1), so pinned has to do that job itself.

How it relates to today's switch: Settings → General already has **Keep this computer awake**
(`ui/gui-features.js:35`, `src/awake.rs`). That one holds an idle-sleep lock all the time, display
included, until switched off or the app quits, and **does not survive a lid close** on any
platform (section 1). The new switch is separate, because it means something else: "only while
agents run, and through a lid close". Proposed label: **Keep working with the lid closed** and the
line under it: "While an agent runs on this Mac, it stays awake even with the lid closed. When
nothing runs, it sleeps as usual. On battery it stops at 20%."

---

## 1. What each platform allows

### 1.1 macOS

**Idle sleep is easy; lid-close sleep is not.** pinned already uses `caffeinate -d -i -w <pid>`
(`src/awake.rs:52`). That makes `PreventUserIdleDisplaySleep` and `PreventUserIdleSystemSleep`
assertions. Apple's own header says these do not stop a lid close:

> `kIOPMAssertPreventUserIdleSystemSleep` … "The system may still sleep for lid close, Apple menu,
> low battery, or other sleep reasons."
> — IOPMLib.h, [apple-oss-distributions/IOKitUser `pwr_mgt.subproj/IOPMLib.h`](https://github.com/apple-oss-distributions/IOKitUser/blob/main/pwr_mgt.subproj/IOPMLib.h)
> (same text as [developer.apple.com IOPMLib](https://developer.apple.com/documentation/iokit/iopmlib_h))

Apple's Q&A QA1340 says the same thing in plain words:

> "even with I/O Kit, it is not possible to prevent forced sleep, only delay it. … Closing the lid
> on a laptop or selecting sleep from the Apple menu both cause forced sleep."
> — [Technical Q&A QA1340](https://developer.apple.com/library/archive/qa/qa1340/_index.html)

`caffeinate -s` ("prevent the system from sleeping … valid only when system is running on AC
power", [caffeinate(8)](https://ss64.com/mac/caffeinate.html)) makes the old `PreventSystemSleep`
assertion, which IOPMLib.h marks "Deprecated in 10.9. This assertion is not supported in any OS X
releases." So it is no answer either.

**What does keep a closed Mac awake** (from Apple's open-source power code):

1. **Clamshell mode** (external display + charger + keyboard/mouse). The kernel will not sleep on
   lid close in "desktop mode with AC": `shouldSleepOnClamshellClosed()` returns
   `!clamshellDisabled && !(desktopMode && acAdaptorConnected) && !clamshellSleepDisableMask`
   ([xnu `IOPMrootDomain.cpp`](https://github.com/apple-oss-distributions/xnu/blob/main/iokit/Kernel/IOPMrootDomain.cpp)).
   Felipe's case has no external display, so this does not help.
2. **An assertion with `AppliesOnLidClose`.** powerd accepts that property only from a caller with
   a private Apple entitlement: `auditTokenHasEntitlement(token, kIOPMAssertOnLidCloseEntitlement)`,
   else "Pid %d is not privileged to set property"
   ([PowerManagement `pmconfigd/PMAssertions.c`](https://github.com/apple-oss-distributions/PowerManagement/blob/main/pmconfigd/PMAssertions.c), around line 2571).
   Third-party apps cannot get Apple-private entitlements, so this is closed to pinned.
3. **`pmset disablesleep 1`** (the system power setting `SleepDisabled`). This is the one that
   works without an external display. Facts, each from Apple's source:
   - `pmset` parses `disablesleep` and writes `kIOPMSleepDisabledKey` into the system power
     settings ([PowerManagement `pmset/pmset.m`](https://github.com/apple-oss-distributions/PowerManagement/blob/main/pmset/pmset.m), `ARG_DISABLESLEEP`, around line 5813). The `pmset(1)` man page does
     **not** list it ([pmset.1](https://github.com/apple-oss-distributions/PowerManagement/blob/main/pmset/pmset.1)): it is undocumented.
   - **It needs root.** `IOPMSetSystemPowerSetting` returns `kIOReturnNotPrivileged` unless
     `getuid()` or `geteuid()` is 0, and the man page says "pmset must be run as root in order to
     modify any settings"
     ([IOKitUser `IOPMEnergyPrefs.c`](https://github.com/apple-oss-distributions/IOKitUser/blob/main/pwr_mgt.subproj/IOPMEnergyPrefs.c), `IOPMSetSystemPowerSetting`).
   - **It is persistent: it survives quit, crash and reboot.** The same function stores it with
     `CFPreferencesSetValue(... kCFPreferencesAnyUser, kCFPreferencesCurrentHost)` and powerd
     re-applies it at start (`PMActivateSystemPowerSettings` in
     [`pmconfigd/PMSettings.m`](https://github.com/apple-oss-distributions/PowerManagement/blob/main/pmconfigd/PMSettings.m)).
     So a pinned crash would leave a Mac that never sleeps, across reboots, until someone turns it
     back. Section 2 is mostly about this.
   - **It blocks every sleep, lid close included, and also low-battery and thermal sleep.** powerd
     passes it to the kernel as `userDisabledAllSleep`; `checkSystemSleepAllowed()` refuses sleep
     when it is set ("1. user-space sleep kill switch"), and `privateSleepSystem()` (used for
     `kIOPMSleepReasonClamshell`, `kIOPMSleepReasonLowPower` and
     `kIOPMSleepReasonThermalEmergency`) returns `kIOReturnNotPermitted` when sleep is not allowed
     ([xnu `IOPMrootDomain.cpp`](https://github.com/apple-oss-distributions/xnu/blob/main/iokit/Kernel/IOPMrootDomain.cpp)).
     Hence pinned's own battery floor and heat guard.

**Answer to the question:** yes. Without an external display, lid-close sleep on macOS can only be
prevented by the root-only `SleepDisabled` setting (`pmset disablesleep`), or by an Apple-entitled
assertion that pinned cannot have.

**Uncertain on macOS (needs Felipe's manual test, section 7):**

- That `disablesleep 1` keeps Wi-Fi associated and the node's WebSocket alive with the lid closed
  on an Apple-silicon MacBook on battery. The kernel stays in full wake, so it should; the source
  does not prove the Wi-Fi driver's behavior.
- That the hardware still shuts down on a real thermal emergency with `SleepDisabled` set. That is
  the SMC's job, not the kernel's sleep path, but I found no Apple document saying so. Our heat
  guard must not rely on it.
- `pmset disablesleep` is undocumented, so Apple may change it. pinned must read the value back
  after writing it and say so in the app when it did not take.

### 1.2 Windows

- `SetThreadExecutionState` (pinned uses it today, `src/awake.rs:125`) "cannot be used to prevent
  the user from putting the computer to sleep. Applications should respect that the user expects a
  certain behavior when they close the lid on their laptop or press the power button."
  ([SetThreadExecutionState](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate)).
  `ES_AWAYMODE_REQUIRED` is for media apps "on desktop computers" and the same page says apps that
  "run on portable computers should not enable away mode".
- The lid action is a power-plan setting: **Lid switch close action**, powercfg alias `LIDACTION`,
  GUID `5ca83367-6e45-459f-a27b-476b1d01c936`, subgroup `SUB_BUTTONS`
  (`4f971e89-eebd-4455-a8de-9e59040e7347`), values 0 Do nothing, 1 Sleep, 2 Hibernate, 3 Shut down
  ([Lid switch close action](https://learn.microsoft.com/en-us/windows-hardware/customize/power-settings/power-button-and-lid-settings-lid-switch-close-action)).
  It has separate AC and DC values (`powercfg /setacvalueindex`, `/setdcvalueindex`,
  [powercfg options](https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/powercfg-command-line-options)),
  written with `PowerWriteACValueIndex` / `PowerWriteDCValueIndex`, which take effect only after
  `PowerSetActiveScheme`
  ([PowerWriteACValueIndex](https://learn.microsoft.com/en-us/windows/win32/api/powersetting/nf-powersetting-powerwriteacvalueindex)).
- Like macOS, it is **persistent** (stored in the power scheme), so it needs the same restore
  machinery (section 2).
- **Admin: uncertain.** Each scheme has its own security descriptor
  (`powercfg /getsecuritydescriptor`); Microsoft's own example SDDL grants Builtin Users `KRKW`
  (read and write). In practice a standard user can usually change the active plan's values, but
  Group Policy or an admin can lock it. The design tries without elevation and handles
  `ERROR_ACCESS_DENIED` (section 2).
- **Modern Standby: uncertain.** On Modern Standby (S0 low-power idle) PCs, standby is entered when
  the screen goes off, and "the networking stacks may initiate disconnection" on battery
  ([Modern Standby wake sources](https://learn.microsoft.com/en-us/windows-hardware/design/device-experiences/modern-standby-wake-sources)).
  With `LIDACTION = 0` the lid itself triggers nothing, and the existing `ES_SYSTEM_REQUIRED` hold
  keeps the idle timer from entering standby; I could not confirm from Microsoft's docs that a
  closed lid with "Do nothing" never drops a Modern Standby laptop into standby. Windows ships as
  **beta** for this switch until someone tests it on real hardware.
- Battery protection stays with Windows: the battery "critical action" is its own setting, not the
  lid action, so Windows still hibernates or shuts down at critical battery.

### 1.3 Linux (beta)

- logind inhibitor locks have a `handle-lid-switch` type. It "need[s] to be taken in the 'block'
  mode", and the lock ends when its file descriptor closes, including when the process dies: "the
  kernel automatically closes the file descriptor so that the lock is automatically released"
  ([systemd Inhibitor Locks](https://systemd.io/INHIBITOR_LOCKS/)).
- **Today's lock does not cover the lid.** pinned takes `--what=sleep:idle` (`src/awake.rs:82`).
  logind's `LidSwitchIgnoreInhibited=` defaults to `yes`, so "high-level inhibitor locks (shutdown,
  reboot, sleep, idle) are ignored for the lid. Low-level `handle-lid-switch` inhibitor locks are
  always honored" ([logind.conf(5)](https://www.freedesktop.org/software/systemd/man/latest/logind.conf.html)).
- **No admin.** polkit `org.freedesktop.login1.inhibit-handle-lid-switch` defaults to
  `allow_active=yes`, `allow_inactive=yes`, `allow_any=no`
  ([systemd `org.freedesktop.login1.policy`](https://github.com/systemd/systemd/blob/main/src/login/org.freedesktop.login1.policy)),
  and "the policy checks for Inhibit() are never interactive". A local desktop user gets it with
  no prompt; an SSH-only session is refused, and pinned says so.
- **No crash risk:** nothing persistent is written; a dead pinned drops the lock.
- **Uncertain:** desktop environments that handle the lid themselves (KDE PowerDevil, some GNOME
  setups) may act on the lid on their own. Manual test only; Linux stays beta.

### 1.4 Summary

| | Lid-close sleep blockable? | How | Admin? | Survives a crash? |
|---|---|---|---|---|
| macOS | Yes | `SleepDisabled` (`pmset disablesleep 1`) | **Yes, root** | **Yes, even reboot** → needs a root watchdog |
| Windows | Yes (Modern Standby uncertain) | power plan `LIDACTION` = 0 on AC and DC | Usually no; maybe | **Yes** → needs a restore at logon |
| Linux | Yes | logind `handle-lid-switch` block inhibitor | No (active session) | No: kernel drops it |

---

## 2. Admin rights, consent, and restoring the setting

Rules, on every platform:

- pinned never writes a power setting without the user turning the switch on themself (or
  approving an agent's request, section 6) and seeing the system's own prompt where one is needed.
- pinned never sees, asks for or stores a password.
- pinned writes the lid setting only while an agent runs, and puts back **what was there before**:
  when the last agent finishes (after the grace time), when the switch goes off, when the app
  quits, when the battery floor or heat guard trips, and, after a crash, without waiting for pinned
  to start again.
- If the machine already had `disablesleep 1` (macOS) or `LIDACTION = 0` (Windows) before pinned
  touched it, pinned leaves it that way and never turns it off.

### 2.1 macOS: a small root helper that holds a lease

Admin is required (section 1.1), and the setting outlives a crash, so the piece that writes it must
be able to put it back when pinned dies. That piece is a **launchd daemon**, `pinned-lid-helper`,
shipped inside the app bundle.

**Install and consent (the system's own prompt).** macOS 13+: `SMAppService.daemon(plistName:)`
registers a plist from `Contents/Library/LaunchDaemons` whose `BundleProgram` points into the
bundle. "The system won't bootstrap the LaunchDaemon until an admin approves the LaunchDaemon in
System Preferences", and once approved it starts "on each subsequent boot"
([SMAppService.register()](https://developer.apple.com/documentation/servicemanagement/smappservice/register()),
[SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice),
[Updating helper executables](https://developer.apple.com/documentation/servicemanagement/updating-helper-executables-from-earlier-versions-of-macos)).
Flow when the user turns the switch on:

1. pinned calls `register()`. The status becomes `.requiresApproval`
   ([status: requiresApproval](https://developer.apple.com/documentation/servicemanagement/smappservice/status-swift.enum/requiresapproval)).
2. The switch shows "Allow pinned in System Settings → Login Items" with a button that calls
   `SMAppService.openSystemSettingsLoginItems()`. The user approves with their admin password
   **in System Settings**. pinned never sees it.
3. When the status turns `.enabled`, the switch reads "On". Turning the switch off calls
   `unregister()` after the setting is restored, so no root code stays installed.

**Two catches, both to be settled in PR 1 (section 7):**

- pinned's `tauri.conf.json` has `minimumSystemVersion` **12.0**, and SMAppService is macOS 13+.
  On macOS 12 the switch is shown off and disabled with "Needs macOS 13 or later". (The old way,
  `SMJobBless`, is deprecated; I would not build it for one OS version.)
- Releases are **ad-hoc signed** unless the Developer ID secrets exist (`release.yml:98`,
  `docs/release/developer-id.md`). launchd checks the daemon's signature, and I could not confirm
  from Apple's docs that SMAppService daemons register at all for ad-hoc-signed apps. If they do
  not, the switch on an ad-hoc build is disabled with "Needs the signed release of pinned". The
  Developer ID path is already written, so this is a reason to turn it on, not new work.

**What the helper does** (small, one job, written in Rust, no network):

- Listens on a Unix socket `/var/run/pinned-lid.sock`, mode 0666, and accepts a connection only if
  `getpeereid()` returns the uid that registered it (written at first approval to
  `/Library/Application Support/pinned/lid/owner`, root-owned). Turning sleep off is not a
  privilege escalation, but this keeps other local users out.
- Speaks four lines: `hold <ttl-secs> <floor-pct>`, `renew`, `release`, `status`.
- **The connection is the lease.** While at least one connection holds and has renewed within its
  TTL (pinned renews every 30 s, TTL 90 s), the helper keeps `SleepDisabled = true`. When the last
  hold ends (release, socket closed because pinned crashed or was killed, or TTL lapsed because
  pinned hung), it restores the saved value at once.
- **Journal before writing.** Before the first `SleepDisabled = true`, it writes the previous value
  to `/Library/Application Support/pinned/lid/previous.json` (root-owned, fsync). It writes back
  only from that file, and deletes the file after restoring.
- **Restore at its own start.** launchd plist: `RunAtLoad = true`, `KeepAlive = true`. Whenever the
  helper starts (boot, launchd restarting it after its own crash, an update), its first step is:
  if `previous.json` exists, restore that value and delete the file, before accepting any client.
  So **a reboot always puts sleep back**, even if pinned is never opened again.
- **Its own guards**, independent of the app: it re-checks battery (`IOPSCopyPowerSourcesInfo`)
  and `ProcessInfo.thermalState` every 30 s and releases when on battery below the floor, or at
  `.critical`. Belt and braces for a hung app.
- **Hard cap.** No hold lasts longer than 24 h without a fresh `hold` (not just `renew`), however
  often it is renewed: a loop bug cannot keep a Mac awake for days.

**How a crash cannot leave the Mac unable to sleep forever:**

| What dies | What happens | Worst case |
|---|---|---|
| pinned crashes / is force-quit | kernel closes the socket → helper restores now | ~0 s |
| pinned hangs, socket stays open | no `renew` → TTL lapses → helper restores | 90 s |
| helper crashes | launchd restarts it (`KeepAlive`) → restores from `previous.json` at start | seconds |
| power loss / kernel panic / forced reboot | helper runs at boot (`RunAtLoad`) → restores before any client | until boot finishes |
| user deletes pinned.app while on | the daemon's `BundleProgram` is gone, so it cannot run; `SleepDisabled` stays set. **Gap.** Mitigation below |
| user turns off pinned in Login Items while on | launchd stops the helper (SIGTERM) → it restores on SIGTERM before exiting | ~0 s |

The one gap: deleting the app while a hold is active. Mitigation: `previous.json` is written only
while a hold is active (minutes to hours, not forever), and the Settings page and docs say "turn
this off before you delete pinned". A second, independent safety net closes it fully: on each hold
the helper also arms a launchd one-shot `com.pinned.lid-restore` in `/Library/LaunchDaemons` whose
`ProgramArguments` is `/usr/bin/pmset disablesleep <previous>` with `RunAtLoad` and
`StartInterval = 900`, gated by `PathState` on `previous.json` existing; that uses only system
binaries, so it works without the bundle. I'd treat this as optional (PR 3b) and decide after
reading what SMAppService allows us to write; it is the only place that writes outside the bundle.

**Fallback considered and rejected:** `osascript -e 'do shell script "pmset disablesleep 1" with
administrator privileges'`. It uses the system prompt, but it asks for the password every time an
agent starts, and nothing would restore after a crash. Rejected.

### 2.2 Windows: no helper; restore at logon

- On "hold": read the current AC and DC `LIDACTION` of the active scheme, write them to
  `%APPDATA%\pinned\lid-previous.json` (fsync), then set both to 0 and `PowerSetActiveScheme`.
- **Crash safety without admin:** before the first write, add
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\pinned-lid-restore =
  "<pinned.exe> --restore-lid"`. At the next logon Windows runs it; it restores from the journal
  and removes itself and the journal. pinned's normal start does the same restore first thing.
- **Watcher for crashes during the session:** pinned starts a tiny child of its own exe,
  `pinned.exe --lid-guard <pid>`, which waits on pinned's process handle (`WaitForSingleObject`)
  and restores when pinned exits for any reason, then exits. If pinned hangs, the guard also
  restores when the lease file's timestamp (renewed every 30 s) is older than 90 s.
- **If Windows refuses the write** (`ERROR_ACCESS_DENIED`, a locked power policy): v1 shows "Your
  administrator has locked this PC's power settings" and stays off. Elevating via UAC would also
  need an elevated restore at logon (a scheduled task with highest privileges); that is a later PR
  only if anyone hits it.

### 2.3 Linux: nothing to restore

The logind lock dies with its file descriptor (section 1.3). pinned changes the existing hold to
`--what=sleep:idle:handle-lid-switch` while an agent runs and the switch is on. The existing
`tail --pid` trick and process group (`src/awake.rs:70-110`) already end it when pinned dies.

---

## 3. What counts as "an agent is running", and the grace time

### 3.1 Where the node already knows it

All of it lives in the node, `crates/core/src/machine.rs`, which runs inside the desktop app:

- **Tabs** (`Machine::tabs`, one `Tab` per session: threads, Captains, workers, the global
  Orchestrator, workflow sessions, connector sessions). `Tab::notice_idle()`
  (`machine.rs:588`) and `Tab::quiet()` (`machine.rs:572`) already decide "between turns":
  a turn is in progress when `turn_pending` is set (typed, no hook yet), `status` is `Working`
  (unless it is only waiting on background work it left, `background_wait`), it is `starting`, or
  it left shells running (`shells_left`). The wire mirror is `SessionState::of`
  (`global_orchestrator.rs:687`): `Running` when `status == Working || orchestrator_turn`.
- **The Orchestrator's turn** on a Captain: `TabView::orchestrator_turn` (`wire.rs:145`), set
  when the global orchestrator's message started a turn before its first hook.
- **Messages waiting to be typed**: `TabView::queued` (`wire.rs:149`), worker news and orchestrator
  requests that will start a turn as soon as the tab is idle.
- **Delegated tasks**: `delegates.running()` (`delegate/registry.rs:1482`), every task not in a
  terminal `WorkState` (`delegate/mod.rs:318`: `Working`, `WaitingForChildren`). `summary()`
  (`machine.rs:5830`) already filters out tasks that run on another machine (`task.remote`).
- **Scheduled workflow / connector runs**: owned by the hub (`graph/runner.rs`), but their work is
  a session on a machine (`talk` opens a tab; `run_now` starts it), so the tab rule covers them.

### 3.2 The rule

New pure function in core, `awake::busy(&tabs, &tasks, now) -> Busy { agents: u32, waiting: u32 }`.
A machine is **busy** when any of these holds, counting only work on this machine:

1. a tab of `Program::Agent` with a turn in progress: `turn_pending`, `starting`,
   `status == Working` (including `background_wait`, since background work it started is still
   agent work), or `shells_left`;
2. a tab with `orchestrator_turn` or `queued > 0`;
3. a delegated task not terminal and not `remote`.

**Does not count:** a tab sitting at its prompt; a `Failed` tab; a tab waiting out a usage limit
(`resume_at`): that can be hours, and the session continues when the machine wakes; a plain shell
in the terminal panel (the brief says agents; `panel.rs:1399` `busy()` exists if Felipe wants
shells too later).

**Waiting on the user mid-turn** (`NeedsInput` while the turn is open: an approval or a question)
counts, so Felipe can answer from the phone and the work goes on. But it is capped: if **every**
busy tab has been waiting on the user for **30 minutes**, the machine is treated as not busy and
the hub shows "Waiting on you; let the Mac sleep". Without the cap one forgotten approval keeps a
laptop awake in a bag all night.

### 3.3 Grace time

**2 minutes** after the last agent ends before giving the lid hold back. Reasons: a Captain often
starts a new turn seconds after a worker reports (`queued` covers most, not all); a worker's end
reaches its parent through the hub; scheduled steps chain. Two minutes of a closed laptop awake is
cheap; sleeping mid-chain loses the next step until the lid opens. The grace time is a constant,
not a setting.

Evaluation: on every tab status change (the node already emits events there) plus a 10 s tick; the
result is debounced, so a flicker between two turns does not toggle the root helper.

---

## 4. Heat guard

A closed MacBook in a bag with agents compiling Rust is the real risk.

**macOS.** `ProcessInfo.processInfo.thermalState` and `thermalStateDidChangeNotification`
([thermalState](https://developer.apple.com/documentation/foundation/processinfo/thermalstate-swift.property),
[thermalStateDidChangeNotification](https://developer.apple.com/documentation/foundation/processinfo/thermalstatedidchangenotification)).
Apple: `.serious` means "Fans are running at maximum speed" and the system is reducing performance;
`.critical` means "the device needs to cool down"
([serious](https://developer.apple.com/documentation/foundation/processinfo/thermalstate-swift.enum/serious),
[critical](https://developer.apple.com/documentation/foundation/processinfo/thermalstate-swift.enum/critical)).
Rule, **only while the lid is closed** (the lid state is the root domain's `AppleClamshellState`
registry property, set in `IOPMrootDomain.cpp` and readable without privileges):

- `.critical` → release the lid hold now.
- `.serious` for 5 minutes in a row → release.
- After a heat release, the lid hold is not taken again until the lid opens: the user has to look.

Remember that with `SleepDisabled` the kernel will not do its own thermal-emergency sleep
(section 1.1), so the helper repeats the `.critical` check itself (section 2.1).

**Windows.** No public user-mode thermal-state API that I found in Microsoft's docs; ACPI thermal
zones go to the kernel's thermal manager. Guard: the **time cap** (8 h per stretch on battery) and
the battery floor. Lid state from `GUID_LIDSWITCH_STATE_CHANGE`
([Power Setting GUIDs](https://learn.microsoft.com/en-us/windows/win32/power/power-setting-guids),
not re-read for this design).

**Linux.** `/sys/class/thermal/thermal_zone*/temp` against each zone's `passive`/`hot` trip points
([kernel thermal sysfs](https://docs.kernel.org/driver-api/thermal/sysfs-api.html), not re-read):
release when any zone passes its `hot` trip, or within 5 °C of `critical`. Plus the time cap. Lid
state: logind's `LidClosed` property.

**What the user sees** (app Settings line, the machine on the hub, and a notification in the app
next time it is opened):

- "Stopped keeping awake at 14:32: the Mac was too hot with the lid closed. It went to sleep."
- "Stopped keeping awake at 02:10: 8 hours on battery with the lid closed."

**Battery floor (all platforms):** read every 30 s (`IOPSCopyPowerSourcesInfo` on macOS,
`GetSystemPowerStatus` on Windows, `/sys/class/power_supply/*/capacity` on Linux). Below the floor
on battery → release; "Stopped keeping awake at 19% battery. It went to sleep." The floor does not
apply on charger. Once released, the Mac sleeps and stays asleep until the lid opens, so plugging
it in afterwards does not bring it back; the next hold starts at the next wake.

---

## 5. Network with the lid closed, and what the hub shows

### 5.1 Does the node keep its hub connection?

- **macOS with `SleepDisabled`:** the machine never sleeps, so it is in full wake with the display
  off. This is **not** Power Nap or dark wake, which pinned must not count on: Power Nap is a short
  dark wake for system tasks, not for third-party apps. The node's WebSocket (`link.rs`, ping every
  20 s, `link.rs:21`) should stay up over Wi-Fi. **To confirm in Felipe's manual test** (1.1).
- **App Nap**: pinned does not opt out of App Nap anywhere today (`grep` finds no
  `beginActivity`). With the window gone and the lid closed, macOS may throttle pinned's timers.
  While the lid hold is on, the app takes
  `ProcessInfo.beginActivity(options: [.userInitiated, .idleSystemSleepDisabled])`
  ([ActivityOptions](https://developer.apple.com/documentation/foundation/processinfo/activityoptions))
  and ends it with the hold. This also replaces `caffeinate` for the idle part on macOS.
- **Windows:** classic S3 laptops stay fully on with `LIDACTION = 0`. Modern Standby laptops:
  uncertain (1.2); on battery "the networking stacks may initiate disconnection" if standby is
  entered. Beta until tested.
- **Linux:** with the lid inhibited the machine is simply on.

### 5.2 What the hub shows for that machine

Precedent: the app already reports Run in the background with `MachineEvent::Background { on }`
(`wire.rs:1005`), kept by the hub as `MachineView::background` (`wire.rs:1468`, `hub.rs:1156`).
Same pattern, with one difference: **the hub keeps the last report after the machine goes
offline**, because "went to sleep at 19%" matters exactly when it is offline.

Settings → Machines (`ui/account.js` `machineStateHtml`) and the machine's row in the web app show
one line:

| State | Line |
|---|---|
| Switch on, nothing running | "Sleeps when idle · keeps awake while agents run" |
| Holding, lid open | "Kept awake · 2 agents running" |
| Holding, lid closed | "Awake with the lid closed · 2 agents running · 64% battery" |
| Grace | "Agents finished · sleeps in 2 min" |
| Released on battery (offline after) | "Went to sleep at 14:32: battery at 19%" |
| Released on heat / time cap | "Went to sleep at 14:32: too hot" / "…: 8 h on battery" |
| Needs approval on the Mac | "Keep awake with the lid closed needs approval on this Mac" |

### 5.3 Wire changes, additive only (AGENTS.md)

All in `crates/core/src/wire.rs`, all new names, nothing renamed or removed:

- `Call::KeepAwakeSettings` → `awake::Settings`; `Call::SetKeepAwakeSettings { settings }` →
  the saved `awake::Settings`. A machine without the app refuses, like `SetBackground`.
- `awake::Settings { lid: bool, battery_floor: u8 /* default 20 */, battery_hours: u8 /* default
  8, 0 = no cap */, heat_guard: bool /* default true */ }`, every field `#[serde(default)]`.
- `MachineEvent::Awake { state: AwakeReport }`, sent on connect and on each change.
  `AwakeReport { lid_switch: bool, holding: bool, lid_closed: Option<bool>, on_battery:
  Option<bool>, battery: Option<u8>, agents: u32, stopped: Option<Stopped> }`,
  `Stopped { reason: StopReason, at: i64 }`,
  `enum StopReason { LowBattery, Heat, TimeCap, WaitingTooLong, NotApproved, Failed }`.
- `MachineView::awake: Option<AwakeReport>` with
  `#[serde(default, skip_serializing_if = "Option::is_none")]`.

An old app never sends the event and never reads the field. An old hub receiving
`MachineEvent::Awake` from a new app: same situation as when `Background` was added; the hub
deploys before apps update (hub-deploy on every push), so in practice the hub is newer. After the
wire PR: `cargo run -p pinned-server --bin wire-contract -- snapshot current` and `-- check`
(exit 0). No existing snapshot is edited.

---

## 6. Agents read and set it like the user

`crates/core/src/surface.rs` `ACTIONS` must list every new `Call`
(`every_wire_call_has_one_action` fails otherwise, `surface.rs:998`). Proposed:

```rust
n("keep_awake_settings", Read, "Whether this machine keeps awake with the lid closed while agents run, its battery floor, battery time cap and heat guard, and what it is doing now: holding, lid closed, battery, and why it last stopped.").wired(),
n("set_keep_awake_settings", Confirm, "Turns keeping awake with the lid closed on or off on the machine, or changes its battery floor, battery time cap or heat guard. The user approves it in the app; on a Mac the first time also needs approval in System Settings.").wired(),
```

- Per machine: `act` already runs a `Call` "on the session's machine or the one it names"
  (`Action::on`), so "keep my MacBook awake while you work" from the Orchestrator on another
  machine works with `machine: "macbook"`.
- **Why `Confirm`, not `Write`:** it changes how hot and how drained a closed laptop gets, and on a
  Mac it starts a root helper. Today's comparable switch, `set_background`, is `Excluded` ("the
  user's own setting"). Felipe asked that agents can set it, so one tap in the app is the middle
  ground. Turning it **off** could be `Write`; one class per action keeps it simple, and the
  confirmation for "off" is a single tap. If Felipe wants "off" without asking, split into
  `set_keep_awake_settings` (Confirm) and a `stop_keeping_awake` (Write) later.
- An agent can never approve the macOS System Settings prompt; if approval is missing, the answer
  says "needs approval on the Mac" (`StopReason::NotApproved`), and the app shows the button.
- The plugin's `actions` list picks it up from `ACTIONS`; no MCP change.

---

## 7. Plan: small PRs, proof, model tier, manual tests

No test runs on Felipe's MacBook except the manual list at the end. Rust tests run on the existing
CI runners (`macos.yml` macos-latest, `windows.yml` windows-latest, `linux.yml`) and on an Upstash
Box (Linux) for core, wire and hub. GitHub's macOS and Windows runners are throwaway VMs with
passwordless admin, so the real power-setting round trips run there, never on a person's machine.

| # | PR | What | Proof | Model |
|---|---|---|---|---|
| 1 | `awake: busy rule and lid policy` | core, pure: `awake::busy()` (3.2), the hold state machine (grace 2 min, battery floor, 8 h cap, 30 min waiting cap, heat inputs), `awake::Settings` saved in the config dir. No platform code. | Unit tests with fake tabs/tasks and a fake clock: every row of 3.2, grace, floor, cap, heat. Runs on all three CI OSes and a Box. | Sonnet 5.5 |
| 2 | `wire: keep-awake settings and report` | 5.3 wire types, `surface.rs` entries (6), node answers `Call`s via `AppWindow` (refuses without the app), hub keeps `MachineView::awake` across offline. | `wire.rs` round-trip tests; `surface.rs` suite; hub test modeled on `a_machine_reports_run_in_the_background_on_its_view_while_it_is_online` (`hub.rs:1500`) plus "kept after offline"; `wire-contract check` exit 0, `snapshot current` committed. | Sonnet 5.5 |
| 3 | `macos: pinned-lid-helper` | The root daemon (2.1): socket + peer uid, lease/TTL, journal, restore at start and on SIGTERM, 24 h cap, its own battery/thermal check. Power-setting writes behind a trait. | Unit tests with a fake power backend (no root): lease ends on socket close, on TTL, restores at start from journal, refuses wrong uid, keeps a pre-existing `disablesleep 1`. **CI-only** job on macos-latest: run the helper under `sudo` with the real backend, hold, `pmset -g` shows `SleepDisabled 1`, kill the client, shows 0 within 2 s; `kill -9` the helper, restart it, journal restores. | Opus 5.5 (root code, security) |
| 3b | optional: restore without the bundle | the system-binary one-shot of 2.1, only if SMAppService allows it cleanly. | CI macOS: delete the helper binary mid-hold, the one-shot restores. | Opus 5.5 |
| 4 | `macos: lid switch in General` | `SMAppService` register/approve/unregister, `AppWindow` methods, lease client, App Nap activity, battery/thermal/lid readers, the switch and its lines in `gui-features.js`, the app notice after a stop. macOS 12 and ad-hoc builds show it disabled with the reason. | Rust unit tests (state → UI line); a `ui/*.test.js` like `settings-page.test.js` for every line of 5.2; CI macOS build of the bundle checks `Contents/Library/LaunchDaemons/*.plist` and the helper's signature. | Sonnet 5.5 |
| 5 | `windows: lid action hold` | 2.2: read/write `LIDACTION` AC+DC, journal, HKCU Run restore, `--lid-guard`, access-denied message. | **CI-only** on windows-latest: hold → `powercfg /q SCHEME_CURRENT SUB_BUTTONS LIDACTION` shows 0; kill pinned's test process → guard restores; run `--restore-lid` with a journal → restored and Run key gone. | Sonnet 5.5 |
| 6 | `linux: handle-lid-switch` | add `handle-lid-switch` to the inhibitor while holding (2.3). | Extend `the_hold_takes_an_inhibitor_and_leaves_nothing_running_or_says_why_not` (`awake.rs`); a runner with no logind must still name the tool in its error. | Haiku 5.5 |
| 7 | `hub/web: show the awake state` | Machines page and machine row lines of 5.2. | `ui/machine-load.test.js`-style tests per line; one Playwright web-sweep on a Box against a local hub with a fake machine reporting each state. | Sonnet 5.5 |
| 8 | `docs + changelog` | Settings help, "turn it off before deleting pinned", CHANGELOG 0.18.2. | Review. | Haiku 5.5 |

Order: 1 → 2 → (3, 5, 6 in parallel) → 4 → 7 → 8. Release once 1–4 and 7 are in; 5 and 6 can
ship labeled beta.

### Manual test list for Felipe (MacBook, signed build, on battery and on charger)

1. Turn **Keep working with the lid closed** on. System Settings opens to Login Items; approve.
   The switch says On. `pmset -g | grep SleepDisabled` shows **0** (nothing runs yet).
2. Start a worker on a long task. Within 10 s `pmset -g` shows `SleepDisabled 1`, the hub's
   Machines page says "Kept awake · 1 agent running".
3. Close the lid, on charger. From the phone, the hub shows "Awake with the lid closed"; send the
   Orchestrator a message; it answers. Wait 10 min; the worker keeps reporting.
4. Same on battery. Lid closed, 10 min; worker still reports; the battery % on the hub moves.
5. Let the worker finish with the lid closed. ~2 min later the Mac sleeps (the hub shows it
   offline). Open the lid: `SleepDisabled 0`.
6. Set the floor to a value just under the current battery %, close the lid with a worker running.
   When it crosses, the hub shows "Went to sleep at …: battery at N%"; the Mac is asleep.
7. With a worker running, `kill -9` pinned. Within a few seconds `pmset -g` shows
   `SleepDisabled 0`.
8. Turn the switch off. pinned leaves System Settings → Login Items, `SleepDisabled 0`, and a lid
   close with a worker running sleeps the Mac as before.

(Heat cannot be tested safely by hand; it is covered by unit tests with a fake thermal state.)

---

## Open questions for Felipe

1. 20% floor and an 8 h battery cap: OK, or no cap?
2. Should a session waiting on your approval keep the laptop awake for 30 min (proposed), or not
   at all?
3. Is the Developer ID signing on for releases? If not, the Mac switch needs it (2.1).
4. Agents turning it **off** without asking: want that as a separate `Write` action?
