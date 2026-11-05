# #1172 — Terminal status from shell prompt marks

Jason requested this mission with the claude trio on 2026-11-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1172-shell-prompt-marks` on `fix/1172-shell-prompt-marks`, based on `origin/main` at `2ae1474b7bbf2fd053846c04c03eab2a42f3794c`. The root checkout stays on main.

## Problem

Issue [#1172](https://github.com/yicheng47/runner/issues/1172): a terminal tab's Working/Idle dot is driven by the generic output-silence detector. Any PTY byte flips the session to Busy, and it returns to Idle only after `DEFAULT_IDLE_THRESHOLD` (2 s) of silence. That threshold exists for agent TUIs, where the router treats Idle as a turn boundary; shells are never mission members, so for them it only adds lag. The same signal misreads a shell in three ways: the dot trails a finished command by 2 s; typing at the prompt (echo) or a prompt redraw shows Working; and a command that prints nothing (`sleep 30`) shows Idle while it runs.

## Intended behavior

The dot follows the shell's own command lifecycle, the way cmux, Ghostty, kitty, iTerm2 and VS Code do it: Working from the moment a command starts until the shell is back at its prompt, Idle at the prompt, with no lag and no reaction to typing or redraws. Interactive foreground programs (`vim`, `less`, `ssh`, a REPL, an agent CLI) count as a running command and show Working while open.

The signal is FinalTerm/OSC 133 prompt marks emitted by Runner's existing shell integration:

- `133;C` when a command starts executing: Busy.
- `133;D` when the shell is back, followed by `133;A` for the new prompt: Idle.

Idle is taken only from `D`. Agent TUIs also emit `A`/`B`/`C` for their own transcript zones (see the comment in `runner-terminal`'s `program_status.rs`), so `A` alone must not mean Idle, and a `C` from a program running inside the command is harmless because the shell is already Busy.

A shell session uses the marks once it has seen its first one. Until then, and for any shell without the integration (fish, sh, bash older than 4.4, every shell on Windows), it keeps today's silence detector unchanged. The first prompt's `D`/`A` therefore switches a freshly spawned zsh or bash to Idle immediately.

## Read first

- `crates/runner-daemon/src/session/pty_runtime.rs`: `IdleDetector` (`on_output_event_at`, `on_bytes_at`, `tick_event_at`), `idle_monitor_thread`, `DEFAULT_IDLE_THRESHOLD`, `RESIZE_GRACE`, and where the spawn path builds the detector from `spec.agent_runtime`. Shells spawn with `agent_runtime == Some(Runtime::Shell)` and no terminal adapter.
- `crates/runner-daemon/src/shell_integration.rs` and `crates/runner-daemon/shell-integration/{zsh/.zshenv,bash/runner.bash}`: the existing OSC 7 hooks (Unix only, zsh through a `ZDOTDIR` wrapper, bash through a `PROMPT_COMMAND` bootstrap).
- `crates/runner-terminal/src/program_status.rs`: the daemon-side OSC scanner that already recognises `133;A/C/D`, uses `D` then `A` as "the shell returned" to clear stale OSC 7501 program-status records, and handles sequences split across reads.
- `crates/runner-app/src/surfaces/sidebar/active.rs` `active_session_status`: a shell's activity is shown only while `program_status` is `None`.

## Deliverables

1. **Shell hooks.** zsh: a `preexec` hook emits `133;C`; a `precmd` hook emits `133;D` then `133;A`. bash: `133;C` through `PS0` (bash 4.4+; preserve any `PS0` the user set), and `133;D` then `133;A` from the existing `PROMPT_COMMAND` hook. Do not use a `DEBUG` trap. The marks stay installed when the existing OSC 7 hook removes itself because the user's configuration already sends OSC 7, and duplicate marks from a user's own integration (Ghostty, iTerm2, a prompt theme) must stay harmless. Preserve `$?` and the existing hooks' bounded, no-fork behavior.
2. **Daemon activity.** For shell sessions, `IdleDetector` reads the marks from PTY output and, once one has been seen, sets Busy on `C` and Idle on `D`, ignoring output bytes, the silence threshold and the resize grace for state. Before the first mark, behavior is exactly today's. Parse sequences split across reads and both BEL and ST terminators. Reuse `runner-terminal`'s scanner if it can be shared cleanly; otherwise keep a small dedicated scanner and say why in the PR. Agent sessions, their adapters, hooks and the router are unchanged.
3. **Program-status interplay.** Because Runner's shells now emit `D` then `A`, `runner-terminal` will clear Working/Idle/Blocked program-status records when an agent started by hand in a terminal exits, which lets the sidebar fall back to shell activity. Confirm this with a test, and confirm a live agent's reports still win while it runs.
4. **Guide.** One or two sentences on what a terminal's status dot means and which shells get exact status, in `docs/guide/getting-started/first-chat.md`'s terminal paragraph (or the better-fitting page), mirrored in `docs/guide/zh-CN/`, passing `python3 script/check-guide-translations.py`.

Non-goals: Windows or PowerShell integration, fish integration, the foreground-process-group (`tcgetpgrp`) approach, exit-code display, notifications on command completion, prompt-jump navigation, and any change to agent-session idle detection.

## Validation

Before testing, list the changed behaviors and why each selected check covers it. Automated: unit tests for the scanner (split sequences, both terminators, `A` without `D` ignored) and for the shell `IdleDetector` (silence fallback before the first mark, `C`/`D` transitions, output and resize ignored after marks); a Unix test that spawns zsh and bash with the integration and checks Idle at the prompt, Busy during `sleep`, and Idle right after it ends, skipping a shell that is missing or bash below 4.4; and the existing shell-integration and program-status tests. Run the touched crates' tests with locked dependencies and this worktree's own target directory, plus workspace Clippy with warnings denied, `cargo fmt --all --check`, `git diff --check` and the guide translation check. CI green on macOS, Windows and Linux.

Live checks for qa in the development app on macOS, each in a terminal tab running zsh:

- `ls`: the dot shows Idle as soon as the prompt returns, without a 2 s trail.
- Typing at the prompt without pressing Enter: stays Idle.
- `sleep 10`: Working for the whole run, Idle right after.
- Ctrl-C during `sleep 30`: Idle at the returned prompt.
- Resizing the window while idle: stays Idle.
- `bash --norc` from `/bin/bash` 3.2 opened as the terminal's shell, or another shell without the integration: behaves as before (silence detector).

## Authorization and handoff

Jason approved the claude trio (coder, reviewer, qa) for this mission. The crew may commit, push this branch and open one PR against main. The PR touches `session/pty_runtime.rs`, so it gets `core-review`, plus a risk label picked from the diff and `needs-live-test` until qa passes every live check above. Stop at an open PR with green CI, reviewed and live-tested, and report the PR URL, final SHA, changes and validation through Runner.
