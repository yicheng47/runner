# 752 — Weekly usage pill shows enabled runtimes, not recent ones

Fix [#752](https://github.com/yicheng47/runner/issues/752). Jason requested this mission on 2026-09-30. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-752-usage-pill-runtimes`, on the existing branch `fix/752-usage-pill-runtimes`, created from `origin/main` at `d485284`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory. Another crew is working in `.worktrees/feat-735-start-chat-modal`; treat it as another machine's checkout.

## The bug

The sidebar's weekly usage pill (added in #747, PR #749) picks its runtimes from `usage_recent`, the runtimes with recorded sessions ordered by recent activity, and only falls back to every installed, enabled runtime when none has a session. So an enabled, installed Antigravity (or Claude Code, or Codex) that has never run in a session is left out of the pill until a session spawns. The usage popover the pill opens already uses the right rule; the pill disagrees with it.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- Issue #752.
- `crates/runner-app/src/surfaces/app_shell.rs`: `usage_installed` (about line 118, which already limits to `ClaudeCode | Codex | Antigravity` with a detected or configured command), the popover's `sections` filter (about line 633), `visible_usage_runtimes` (about line 231), its call site (about line 1045) and its test `usage_pill_selects_recent_runtimes_but_displays_runtime_order`.
- `crates/runner-app/src/main.rs`: the `usage_recent` field, its initialiser, and its refresh on `session/spawned`.
- `crates/runner-backend/src/ops/session.rs` and `repo/session.rs`: `recently_used_runtimes` and its test. Its only caller is `usage_recent`.
- `docs/features/706-agent-usage.md`, the #747 paragraph that describes the recent-activity selection.

## Deliverables

1. **One visibility rule.** A runtime appears in the pill when it supports usage (`ClaudeCode`, `Codex`, `Antigravity`), is installed (`usage_installed`), and is enabled in Settings (`model_runtimes()`), whether or not it has ever run in a session. Display order stays Runner's runtime order (`Runtime::ALL`: Codex, Claude Code, Antigravity). The pill and the popover sections must use the same function so they cannot drift again. Filter to the usage-capable set explicitly, so a non-usage runtime can never be selected even if it reaches the installed or enabled lists. With only three usage runtimes the old `take(3)` cap does nothing; drop it rather than keep a cap nobody chose. When nothing qualifies the pill stays hidden, as today.
2. **Remove the recent-activity path.** Delete `usage_recent` from `main.rs` (field, initialiser, `session/spawned` refresh) and `recently_used_runtimes` from `ops/session.rs` and `repo/session.rs` with its test, since nothing else uses them. Keep the `session/spawned` handling that other features need.
3. **Tests.** Replace the old test with one for the new rule: an installed, enabled runtime with no sessions appears; a disabled or uninstalled one does not; a non-usage runtime such as Copilot never appears; order follows `Runtime::ALL`; nothing qualifying yields an empty list. Cover the pill and popover agreeing if the shared function makes that cheap to assert; do not build new test harnesses for it.
4. **Docs.** Rewrite the #747 paragraph in `docs/features/706-agent-usage.md` to state the enabled-and-installed rule and cite #752. Leave `docs/tests/747-antigravity-followups.md` alone; it is a dated record of what ran then.

Keep the change scoped to this. No UI or layout change, no design file edits, no README change unless a README states the old rule (check both `README.md` and `README.zh-CN.md`; they change together).

## Boundaries

Crews never run the dev app or drive Jason's Runner for UI checks; Jason smoke-tests. Do not start extra agents, crews or subagents.

## Review, verification and authorization

The coder owns implementation and checks. The reviewer waits for an explicit Runner handoff, then reviews the full branch diff against #752 with must-fix findings first and file:line pointers. Focus on the pill and popover sharing one rule, no leftover references to the removed path, and no regression in other `session/spawned` handling. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

Run `cargo test --locked -p runner-app --profile ci` and `cargo test --locked -p runner-backend --profile ci`, workspace Clippy with warnings denied (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`), macOS updater Clippy (`--features updater`), `cargo fmt --all --check`, and `git diff --check`. Record the exact commands and exit codes. Windows CI has repeatedly failed on imports or helpers used only by `cfg(unix)` tests; gate any such import or helper with `#[cfg(unix)]`.

After a clean review, Jason authorizes squashing all work on this branch, this brief included, into one commit on top of current `origin/main` with a subject that names the fix (for example `fix(ui): show enabled usage runtimes in the weekly usage pill`), pushing `fix/752-usage-pill-runtimes`, and opening a PR against main whose body says `Fixes #752`. If main has moved, rebase; never merge main into the branch. Review or CI fixes after the push are amended into the same commit and pushed with `git push --force-with-lease`. Drive CI green on macOS and Windows. Do not merge, delete the branch or worktree, or cut a nightly or release. Final Runner handoff: PR URL, what changed, tests and exit codes, CI result, the reviewer's verdict, and what Jason should smoke-test (Antigravity enabled but never run shows in the pill; disabling it in Settings hides it). Then stand by.
