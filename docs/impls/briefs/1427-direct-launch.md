# #1427 — One `DirectLaunch` options struct for direct session launches

Jason approved `agy duo` (neon duo: Antigravity `gemini-4-neon` coder + Codex `gpt-6.2-astra` reviewer) on 2026-11-23. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/refactor-1427-direct-launch` on existing branch `refactor/1427-direct-launch` (base `origin/main` `226ecdf7`, local `target/`, never share `CARGO_TARGET_DIR`). Never touch the root checkout or sibling worktrees under `.worktrees/`.

This brief is committed as the initial commit on `refactor/1427-direct-launch` so the worktree starts clean. Fold it into the implementation commit (`git commit --amend` or autosquash) before pushing so it does not land as a standalone brief commit.

## Problem and outcome

Starting a direct chat threads runtime, model, effort, speed, project scope, cwd, worktree, geometry, and prompts as positional arguments across `ops/session.rs` and `session/manager/direct_spawn.rs`, adding a new function variant for every launch option (`_with_speed`, `_and_prompt`, `_after_insert`). Today `ops/session.rs` carries 8 `#[allow(clippy::too_many_arguments)]` and `direct_spawn.rs` carries 10 (9 functions plus a duplicate allow on lines 68–69), up to 16 parameters on `spawn_direct_with_speed_and_prompt_after_insert` and 18 on `spawn_direct_inner`.

Outcome: clean `Default`-able options struct(s) (`DirectLaunch`) and one direct-launch entry point per layer, removing all `_with_speed` / `_and_prompt` / `_after_insert` wrapper chains and all 18 `#[allow(clippy::too_many_arguments)]` in `ops/session.rs` and `direct_spawn.rs`, with zero behavior or wire-protocol change. This lands ahead of #795 so shell/agent launch separation starts from one clean launch type.

## Read first

- `AGENTS.md` (Engineering Conventions, Commit And PR Conventions, Crew Missions) and `gh issue view 1427`.
- `crates/runner-daemon/src/ops/session.rs` (lines 1430–1662, 1746–1784, 1878–1915).
- `crates/runner-daemon/src/session/manager/direct_spawn.rs` (entire file, including both `spawn_direct*` and `spawn_runtime_direct*` families and `spawn_direct_inner`).
- `crates/runner-core/src/protocol/api.rs` (lines 51, 145–148) and `crates/runner-core/src/protocol/session.rs` (`StartDirectSessionArgs`, lines 147–186).
- `crates/runner-daemon/src/companion/actions.rs` (lines 598–666).
- `crates/runner-cli/tests/goldens/cli.txt` (lines 670–695) for error-context golden invariants.

## Deliverables

