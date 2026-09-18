# 625 — Remove the title-spinner status heuristic

Tracking issue: [#625](https://github.com/yicheng47/runner/issues/625). Slice 6 of the [347 plan](./347-hook-status/plan.md), split out of [#610](https://github.com/yicheng47/runner/issues/610) on 2026-09-16. Chore, P2. Shipped 2026-09-17 in [#638](https://github.com/yicheng47/runner/pull/638). Implemented by a codex peer crew mission from the [brief](../briefs/625-m1-title-heuristic-removal.md), landed by the driving Claude session.

## Why

Status has two layers: byte activity, the permanent baseline, and the hook adapters on top. Title-spinner classification (#584, implemented by #585) sat between them: it armed on a braille-prefixed title and then overrode byte activity, reading presentation Runner does not control. The 347 spec named its removal condition, a hook adapter for every runtime that animates its title. Claude Code and Codex have had one on macOS since #589 and on Windows since #610 (PR #627, smoke passed 2026-09-16).

## What ships

- **runner-terminal.** `classify_title`, `TitleStatus` and the terminal event thread's call into `report_declared_status` are gone. Title display, `sanitize_title`, `provider_title` and #587's persistence are unchanged.
- **runner-backend.** `SessionManager::report_declared_status`, `SessionRuntime::note_declared_status` with its default and both implementations, the PTY handle's retained `status_tx`, `SessionState::title_status_armed` with its `is_empty` check and lifecycle resets, and the `title` source in `note_forwarder_transition`. Every other source keeps its gating; EOF still disconnects the forwarder once the reader and idle monitor drop their senders.
- **Byte-baseline proof.** `recorded_fixture_byte_activity_transitions` in `pty_runtime.rs` replays `codex-title-working.ndjson` and `claude-session.ndjson` from `runner-terminal/fixtures` through the private `IdleDetector`. Codex pins `[(12893, Idle)]`: Busy through its whole animation tail, Idle only 2 s after its last output. Claude pins three quiet gaps that idle and wake, then Idle at 43517 ms. `spawn_exit_seven_records_exit_code` now asserts the output stream disconnects after child exit.
- **Docs.** `docs/arch/arch.md` §5.10 describes byte activity (#124) as the only baseline; the 347 plan's slice 6 records the removal on both platforms.

Sessions without a working hook bridge (TRAE, roles that opt out of Runner's hooks, bridge loss) now follow output traffic alone, labelled estimated as before.

## Verification

- Codex peer mission `01M2Q1HEBE8KTBTK5XY5DXZY78`: review clean after one round (the must-fix was a wrong issue citation in §5.10).
- The landing session's own pass: `cargo test --locked --workspace`, workspace Clippy with `-D warnings`, `cargo fmt --check`, `git diff --check`, and a search over `crates/` for `TitleStatus|classify_title|title_status_armed|declared_status|source: "title"` returning nothing; rerun after the rebase onto `main`.
- Jason's smoke on macOS passed on 2026-09-17: a hook-less session reads Working · estimated while output flows and Idle · estimated after it stops; Claude Code and Codex chats with hooks are unchanged.
