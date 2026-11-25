# #1485 — Keep claude_accounts tests from starting the real `claude auth login`

## Starting point

- Issue: [#1485](https://github.com/yicheng47/runner/issues/1485) (0.20)
- Branch: `fix/1485-claude-accounts-test-isolation`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1485-claude-accounts-test-isolation` (base `origin/main` `1d6e9b5c`, own `target/`).
- Other live worktrees: `refactor-1431-claude-accounts-seam` (PR [#1484](https://github.com/yicheng47/runner/pull/1484), which moves `claude_accounts.rs` to `runtimes/claude_code/accounts/mod.rs`), `refactor-1429-start-chat-methods`, `chore-1477-gpui-pre-0.3.14` and `fix-818-terminal-round-trips`. Leave them alone.
- Whichever of #1484 and this PR merges second rebases onto `origin/main`; git follows the rename, so fix any leftover conflict by hand in the moved file. If #1484 has merged by the time you start, rebase first and work in the new location.
- Fold this brief into the fix commit before the final push.

## Problem and outcome

The authenticating test cores in `crates/runner-daemon/src/claude_accounts.rs` (`AccountService::default()`, `authenticate: true`) point the Claude Code override at a fake script inside a `tempfile::tempdir()`, but leave `PATH` unisolated. When a test ends or fails while a sign-in worker thread is still running, the temp folder and the fake script are deleted. The worker's `command()` then falls back to the detected executable, which `direct_chat_path` finds on the developer's real `PATH`. It runs the real `claude auth login` with `CLAUDE_CONFIG_DIR` set to the deleted test folder, which opens Claude Code's OAuth page in the default browser, and `prepare_headless_fork` detaches the process so it never exits. Eight such processes had piled up on Jason's Mac by 2026-11-25; the issue has the evidence.

Outcome: no daemon test can run a real agent CLI, and every sign-in worker a test starts has exited before that test's temp folder is dropped. Production behaviour does not change, including the deliberate fallback from a missing override to the detected executable.

## Read first

- `crates/runner-daemon/src/claude_accounts.rs`: `fn command`, `fn login`, `fn auth_status_for_job`, `LoginJob`, `stop_logins`, and the tests `slow_login_core`, `sign_out_core`, `login_uses_the_selected_folder_and_cancel_stops_only_its_process`, the `AddAndLogin` test and `shutdown_rejects_login_requests_delayed_before_registration`
- `crates/runner-daemon/src/runtime_status.rs`: the effective-command fallthrough and `direct_chat_path` with `TEST_ISOLATE_PATH_ENV`
- `crates/runner-daemon/src/ops/runtime_tests.rs`, around the test that sets `AccountService::default()`: the isolated-`PATH` pattern to copy
- `crates/runner-daemon/src/test_support.rs` (`test_core_in`)

## Deliverables

1. **Isolate `PATH`.** Every test core that can authenticate gets `TEST_ISOLATE_PATH_ENV` and a `PATH` holding only its temp `bin` (or nothing), so a missing fake resolves to "Claude Code is not installed", never to a real binary. Prefer one shared helper over repeating the setup in each test. Check the rest of `runner-daemon` for other test cores that run an agent CLI through an override with an unisolated `PATH` (`grep` for `AccountService::default()`, `set_runtime_override` and `authenticate`), and fix those the same way.
2. **Join workers.** Each test that starts a sign-in, status or sign-out worker waits for it to exit (for example `Arc::strong_count(&job) == 1`, or `stop_logins` followed by a wait) before its temp folder drops, including when an assertion fails early. A drop guard on the fixture is fine.
3. **Regression test.** Add a deterministic test showing that an authenticating test core whose fake executable has been deleted fails with "not installed" instead of resolving another `claude`, and that no sign-in process is started.
4. **Tests only.** No production code changes, unless a test-only seam is unavoidable; if so, keep it behind `#[cfg(test)]` and explain it in the PR.

Non-goals: changing the production override fallback, the sign-in flow or `prepare_headless_fork`.

## Validation and handoffs

- **@coder:** `cargo test -p runner-daemon`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`. Stress-loop the changed and new tests with `cargo nextest run -p runner-daemon --stress-count 20 -E 'test(claude_accounts)'` and state the count. Before and after the loop, record `pgrep -fl 'claude auth login'`: it must show no new process. Other missions run heavy suites on this Mac, so run the loop when none of their `cargo test` is running. Hand `@reviewer` an unpushed commit SHA.
- **@reviewer:** Confirm that no test core in `runner-daemon` can reach a real agent executable, that every started worker is joined before its temp folder drops (including on assertion failure), that the regression test is deterministic, and that production behaviour is unchanged. Test-only, so no development-app live checks.
- **@qa:** No development-app checks and no QA lock, since nothing user-visible changes. On the reviewed candidate, independently:
  1. Record `pgrep -fl 'claude auth login'`, run `cargo nextest run -p runner-daemon --stress-count 20 -E 'test(claude_accounts)'` in the worktree, and confirm the loop passes and no new `claude auth login` process or browser sign-in page appears. Reason: this is the leak the issue reports.
  2. Run the new regression test alone and confirm it passes. Then temporarily remove only the new `PATH` isolation in your own scratch copy of the test helper (never committed, restored afterwards) and confirm the test fails. Reason: proves the test guards the fallback.

  Record results in `docs/tests/runs/1485-claude-accounts-test-isolation-mac.md` and report the verdict to `@coder`.

## Authorization

Approved crew: `neon trio` (`coder`, `reviewer`, `qa`). After `@reviewer` and `@qa` pass, `@coder` may push the branch and open a PR against `main` with `Fixes #1485`, labelled `no-behavior-change` and `risk-low`, and drive CI green on both platforms. Do not merge.
