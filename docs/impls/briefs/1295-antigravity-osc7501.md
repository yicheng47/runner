# Mission Brief — #1295 Use OSC 7501 Instead of Status Hooks for Antigravity CLI

- **Issue & Draft PR:** [#1295](https://github.com/yicheng47/runner/issues/1295) / PR [#1296](https://github.com/yicheng47/runner/pull/1296)
- **Feature spec:** `docs/features/1295-antigravity-osc7501.md`
- **Starting point:** worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1295-antigravity-osc7501`, branch `feat/1295-antigravity-osc7501` (`ecf6c2c6`, off `origin/main` `95d74efa`). Keep the root checkout on `main` and work strictly in this worktree with its own `target/` (never a shared `CARGO_TARGET_DIR`).

## Problem And Goal

Antigravity CLI (`agy 1.5.8+`) natively emits OSC 7501 terminal program status (`working`, `blocked` with `kind=permission|question|auth` + base64 `msg`, `done`, `error`, `idle`), but Runner still wires 4 lifecycle hooks (`PreInvocation`, `PostToolUse`, `PostInvocation`, `Stop`) via `--add-dir antigravity-hooks`. Because `SessionModel` gives hooks precedence over OSC 7501 (`hook_status_armed`), Runner misses interactive waits (`approval`/`question`/`auth`), misclassifies `Stop` / `USER_CANCELED` and raw `Esc` bytes, and pays synchronous hook + `#787` `--add-dir` overhead while `agy` already handles conversation IDs via `--log-file` (`KeyCapture::LogTail`).

Implement `docs/features/1295-antigravity-osc7501.md` in full on top of `origin/main` (`95d74efa`), retiring Antigravity status hooks and `--add-dir antigravity-hooks` while promoting OSC 7501 (`ProgramStatus`) across direct chats, completion badges, `StartupDelivery`, mission `turn_started`/`NeedsYou`, Settings UI, docs, and regression tests.

## Read First

1. `docs/features/1295-antigravity-osc7501.md` — authoritative design, out-of-scope rules, and verification scope.
2. `crates/runner-daemon/src/runtimes/antigravity/mod.rs`, `agy_status.rs`, `agy_capture.rs`, `tests.rs` — Antigravity adapter, legacy hooks/cleanup, and `LogTail` capture.
3. `crates/runner-daemon/src/session/state/mod.rs`, `tests.rs` — `SessionModel`, `hook_status_armed`, `ProgramStatus`, `Activity`/`TurnOutcome` transitions.
4. `crates/runner-daemon/src/session/manager/events.rs`, `startup.rs`, `status.rs`, `spawn_spec.rs` — completion notification eligibility (`#861`), `StartupDelivery`, `turn_started`, command preview (`OptionKey::StatusHooks`).
5. `crates/runner-core/src/protocol/runtime.rs` & `crates/runner-app/src/surfaces/settings/agents/launch.rs`, `tests.rs` — `RuntimeLaunchOption::StatusHooks` & Settings Launch UI.
6. `docs/guide/reference/supported-agents.md` (+ `docs/guide/zh-CN/reference/supported-agents.md`), `docs/arch/runtime-integration.md`, `docs/tests/regression/runtimes.md`.

## Deliverables And Boundaries

- Retire Antigravity `StatusHooks` implementation (`status_hooks() -> None`), remove `antigravity_status_args`, `agy_status.rs`, `#787` `WORKSPACE_CONTEXT_ENV`, and legacy `antigravity-hooks` directory cleanup on daemon startup/housekeeping without altering `KeyCapture::LogTail` (`--log-file`) or user configs.
- Remove `Runtime::Antigravity` from `RuntimeLaunchOption::StatusHooks`, Settings Launch option UI/copy/previews (`launch.rs`), and affected goldens/tests (`spawns.json`, `permissions.rs`, `settings/agents/tests.rs`, `replay.rs`), keeping other runtimes' `StatusHooks` intact.
- Promote OSC 7501 (`ProgramStatus`) for managed hookless agent sessions (`min_version: None`, soft fallback to output baseline on older `agy`): arm completion on `Working` and trigger `record_session_completion` on `State::Done` (`TurnOutcome::Completed`) without triggering on `Interrupted`/`Failed` or bare shell/hand-launched `#861` tabs (`RT-STATUS-14`); hold/release `StartupDelivery` deterministically on `ProgramStatus` with `STARTUP_OUTPUT_GRACE` fallback; increment `turn_started` on `State::Working` and preserve `NeedsYou` delivery holds on `State::Blocked`.
- Update `docs/guide/reference/supported-agents.md`, `docs/guide/zh-CN/reference/supported-agents.md` (bilingual parity), `docs/arch/runtime-integration.md`, and `docs/tests/regression/runtimes.md`; add thorough daemon, state, startup, golden, and UI tests.
- Parallel awareness: `#1292` runs concurrently in another worktree; build purely against `origin/main` without assuming `#1292` has merged, and never touch the other worktree.

## Validation And Handoffs

1. **Coder:** implement cleanly, pass `make fmt`, `make guide-cli-check`, workspace Clippy (CI profile + `--features updater`), and `cargo nextest run --locked --workspace --cargo-profile ci` (plus `make verify` checks). Do NOT launch `make run`. Fold this brief (`docs/impls/briefs/1295-antigravity-osc7501.md`) into an implementation commit alongside the code changes (never leave a standalone brief commit in final history) while preserving `docs/features/1295-antigravity-osc7501.md` and `docs/features/README.md`. Hand candidate SHA to Reviewer.
2. **Reviewer:** review diff, state-machine invariants (`core-review` paths: `session/state/`, `session/manager/`, `runtimes/`), `#861` / `RT-STATUS-15` non-regression, `StartupDelivery` fallback, and bilingual docs without running the app; pass clean candidate SHA + live QA cases (`RT-LIFE-01`, `RT-LIFE-04`, `RT-LIFE-05`, `RT-LIFE-08`, interactive `blocked` waits `permission`/`question`/`auth`, `RT-START-01`/`RT-START-04` replacement check without `--add-dir antigravity-hooks`, `RT-STATUS-14`, `RT-STATUS-15`) to `qa`.
3. **QA (`qa`):** acquire `script/qa-lock acquire --timeout 540` (another `agy trio` mission may share this lock, so wait politely if held), run `make run` in this worktree, preserve any pre-existing dev sessions/daemon untouched, execute the selected live checks with bounded tiny prompts in `/tmp/...`, clean up only QA-owned sessions/missions and temporary trust/wrapper state, release `script/qa-lock release` once app + daemon exit (or follow operator/Jason lock policy if a pre-existing dev chat keeps daemon alive), and add a clearly identified run block in `docs/tests/runs/2026-11-13-mac.md` (alongside any existing run blocks in that file, never overwriting earlier entries) plus PR `#1296` results.

## Authorization

`agy trio` is authorized for `#1295` / draft PR `#1296`: once review and macOS QA pass cleanly, fold the brief cleanly into implementation commits (using `git push --force-with-lease` to `origin/feat/1295-antigravity-osc7501` if rewriting pushed draft history), update PR `#1296` description with live QA results, replace draft placeholder labels (`no-behavior-change`, `risk-low`) with `risk-high` (preserving automatic `core-review`, removing `needs-live-test` only if all required checks passed per `AGENTS.md`, and having `reviewer` add `reviewed` on the final head SHA), mark PR `#1296` ready (`gh pr ready 1296`), drive CI green on all platforms, and stop at the open PR without merging.
