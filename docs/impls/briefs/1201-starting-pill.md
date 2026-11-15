# Mission Brief: Keep Starting pill visible until Claude Code paints its first frame

**Tracking issue:** [#1201](https://github.com/yicheng47/runner/issues/1201)
**Branch:** `fix/1201-starting-pill`
**Worktree:** `.worktrees/fix-1201-starting-pill`
**Crew:** `agy trio` (`coder`, `reviewer`, `qa`)

## Problem

When starting a new Claude Code chat, the `"Starting Claude Code…"` (`SessionOverlayKind::Starting`) pill drops before Claude Code renders its TUI, leaving the pane completely black for roughly a second until the first frame appears.

While `PaneOverlayState::Starting | PaneOverlayState::Resuming` is active (`crates/runner-app/src/surfaces/panes/pane.rs:630-653`), the terminal element has `opacity(0.)` and the pill sits over the pane. As soon as `begin_chat_transition` (`crates/runner-app/src/surfaces/chat/mod.rs:671-765`) settles and removes `session_id` from `self.chat_transitions`, the terminal becomes visible (`opacity(1.)`). If the terminal buffer is still empty or cleared, the user sees a blank black screen.

## Investigation Findings & Candidate Root Causes

Inspect and verify all startup and transition paths across `crates/runner-app/src/surfaces/chat_lifecycle.rs`, `crates/runner-app/src/surfaces/chat/mod.rs`, `crates/runner-app/src/surfaces/start_chat/launcher.rs`, `crates/runner-app/src/surfaces/start_chat/launch.rs`, and `crates/runner-terminal/src/terminal.rs` (plus `crates/runner-daemon/src/session/manager/terminal.rs`):

1. **1-second `!output_seen` cutoff in `transition_should_settle` (`crates/runner-app/src/surfaces/chat_lifecycle.rs:87-102`):**
   - `kind == TransitionKind::Starting && !output_seen && elapsed >= TRANSITION_MIN_VISIBLE` unconditionally settles a `Starting` transition at `1.0s` (`TRANSITION_MIN_VISIBLE`) whenever the spawned process has emitted no PTY bytes yet. Cold or warm Claude Code startup (Node/Bun init, keychain/settings/MCP setup) often takes >= 1s before emitting its first TUI frame; dropping the pill at 1.0s when `!output_seen` exposes a blank black terminal right before Claude Code paints (`TransitionKind::Resuming` intentionally does not have this 1s no-output cutoff).
2. **`TerminalMirror::restore` (`crates/runner-terminal/src/terminal.rs:777-825`) does not restore `tui_ready_seq` or `last_output_at` from the attach snapshot:**
   - `client.launch_chat` spawns the PTY on `runnerd` before the UI thread finishes `finish_local_chat_launch` → `ensure_active_tab_attached` → `TerminalMirror::attach`.
   - Any bytes emitted before `attach` arrive in `attachment.snapshot.bytes` via `restore`. `restore` updates `sequence.last` and calls `mark_paint`, but never sets `sequence.tui_ready_seq` (even when `chunk_indicates_tui_ready(&attachment.snapshot.bytes)` or `term.mode().intersects(TermMode::ALT_SCREEN | TermMode::BRACKETED_PASTE)` holds) and never initializes `sequence.last_output_at`.
   - If initial TUI setup bytes (`\x1b[?1049h`, `\x1b[?2004h`) arrived in the snapshot—or if Claude Code emits non-alt-screen startup sequences first (such as `\x1b[?25l` cursor hide, `\x1b[?2026h` synchronized output, `\x1b[?1004h` focus reporting, kitty keyboard protocol, or OSC queries) and then pauses >= 400ms before painting—`tui_ready_seen` stays `false` and the 400ms `TRANSITION_IDLE` backstop (`output_seen && !tui_ready_seen && elapsed >= 1s && idle >= 400ms`) drops the pill onto a black screen.
3. **Premature `first_paint_seq` or transition replacement during launch (`crates/runner-terminal/src/terminal.rs:1416-1435`, `crates/runner-app/src/surfaces/chat/mod.rs:505-530`, `crates/runner-app/src/surfaces/start_chat/launch.rs:29-67`):**
   - Check what bytes Claude Code (and the PTY/daemon) actually emits during the first 0–2s of a fresh chat launch and verify whether `mark_paint` sets `first_paint_seq` prematurely (for example, on transient pre-clear output or cursor/mode sequences) or whether `"session/spawned"` / `finish_local_chat_launch` / `ensure_active_tab_attached` causes the first poll to see stale or premature state.

## Scope & Requirements

1. Trace the exact startup sequence of a fresh Claude Code chat launch (and check other agent runtimes and plain terminals) to confirm which condition(s) cause the Starting pill to settle before the first visible UI frame paints.
2. Fix the transition settling logic in `chat_lifecycle.rs` / `chat/mod.rs` and terminal readiness tracking in `runner-terminal` (`TerminalMirror::restore`, `feed_output`, `mark_paint`, `chunk_indicates_tui_ready`) so that:
   - Starting a new Claude Code chat (from the launcher or inside a pane) visibly shows the `"Starting Claude Code…"` pill right away and keeps it on screen until Claude Code's TUI frame actually paints (`first_paint_seen`), with no black screen gap.
   - `TerminalMirror::restore` properly restores `tui_ready_seq` (and `last_output_at` when snapshot bytes are non-empty) so snapshot-delivered TUI mode sequences are respected.
   - Resuming a stopped chat and starting a plain terminal or non-alt-screen CLI agent still settle promptly once their prompt or first visible output paints, and `TRANSITION_HARD_TIMEOUT` remains as the final safety backstop if a process truly never writes anything.

## Validation

- **Automated tests:**
  - Unit tests in `crates/runner-app/src/surfaces/chat_lifecycle.rs` and `crates/runner-terminal/src/terminal.rs` covering `transition_should_settle`, `TerminalMirror::restore` with snapshot bytes, `tui_ready_seq`, and `first_paint_seq` across Claude Code's startup byte sequence.
  - Run `cargo test -p runner-terminal -p runner-app`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings`, and `cargo fmt --all -- --check`.
  - Note: another mission (`#1251`) is active on this machine in its own worktree; avoid running heavy workspace-wide test suites concurrently with it.
- **Live QA (`qa` slot, macOS dev app):**
  - Acquire the shared QA lock first (`script/qa-lock acquire --timeout 540`) and release it after shutting down the dev app and daemon (`script/qa-lock release`).
  - Record results in `docs/tests/runs/1201-starting-pill-mac.md`:
    1. Start a fresh Claude Code chat from the launcher and from a split pane (using a tiny prompt in a fresh `/tmp` folder): verify the `"Starting Claude Code…"` pill is visible immediately and stays up until Claude Code's TUI renders, with no black screen gap.
    2. Resume a stopped Claude Code chat and open a new terminal tab: verify the Resuming/Starting pill settles cleanly as soon as the TUI or shell prompt paints.

## Authorization & Delivery

- Authorized by Jason on 2026-11-15 to run as an `agy trio` mission, commit (folding this brief in), push `fix/1201-starting-pill`, open one PR against `main` with appropriate labels (`frontend-only` if daemon/protocol are untouched, `risk-low` or `risk-medium`), and drive CI green.
- Remove `needs-live-test` once the required live QA checks pass and are recorded in the PR body and `docs/tests/runs/1201-starting-pill-mac.md`. Do not merge the PR.
