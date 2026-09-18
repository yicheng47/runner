# 623 — Seed mission slots Busy at spawn

Tracking issue: [#623](https://github.com/yicheng47/runner/issues/623). Bug, P2. Shipped 2026-09-17 in [#639](https://github.com/yicheng47/runner/pull/639). Found on JASONPC during the #610 mission 1 smoke. Implemented by a codex solo crew mission on 2026-09-17 from the [brief](../briefs/623-m1-mission-slot-spawn-seed.md), landed by the driving Claude session.

## Why

A mission slot whose status comes only from byte activity showed Status unavailable for its whole first turn. `IdleDetector` starts in Busy and reports only changes, so an agent that works from its first byte with no 2 s gap emitted no transition and the slot had no `session_status` row. Direct chats were seeded Busy with source `spawn` at spawn; mission slots were not. The 0030 plan had skipped the seed on purpose because the router already reads a live slot with no row as busy, but the #347 status UI came later and reads the observation, which stayed empty. Hook adapters hide the gap within milliseconds, so it showed wherever the baseline is the only source: TRAE, roles that opt out of Runner's hooks, and bridge loss.

## What ships

- **`SessionManager::publish_mission_activity`** beside `publish_direct_activity`: passes `note_forwarder_transition`, emits `session/status`, and appends a `session_status` row through the session's `mission_status_sink`. A failed append logs a warning and never fails the spawn.
- **The mission slot spawn** calls it with Busy and source `spawn` after `install_handle` and `arm_completion`, before the Windows first-turn queue and `start_forwarder_thread`. The seed is Baseline and does not arm hook ownership. Direct chats, resume, fork and router delivery are unchanged.
- **Tests.** `mission_spawn_seeds_status_and_allows_hook_takeover`: the spawn publishes Busy with source `spawn`, activity Working and a Baseline observation; the slot's first mission-log row is busy/spawn; a later hook Busy is accepted with source `hook`. The shared direct/mission consumer test, the mission dedupe and typing tests and the synthetic wake tests now expect the seed row first; the dedupe test still proves duplicate forwarder Busy transitions append nothing.
- **Docs.** `docs/arch/arch.md` §5.10 says mission sessions and direct chats are both seeded at spawn.

## Verification

- Codex solo mission `01M2Q4JKQBSKYH4CTYHTMVHZGB`: the coder's checks and self-review were clean.
- The landing session read the whole diff and reran `cargo test --locked --workspace`, workspace Clippy with `-D warnings`, `cargo fmt --check` and `git diff --check` on the branch rebased onto `main`.
- Jason's smoke passed on 2026-09-17.
