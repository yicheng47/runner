# 623 — Mission 1: seed mission slots Busy at spawn

Jason requested a codex solo crew mission on 2026-09-17. Work in `/Users/jason/repos/yicheng47/runner` on branch `fix/623-mission-slot-spawn-seed`, whose tip is this brief on top of `chore/625-title-heuristic-removal` (PR #638, green locally, in CI). Build on top of it; do not rebase, squash or create another branch. The landing session rebases onto `main` after #638 merges.

Read first: this brief and issue [#623](https://github.com/yicheng47/runner/issues/623) (`gh issue view 623`), which has the cause and the code pointers.

## Authorization

You implement, run the checks and self-review the whole diff against this brief. No subagents, new checkout or worktree. Stop with every change uncommitted: no commit, push or PR. Report once with `runner msg post`.

## The bug

A mission slot whose status comes only from byte activity shows Status unavailable for its whole first turn. `IdleDetector` starts in Busy and reports only changes, so an agent that works from its first byte with no 2 s gap emits no transition, and the slot has no `session_status` row. Direct chats avoid it because `spawn.rs` seeds `publish_direct_activity(…, SessionActivityState::Busy, "spawn", …)` after installing the handle. Mission slots have no seed.

## Deliverable

1. **Seed the mission spawn.** In the mission slot spawn path in `crates/runner-backend/src/session/manager/spawn.rs`, after `install_handle` and `arm_completion` and before `start_forwarder_thread` (the same place the direct spawn seeds), publish one Busy transition with source `spawn` that does all three things a forwarder transition does for a mission session: passes `note_forwarder_transition`, emits `events.status` with a `SessionActivityEvent`, and appends a `session_status` row to the mission log through the session's `mission_status_sink` (`session_status_draft` with state, source and the current `agent_status`). Put this in one `SessionManager` helper beside `publish_direct_activity` rather than copying the output-forwarder code; the append may block (spawn is not the forwarder consumer thread), and a failed append logs a warning and never fails the spawn. Direct chats keep `publish_direct_activity` unchanged. No `cfg` branch: the seed behaves the same on macOS and Windows.
2. **Nothing else changes.** `spawn` is neither `forwarder` nor `hook`, so the seed is labelled Baseline and does not arm hook ownership; the first hook transition afterwards must still be accepted and published. Router delivery is unchanged, since a live slot with no row already reads busy. Mission resume and fork are out of scope: a resumed slot sits quiet and reads Idle after 2 s.
3. **Tests**, in `crates/runner-backend/src/session/manager/tests.rs` beside the direct-chat seed test (the one asserting `seeded.source == "spawn"`): a mission spawn publishes a `session/status` event with source `spawn`, activity Working and source Baseline; the mission log's first `session_status` row for that slot has state `busy` and source `spawn`; a later hook Busy transition is accepted and publishes source `hook`. If `assert_status_uses_existing_direct_and_mission_consumers` or another existing test now sees the extra seed row, update its expectation and say why in your report; do not weaken any other assertion.
4. **Docs.** In `docs/arch/arch.md` §5.10, the paragraph on mission sessions and direct chats says both are seeded Busy with source `spawn` at spawn, so a slot reads Working · estimated from its first byte.

## Verification

`cargo test --locked -p runner-backend -p runner-app --no-fail-fast --profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check`, `git diff --check`. Report each exit code.

## Report

Branch and base commit, the helper's name and where the seed is called, the tests added or changed and why, each check with its exit code, what your self-review checked, and anything you could not prove.
