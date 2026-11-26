# #1485 — Claude account test isolation — macOS QA

## 2026-11-26, Asia/Shanghai

**Passed. NO REMAINING MUST-FIX QA ISSUES.** The reviewed candidate passed both assigned checks. The isolated regression failed as expected when the test helper stopped setting the isolation flag.

Candidate `39ed73294e056daa6b4efaa0f0d67c9be40237cd`, branch `fix/1485-claude-accounts-test-isolation`, worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1485-claude-accounts-test-isolation`. The worktree was clean before validation and remained clean until this record was added. Environment was macOS 26.6.2 (25G83), arm64, Rust 1.98.1 and cargo-nextest 0.9.148. No other Cargo or nextest process was running at stress-test launch.

| Check | Result | Evidence |
| --- | --- | --- |
| Stress the Claude account tests to reproduce the reported process and OAuth leak | Passed | `cargo nextest run -p runner-daemon --stress-count 20 -E 'test(claude_accounts)'` passed 20 of 20 iterations, with 37 tests per iteration and 740 passing executions. Before and after, `pgrep -fl 'claude auth login'` matched only the same mission processes, Antigravity PID 43423 and Codex PID 43431, whose arguments contain the brief. The 240 process and browser samples found no Claude auth login process and no launch of Zen, the default HTTPS handler for the Claude OAuth URL. `stress.log`, `process-summary.txt`, `process-observations.txt`, `default-browser.txt`. |
| Run the deleted-override regression alone to verify it detects the fallback | Passed | `cargo test -p runner-daemon --lib deleted_override_fails_as_not_installed_without_falling_back_to_path_claude` passed 1 of 1 on the reviewed candidate. The test covers the missing-executable error and asserts that the spawn count stays unchanged and the decoy never runs. `regression.log`. |
| Remove the isolation flag in a scratch copy to verify the regression fails for the intended reason | Passed | Archived the reviewed SHA into scratch and removed only the `TEST_ISOLATE_PATH_ENV` insertion in `test_support::isolate_shell_env`. Built with `cargo test -p runner-daemon --lib --no-run --message-format=json` using a separate target directory, then ran the exact test binary with `PATH=/usr/bin:/bin`. The single test exited 101 at `claude_accounts.rs:1371` because command resolution found the temporary decoy `claude`. It failed before the resolved command could run. The scratch helper was restored byte for byte. `mutation.patch`, `mutation-test.log`, `mutation-restore.txt`. |
| Development app and other platforms | Skipped | The assigned QA was macOS daemon test validation. It used no development app, QA lock, real agent account or browser interaction. |

Raw evidence lives at `/private/tmp/runner-1485-qa.eyE8QchK`. QA made no source changes in the branch worktree and left this record for the coder to include in the PR.
