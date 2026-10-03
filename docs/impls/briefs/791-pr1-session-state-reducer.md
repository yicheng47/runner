# 791 PR 1 — Scenario corpus, goldens and the session state reducer

Implement phases 0 and 1 of [#791](https://github.com/yicheng47/runner/issues/791), as one PR, with no behavior change. Jason requested this mission on 2026-10-03 after settling the spec's decisions. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-791-session-state-reducer`, on the existing branch `refactor/791-session-state-reducer`, created from `main` at `5e6ae6aa`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory. Other worktrees under `.worktrees/` belong to other work; treat them as another machine's checkout.

The spec is the contract: [`docs/features/791-session-state.md`](../../features/791-session-state.md). Read all of it. This brief resolves what the spec leaves open for PR 1 and sets the process.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- The spec, especially How it works today, Proposal, Phases, Rules and Relevant code.
- `docs/arch/arch.md` §5.10 (busy / idle inference) and §8.5 (the delivery gate), and `docs/arch/runtime-integration.md` P0.7 and P0.8.
- `crates/runner-backend/src/session/manager/tests/golden.rs` and `tests/expectations/`, the golden pattern #777 left: normalized JSON expectations compared in tests, with `eol=lf` in `.gitattributes`.
- `crates/runner-terminal/fixtures/input-*.ndjson` and `tests/input_state_replay.rs`, including #794's four recorded composer fixtures.

## Phase 0: corpus and goldens of today's behavior

1. **Clock first.** Where a timer in the paths below reads the wall clock (`Instant::now`, `SystemTime::now`), inject a clock so replay runs on a fake one. Existing `_at` methods on `IdleDetector` show the shape. No behavior change.
2. **Replay harness** in `runner-backend`, test-only. It feeds one scenario through today's seams: `IdleDetector`'s `_at` methods, a watcher's `drain_observations` over temp files, `note_forwarder_transition`, `publish_observation`, `status_bridge_failed`, `synthesize_wake_busy`, `report_input_state` and `reserve_delivery`, and the key paths (`rekey_agent_session_key`, `capture_agent_session_key`). It records the published timeline: each `AgentStatus`, the busy/idle `session_status` row with its `source`, delivery-gate decisions with their hold reason, completion and unread effects, and persisted keys.
3. **Scenarios** as NDJSON under `crates/runner-backend/src/session/fixtures/scenarios/<runtime>/`, one file per scenario: timestamped PTY output and titles, local input, hook feed lines and agent records. Cover the spec's scenario list for each runtime where the CLI supports it (Claude Code, Codex, Copilot, pi, Antigravity), plus at least one scenario per rule row (1 to 10) and per bug in the spec's Motivation table. Build agent records from the watchers' existing test samples. Where a record shape is missing, you may read a CLI's own files to copy the shape, but replace every prompt, reply, path and identifier with throwaway text; no conversation content, account identifiers or usage figures enter the repo.
4. **Goldens.** Run today's code over every scenario and commit the timelines as normalized expectations, following `golden.rs`, with `eol=lf` added to `.gitattributes` for any new text fixture or golden directory. Mark each known-wrong timeline with its bug number in the scenario (#781, #784, #785, #786); the golden records today's wrong behavior, and phase 3 flips it.
5. **Checkpoint.** When the harness, scenarios and goldens pass over unchanged session code, hand phase 0 to the reviewer as its own round. After a clean round, make a local checkpoint commit on the branch (squashed later) and tell the reviewer its hash.

## Phase 1: the reducer

1. Add `crates/runner-backend/src/session/state/` with `SessionModel`, `SessionEvent`, `Effects` and one pure `apply(event, now) -> Effects`, with no IO and no locks, held inside `SessionState` under the existing per-session mutex. The spec's enums are shapes, not final signatures.
2. Route every writer in the spec's Status writers table, the draft inputs and the key reports through `apply`. The manager performs the returned effects. Afterwards nothing else mutates status, draft or key fields.
3. The twelve flags leave `SessionState`: `activity`, `status`, `baseline_activity`, `activity_revision`, `suppress_local_input_busy`, `hook_status_armed`, `provisional_idle`, `local_input_pending`, `observed_input`, `last_local_input_at`, `completion_armed`, `compaction_failed_since`.
4. `StatusSource` replaces the free `source` strings and serializes to today's strings exactly.
5. `CTRL_C_INTERRUPT` and `ESCAPE_INTERRUPT` leave `runtimes/claude_code/claude_status.rs` for a runtime-neutral home; importers follow.
6. One reducer test per rule row in the spec's "The rules, written down once".

PR 1 decisions, made in this brief:
- **Watchers still return snapshots.** Phase 2 turns them into `AgentEvent` producers. In PR 1, feed a watcher's `AgentObservation` to the reducer as one transitional event, for example `SessionEvent::Observation`, and keep the watchers' code unchanged apart from imports.
- **Both key write paths stay.** `ConversationChanged` produces a persist effect, but the effect still calls `rekey_agent_session_key` or `capture_agent_session_key` with today's guards. Merging them into one guarded write changes behavior and belongs to a later phase.
- **The draft rule stays today's combined rule**, including #794's hold reasons and their logging, now read from the model.

Out of scope: phase 2 and phase 3 work, any bug fix, the delivery gate's timing (cooldown, reconciliation, outbox), the router's own busy/idle projection, UI, copy, and the Windows reporters.

## The no-change rule

Phase 1 must leave every golden byte-identical: `git diff <checkpoint> -- crates/runner-backend/src/session/fixtures/scenarios <goldens dir>` is empty at the end, and existing behavior tests pass unchanged. A golden that changes is a defect in the refactor, never an expectation to update. If a test outside the corpus must change, state why in the handoff; a change to its expected behavior is not allowed.

## Constraints

- No live sessions. Do not run the app (`make run`), start agents, chats or missions, or use accounts. This crew has no QA slot; Jason smoke-tests the PR.
- Do not run `target/debug/runner`: on this case-insensitive volume it is the GUI app. The CLI in the build tree is `target/debug/runner-agent-cli`, and nothing in this mission needs it.
- Tests never read or write the real `$HOME` or app data; pass roots and homes in.
- Gate imports and helpers used only by `cfg(unix)` tests with `#[cfg(unix)]`; Windows CI keeps failing on them.
- Tests that build a core or DB pool must not leak threads into later tests (an r2d2 pool leak cost #777 a CI round).
- Match the surrounding code; keep comments rare.

## Review, checks and authorization

The reviewer waits for an explicit Runner handoff. Round one is phase 0: do the scenarios cover the rule rows, the spec's scenario list and every Motivation bug, and does the harness observe everything a reducer could change? Later rounds review the full diff against the spec and this brief. Focus: every writer goes through `apply`; nothing outside `session/state/` mutates the model; the goldens are untouched since the checkpoint; lock order and threads are unchanged (no new lock held across IO, no deadlock between the reader, idle monitor, forwarder, input path and router); serialized sources and payloads are unchanged. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

Run these and record each exact command with its exit code:
- `cargo test --locked -p runner-backend --profile ci`
- `cargo test --locked -p runner-terminal --profile ci`
- `cargo test --locked --workspace --no-fail-fast --profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- the macOS updater Clippy, with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

After a clean review, Jason authorizes the following:
- Squash all work on this branch, this brief and the checkpoint included, into one commit on top of current `origin/main`, with a subject that names the change, such as `refactor(session): route session state through one reducer`.
- Push `refactor/791-session-state-reducer` and open a PR against main with `Refs #791` (PR 1 of 3; the issue stays open).
- If main has moved, rebase; never merge main into the branch.
- Amend review and CI fixes into the same commit and push with `git push --force-with-lease`.
- Drive CI green on macOS and Windows.
- Do not merge, delete the branch or worktree, or cut a nightly or release.

Final Runner handoff: the PR URL; the scenario count per runtime and which are marked known-wrong; the checkpoint hash and the empty golden diff; what moved into `session/state/`; tests and exit codes; the CI result; the reviewer's verdict; and what Jason should smoke-test on each runtime (a turn with a tool, an approval, Esc and Ctrl+C mid-reply, `/clear` then resume, a crew message to an idle slot, and a typed draft held against a delivery). Then stand by.
