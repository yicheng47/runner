# 647 — Flush synchronized updates at their deadline

Fix [P1 #647](https://github.com/yicheng47/runner/issues/647), milestone 0.11: a terminal pane stuck black after a sidebar toggle. Jason asked for a claude pair crew mission on 2026-09-23 that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-647-sync-update-timeout`, on branch `fix/647-sync-update-timeout`. The mission's directory is this worktree. Its tip is this brief, on top of `main` `2bee326`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Do not touch the older `fix/647-terminal-black-sidebar` branch: it holds #709's architecture draft. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## The cause

Read #647's latest comment first; it is the diagnosis.

- `vte` 0.15's `Processor` holds every byte between `ESC[?2026h` and `ESC[?2026l` (a synchronized update) and records a deadline, `SYNC_UPDATE_TIMEOUT` = 150 ms, in its `StdSyncHandler`. It never flushes on its own. It releases the held bytes only on the end marker, on a buffer past `SYNC_BUFFER_SIZE` (2 MB), or when the caller invokes `stop_sync` (`~/.cargo/registry/src/*/vte-0.15.0/src/ansi.rs`, lines 36, 39, 292, 315, 451).
- Alacritty's event loop services that deadline: it waits on `parser.sync_timeout().sync_timeout()` and calls `parser.stop_sync(&mut *terminal)` when it passes (`alacritty_terminal-0.26.0/src/event_loop.rs:229-246`).
- Runner feeds the parser directly in `TerminalSession::feed_output` (`crates/runner-terminal/src/terminal.rs:527`) and nothing calls `stop_sync`. A redraw that starts with a clear and a hidden cursor and loses its end marker leaves the pane blank until another end marker or 2 MB of output, and resizes do not release it. That is the sticky black pane.

Why the end marker went missing in Jason's SSH → Codex session is not known and is not this mission's question. With the deadline serviced, any missing end marker recovers in 150 ms, as in Alacritty.

## Deliverable

1. **Service the deadline** in `runner-terminal`, on the current runtime:
   - after each `feed_output`, when the parser holds a synchronized update with a deadline, make sure a flush is scheduled for it;
   - at most one pending flush per session, not a thread per chunk; a flush re-reads the deadline, because a new begin marker extends it;
   - hold the session weakly, so a closed or replaced session is neither kept alive nor flushed;
   - the flush takes the locks in `feed_output`'s order (sequence, parser, input tracker, term), and calls `stop_sync` only if the deadline is still due and the update is still pending;
   - after a flush, do the post-parse work `feed_output` does: the first-paint sequence, clearing the selection when mouse mode turns on, the input observation and `report_input_state`, and the waker when a viewer is attached. Replies produced by the flushed bytes leave through the existing event channel, in order.
2. **Log** each timeout flush at info level with the session id and the number of bytes that were held, so a recurrence shows whether a missing end marker was involved. Never log the bytes.
3. **Tests**, built from the 09-21 probe:
   - populate a grid, then send a begin marker, a clear, a hidden cursor and redraw text with no end marker; within about 150 ms the grid shows the text and the cursor, with no new bytes and no direct `stop_sync` call from the test (drive time through an injected clock or a bounded wait, not a long sleep);
   - resizes while the update is held do not release it early and do not lose it after the deadline;
   - a complete update split at every byte position applies exactly once, and no timeout flush fires for it;
   - a flush scheduled for a session that has since closed does nothing.
4. **Docs, same diff**: a sentence in `docs/arch/arch.md` where the terminal model's parser feed is described, saying Runner services the synchronized-update deadline itself because it does not run Alacritty's event loop.

Out of scope: adopting Alacritty's PTY or `EventLoop` (#709), a `vte` or `alacritty_terminal` upgrade, renderer or glyph changes, resize-path changes, and chasing the SSH trigger.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-terminal --profile ci --no-fail-fast`
- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app, and do not start, stop, restart or type into Jason's Runner apps, chats or missions; Jason smoke-tests the UI. Do not change agent configuration. Native Windows is unavailable; say what is unverified there.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff. Then it reviews the whole working-tree diff against this brief and #647's diagnosis, must-fix findings first with file:line pointers. It checks in particular:

- lock order and that no lock is held across the waker, `report_input_state` or any blocking call;
- that a flush can never apply bytes twice, apply them to a replaced session, or fire for an update that completed normally;
- that no timer, thread or channel outlives its session;
- that the tests fail on `main` and pass with the fix.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: imperative subject, scope `terminal` or `docs`, no co-author trailers. Keep the brief commit.
- **Push** with `git push -u origin fix/647-sync-update-timeout`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #647`, the cause in two sentences, the fix, test evidence, what is unverified (native Windows, the original SSH incident), and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner: the PR URL and CI result, changed files, checks with exit codes, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
