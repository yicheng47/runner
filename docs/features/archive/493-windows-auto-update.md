# Automatic updates on Windows

Tracking issue: [#493](https://github.com/yicheng47/runner/issues/493), closed. Shipped in [Runner 0.8.2](https://github.com/yicheng47/runner/releases/tag/v0.8.2) on 2026-09-07 through [#499](https://github.com/yicheng47/runner/pull/499) and [#500](https://github.com/yicheng47/runner/pull/500); archived with the [implementation brief](../../impls/archive/493-windows-auto-update.md). Current behavior is documented in [Windows development](../../arch/windows.md#update-behavior). Two things changed after the first real run and are folded into the text below: the installer renames in-use binaries aside instead of refusing, and every installer launch of Runner goes through the shell to escape Redirection Guard.

## Motivation

Runner 0.8.0 is the first stable release with a Windows build. macOS has had in-app updates since 0.6.0 through Sparkle, and the Tauri-era 0.5.x builds had them through `tauri-plugin-updater`; the GPUI rewrite dropped that plugin with the runtime it depended on. Windows today checks GitHub at startup and every six hours (`crates/runner-app/src/updater/windows.rs`), shows a download icon beside Settings when a newer installer exists, and then hands the user to a browser: download the EXE, close Runner, run the installer, reopen Runner. Bring the whole loop into the app.

## Trust model

The app must not run a downloaded installer it cannot prove came from Runner's CI. The check is a detached signature, the same mechanism Sparkle uses on macOS and Tauri's updater used on 0.5.x, and it is independent of Windows code signing:

- **What is signed.** CI signs the installer bytes with the minisign key already in the repository's secrets (`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, key id `596D7429FAE1FE23`), using the same `@tauri-apps/cli signer sign` step `release.yml` already runs for the 0.6.0 bridge. The output is uploaded as a release asset next to the installer: `Runner-Setup-<version>-x64.exe.sig`, the base64-encoded minisign signature text that `signer sign` produces.
- **What verifies it.** The minisign public key that 0.5.2's `tauri.conf.json` trusted lives in the tree as `packaging/windows-update-public-key` and is compiled into `Runner.exe`. Verification uses the `minisign-verify` crate, the one `tauri-plugin-updater` uses: decode the `.sig` asset, decode the public key, verify the full installer file, and only then rename the download into place. Nothing is ever executed before verification succeeds.
- **Relation to #497.** Authenticode tells Windows and SmartScreen who published the binary; the minisign signature tells Runner that the bytes are the ones CI built. [#497](./497-windows-code-signing.md) is not a prerequisite. When it ships, the updater adds a second gate, a `WinVerifyTrust` check on the downloaded installer, and the spec records it there.
- **SmartScreen.** Files the app writes itself carry no Mark-of-the-Web, and `CreateProcess` does not consult SmartScreen, so an in-app install should not hit the **Windows protected your PC** interstitial that manual downloads hit today. Verification confirms this on a fresh PC rather than assuming it.
- **Failure is inert.** A release whose installer has no `.sig` is reported the way it is today, notify-only with the browser link. A signature that fails to verify deletes the download, shows a verification error with retry and manual-download actions, and never runs anything. The installed app is untouched in every failure path.

## Behavior

### Checking

Unchanged in schedule and channel: production builds read the latest stable release, nightlies read `nightly-win`, both selected by `RUNNER_RELEASE_CHANNEL` at build time, and the installer's `AppUpdatesURL` keeps pointing at the same channel. A build never switches channels; moving from nightly to stable is a manual production install, as today.

Checking is always on for packaged Windows builds and loses its toggle. A check is a single GitHub API request every six hours, and Runner wants people on the current build, so there is nothing to opt out of; what the user controls is the download. The `automatically_check_for_updates` setting stays in `AppSettings` for the macOS Sparkle pane and the Windows updater ignores it.

Asset selection changes in one way. The newest completed `Runner-Setup-<version>-x64.exe` is still the candidate, and it is **installable** only when a completed `<name>.sig` asset sits beside it. Without the sig the update is reported as available but not installable, which is exactly today's behavior and keeps 0.8.0 and 0.8.1 clients, unsigned local builds, and older releases working unchanged. The legacy `Runner-Nightly-*.zip` form stays recognised for the version comparison but is never installable in-app.

### Downloading

A new setting, **Automatically download updates** (`automatically_download_updates`, default on), is the one toggle on the Windows pane. When it is on and a check finds an installable update, the download starts in the background immediately, and the sidebar icon appears only once the download is verified and ready or has failed, so a download in progress is not a nag. When it is off, the check only notifies: the update dialog offers **Download** (in-app) and **View downloads** (browser), and nothing is fetched until the user asks.

The download streams through the existing blocking `reqwest` client on the background executor into `<app data>\updates\<installer name>.partial`, reporting received and total bytes (from `Content-Length`) to the updater entity a few times a second. Progress is shown only in the update dialog, as a bar with "18.4 MB of 41.2 MB · 45%" under it; the Settings row shows a one-line status and never a bar. **Cancel** in the dialog stops the stream and deletes the partial file. A failed download shows the error with **Retry** (restart from zero; range resumption is a non-goal) and **View downloads**. On success the file is verified as described above and renamed to its final name; the state becomes **Ready**.

At startup the updater sweeps `updates\`: partial files and installers for any stamp other than the one the next check reports are deleted, and a complete installer for the current candidate is re-verified and becomes Ready without a second download. After a successful install the relaunched app finds the staged installer older than itself and deletes it.

### The update dialog

One dialog is the whole update UI. It opens centered in the window over the same 60% scrim as every other confirm (`crates/runner-app/src/ui/overlay.rs`), from two triggers that behave identically: the existing sidebar icon beside Settings, and the **Update** button in the hero card of Settings → Updates. Click outside, Escape, or **Later** closes it without stopping anything; a download keeps running behind it. The Settings page itself never gains a progress bar or per-state buttons: the hero shows a one-line status, and its button reads **Check for updates** when up to date (spinning and disabled while a check runs) and **Update** in every other state.

The dialog is the 420 px card the confirms use: app icon, a title naming the version, a subtitle with the installed version and download size, an optional body line, an optional progress block, and one or two buttons. There are five user-facing states, and the updater enum mirrors them:

- **Up to date**: no dialog; the hero's button checks. Checking is that button spinning, not a state of its own.
- **Available**: "Runner <version> is available", **View downloads** and **Download**. When the release has no `.sig`, the same dialog explains that this release cannot be installed from Runner and offers only **View downloads**; that is today's behavior in a dialog, not a separate state.
- **Downloading**: progress bar with received/total and percent, **Cancel**. The signature check is the last moment of the bar, not a state.
- **Ready**: "Runner <version> is ready to install", body "Runner closes, the installer runs, and Runner reopens on the new version.", **Later** and **Install and restart**. No running-work warning: quitting stamps sessions for auto-resume exactly as ⌘Q does today, and [#491](./491-confirm-quit-running-work.md) was closed on 2026-09-07 in favor of a detached session host, after which nothing is interrupted at all.
- **Failed**: one state whose body names what failed (download, signature, or installer), with **View downloads** and **Retry**; after an installer failure the primary is **Install and restart** again.

There is no separate confirm dialog: **Install and restart** inside the Ready dialog is the one action that quits, and reaching it takes two deliberate clicks (open the dialog, press the button), the same as Sparkle's prompt on macOS.

The sidebar icon shows whenever the dialog has something to offer (available, ready, failed) and hides while a download runs. Its tooltip names the state: "Runner <version> is ready to install" or "Runner <version> is available".

### Install and restart

Pressing **Install and restart** performs the handoff in this order, and nothing installs unless the user reaches this point:

1. Spawn the verified installer detached, with no inherited handles so it survives Runner's exit and holds none of Runner's pipes: `Runner-Setup-<version>-x64.exe /SILENT /NORESTART /WAITPID=<Runner pid> /RELAUNCH=1 /LOG=<app data>\logs\update-<stamp>.log`. `/SILENT` keeps the progress window and any error message box visible; `/VERYSILENT` would hide a failure.
2. Run the normal quit path (`cx.quit()`): `on_app_quit` stamps running sessions for auto-resume and stops the PTYs exactly as ⌘Q or closing the last window does today. No new teardown.
3. The installer's `PrepareToInstall` waits up to 30 seconds for `/WAITPID` to exit, then renames any in-use `Runner.exe`, `runner-agent-cli.exe`, or `runner-mcp.exe` aside before installing their replacements. A plain `[Run]` entry gated on `/RELAUNCH=1` (no `postinstall`, no `skipifsilent`) starts the new `Runner.exe` and lets Setup exit.

An external process running `{app}\runner-mcp.exe` directly does not block the upgrade: Setup renames that binary to `runner-mcp.exe.old`, writes the new binary at its original path, and leaves the existing process running on the old image. If a previous `.old` is still running and cannot be deleted, Setup uses `.1.old`, `.2.old`, and so on. Every install and uninstall attempts to delete leftover `.old` files, ignoring failures for running images. A running `.old` does not block uninstall; it remains until a later install can remove it. Uninstall only renames canonical binaries after confirmation, so clicking **No** leaves the installation intact.

Genuine installation failures still leave the current installation usable. If file copying fails midway, Inno Setup aborts and rolls back the copy; Setup restores renamed binaries whose original paths are missing before the existing `DeinitializeSetup` relaunch check. With `/RELAUNCH=1`, the old `{app}\Runner.exe` reopens when it exists. On relaunch the updater finds the staged installer still newer than the running app, re-verifies it, and the dialog shows the Failed state with **Install and restart** again and the log path in Settings → Diagnostics.

### Settings → Updates

The Windows pane (`crates/runner-app/src/surfaces/settings/updates_windows.rs`) takes the macOS pane's shape so the two pages match. The header line reads "Runner checks for new Windows builds at startup and every six hours, downloads them in the background, and installs when you choose." Then:

- **Hero card**, as on macOS: the app icon, "Runner" with the installed version chip, a one-line status ("Runner 0.8.2 is ready to install.", "Downloading Runner 0.8.2…", "Update failed."), and the state's button on the right: **Check for updates** when up to date, **Update** otherwise, which opens the update dialog. No progress bar on the page.
- **Automatically download updates**: the one toggle, subtitle "Downloads are verified before Runner offers to install them. Turn off to be notified only."
- **Last checked**: unchanged.

The **Installed version**, **Update status**, and **Windows downloads** rows and the **Automatically check for updates** toggle are gone. The version and status live in the hero, the manual download link lives in the dialog's Available and Failed states, and checking is always on. What still differs from macOS is only what the platform dictates: macOS keeps the Sparkle check toggle and its button hands off to Sparkle's prompt; Windows has the download toggle and opens the dialog.

The updater states, which the dialog, the hero, and the sidebar icon render and the impl plan turns into an enum:

| State | Meaning |
|-------|---------|
| UpToDate | No update known, or up to date; carries whether a check is in flight. |
| Available | Newer installer found; carries whether it is installable. |
| Downloading | Bytes flowing; received and total, then the signature check. |
| Ready | Verified installer staged on disk. |
| Failed | Which step failed and a message; retry and manual download available. |

### Data preservation

Nothing new is needed: the installer only touches `%LOCALAPPDATA%\Programs\Runner`, never `%APPDATA%\com.wycstudios.runner`, and `script/windows/test-installer.ps1` already asserts that upgrades retain data. The verification list checks it end to end anyway, because the point of the feature is that a user never has to think about it.

## Non-goals

- macOS changes. Sparkle stays, with its own prompts and the existing macOS Updates pane. The sidebar icon is the only shared element, and it already dispatches per platform: Sparkle's prompt on macOS, this dialog on Windows.
- Authenticode signing. [#497](./497-windows-code-signing.md) owns it; this feature consumes it when available.
- Unattended installation or a scheduled restart. Install happens only when the user confirms, every time.
- Resumable or delta downloads. The installer is tens of megabytes; a restart from zero is fine.
- In-app channel switching. Nightly and stable stay separate installers.
- Portable ZIP updates, Windows ARM64, CLI self-updates ([#475](https://github.com/yicheng47/runner/issues/475)), and a session host ([#466](./466-sessions-outlive-the-app.md)).

## Design

Sparkle draws its own windows on macOS; on Windows the whole update UI is the centered dialog component `cmp/UpdateDialog` in `design/windows-updates.pen`. The canvas holds the dialog in its four visible states, the main window with the dialog opened from the sidebar icon, the Updates page with the dialog open while downloading, the Updates page in the Ready state, the hero's status and button per state, and the sidebar icon states; frame ids are listed in `design/README.md`. No progress bar, extra confirm, or per-state buttons appear on the Settings page.

## Implementation Phases

1. **Signed assets and installer handoff.** Sign the installer in both `release.yml` and `nightly.yml` right after `test-installer.ps1`, require the key the way the macOS job requires Sparkle's, upload the `.sig` with the installer, and widen the nightly pruning pattern so stale `.sig` files go with their installers. Add `/WAITPID`, `/RELAUNCH`, rename-aside for in-use binaries, and relaunch-on-failure to `script/windows/runner.iss`, with `test-installer.ps1` cases for waiting on a live pid, installing with running binaries, cleaning up old files, uninstalling with a running `.old`, and relaunching after success or a copy failure. Commit `packaging/windows-update-public-key`. Ships on its own with no app change; cut a nightly and verify the asset with the `minisign` CLI.
2. **Updater core.** The state enum, installer/sig pairing, streaming download with progress and cancel, `minisign-verify` verification, the startup sweep, retry, and the `automatically_download_updates` setting; unit tests against release JSON fixtures and a committed minisign test vector (file, signature, public key).
3. **UI and handoff.** The update dialog with its states and both triggers, the hero card with its Update button and status line in the Updates pane, the sidebar icon visibility and tooltips, the detached spawn plus quit, and the doc updates: the Update behavior section of `docs/arch/windows.md`, `README.md`, `script/windows/release-notes.md`, and the release notes string in `release.yml`.

## Verification

- [ ] A packaged nightly on `nightly-win` finds the next nightly, downloads it in the background, the sidebar icon appears, and **Install and restart** in the dialog upgrades it without a browser; the relaunched app reports the new version and no SmartScreen interstitial appears.
- [ ] The sidebar icon and the Update button in Settings open the same dialog in the same state; closing it mid-download leaves the download running.
- [ ] The same flow on a production build against a stable release.
- [ ] After the upgrade: settings, chats, missions, the MCP pipe, and both CLI sidecars work; `updates\` no longer holds the installer.
- [ ] With automatic downloads off, a check shows the icon and the dialog offers **Download** and **View downloads**; nothing is fetched until asked.
- [ ] Cancel in the dialog deletes the partial file and returns to Available; Retry after a simulated network failure downloads again.
- [ ] A tampered installer or `.sig` fails verification, is deleted, shows the error with Retry and View downloads, and the installed app keeps running.
- [ ] A release with no `.sig` is reported notify-only, exactly as 0.8.1 reports it today.
- [ ] One agent mid-turn: Later leaves it running and the window focused; Install and restart stamps it and it resumes after the relaunch with auto-resume on.
- [ ] An external process holding `%LOCALAPPDATA%\Programs\Runner\runner-mcp.exe` open: the install succeeds, the new binary is in place, `.old` holds the previous binary, and the external process keeps running. After it exits, the next install removes the stale `.old`. Use the installed executable directly; current mission shims run a separate copy under `%APPDATA%\com.wycstudios.runner\bin`.
- [ ] `test-installer.ps1` covers `/WAITPID`, `/RELAUNCH`, rename-aside with numbered suffixes, stale-file cleanup, uninstall with a running `.old`, cancellation at the uninstall confirmation, and restoration/relaunch after a copy failure; Windows CI, the macOS updater checks, and `make verify` pass.
