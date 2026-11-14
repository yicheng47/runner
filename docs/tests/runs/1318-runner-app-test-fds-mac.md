# #1318 Fix runner-app test FD leak under cargo test — macOS validation

## Summary and scope

Branch `fix/1318-runner-app-test-fds` in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1318-runner-app-test-fds` (branched off `627797fd`) eliminates the file-descriptor leak that caused `cargo test -p runner-app --lib --no-fail-fast` on macOS (`Darwin arm64`, `26.6.2`, `ulimit -n = 10240`) to fail 122 of 1,464 tests on `main` with `Too many open files (os error 24)` (`98` -> `2,288` open descriptors in `7.5s` even under `--test-threads=1`).

This is an automated-only bug-fix validation record on macOS arm64. No live application launch or computer-use QA check was run because the final diff preserves existing runtime recovery (`NativeRoot._event_tasks` retains the existing `chat_events` and `app_events`/`runtime_client` tasks rather than altering event-lag recovery semantics) and makes no UI or user-facing behavior change.

Cross-platform coverage on Windows and Linux is deferred to GitHub Actions CI once the pull request opens.

## Nextest omission and test serialization

Local `cargo nextest run --locked --workspace --cargo-profile ci` (listed in `docs/impls/briefs/1318-runner-app-test-fds.md`) was skipped on explicit instruction from `@human` / `@manager` (`01M7CMACDJDV7ACY8VMTV42ZQ7`): `cargo test` (`make test`) is sufficient locally, and concurrent workspace suites were serialized across active missions (`#1318` and `#1315`) so spawned daemon/CLI subprocess tests do not contend for CPU and time out. Earlier overlapping `nextest` execution against a running `make test` hit CPU-contention subprocess timeouts in `runner-daemon` `claude_accounts::tests` and `runner-cli` `golden.rs` and was canceled so `make test` could run uncontended.

## Round 1 — pre-review candidate `f81594ff`

Validated `f81594ffc04405ec3eb108c0245d841683c9dd72` cleanly before review, then released the serialized workspace test slot to `#1315`.

| Check | Result | Observation |
| --- | --- | --- |
| `env -u RUNNER_SESSION_CLI make test` (`cargo test --workspace`, solo) | Passed | Zero failures across every crate: `runner-app` (`lib` 1,464 passed / 0 failed / 1 ignored in `34.58s`; `bundle_mac` 6; `pane_layout` 20; `session_manager_integration` 4; `terminal_ime` 3; `text_util` 6; 3 ignored smoke/benchmark tests), `runner-cli` (`lib` 12; `bin` 87; `daemon_process` 24 passed / 2 ignored in `114.24s`; `golden` 188 in `399.28s`; `hook_process` 6; `remote` 3; `roundtrip` 31), `runner-core` (`lib` 182 in `11.06s`; `daemon_environment` 0 failed), `runner-daemon` (`lib` 1,934 passed / 0 failed in `40.99s`), `runner-terminal` (`lib` 149 passed / 1 ignored; `delivery_probe_replay` 1; `fixture_replay` 12; `input_state_replay` 7; `osc_color_query` 9; `snapshot` 34 in `25.19s`), doc-tests (all 0 failed). `RUNNER_SESSION_CLI` was unset so the outer `0.19.8` mission shim did not leak into `roundtrip::a_missing_or_unrunnable_session_cli_falls_through` expecting `0.19.9` (`roundtrip.rs` left untouched as out of scope). |
| `cargo fmt --all --check` | Passed | Exit 0. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Passed | Exit 0. |
| `cargo clippy --locked -p runner-app --features updater --all-targets --profile ci -- -D warnings` | Passed | Exit 0. |
| `lsof` FD sampling on `runner_app` (`--test-threads=1`) | Passed | 1,464 passed / 0 failed / 1 ignored (`106.46s`), 203 samples: total FDs `min=8`, `max=43`, `last=28`; `runner.db*` FDs bounded at `0..4`. |
| `cargo nextest run --locked --workspace --cargo-profile ci` | Skipped | Omitted per `@human` / `@manager` instruction (`01M7CMACDJDV7ACY8VMTV42ZQ7`); `cargo test` (`make test`) used instead. |

## Round 2 — post-review revision (`NativeRoot._event_tasks`)

Review of `f81594ff` identified two P2 issues in the `surfaces/root/init.rs` runtime-event change (lossy `runtime/resync` coalescing and synchronous `runtime_task_list()` on the GPUI foreground thread). Both were resolved by restoring the original background `runtime_client` + `EventError::Lagged(_)` snapshot code and retaining all four spawned event tasks in `NativeRoot._event_tasks` (`crates/runner-app/src/surfaces/root/mod.rs` and `crates/runner-app/src/surfaces/root/init.rs`) instead of detaching them.

| Check | Result | Observation |
| --- | --- | --- |
| `cargo test -p runner-app --lib test_support::tests::app_store_and_native_root_release_db_pool_on_test_teardown` | Passed | 1 passed / 0 failed (`0.09s`), verifying `App` and `DbPool` weak references reach `strong_count() == 0` when `TestAppContext::single()`, `AppStore`, and `NativeRoot` drop. |
| `cargo fmt --all --check` | Passed | Exit 0. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Passed | Exit 0. |
| `cargo clippy --locked -p runner-app --features updater --all-targets --profile ci -- -D warnings` | Passed | Exit 0. |
| `cargo test -p runner-app --no-fail-fast` | Passed | Exit 0: `runner_app` `lib` (`1464 passed; 0 failed; 1 ignored` in `29.21s`), `bundle_mac` (`6 passed`), `pane_layout` (`20 passed`), `session_manager_integration` (`4 passed` in `1.38s`), `terminal_ime` (`3 passed`), `text_util` (`6 passed`), plus the 3 intentionally `ignored` manual/credential binaries (`copilot_runtime_smoke`, `pi_runtime_smoke`, `terminal_benchmark`). |
| `lsof -n -P -p <pid>` sampling (`0.5s` interval) on `target/debug/deps/runner_app-e8964e3f8fd1927c --test-threads=1` | Passed | Exit 0 (`1464 passed; 0 failed; 1 ignored` in `100.53s`), 195 samples: `MIN_TOTAL=7`, `MAX_TOTAL=40`, `LAST_TOTAL=30`, `MAX_DB=20`, `LAST_DB=8`. Sampled progression: `sample=1 total=7 db=0`, `sample=20 total=22 db=0`, `sample=40 total=24 db=4`, `sample=60 total=24 db=4`, `sample=80 total=24 db=4`, `sample=100 total=22 db=0`, `sample=120 total=24 db=4`, `sample=140 total=25 db=4`, `sample=160 total=28 db=4`, `sample=180 total=25 db=4`. Descriptor count remained flat across all 1,464 unit tests with no leak. |
| `make test` (`cargo test --workspace`) | Not re-run after Round 2 | Last full workspace run passed on `f81594ff`; not re-run locally after Round 2 because the serialized workspace test slot was released to `#1315` and Round 2 touched only `crates/runner-app/src/surfaces/root/{mod.rs,init.rs}` (`_event_tasks`) and this run log. GitHub Actions CI will execute the full multi-platform workspace suite when the PR opens. |
