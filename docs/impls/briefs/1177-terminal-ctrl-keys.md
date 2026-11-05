# #1177 — Ctrl keys in terminals on Windows and Linux

Jason requested a claude trio for [#1177](https://github.com/yicheng47/runner/issues/1177) on 2026-11-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1177-terminal-ctrl-keys` on `fix/1177-terminal-ctrl-keys`, based on `origin/main` at `b7be3b5a`. The root checkout stays on `main`.

## Problem and outcome

On Windows and Linux, Runner's Ctrl shortcuts take keys a focused terminal needs, so Ctrl+D, Ctrl+K, Ctrl+S and Ctrl+M never reach the shell or agent. The outcome: split, command palette and sidebar move to new Windows/Linux defaults, minimize loses its shortcut on Windows and Linux while macOS keeps ⌘M (Jason, 2026-11-05), a **Ctrl keys in terminals** setting (Runner first / Terminal first) lets terminals have the bare Ctrl keys, and Settings → Keyboard shortcuts is grouped into sections.

The spec is [`docs/features/1177-terminal-ctrl-keys.md`](../../features/1177-terminal-ctrl-keys.md); it is authoritative on behaviour and names the design, `runner/desktop/specs/1177-terminal-shortcuts.pen` in the design repo (frames `ss4SN` and `u47PjU`), which wins on visual detail.

## Read first

- `crates/runner-app/src/keymap.rs`: `windows_default`, `entries()`, `binding_context`, `install_bindings`.
- `crates/runner-app/src/surfaces/settings/page/shortcuts.rs`: `shortcut_rows` and `render_shortcuts_settings`.
- `crates/runner-app/src/app_settings.rs`: how `keymap_overrides` and `option_as_meta` persist.
- `crates/runner-app/src/startup.rs`: the Minimize action and the macOS Window menu.
- The guide pages and i18n files the spec lists under Affected docs.

## Deliverables and boundaries

Everything in the spec's Scope and Affected docs, in `runner-app` and the docs. Non-goals are the spec's Out of scope: no macOS shortcut changes, no Ctrl+Shift+C/V, no change to the Ctrl+C copy-or-interrupt rule.

## Validation and handoffs

The coder lists the changed behaviours and the check covering each, runs the spec's in-process tests, `runner-app` tests, workspace Clippy with the `ci` profile and `-D warnings`, `cargo fmt --all --check` and `git diff --check`, and drives macOS, Windows and Linux CI green.

The reviewer reviews the working-tree diff before any commit. `qa` runs the spec's macOS live check on the reviewed candidate in the development app. The Windows live checks need a Windows host: qa records them as not run, the PR carries `needs-live-test` and lists them for Jason.

## Authorization

Jason approved a claude trio (coder, reviewer, qa). The coder may commit, push this branch and open one PR against `main` once the review is clean.
