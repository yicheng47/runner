# #1204 — Minimal update flow

Jason approved this mission on 2026-11-06. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1204-update-experience`, on `feat/1204-update-experience`, cut from `origin/main`.

## Problem and outcome

macOS still shows Sparkle's stock windows, and Windows and Linux use a centered dialog. Replace both with one minimal flow on all three platforms: a sidebar icon with a progress ring, a small popover at Ready only, and Settings → Updates showing the same state. Clicking the icon starts the download, Ready opens the popover, Failed and Manual download only open Settings → Updates, and a click does nothing during Downloading, Verifying and Installing. Application logs also moves from the Diagnostics page to Settings → About. The Mac update flow stays as it is today: nothing downloads until the person clicks the icon or Download.

The spec is the source of truth: [`docs/features/1204-update-experience.md`](../../features/1204-update-experience.md). The signed-off design is `runner/desktop/specs/1204-update-experience.pen` in `yeelinks_design`. Pull that repo first and open it through the Pencil tools only:

- Icon and popover: `Pmvhy` (downloading), `GSafE` (ready, popover open), `hp3KW` (every state, zoomed and at actual size, with what clicking does).
- Settings: `tB5DV` and `M1bap` (Updates), `P7Wym` (About).

The spec wins on behavior, and the design wins on appearance.

## Read first

- `crates/runner-app/src/updater.rs`, `updater/windows.rs`, `updater/linux.rs`, `updater/linux/install.rs`
- `crates/runner-app/src/surfaces/update_dialog.rs`, `update_dialog_linux.rs`, `surfaces/settings/updates*.rs`, `diagnostics.rs`, `about.rs`
- `crates/runner-app/src/surfaces/app_shell/sidebar.rs` and `render_sidebar_brand`
- `crates/runner-app/src/terminal/glyphs.rs` for `PathBuilder` and `arc_to`
- Sparkle 2.9.5's `SPUUserDriver.h`, `SPUUpdaterDelegate.h` and `SPUUpdater.h`; the pinned archive is in `script/bundle-mac`
- `script/windows/runner.iss`, and `docs/arch/windows.md` under Update behavior

## Deliverables

Work through the spec's four phases. In short:

1. **Sparkle:** a Runner `SPUUserDriver` on `SPUUpdater`, with no Sparkle windows and today's flow unchanged. No automatic downloads on Mac and no Automatically download updates setting there. Do not write a Mac updater or try to stop install on quit.
2. **Shared state:** one update state for the UI on all platforms, including Verifying and Installing, with the debug `RUNNER_DEV_UPDATE_AVAILABLE` preview able to show every state.
3. **UI:** ring, popover and Settings hero in shared code, strings in both i18n files, and `/VERYSILENT /SUPPRESSMSGBOXES` for the Windows installer.
4. **Removal and docs:** the old dialogs, the Diagnostics page and nav item, plus the guide, architecture and regression-suite updates the spec lists, in English and `zh-CN`.

Non-goals: merging the Windows and Linux updaters into a shared engine (keep their changes small), rendering release notes in the app, a warning about install on quit, a Help menu, Homebrew changes, keeping agents running through updates (#1136), and a new update engine on any platform.

## Validation and handoffs

The coder runs the automated checks the spec lists and the CI commands in `AGENTS.md`, including `--features updater` on macOS, and drives CI green on macOS, Windows and Linux. The coder and reviewer do not run the app.

The reviewer reviews against the spec and pays particular attention to: Sparkle reply and acknowledgement blocks called exactly once, main-thread use in the `objc2` bridge, signature checks staying in place on every platform, and quit marking through `mark_update_quit`. Its clean review names the live checks from the spec.

After the clean review, the coder hands the candidate to `qa`, which runs the development-preview checks. Install checks run on nightly builds as the spec describes: the crew doesn't cut a nightly, and `needs-live-test` stays on with those checks listed in the PR as pending. Never touch the installed Runner.

## Authorization

Jason approved the `claude trio`: a Claude coder, the codex `reviewer` and the codex `qa`. The crew may commit, push this branch and open one PR against `main`, labelled `frontend-only` if the diff stays in `runner-app`, `risk-high` and `needs-live-test`.
