# 644 — Antigravity runtime continuation

Continue [#644](https://github.com/yicheng47/runner/issues/644) and the existing [PR #714](https://github.com/yicheng47/runner/pull/714). Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-644-antigravity-runtime` on `feat/644-antigravity-runtime`. Do not create another branch or checkout, touch the root checkout, or share another worktree's Cargo target directory.

## Starting state

The branch was rebased onto the latest fetched `origin/main` (`ba33bf2`, Runner 0.12.2) before this mission. The old PR was green on macOS and Windows before that rebase; those checks do not validate the new tree. The rebase resolved overlaps in role tests, session deletion, CLI runtime mapping, and `docs/arch/arch.md`. Confirm the branch remains based on `origin/main` before editing; if main moves again before the final push, fetch and rebase onto its newest tip. Never merge main into the branch.

The prior mission implemented the adapter, settings/identity and macOS hook status. Read `AGENTS.md`, `docs/features/644-antigravity-runtime.md`, `docs/impls/briefs/644-antigravity-runtime.md`, `docs/tests/644-antigravity-smoke.md`, and the current PR body. The spec wins on runtime behavior. PR #714 already exists; update it rather than opening another PR.

## Work

1. Validate the rebased feature against current main. Run the relevant backend, app, terminal and CLI tests, workspace Clippy, updater-feature Clippy on macOS, format check and `git diff --check`. Record exact commands and exit codes. Fix integration regressions caused by the rebase. Inspect the complete `origin/main...HEAD` feature diff and resolve any other non-live omissions that the spec and tests expose.
2. Preserve the new main behavior in the four conflict areas. In particular, session deletion must retain the current transaction and attention cleanup while deleting an Antigravity log only after the row transaction succeeds. Keep the latest role page tests, plus Antigravity permission coverage. Keep CLI runtime names and architecture text accurate.
3. Hand the complete branch diff and any working-tree fixes to the reviewer through Runner. The reviewer checks the entire feature against the spec and current main, with must-fix findings and file:line pointers. Iterate until the reviewer says `NO REMAINING MUST-FIX ISSUES`.
4. After clean review and green local gates, squash this feature's branch commits, including both briefs, into one focused commit on top of main, then update the existing PR with `git push --force-with-lease`. Update its body with current test evidence and remaining smoke items. Watch both platform CI jobs to completion. Route substantial CI fixes back through review, amend the same commit, push with lease again, and repeat until green.

The live `agy` checks remain on Jason's smoke checklist: key capture and resume, trust and permission behavior, hook payloads and Orca interaction, wheel behavior and the terminal fixture, and Windows enablement. Do not launch a live `agy` session or edit Jason's `~/.gemini` configuration. Do not edit `.pen` files or invent the provider mark; the current placeholder awaits Jason's design. Do not run or control Jason's Runner app. If a non-live gap needs those actions, document it precisely in the PR and handoff.

## Authorization and handoff

This mission is authorized to edit the existing branch, run checks, squash its commits, push the rebased branch with `--force-with-lease`, and update PR #714 after clean review. Do not merge, delete the branch or worktree, cut a release, or start another crew or subagent. The crew stops at an open PR with green CI and reports the PR URL, changed files, checks and exit codes, review verdict, and any remaining live smoke or design work through Runner.