1. **Manager layer (`crates/runner-daemon/src/session/manager/direct_spawn.rs`).** Introduce `DirectLaunch` (deriving `Default`) for the `SessionManager` direct-spawn options: `runtime_override`, `model_override`, `effort_override`, `speed_override`, resolved `project_id`, resolved `cwd`, `cols`, `rows`, `first_turn`, `initial_prompt`, plus the role-backed vs. runtime-direct distinction (`persisted_role_id` / `emit_activity` or a clear mode field/method). Collapse all 4 `spawn_direct*` wrappers, all 4 `spawn_runtime_direct*` wrappers, and `spawn_direct_inner` onto the struct-based entry point (keeping a clean `pub(crate)` `after_row_insert` hook for the worktree gate) and remove all 10 `#[allow(clippy::too_many_arguments)]` in `direct_spawn.rs`.
2. **Ops layer (`crates/runner-daemon/src/ops/session.rs`).** Replace the 8 positional functions (`session_start_direct`, `session_start_direct_with_speed`, `session_start_direct_impl_with_speed`, `session_start_direct_impl_with_speed_and_prompt`, `spawn_direct_chat_with_prompt`, `session_start_runtime`, `session_start_runtime_with_speed`, `session_start_runtime_with_speed_and_prompt`) with a clean struct-based ops entry point (using an ops-level options struct with `Default` — keeping `ProjectScope` and `WorktreeSpec` in ops rather than passing dead fields into `SessionManager` — and keeping raw `Option<String>`/`&str` runtime handling so invalid runtime names on legacy protocol or companion paths still hit `resolve_runtime_override` / `runtime_direct_role` validation instead of silently turning into `None` via `Runtime::parse`). Route `session_start_chat` and `session_start_shell_in` cleanly through the updated layer(s), and remove all 8 `#[allow(clippy::too_many_arguments)]` in `ops/session.rs`.
3. **Wire protocol compatibility (`crates/runner-core/src/protocol/api.rs`).** Keep the wire request names, parameter lists, and return types of `session_start_direct`, `session_start_direct_with_speed`, `session_start_runtime_with_speed`, and `session_start_chat` completely unchanged. Update only the 3 daemon-side handler closures (lines 145–147) to invoke the new `ops::session` entry point.
4. **Exact behavior preservation.** Verify every subtle difference between role-backed and runtime-direct launches remains unchanged:
   - `persisted_role_id` (`Some(role.id)` vs `None`) and `emit_activity` (`true` vs `false`).
   - `first_turn` composed from `role.system_prompt` only on role-backed launches.
   - Error context strings `"session_start_direct"` vs `"session_start_runtime"` (asserted by `crates/runner-cli/tests/goldens/cli.txt`).
   - `session/updated` event emission on the runtime-direct and shell paths (unchanged on role-backed).
   - `onboarding::mark_completed_on_activity` skipped for `Runtime::Shell`.
   - `worktree_gate().read()` guard lifetime across row insertion and `#[cfg(test)] fire_before_registration()`.
5. **Caller migration (~183 references across ~38 files).** Update `crates/runner-daemon/src/companion/actions.rs` and all daemon/app unit, integration, benchmark, and smoke tests (`crates/runner-daemon/src/session/manager/tests/*`, `session/manager/terminal/tests.rs`, `ops/session/client_tests.rs`, `ops/runtime.rs`, `ops/runtime_tasks.rs`, `remote/channels/tests.rs`, `crates/runner-app/src/surfaces/agent_recovery.rs`, `crates/runner-app/src/terminal/element.rs`, and `crates/runner-app/tests/{session_manager_integration,terminal_benchmark,pi_runtime_smoke,copilot_runtime_smoke}.rs`) to construct `DirectLaunch { ..., ..Default::default() }`.

Non-goals: changing client-protocol request names or wire shapes; refactoring `resume.rs`, `mission_spawn.rs`, or `fork.rs`; separating shell and agent launch preparation (#795); changing any runtime behavior, CLI output, or goldens.

## Validation

- **Automated (`@coder`):**
  - `make verify` (covers workspace check, tests, clippy across `--all-targets`, and `fmt --check`).
  - `git diff --check`.
  - Confirm `rg 'too_many_arguments' crates/runner-daemon/src/ops/session.rs crates/runner-daemon/src/session/manager/direct_spawn.rs` returns zero matches and no new `too_many_arguments` allows were introduced elsewhere.
  - Confirm ignored smoke/benchmark test targets (`pi_runtime_smoke`, `copilot_runtime_smoke`, `terminal_benchmark`) compile cleanly under `--all-targets`.
- **Live QA:** Not required. Pure behavior-preserving refactor with no UI or wire-format change, covered by the daemon, app, and CLI golden test suites.

## Authorization

Approved crew: `agy duo` (neon duo). Review runs first on unpushed local commits in the worktree (`@reviewer` stays idle until `@coder` hands off a local commit SHA). Once `@reviewer` reports no remaining must-fix findings: `@coder` folds this brief and any review fixes cleanly into the branch commit history on `refactor/1427-direct-launch`, pushes to `origin`, opens a PR against `main` with `Fixes #1427`, sets labels `no-behavior-change` and `risk-low` (keeping `core-review` added automatically by workflow), confirms `@reviewer` applies `reviewed` and comments the reviewed head SHA on the PR, and drives CI green on all platforms (`gh pr checks <pr> --watch`). Do not merge — stop once the PR is open, reviewed, labeled, and CI is green.
