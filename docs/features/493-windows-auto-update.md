# Automatic updates on Windows

Tracking issue: [#493](https://github.com/yicheng47/runner/issues/493). Status: planned. Priority P2.

## Motivation

Runner 0.8.0 is the first stable release with a Windows build. macOS already has an integrated Sparkle updater. Windows checks for newer releases and shows a download icon, but the user still has to open GitHub, download an EXE, close Runner, and run the installer. Bring Windows updates into the app.

## Scope

- Extend the existing Windows updater and per-user installer with in-app download, progress, verified staging, and an **Install and restart** action.
- Retain automatic/manual update checks and add a preference for automatic downloads. Keep a notify-only path when automatic downloads are disabled.
- Respect the packaged release channel: stable releases for production, `nightly-win` for nightlies. Do not switch channels implicitly.
- Install only when the user chooses to restart; never interrupt agents or shell processes with an automatic restart. Route the restart through the running-work confirmation planned in #491.
- Verify update authenticity before running a downloaded installer. Choose and document the signing/trust mechanism with the Windows signing work tracked in #437; a failed verification must leave the installed app usable.
- Preserve settings, chats, missions, and the existing per-user installation path. Relaunch the updated app after installation and surface actionable download/install failures with retry and manual-download options.

macOS keeps Sparkle. Automatic CLI updates (#475), a session daemon (#466), additional CPU architectures, and a packaging-framework migration are outside this feature's scope.

## Implementation Phases

1. Choose the Windows update verification and installer handoff mechanism; design the update states in a feature-scoped Pencil file before UI implementation.
2. Add download, progress, verification, cancellation/retry, and staged-update state to the Windows updater.
3. Add the user-confirmed installer handoff, restart, data-preservation checks, and Settings controls.

## Verification

- A packaged Windows build downloads a newer build from its own channel and upgrades from inside Runner without visiting GitHub.
- Installation preserves settings, chats, missions, and CLI/MCP functionality; the relaunched app reports the new version.
- Busy agents and foreground terminal processes are never stopped by an unattended update. Cancelling installation leaves them running.
- Interrupted downloads, corrupt or untrusted payloads, and installer failures leave the current installation usable and expose retry/manual recovery.
- Disabling automatic downloads retains update notifications and manual download/install actions.
- Existing Windows installer tests, Windows CI, and macOS updater checks pass.
