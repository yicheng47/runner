# 582 — Split the oversized test modules (pass 1)

Tracking issue: [#582](https://github.com/yicheng47/runner/issues/582). Chore, P3. Baseline `main` at `54798b6` (2026-09-23). Branch `chore/582-split-test-files`, worktree `.worktrees/chore-582-split-test-files`. First pass of #582; the production files in its table (`panes.rs`, `ops/mission.rs`, `router/runtime.rs`, …) get their own passes and briefs.

## What ships

Two moves, one commit each, no behavior change:

1. `crates/runner-backend/src/session/manager/tests.rs` (10,127 lines, 154 `#[test]`s) becomes the directory `session/manager/tests/`.
2. `crates/runner-backend/src/router/tests.rs` (2,401 lines, 48 `#[test]`s) becomes the directory `router/tests/`.

No new file over ~900 lines, most under 700. `session/manager/mod.rs:53` and `router/mod.rs:1633` keep `#[cfg(test)] mod tests;` exactly as they are; if either needs an edit, the cut is wrong, so stop and report.

## The pattern

The precedent is 478 pass 2, [`docs/impls/archive/478-consolidation-pass-2.md`](../archive/478-consolidation-pass-2.md). Read its "The pattern" and "Verification" sections first: this pass inherits both, with these differences.

- **`tests/mod.rs`** holds the existing top-of-file `use` block and comment, every fixture and helper used by two or more children, and the `mod` declarations. Private items in `mod.rs` are visible to every child and need no visibility change.
- **A helper used by exactly one child moves with that child.** A helper that ends up in a child and is used by a sibling gets `pub(super)` only when the compiler names it. Never `pub(crate)` or `pub`. List every `pub(super)` as `file::item` in the handoff.
- **Imports.** Each child opens with `use super::*;` plus anything it needs. `router/tests.rs` imports with `use super::{…}`; in `router/tests/mod.rs` that `super` is still `router`, so it stays verbatim. Prune with `cargo fix --allow-dirty --allow-staged -p runner-backend --tests` and `make fmt`; hand-curate only what the compiler flags without a machine fix, and list it.
- **`#[cfg(unix)]`, `#[cfg(windows)]` and `#[cfg(target_os = …)]` items move with their attributes.** The manager tests carry several (`fixture_tmp_dir`, the fork materializers, `RepairingCapture`, the Windows forwarder test). A lost `cfg` compiles on macOS and breaks the Windows job.
- **Order.** Within a child, tests keep their source order.

## The cut

Group by concern. The file names below are the intended shape; derive the line ranges at the branch point and post the final table (file, source ranges, line count) with the review handoff. If a group lands over ~900 lines, split it along its own seam; if it is tiny, fold it into its nearest neighbour. Say which.

**`session/manager/tests/`**

| File | Holds |
| --- | --- |
| `mod.rs` | Imports; `fixture_tmp_dir`, `ci_scaled_budget`, `InertRuntime`, `FakeRuntime` / `FakeSpawn` / `RuntimeGate`, `Capture`, the `role` / `slot_for` / `mission` / `pool_with_schema` / `insert_crew_role` fixtures, the wait helpers, whatever else two or more children share |
| `forwarder.rs` | `forwarder_*` tests with `ForwarderCapture` and `forward_queued_output` |
| `mission_lifecycle.rs` | Concurrent missions, slot-exit reaping, `kill` / `kill_many` / `kill_all_for_mission`, spawn-failure reaping, runtime channel close, `spawn_direct_writes_session…`, `role_activity_event…` |
| `input.rs` | `inject_stdin_*`, local input byte classes, observed input tiers and drafts, `DeliveryEventCapture` |
| `status.rs` | Interrupts and escape, hook-status ownership, direct and mission status transitions, typing-stays-idle, the input gate timeout, `mission_spawn_seeds_status…` |
| `permissions.rs` | Persona and preamble argv, codex sandbox grants, permission-mode convergence and stripping, `trae_first_turn_gets_capture_prompt_marker` |
| `terminal_size.rs` | Mission registration size, fork-at-hint, `mission_fork_*`, first-spawn default size, resize storms and settle |
| `resume.rs` | Resume row reuse, `launch_resume_*`, resume refusals, mission resume env, codex mission resume grants, `codex_resume_skips…`, `mission_spawn_cwd…` |
| `spawn_env.rs` | Login-shell proxy env, locale fallback |
| `wake.rs` | Synthetic wake, `forwarder_status_emit_*` and the event-log contention helpers |
| `launch_gate.rs` | `compute_gate_wait_*`, `enter_claude_launch_gate_*`, custom claude settings, `spawn_argv_injects_runtime_settings…` |
| `runtime_override.rs` | `runtime_direct_*`, shell runtime, the override helper, slot overrides, pinned runtime, catalog default role, `runtime_only_resume…` |
| `fork.rs` | Direct-chat forks and the headless fork tests |
| `windows_batch.rs` | `windows_batch_*` and their helpers |
| `slot_restart.rs` | `slot_respawn_fixture` and the restart and resume-after-restart tests |
| `attention.rs` | Normalized status snapshot, failure attention, bridge loss, hook session start, backpressure |
| `codex.rs` | Codex hook composition and observations, `codex_pre_hook_*`, `codex_windows_batch…` |
| `copilot.rs`, `pi.rs` | The per-runtime tests |

**`router/tests/`**

| File | Holds |
| --- | --- |
| `mod.rs` | The header comment, imports, `RecordingInjector` and the fixtures through `set_unread` |
| `delivery_blocked.rs` | `delivery_blocked_*`, transient reservations, concurrent parks |
| `reconciliation.rs` | `reconciliation_*` |
| `nudges.rs` | Directed and broadcast messages, human messages, input-clear flushes, deferred nudges, typing retries, `mission_goal_handler…` |
| `asks.rs` | `human_said…`, `ask_lead…`, `ask_human…`, `human_response_*` |
| `status.rs` | `session_status_*`, `directed_wake…`, `synthetic_busy…` |
| `reconstruct.rs` | `pending_ask_map…`, `reconstruct_*`, `fresh_mission_start…`, `stopped_session_delivery…` |
| `restart.rs` | `registry_register…`, `slot_restart_events…`, `lead_restart…` |

## Rules of the road

- **Moves only**, plus the mechanical exceptions in the 478 pattern: imports, module declarations, split `impl` wrappers, compiler-proven `pub(super)`, rustfmt reflow. No renames, no extracted helpers, no comment rewrites, no clippy tidying, no test logic changes. Anything worth changing goes in the handoff.
- Touch only the two test module trees, plus this brief if the human approves a correction.
- Stage by path, never `git add -A`.
- Do not launch the app (`make run`); Jason smoke-tests. No extra worktrees, checkouts or agents.

## Verification

All three gates go in the review handoff, per file split.

1. **Parsed item equality.** Parse the original file at the branch point and every new file with `syn`. Ignore `use` items and `mod` declarations, flatten `impl` blocks into members keyed `Type::method` (with the trait where there is one), key free functions, structs and statics by name, and normalize only the approved `pub(super)` prefixes. Every original item must match exactly one moved item, with zero differences; report the counts. Validate the comparator with five negative controls against temporary copies, never the repository: delete a statement from a test body, change a literal, drop a match arm, widen a `pub(super)` to `pub(crate)`, and remove a `#[test]` or `#[cfg(windows)]` attribute. Each must fail and name the item. Keep the script in the scratchpad or `/tmp`; do not commit it.
2. **Test count.** Record `cargo test --workspace` passed and ignored counts at the branch point before moving anything; they must be identical after. Run with `set -o pipefail` or check Cargo's exit status directly: a green `grep` or `tail` proves nothing.
3. **`make verify` green**, plus `cargo clippy -p runner-app --features updater --all-targets -- -D warnings`.

Also report: each new file's line count, every `pub(super)` as `file::item` with a count, every boundary you adjusted, and the imports you curated by hand.

## Authorization

After the reviewer reports clean, PR mode is authorized: commit on this branch (one commit per file split), push it, and open a PR against `main` titled `chore: split the session manager and router test modules (582 pass 1)`, with a body that says `Refs #582` (not `Closes`: more passes follow) and summarizes the gates. No Claude session link in the body. Drive CI green with `gh pr checks <n> --watch`, checking its exit status; the required checks are `Rust / macOS` and `Rust / Windows`. Then stop. Do not merge, delete the branch or worktree, or cut a nightly or release: Jason reviews and merges.

## Non-goals

The production files in #582's table, every other oversized file, behavior, test logic, and docs beyond this brief.
