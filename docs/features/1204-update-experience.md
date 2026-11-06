# 1204 — Minimal update flow

> Tracking issue: [#1204](https://github.com/yicheng47/runner/issues/1204). Scheduling and priority live on the issue and in the [roadmap](../roadmap.md). Platforms: macOS, Windows and Linux.
> Status: design signed off by Jason on 2026-11-06 in `runner/desktop/specs/1204-update-experience.pen` in `yeelinks_design`. Jason weighed writing Runner's own Mac updater on 2026-11-06 and kept Sparkle, since the extra risk wasn't worth it for this redesign, and chose to keep today's Mac update flow. Later that day Jason cut the popover down to the Ready state, so clicking the icon starts the download. Failed and Manual download only open Settings → Updates, and Jason decided the same day that a click does nothing during Downloading, Verifying and Installing. Implemented; the update-install checks below are pending nightly builds.

## Motivation

On macOS, Runner still shows Sparkle's stock AppKit windows, because `SPUStandardUpdaterController` runs with no custom user driver. They look dated, don't match the app, and give no warning when agents are working, even though a restart can interrupt them. Windows and Linux use Runner's own centered dialog, so the update flow differs by platform.

Runner should treat an update as something that can wait. People leave agents running for long stretches, so the update shouldn't cover their terminals or ask for attention until they choose to act.

## Scope

### Sidebar icon

The update icon stays beside the Runner name, in the 24 px button that exists today, and appears only when there is something to show. It stays hidden while a window shows a remote computer, as it does now.

| State | Icon | Tooltip |
| --- | --- | --- |
| Downloading | Accent ring on a muted track, filling clockwise from the top, with no glyph | Downloading Runner 0.19.5 · 45% |
| Verifying | Short accent arc on the track, rotating | Verifying Runner 0.19.5 |
| Ready | Green `rotate-cw` restart arrow, matching the Restart to update button | Runner 0.19.5 is ready to install |
| Available, not downloading | Green `circle-arrow-down`, as today | Download Runner 0.19.5 · 40.4 MB |
| Manual download only | Green `circle-arrow-down`, as today | Runner 0.19.5 is available |
| Installing | Short accent arc on the track, rotating | Installing Runner 0.19.5 |
| Failed | Amber `triangle-alert` | Runner 0.19.5 couldn't be installed |

The ring occupies the same 14 px box as the icons, so nothing shifts between states. A download of unknown size uses the rotating arc.

**macOS keeps today's flow:** Runner checks in the background and shows the icon, and nothing downloads until the person clicks the icon, or **Download** in Settings → Updates. Windows and Linux keep their **Automatically download updates** setting, on by default, so their ring can appear without a click.

### Clicking the icon

| State | Clicking the icon |
| --- | --- |
| Available, not downloading | Starts the download. The icon changes at once, to the rotating arc until the size is known and then the ring. |
| Downloading | Nothing |
| Verifying | Nothing |
| Ready | Opens the popover |
| Installing | Nothing |
| Failed | Opens Settings → Updates |
| Manual download only | Opens Settings → Updates |

A download needs no confirmation, because nothing installs until Restart to update and a restart is the step that can interrupt agents. On Windows and Linux, a click during a check shows the arc at once and the download starts if the check succeeds, since the check can replace the update it would download; a failed check reports its error instead, and Cancel holds off an automatic download for that check. During Downloading, Verifying and Installing a click does nothing and the tooltip has the progress, so a second click leaves the person in their terminal, and a double-click at Available starts one download. The icon looks the same as in any other state and isn't drawn as disabled. Cancel is in Settings → Updates, reached from Settings or the command palette's **Show Runner update**. Only Ready has a popover, so there is no Downloading, Verifying, Available, Manual, Installing or Failed popover.

### Ready popover

The 264 px popover opens anchored below the icon, using the app's existing popover behavior: a click outside or Escape closes it, and focus returns to where it was. It shows "Runner 0.19.5 is ready", "N agents working" with an amber dot when any are, and Restart to update, What's new and Later. Tab moves through its actions without leaving it, because a terminal keeps Tab for itself.

When a download the person started finishes, the popover opens by itself in the window where they clicked, without taking keyboard focus, so keys keep going to the terminal. An automatic download on Windows or Linux only changes the icon. Nothing else opens the popover by itself.

From the keyboard, the command palette's **Show Runner update** opens the popover at Ready, and Settings → Updates in every other state, including when there is no icon to anchor to or the update has gone.

Clicking the Ready icon doesn't restart directly, because a restart can interrupt running agents. **Later** only closes the popover; the icon stays until the update is installed. The working count uses the same sessions as the Windows dialog's note today (`working_agent_count`), which counts direct and mission agents and excludes terminals.

**What's new** opens the release page in the browser, and appears only in this popover. Runner doesn't fetch or render release notes, which also avoids the production appcast being generated before the notes are written.

### Settings → Updates

The page stays the home for manual checks and settings, shows the same state as the icon, and is where every state but Ready lands from the icon. Its hero runs the action directly instead of opening a dialog: Check for updates, Download and Skip this version, Cancel with a progress bar, Restart to update with the working count, or Retry. A failed state shows its error and View downloads, and Manual download only says why and offers View downloads.

**Skip this version** hides the icon for that version until the next manual check. All three platforms use one persisted skipped version in Runner's settings.

macOS keeps its current settings, **Automatically check for updates** and **Last checked**, and doesn't gain Automatically download updates. Sparkle is removed from the subtitles and from user-facing copy.

### Check for Updates menu item

**Runner → Check for Updates** opens Settings → Updates and starts the check, so the hero reports the result, including "You're on the latest version". There is no separate up-to-date surface, since the icon is hidden when no update exists.

### Settings → About

The Diagnostics page holds a single row, so it leaves the Settings nav. Its **Application logs** row moves to About, in its own card under the version, because a bug report needs both. The row behaves as it does today, including the installer log path on Windows and Linux and the error when the folder can't be revealed. A saved `diagnostics` pane opens About, and in-app text that names Settings → Diagnostics, such as the Windows installer failure, names About instead.

### Removed

- The Diagnostics item in the Settings nav and its page.
- Sparkle's update, progress, release-notes and up-to-date windows on macOS.
- The centered update dialog on Windows and Linux (`surfaces/update_dialog.rs`, `surfaces/update_dialog_linux.rs`) and `OpenUpdateDialog`, once the icon, popover and hero replace their logic. Their tested logic, such as the Windows installer arguments, moves rather than being lost.

### Installing with no installer window

Restart to update closes Runner and reopens it on the new version, with no installer window on any platform. Windows changes its installer arguments from `/SILENT` to `/VERYSILENT /SUPPRESSMSGBOXES`, since `/SILENT` still shows Inno Setup's progress window. A failed install then shows at the next launch's check, through the attempt record Runner already keeps, instead of in a window. Linux already shows no installer window: tar installs swap in place, and deb installs show only the system's authorization prompt. On macOS, Restart to update closes the popover, and the icon and hero show Installing until Runner quits.

Jason saw the Windows installer window appear from the sidebar icon but not the same way from Settings → Updates. Both entry points reach the same install code today, so the cause is unconfirmed. The popover and the hero share one install action, and a Windows install check below compares the two.

### Unchanged

Each platform keeps its engine, feed and signature checks: Sparkle and its appcasts on macOS, the minisign-verified installer on Windows, and the signed packages on Linux. Update quits still skip the quit dialog and leave `runnerd` to the new build's controlled replacement. Linux deb installs still request system authorization.

## Implementation Phases

### 1. Sparkle driver

Replace `SPUStandardUpdaterController` with `SPUUpdater` and a Runner `SPUUserDriver`, created through `initWithHostBundle:applicationBundle:userDriver:delegate:`. The flow stays as it is today; only the UI changes.

- **Background checks.** Keep the `should_proceed` decline for background checks, so they only record the update and show the icon. Leave `SUAllowsAutomaticUpdates` and `SUAutomaticallyUpdate` off.
- **Download.** Clicking the Available icon, or Download in Settings → Updates, starts a user-initiated check, and the driver replies Install to `showUpdateFoundWithAppcastItem:state:reply:` without asking again. Download callbacks drive the ring, extraction drives Verifying, and Cancel calls the download's cancellation block.
- **Ready.** The driver holds the `showReadyToInstallAndRelaunch:` reply until Restart to update, which replies Install. Later only closes the popover.
- **Quitting with an update ready.** Sparkle may still install the update on quit, as it can today. Jason chose on 2026-11-06 to keep this behavior, with no added warning.
- **Restart.** Confirm `mark_update_quit` still runs through `updaterWillRelaunchApplication:` and the quit dialog stays away. Handle `retryTerminatingApplication` if a window delays the quit.
- **Other replies.** Map `informationOnlyUpdate` to Manual download only, and call every acknowledgement and reply block exactly once.

### 2. Shared update state

Keep `updater/windows.rs` and `updater/linux.rs` where they are, and have each report into a small shared state; don't merge them into one engine. Give the three updaters one view of the update for the UI: state, version, received and total bytes, working count inputs and the release URL. macOS moves from `available: Option<UpdateInfo>` to the state Windows and Linux already have (`Available`, `Downloading`, `Ready`, `Failed`), plus the Verifying and Installing states the design adds. Keep the current nightly and preview labels from `UpdateInfo`.

Extend the debug-only `RUNNER_DEV_UPDATE_AVAILABLE` preview so a development build can show each icon and hero state, and the Ready popover, on every platform, without staging or installing anything. Release builds keep ignoring it. The live checks below depend on this.

### 3. Icon, popover and Settings

Build the ring element, the popover and the hero states from the design file, in the shared app shell so all three platforms use the same code. The ring is drawn with GPUI's `PathBuilder` and `arc_to`, as `terminal/glyphs.rs` already does, and must stay sharp at every app zoom. `working_agent_count` is compiled only for Windows, Linux and tests today, so it becomes available on macOS. Pause the rotating arc when the window isn't visible, so an update never adds idle CPU. Move every string to `i18n/en.toml` and `i18n/zh-hans.toml`.

### 4. Remove the old surfaces and update docs

Remove the dialogs and the `SPUStandardUpdaterController` use, move Application logs to About and remove the Diagnostics pane, and update tests that select them. Update these docs in the same pull request:

- Guide: `docs/guide/getting-started/install.md`, `docs/guide/guides/troubleshooting.md` and `docs/guide/concepts/sessions.md`, with their `zh-CN` pages.
- Architecture: `docs/arch/arch.md` (the updater row, Updates, and the channel notes on Sparkle's native alert), `docs/arch/windows.md` (update behavior and the installer log's location) and `docs/arch/linux.md`.
- Regression suite: retire `SET-UPDATE-02`, `SET-UPDATE-04`, `SET-UPDATE-05` and `SET-LINUX-02` for new cases that describe the icon, the Ready popover and the hero, and `SET-AGENT-01` for a case without the Diagnostics pane, keeping their IDs under the suite's retirement rules.

## Verification

### Automated

- Each engine event maps to the right shared state, including cancel, failure, retry and a repeated found event. The existing `updater.rs` transition tests move to the new state.
- Icon, tooltip and click result for each state, with a known and an unknown download size, and the ring's fraction clamped to 0–1. One click or a double-click at Available starts one download, only Ready opens the popover, and a click during Downloading, Verifying or Installing changes nothing and leaves focus where it was.
- **Show Runner update** opens the popover at Ready and Settings → Updates in every other state.
- The working note appears only at Ready and counts direct and mission agents, carried over from the dialog tests.
- The Ready popover opens by itself only after a download the person started, in that window, and doesn't take keyboard focus.
- The popover fits a small window at the app's zoom sizes, and a long nightly version wraps inside it.
- A skipped version hides the icon until a manual check, and stays skipped across a restart on Windows and Linux.
- The Windows installer arguments test moves with its code and expects `/VERYSILENT /SUPPRESSMSGBOXES`.
- The Settings nav no longer lists Diagnostics, and a saved `diagnostics` pane opens About.

### Live checks

The change alters what people see and operate in the running app, so it needs live checks by the `qa` role. Each runs on macOS, and on Windows and Linux where a host is available; a platform gap blocks `needs-live-test` from coming off and is recorded in the pull request.

| Check | Reason |
| --- | --- |
| In a development preview, step through each state and compare the icon, the Ready popover and the hero with the design, in dark and light themes and at a large app zoom | The ring, popover and hero are new drawing, and a 14 px ring is where rendering problems show |
| Click the icon in each state and confirm it starts the download at Available, opens the popover at Ready, opens Settings → Updates at Failed and Manual download only, and does nothing during Downloading, Verifying and Installing, where a terminal keeps focus. Double-click it at Available and confirm one download starts and the terminal stays in front. No other popover appears anywhere | The icon's click is now the main control |
| Open the Ready popover from the keyboard and mouse, then close it with Escape and with a click outside, and confirm a terminal gets its focus back | The popover sits beside terminals, so losing focus breaks typing |
| At Ready with two test chats working, confirm the amber count, then let them finish and confirm it clears | The warning is the main safety improvement on macOS |
| Run Check for Updates from the menu and from Settings → Updates when up to date | The up-to-date result moved out of Sparkle's window |
| Confirm the icon is hidden while a window shows a remote computer, if a second host is available | The existing gating must survive the rebuild |
| Open Settings, confirm Diagnostics is gone from the nav, and use Application logs in About to reveal the log folder | The row moved, and it's how people get logs for bug reports |

### Update installs

These need real signed builds, so they run on nightly builds, not in development. Sparkle's first update to the new build still uses the old UI, so the Mac flow needs two nightlies containing this change. A crew doesn't cut a nightly: Jason decides when to merge and cut them, and stable waits until these pass. `needs-live-test` stays on the pull request with these listed as pending, until they've passed or Jason decides otherwise.

| Check | Reason |
| --- | --- |
| macOS: click the icon, watch the ring, reach Ready, choose Restart to update, and relaunch on the next nightly with owned sessions resumed | Sparkle's whole UI is replaced, and this is the path every Mac user takes |
| macOS: confirm nothing downloads until the icon or Download is clicked | Today's flow must not change |
| macOS: cancel, failed download and retry | These paths move from Sparkle's window to Runner's driver |
| Windows and Linux: download, cancel, retry, signature failure, and Restart to update | The engines are unchanged, but their UI entry points are not |
| Windows: Restart to update from the popover and from Settings → Updates, confirming no installer window from either and a relaunch on the new version | Jason reported a difference between the two entry points, and the installer arguments change |
| Windows: a failed install, confirming the old version reopens and the failure shows at the next check | With `/VERYSILENT`, no window reports the failure |

## Risks

- **Driving Sparkle.** Runner now answers Sparkle's prompts, so a missed reply could leave an update stuck until someone reinstalls by hand. Replies are unit-tested, and the nightly checks prove the path.
- **Install on quit.** Quitting with an update ready may still install it, and agents left running restart at the next launch, with no warning, as they can today. #1136 is the lasting fix.
- **A silent Windows install.** With no window, a few seconds pass with nothing on screen between Runner closing and reopening, and a failure only shows at the next launch.
- **Testing the real flow.** Sparkle only runs in bundled, signed builds, so the development preview can't prove installs. The nightly checks above cover that gap.
- **A stray click.** Clicking the Available icon starts a download with no confirmation, and Cancel and Skip this version live in Settings → Updates. Jason accepted this on 2026-11-06, since nothing installs until Restart to update.
- **Discoverability.** A small icon is easier to miss than a window. That is deliberate, and the Later state keeps the icon up until the update is installed.
- **Ring rendering.** A 14 px ring needs custom drawing that stays sharp at every zoom, and the live checks look at it directly.
