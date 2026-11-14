# Mission Brief — #1318 Fix runner-app test FD leak under cargo test

- **Issue:** [#1318](https://github.com/yicheng47/runner/issues/1318) (`bug: runner-app tests run out of open files under plain cargo test`).
- **Starting point:** worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1318-runner-app-test-fds`, branch `fix/1318-runner-app-test-fds`, off `origin/main` (`627797fd`). Work only in this worktree, with its own `target/` (never share `CARGO_TARGET_DIR`).
- **Parallel isolation:** sibling mission `#1315` is active in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1315-remote-unread-dot` (`fix/1315-remote-unread-dot`). Never edit or assume changes from that branch, and do not touch the root checkout on `main`.

## Problem And Goal

On `main` (`627797fd`), `cargo test -p runner-app --lib --no-fail-fast` fails 122 of 1,464 tests on macOS (`ulimit -n` = `10240`) with `Too many open files (os error 24)` and watcher timeouts (`surfaces::files::tests`). Sampling `lsof` on the running test binary confirms open descriptors climb steadily (`98` → `2,288` within 7.5s even with `--test-threads=1`), with ~99% of leaked FDs being `runner.db` / `runner.db-wal` descriptors held by `DbPool`/`AppCore` instances that outlive their tests. Fix the root descriptor leak so plain `cargo test -p runner-app` and `make test` pass reliably without descriptor exhaustion.

## Read First

1. `crates/runner-app/src/test_support.rs` — `core`, `DrainTasksOnDrop`, and `drain_tasks_when_app_drops`.
2. `crates/runner-app/src/app_store.rs` — `AppStore::new` (detached tasks holding `InProcessTransport(core)` and `drain_tasks_when_app_drops(cx)` registration order vs `GlobalAppStore`), plus `test_lifecycle_store`.
3. `crates/runner-app/src/surfaces/start_chat/mod.rs` and other `TestAppContext::single()` harnesses setting `GlobalAppStore` after `AppStore::new` (note `FxHashMap` global drop order can drop `DrainTasksOnDrop` before `GlobalAppStore` drops `AppStore`).
4. `crates/runner-app/src/bootstrap.rs` (`startup_removes_stale_mcp_bridge_and_tolerates_a_missing_one`) and `crates/runner-daemon/src/daemon/boot.rs` (`boot_core`, `start_runtime_watchers`, `start_background_discovery`) — `first/runner.db` / `second/runner.db` stayed open in `lsof`.
5. `crates/runner-daemon/src/db/mod.rs` — `open_fixture_pool` (`build_pool(manager, 8, true)` opening 8 WAL connections per fixture).

## Deliverables And Boundaries

- **Fix fixture teardown & FD accumulation:**
  - Ensure `AppStore` / `AppCore` / `DbPool` and test tasks/watchers are actually dropped/drained when each `runner-app` test ends (regardless of GPUI global `FxHashMap` drop order or `TestAppContext` vs `#[gpui::test]`), and ensure `bootstrap::tests` (`boot_core`) does not leak background pools/threads across tests.
  - Evaluate and adjust `open_fixture_pool` pool sizing (`build_pool` max connections) if appropriate for test concurrency without breaking multi-connection fixture tests, while still fixing the actual leak so FD count stays bounded across the entire `runner_app` test run rather than growing linearly with test count.
  - Keep `Makefile`'s `make test` running `cargo test --workspace` (do not mask the leak by switching `make test` to `nextest`).
- **Verification tests:** add/keep regression assertions or harness checks verifying fixture teardown does not retain `AppCore`/`DbPool` across tests, and confirm `lsof` FD counts stay bounded across `cargo test -p runner-app --lib`.
- **Non-goals:** production runtime behavior changes unrelated to test teardown/fixture pools, switching `make test` away from `cargo test`, or touching `runner-app` UI layout/behavior.
- **Commit hygiene:** fold this brief commit into your implementation commit(s) before final push so PR history contains clean change commits and no standalone brief commit.

## Validation And Handoffs

1. **coder:** run `make fmt`, `cargo test -p runner-app --lib --no-fail-fast`, `cargo test --workspace` (or `make test`), `cargo nextest run --locked --workspace --cargo-profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and the `--features updater` clippy pass. Also sample `lsof -p <pid> | wc -l` during `runner-app` tests to verify FD count stays bounded and does not climb monotonically. Do not run the live app. Hand the candidate SHA to `reviewer`.
2. **reviewer:** review the diff, focusing on GPUI global/task teardown ordering, whether any test `AppCore`/`DbPool`/`SessionManager` watcher still leaks, whether `open_fixture_pool` sizing preserves WAL/concurrent fixture test semantics, and production-code neutrality (`#[cfg(test)]` / `test-support` boundaries). Hand the clean SHA forward once review is clean; no live app QA checks are needed unless the diff unexpectedly alters production runtime behavior.

## Authorization

The `agy duo` is authorized for [#1318](https://github.com/yicheng47/runner/issues/1318): commit, push `fix/1318-runner-app-test-fds`, open a PR against `main` closing `#1318`, apply the appropriate `risk-*` label (and `no-behavior-change` if only tests/test-support change, or keep any automatic `core-review` if `runner-daemon` non-test code is touched), apply `reviewed` for the final head, and stop at the open PR with CI green. Do not merge.
