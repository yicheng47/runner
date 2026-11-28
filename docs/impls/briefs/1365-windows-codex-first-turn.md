# #1365 — Windows Codex first turn submits without a manual Enter

Jason approved the neon duo (coder and reviewer) for this fix on 2026-11-28. Work only in `.worktrees/fix-1365-windows-codex-first-turn` on `fix/1365-windows-codex-first-turn`, based on `origin/main` at `3f483a34`. The root checkout stays on `main`.

## Problem

On Windows, when Codex is installed through npm, pnpm or bun, Runner launches it through the `codex.cmd` shim. A `.cmd`/`.bat` argument cannot carry `\r` or `\n`, so `SessionManager::apply_runtime_args` (`session/manager/spawn_spec.rs`) leaves the first turn out of argv and `queue_windows_batch_first_turn` (`session/manager/capture.rs`) queues it for a post-spawn paste. `deliver_windows_batch_first_turn` (`session/manager/output.rs`) pastes it as soon as `TuiReadiness` (`session/runtime.rs`) sees `\x1b[?2004h`, `\x1b[?1049h` or `\x1b[?47h`. Then `inject_paste` sleeps a fixed 120 ms, sends one `\r` and never checks that a turn started.

Codex emits `?2004h` about 50 ms in, while it still shows its startup composer and its app-server bootstrap runs for 1–8+ s (#1262). In Codex 0.172 (`codex-rs/tui/src/startup_draft_input.rs`), an Enter that arrives during the startup paste window becomes a pasted newline (`pending_paste_newline`). Only a clean Enter sets `submission_pending`, which submits once bootstrap ends, and any later input cancels it. Codex 0.149–0.166 (`startup_draft.rs`) ignored a plain Enter until bootstrap finished. On Windows, Codex clears `ENABLE_VIRTUAL_TERMINAL_INPUT`, so ConPTY delivers the multi-KB paste as key records rather than one paste event (see #753). The single Enter lands inside the paste window, and the prompt sits unsubmitted. Codex is behaving as designed. Runner's delivery is what is wrong.

The same path serves mission starts (`mission_spawn.rs`), fresh slot restarts (`resume.rs`) and direct chats with a role system prompt or initial prompt (`direct_spawn.rs`). macOS, Linux and a native `codex.exe` pass the first turn in argv and are unaffected.

## Outcome

For a `.cmd`/`.bat` agent launch with a pending first turn, the first turn starts on its own, exactly once: no manual Enter, no second turn, and no stray blank submit, on Codex 0.149 through current. Once the turn has started, Runner never sends another Enter.

## Direction

The coder chooses the design, within these bounds:

- **Confirm the submit and retry it.** After pasting, send Enter after a delay suited to Windows Codex (the inbox path's `SUBMIT_DELAY` of 300 ms on Windows, from #753, is the reference). Then check whether the turn started, and resend Enter until it has or until a bounded deadline long enough to cover a slow bootstrap; log a warning when the deadline passes. Reuse the existing confirmation machinery rather than inventing a new one: the inbox dispatcher's type, Enter, confirm loop (`router/dispatch.rs`, `CONFIRM_WINDOW`, `runner_terminal::delivery_probe::InputProbe`) and the session's existing turn-start signals.
- **Readiness.** Consider whether the paste should wait for the startup delivery gate (`session/manager/startup.rs`, `StartupDelivery`), which already knows when an agent is ready for input, instead of `?2004h`. Keep the existing timeout fallback.
- **Keep it scoped** to the Windows batch first-turn path and whatever `inject_paste` changes it needs. If `inject_paste`'s fixed 120 ms also serves other callers, leave their behaviour unchanged.

## Non-goals

- Do not bypass the npm shim by launching the packaged `codex.exe` directly. The shim locates the binary differently for npm, pnpm, bun and vite-plus, and sets `CODEX_MANAGED_BY_*` and `CODEX_MANAGED_PACKAGE_ROOT`, which Codex uses for its update hints. Copying that ties Runner to Codex's packaging.
- No change to the argv first-turn path on macOS, Linux or for native executables, to `SUBMIT_DELAY`, or to inbox and report delivery.
- No UI change, and no change to `crates/runner-terminal` beyond using its existing API: PR #1509, which replaces its terminal engine, is open.

## Tests

The delivery path is `#[cfg(windows)]`, and this crew runs on macOS. Put the submit and retry decision in code that compiles and is tested on every platform, with only the Windows trigger gated, so macOS tests cover the logic and Windows CI runs the gated path. Cover: a turn that starts after the first Enter gets no second Enter; an Enter swallowed as a pasted newline is retried and the turn starts once; a startup composer that ignores Enter until bootstrap still submits after it ends; the deadline stops retries and logs. Follow the repository's rules for tests with background work, and loop the new tests 20 times with nextest stress before handing off, stating the count.

## Validation

Scoped `cargo test -p runner-daemon`, workspace Clippy with warnings denied (the CI profile command in `AGENTS.md`), `cargo fmt --all --check` and `git diff --check`. Drive the macOS, Windows and Linux CI checks green.

Live checks are left to Jason, on Windows with Codex installed through npm (`codex.cmd`), on a nightly build:

1. Start a mission with a Codex slot: its first turn starts without pressing Enter, exactly once.
2. Restart that slot fresh: the same.
3. Start a direct Codex chat from a role with a system prompt: the same.

The reviewer may name more checks in its clean review. Neither slot runs the app.

## Authorization

Neon duo: one coder and one reviewer. The coder may commit, push this branch and open one PR against `main` with `Fixes #1365`, labelled `risk-medium` and `needs-live-test`, with the live checks above listed in its description. `core-review` is added automatically and stays on. The crew does not cut a nightly.
