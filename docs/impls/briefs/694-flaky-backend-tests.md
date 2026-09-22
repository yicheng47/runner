# 694 — Make three flaky backend tests deterministic

Fix [#694](https://github.com/yicheng47/runner/issues/694). Jason asked for a codex-crew mission that ends in an open PR, not a merge. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-694-flaky-backend-tests`, branch `fix/694-flaky-backend-tests`, whose tip is this brief on top of main `f5536fe`. Do not create another branch or checkout, touch the root checkout, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

Read first: AGENTS.md; issue #694 (`gh issue view 694`), which is the spec and wins on any detail; each test below with its helpers.

## The three tests

Each failed only under load and passed in isolation. The bar for all three: the test fails when the behaviour it guards breaks, never because the machine or runner is busy. Fix the tests, not production code. No `#[ignore]`, no retry loops around a whole test, no weakened assertion beyond the timing that caused the flake.

1. **`session::hook_feed::tests::powershell_reporter_loses_no_record_when_appends_overlap`** (Windows only; `hook_feed.rs`, reporter body in `powershell_reporter`). It guards the seek race the per-feed mutex closed. On a slow runner ids 25–27 went missing cleanly, which fits the reporter's `WaitOne(1000)` giving its report up by design, not a lost append. Make the test tell those apart. The reporter leaves its payload file behind only when its append landed and removes it when it gives up, so an append that took the mutex and then vanished leaves an orphaned payload with no record. Asserting that no acquired append was lost or torn, while dropped reports are allowed, is one way; choose your own if it is sounder, and say why. Keep the test meaningful: it must still prove appends actually overlapped, not pass because every hook gave up. The production reporter, its one-second wait and its drop-don't-stall behaviour stay unchanged; if the only fix you can find needs a change there, stop and escalate to Jason with `runner ask --human "…"`, giving the options and your recommendation. The sibling tests `powershell_reporter_waits_for_the_feed_mutex` and `powershell_reporter_drops_its_report_when_the_feed_mutex_stays_held` must keep what they pin.
2. **`session::manager::tests::codex_pre_hook_startup_ignores_continuing_idle_redraw`** (`session/manager/tests.rs`). `wait_for_output_event` gives a real PTY spawn 2 s to produce its first byte, and it has ten call sites. That deadline only bounds the failure case, so make it generous. Check the file's other wait helpers: raise a deadline only where it bounds arrival, never where the timing is the behaviour under test.
3. **`runtime_status::models::tests::process_tests::spawn_failure_timeout_and_cache_write_failure_are_contained`** (`runtime_status/models/tests.rs`). The fixture script writes `pid`, then `exec sleep 30`, while `run` times out at 2 s; under load the child was killed before its first line, so reading `pid` hit NotFound. Keep asserting that a timed-out child returns `Reason::Timeout` and is killed, but do not charge process startup to that timeout. Look for the same pattern in the module's other process tests and fix it where it applies.

Out of scope: production behaviour, the reporter script, CI workflow files, and flaky tests not named in #694. If you meet another flake, list it in the handoff instead of fixing it.

## Validation

Run `cargo test --locked -p runner-backend --profile ci --no-fail-fast`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`, and report each exit code. Log to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Tests 2 and 3 run on macOS: show each one's fix under load. Run the test repeatedly under a bounded CPU load, before the change where you can reproduce the failure and after it, and report the counts. Every load generator must be bounded (a timeout or a trap that kills it), and before any handoff confirm with `pgrep` that none is still running; orphaned busy loops from an earlier crew's stress test once made this machine unusable.

Test 1 compiles and runs only on Windows, so macOS clippy prunes its `#[cfg(windows)]` code: check imports and helpers by reading them. Its only execution is the Windows CI job. After the PR is open and CI has passed, rerun the Windows job twice (`gh run rerun <run-id> --job <job-id>`) and report all three results.

Do not start, stop, restart or type into Jason's Runner apps, direct chats or mission sessions, and do not run the dev app. Do not change agent configuration.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against this brief and the issue, must-fix findings first with file:line pointers; for test 1 it verifies the drop-versus-loss reasoning against `powershell_reporter` itself. Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes: commit the work on this branch in focused commits (imperative subject, scope `session` or `validation`, no co-author trailers), `git push -u origin fix/694-flaky-backend-tests`, and `gh pr create --base main` with `Closes #694`, a summary per test (root cause, fix, why the test still guards its behaviour), the load-test counts, the three Windows results and anything unverified; no agent session links in the body. Then `gh pr checks <n> --watch` and poll until nothing is pending; if CI fails, fix it on the branch, have the reviewer check the fix, and push again. **Do not merge**, do not delete the branch or worktree, do not cut a nightly or release.

Final handoff to everyone through Runner: PR URL and CI results including the Windows reruns, each test's root cause and fix, changed files, checks with exit codes, load-test counts, and the reviewer's verdict. Then both slots stand by.
