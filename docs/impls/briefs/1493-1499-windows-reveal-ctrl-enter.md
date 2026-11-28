# #1493 + #1499 — Windows Show in Explorer and Ctrl+Enter

Two small, unrelated Windows bugs fixed in one mission and one pull request, with one commit per issue.

## Starting point

- Issues: [#1493](https://github.com/yicheng47/runner/issues/1493) and [#1499](https://github.com/yicheng47/runner/issues/1499). Read both in full, including the reporters' cause analysis.
- Branch `fix/1493-1499-windows-reveal-ctrl-enter`, worktree `.worktrees/fix-1493-1499-windows-reveal-ctrl-enter`, cut from `origin/main` at `e30922d6`.
- In flight elsewhere, not on `main`; do not assume they landed and do not edit their checkouts: a neon duo on #1365 in `.worktrees/fix-1365-windows-codex-first-turn` (daemon session manager), PR #1509 `feat/1508-libghostty-vt` (0.21's main feature, rewrites `crates/runner-terminal/src/mappings.rs`), `.worktrees/feat-1489-kitty-graphics-protocol` and `.worktrees/preview`.
- Scratch files go under `/tmp/1493-1499-*` only.

## 1. #1493 — Show in Explorer opens Documents

Choosing **Show in Explorer** opens a new Explorer window in Documents instead of selecting the file, at least for paths with spaces or non-ASCII characters. Both Windows reveal sites spawn `explorer.exe` with `/select,<path>` as one `Command::arg`, which Rust quotes as a whole (`"/select,C:\a b\c"`); Explorer does not parse that and falls back to Documents.

- `crates/runner-app/src/surfaces/files/open_file/open_in.rs` — `reveal()` and its caller `reveal_active()` (Files viewer toolbar).
- `crates/runner-app/src/surfaces/mission_workspace/rail.rs` — `reveal_mission_cwd()` (mission working directory).

Fix: on Windows, reveal through GPUI's `cx.reveal_path`, as Settings → Skills already does (`surfaces/settings/skills.rs`); on Windows it calls `SHOpenFolderAndSelectItems` (`open_target_in_explorer` in `gpui-pre-windows` 0.3.14 `src/platform.rs`), which handles spaces and Unicode and selects a directory inside its parent. If a call site cannot use it cleanly, the fallback is `std::os::windows::process::CommandExt::raw_arg` with `/select,"<path>"` built as an `OsString`. macOS and Linux behaviour stays exactly as it is.

## 2. #1499 — Ctrl+Enter does nothing in terminal sessions

In a busy Claude Code session on Windows, Ctrl+Enter should send the queued message at once, as it does in Windows Terminal; in Runner nothing reaches the PTY. It fails the same way when a macOS client controls a Windows host.

Cause: `encode_key` in `crates/runner-terminal/src/mappings.rs` returns `None` for Ctrl+Enter on both paths (the legacy Ctrl table has no `enter`; `encode_kitty_key` excludes Ctrl from `modified_named`), so `handle_key` in `crates/runner-app/src/terminal/input.rs` leaves the key unclaimed. The decline was deliberate since #758 and is pinned by `kitty_disambiguate_does_not_claim_chords_the_legacy_path_declines`.

Settle with evidence before coding, and record the answer in the commit message and PR:

- Which bytes Ctrl+Enter sends with Kitty disambiguate on (the protocol's `CSI 13;5u` is the expected answer) and in legacy mode (the reporter proposes `\n`, which is what conhost and Windows Terminal send; xterm sends `\r`).
- Whether the new encoding applies on every platform or only Windows. The encoder is shared, so a cross-platform change also changes macOS and Linux shells and agents; justify the choice. Ghostty's own key encoder is a useful reference: read it from `origin/feat/1508-libghostty-vt` with `git show` (do not check that branch out).
- What actually reaches Claude Code on Windows through ConPTY, which turns input VT into console key records. If the only correct fix needs win32-input-mode (`CSI ? 9001 h`) or anything beyond the key encoder, stop and report to the operator instead of widening the scope.
- That claiming the key does not steal Runner's own Ctrl+Enter bindings where a terminal can have focus (on Windows `cmd-enter` defaults become `ctrl-enter`, see `keymap.rs`; the Start chat binding is scoped to `StartChat`).
- That the remote path picks the fix up (the reporter says remote control uses the same client encoder); confirm, do not assume.

Scope is Ctrl+Enter (and Ctrl+Shift/Ctrl+Alt+Enter only where the same rule covers them for free). Do not change Ctrl+Tab, Ctrl+Escape or other declined chords. Update the #758 test to the new contract and add encoder tests for each mode.

The PR notes that it conflicts with #1509's rewrite of `mappings.rs` and says where the same change belongs on that branch.

## Validation

- `cargo test -p runner-terminal`, `cargo test -p runner-app`, `make clippy`, `make fmt`. Skip nextest locally. A workspace-wide run waits until the #1365 mission is not running one.
- CI green on both platforms.
- Coder and reviewer do not run the app. The reviewer's clean review names the live checks. Expected: on Windows, Show in Explorer from the Files viewer and from the mission rail on a path with spaces and CJK characters selects the item; Ctrl+Enter in a busy Claude Code session sends the queued message, locally and from a macOS remote client. If the encoding changes outside Windows, also a macOS check that Ctrl+Enter in a shell and in a Claude Code chat behaves as the commit message says. Windows checks are left to Jason: the PR carries `needs-live-test` and lists them.

## Authorization

- Crew: neon duo (Jason, 2026-11-28).
- The crew may commit, push the branch and open one PR against `main` with `Fixes #1493` and `Fixes #1499`, one commit per issue, with this brief folded into them. It does not merge.
- Labels follow AGENTS.md: one risk label from the diff, `frontend-only` or not by what changes, `needs-live-test`, and `reviewed` from the reviewer on the clean head.
