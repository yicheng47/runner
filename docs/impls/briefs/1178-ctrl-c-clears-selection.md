# #1178 — Ctrl+C clears the selection it copies

Jason requested a claude duo for [#1178](https://github.com/yicheng47/runner/issues/1178) on 2026-11-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1178-ctrl-c-clears-selection` on `fix/1178-ctrl-c-clears-selection`, based on `origin/main` at `c5247a64`. The root checkout stays on `main`.

## Problem

On Windows and Linux, Ctrl+C in a terminal copies when text is selected and otherwise reaches the terminal as an interrupt. `copy_selection` writes the selection to the clipboard and leaves it in place, so the next Ctrl+C copies the same text again instead of interrupting. The selection only clears when the user types into the terminal or clicks, so a running command cannot be stopped with Ctrl+C right after a copy.

## Outcome

On Windows and Linux, a Ctrl+C that copies a selection also clears it, as Windows Terminal does, so the next Ctrl+C reaches the terminal as `\x03`. This holds in every terminal surface: chat panes and terminal tabs, mission slots, the mission terminal drawer and remote terminals. macOS is unchanged: ⌘C copies and the selection stays.

## Read first

- `crates/runner-app/src/terminal/input.rs`: `copy_selection`, shared by every caller below.
- Callers: `surfaces/chat/attachment.rs` `on_terminal_copy`, `surfaces/mission_workspace/input.rs` `on_mission_terminal_copy` (slots and the drawer), `surfaces/remote_client/workspace/render.rs` (the remote terminal's `Copy` handler), and `surfaces/agent_update.rs` `on_terminal_copy`.
- `crates/runner-terminal/src/terminal.rs`: `clear_selection`, `selection_text`, and `write_user_bytes`, which already clears the selection on typing.
- `surfaces/remote_client/tests.rs`: the remote copy-then-interrupt test, which clears the selection by hand before its second Ctrl+C.

## Deliverables and boundaries

- Clear the copied selection on Windows and Linux in one place, so every caller gets it. Keep the macOS behaviour as it is.
- Tests: on Windows and Linux, a second Ctrl+C after a copy sends `\x03` without clearing the selection by hand, for a local terminal and a remote one; on macOS, ⌘C keeps the selection. Update the remote test rather than adding a parallel one where it already covers the flow.
- Guide: `docs/guide/reference/keyboard-shortcuts.md` and its zh-CN copy already say Ctrl+C copies when text is selected; add that copying clears the selection on Windows and Linux.
- Non-goals: the Windows/Linux shortcut remap (#1177), Ctrl+Shift+C, any change to paste or to the copy-or-interrupt rule itself.

## Validation

Name the changed behaviour and the check that covers it before running anything. Run the `runner-app` tests that cover terminal copy, workspace Clippy with the `ci` profile and `-D warnings`, `cargo fmt --all --check` and `git diff --check`. The Windows-only path is exercised by the Windows CI job; drive macOS and Windows CI green.

Live checks: the behaviour changes only on Windows and Linux, so there is no macOS live check and no `qa` slot. The reviewer's clean review names the Windows live check; the coder labels the PR `needs-live-test` and lists that check in the description for Jason.

## Authorization

Jason approved a claude duo (one coder, one reviewer). The coder may commit, push this branch and open one PR against `main` once the review is clean.
