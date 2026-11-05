# 1163 — Drop Antigravity usage support

Fix [P2 #1163](https://github.com/yicheng47/runner/issues/1163). Jason asked on 2026-11-05 for a claude crew. Instead of teaching Runner to detect agy's sign-in, Antigravity CLI loses usage support and becomes a secondary runtime.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1163-agy-usage-signin`, on branch `fix/1163-agy-usage-signin`. Its tip is this brief, on top of main `a06bd8a8`.

## Problem

The usage poller runs `agy -p /usage --output-format json` for every detected, usage-enabled runtime. A signed-out agy treats that probe as a session start and opens its browser login ("Waiting for you to sign in..."), with no flag or environment variable to suppress it. agy has no `auth status` command, and it keeps its token in the OS keychain through go-keyring. The fallback file `~/.gemini/antigravity-cli/antigravity-oauth-token` can be stale, so it is no sign-in signal. Jason decided not to support agy usage at all.

## Uncommitted draft in the worktree

The chat that wrote this brief left an uncommitted, unreviewed draft of the change in this worktree. App, core and daemon usage tests and workspace clippy passed on it; the full daemon suite has not been run. It is yours to keep, rework or replace: review it as your own work, not as an approved starting point.

## Deliverable

1. **No agy usage probe.** `Antigravity` has no `UsageSource`; `runtime_metadata::antigravity::capabilities()` drops `usage`. The daemon never runs `agy -p /usage`; the `usage_catalog_golden` row reads `supported: false` with no commands.
2. **No agy usage UI.** The sidebar footer and usage popover show only Claude Code and Codex. Remove what only served agy: its `runtime_ui` usage fields, the `usage_groups` popover grouping, the agy unavailable lines and the agy-only i18n keys in both locales (`ui.antigravity_cli_didn_t_answer`, `ui.antigravity_cli_returned_invalid_usage_data`, `usage.gemini_models`, `usage.claude_gpt_models`). Claude's scoped windows such as `Opus · week` keep rendering.
3. **Protocol.** Remove `UnavailableReason::AntigravityNoAnswer`. The usage last-known-good row stores values only, never reasons, so saved data still loads; confirm this. Any saved agy value is simply no longer shown.
4. **Tests and goldens.** Update the tests that used agy as a usage runtime (footer fit, popover scroll, weekly percent, visible and popover runtime order) so they still test what they tested, with Claude Code and Codex. Regenerate goldens with `RUNNER_UPDATE_GOLDEN=1` (daemon) and `RUNNER_UPDATE_CATALOG_GOLDEN=1` (app), and read the diffs.
5. **Docs.** `docs/guide/reference/supported-agents.md` and its zh-CN copy: Claude Code and Codex are the primary integrations; Antigravity CLI is secondary, and its usage indicator row reads No. Look for other guide pages that promise agy usage.

Out of scope: agy sign-in detection, other agy features, README edits beyond what accuracy requires.

## Validation

- `cargo nextest run --locked -p runner-app -p runner-core -p runner-daemon --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check` and `git diff --check`

Live checks for `qa`, in the development app with agy installed and enabled:

1. The sidebar footer and usage popover show Claude Code and Codex only, without an Antigravity row or error. Reason: deliverable 2.
2. A usage refresh starts no `agy` process and no agy login: watch `ps` for `agy -p /usage` while pressing refresh, and confirm no browser sign-in page opens. Reason: the bug itself.
3. Claude Code and Codex usage still load and their windows render. Reason: the shared popover code changed.

## Authorization

Approved: the claude trio. The crew may commit, push and open the PR against `main`, labelled `bug`, `risk-medium` and `needs-live-test` until qa passes; the commit closes #1163.
