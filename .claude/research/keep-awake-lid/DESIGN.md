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
