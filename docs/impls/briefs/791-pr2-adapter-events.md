# 791 PR 2 — Runtime watchers emit events, the reducer owns the snapshot

Implement phase 2 of [#791](https://github.com/yicheng47/runner/issues/791), as one PR, with no behavior change. Jason requested this mission on 2026-10-03 after PR 1 merged as #796 (`b5203daa`). Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-791-adapter-events`, on the existing branch `refactor/791-adapter-events`, created from `origin/main` at `f8000ddc`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory. Other worktrees under `.worktrees/` belong to other work.

The spec is the contract: [`docs/features/791-session-state.md`](../../features/791-session-state.md), especially "Adapters translate, the reducer decides", the rules table and Decisions. This brief resolves what the spec leaves open for PR 2.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- The spec, and `docs/tests/791-session-state-reducer.md` (what PR 1 did and the review findings it met).
- `session/state/` (the reducer, including the transitional `SessionEvent::Observation`), `session/hook_feed.rs` (`HookWatcher`), `session/status.rs` (`AgentObservation`), and the five watchers in `runtimes/*/*_status.rs`.
- `session/pty_runtime.rs`: `IdleDetector`, `CodexStartup`, `CodexTitleHint` and the `hook_interrupt` plumbing; `session/runtime.rs` (`codex_pending_turn`).
- `session/fixtures/README.md` and `session/manager/tests/replay.rs` and `replay/render.rs`: the corpus and the compact golden format.

## Work

1. **Watchers emit `AgentEvent`s.** `HookWatcher::drain_observations` becomes `drain_events`, yielding the spec's `AgentEvent`s (turn started, working with detail, turn ended with outcome, interaction opened and closed, ready). A watcher keeps only parsing and the correlation specific to its CLI, such as Copilot's `permission.completed` matched to its request or Claude's user-rejected `tool_result`. It stops building `AgentObservation` snapshots and deciding outcomes.
2. **The reducer owns the snapshot.** Turn state, open interactions, outcome and detail move into `SessionModel`, and the transitional `SessionEvent::Observation` goes away. `AgentObservation` stays only as the published form.
3. **Local cancel as an argument.** The `AtomicU8` `hook_interrupt` signal and `interrupt_signal()` are removed. A watcher that needs to know a local Esc or Ctrl+C happened, to read a later record correctly, receives it as an argument to `drain_events`.
4. **Codex startup and title leave the shared terminal layer.** `CodexStartup` and `CodexTitleHint` move out of `IdleDetector` into the Codex adapter. The PTY layer passes the output it already sees, and the adapter turns it into the spec's `Title` and readiness events for the reducer. Afterwards `IdleDetector` and `session/runtime.rs` hold no Codex-specific state (`codex_pending_turn` included); rule 1's behavior is unchanged. Do this as the last step, after items 1 to 3 pass review. If it threatens the no-change rule, report to Jason through Runner instead of forcing it.

PR 2 decisions, made in this brief:
- **Conversation keys stay as PR 1 left them.** Key capture sources and both SQL guards are untouched; merging them into one guarded write changes behavior and belongs to phase 3.
- **Shells are out of scope.** Separating shell launch from agent orchestration is #795, which follows #791.
- **No bug fixes.** #781, #784, #785 and #786 are phase 3. A known-wrong golden stays wrong.

## The no-change rule and golden format

Every existing scenario and golden stays byte-identical: `git diff f8000ddc --exit-code -- crates/runner-backend/src/session/fixtures` must be empty at the end, unless you add scenarios. A changed golden is a refactor bug, never an expectation to update. The harness may change to drive `drain_events`, but it must still compute and compare the full timeline.

If a watcher's event translation exposes a path no scenario covers, add a scenario. New goldens use the existing compact text format from `fixtures/README.md`, one line per step with only what changed, and the whole `expectations/` tree stays under 3,000 lines (it is 2,148 now). Report the line count at each review handoff.

## Constraints

- No live sessions: do not run the app, agents, chats, missions or accounts. PR 2 gets no live smoke (spec decision 3); the full QA pass comes with phase 3.
- Never run `target/debug/runner`, which is the GUI app on this volume. The CLI in the build tree is `target/debug/runner-agent-cli`, and nothing here needs it.
- Tests never read or write the real `$HOME` or app data.
- Gate imports and helpers used only by `cfg(unix)` tests with `#[cfg(unix)]`. PR 1 lost a CI round to exactly this.
- Tests that build a core or DB pool must not leak threads into later tests.
- Match the surrounding code, and keep comments rare.

## Review, checks and authorization

The reviewer waits for an explicit Runner handoff. Focus: each watcher's events reproduce what its snapshots published, record for record; no outcome or interaction logic is left in a watcher; the local-cancel argument covers every path the `AtomicU8` did, including Copilot approve-then-cancel and Claude rejected-tool settling; `IdleDetector` has no Codex state; lock order and threads are unchanged; and the fixture diff is empty or holds only new compact scenarios within the budget. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

Run these and record each exact command with its exit code:
- `cargo test --locked -p runner-backend --profile ci`
- `cargo test --locked -p runner-terminal --profile ci`
- `cargo test --locked --workspace --no-fail-fast --profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- the macOS updater Clippy, with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`
- the fixture diff above

After a clean review, Jason authorizes the following:
- Squash all work on this branch, this brief included, into one commit on top of current `origin/main`, with a subject that names the change.
- Push `refactor/791-adapter-events` and open a PR against main with `Refs #791` (PR 2 of 3).
- If main has moved, rebase; never merge main into the branch.
- Amend review and CI fixes into the same commit and push with `git push --force-with-lease`.
- Drive CI green on macOS and Windows.
- Do not merge, delete the branch or worktree, or cut a nightly or release.

Append a dated "PR 2" section to `docs/tests/791-session-state-reducer.md` with the commands, exit codes and review corrections. Final Runner handoff: the PR URL; what left each watcher and what moved into `session/state/`; where the Codex startup and title code now lives; the fixture diff result and the `expectations/` line count; tests and exit codes; the CI result; and the reviewer's verdict. Then stand by.
