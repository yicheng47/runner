# #984 — Agent usage survives a runnerd restart

Jason requested a `claude duo` mission (coder: Claude Code, reviewer: Codex) on 2026-11-02. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-984-usage-survives-restart` on `fix/984-usage-survives-restart`, based on `origin/main` at `966632396d5a752ec4058959662057d16cbdb866`. The root checkout stays on `main`; use this worktree's own `target/`.

## Problem

Agent plan usage (the footer pill and the Usage popover) lives only in runnerd's memory (`UsageState` in `crates/runner-daemon/src/usage.rs`). Anything that restarts the daemon loses it: Quit & Stop Sessions, an update restart, `runner daemon stop`, a reboot, and every `make run`. The pill and popover then show "Checking…" until the first fetch answers, and if that fetch fails (offline, Anthropic unreachable, Keychain denied, Codex not answering) the agent shows its unavailable line instead of its last numbers, because "a failed refresh keeps the last good numbers" only holds within one daemon run. [Issue #984](https://github.com/yicheng47/runner/issues/984) is the source of truth for scope; it reverses #706's in-memory decision with Jason's approval (2026-10-21). Only percentages and reset times are stored, never a token.

## Outcome

- The last good usage for each runtime, and for each Claude account, survives a daemon restart and is shown with its "updated … ago" age until the launch refresh replaces it.
- A failed launch fetch keeps the restored numbers, as a failed refresh already does within a run.
- A restored window whose `resets_at` has passed is dropped, so a stale percentage is never shown for a window that has already reset; an agent left with no windows is dropped.

## Read first

- `crates/runner-daemon/src/usage.rs`: `UsageState` (`snapshot`, `claude_accounts`, `active_claude`), `fetch`, `apply_result`, `start_scheduler`, and the `refresh_clock_and_failed_fetch_keep_good_numbers` test.
- `crates/runner-daemon/src/db/app_state.rs`: the `_app_state` key/value table and its typed JSON helpers; `login_shell_env_lkg` is the last-known-good precedent to follow.
- `crates/runner-daemon/src/daemon/boot.rs`: where `UsageService` is created and the scheduler started.
- `crates/runner-core/src/protocol/usage.rs`: `AgentUsage`, `UsageWindow`, `RuntimeUsage`, `UsageSnapshot` (already `Serialize`/`Deserialize`).
- `crates/runner-app/src/surfaces/app_shell/usage.rs`: the app renders `snapshot.get(runtime)` values and the popover age from the newest `updated_at`; it does not depend on `last_fetch_at`, so no app change is expected. Confirm this rather than assume it.

## Deliverables

- Store the last good usage as JSON in `_app_state` under a new key, with typed get/set helpers beside the existing ones. No schema change or migration.
- Store only good values: each runtime's `AgentUsage` and each Claude account's `AgentUsage`. Never store errors, `refreshing`, `keychain_denied`, the denied-account set or `last_fetch_at`; those stay per run.
- Write after each fetch that produced at least one good value, saving all current good values (a runtime that failed this time keeps its earlier stored value). Write outside the `UsageState` lock.
- On boot, load into `UsageState` before the scheduler starts, applying the expiry rule above. Do not set `last_attempt_at`, so the launch refresh still runs. With several Claude accounts, the snapshot's Claude Code entry is the active account's; keep `select_claude_account` and the post-fetch active-account copy consistent with what was restored.
- A corrupt or unreadable stored value logs a warning and starts empty; it never fails boot.
- Tests: a round trip through `_app_state`; expired windows and empty agents dropped on load; a failed first fetch keeping the restored numbers (through `apply_result` or the fetch path); restored Claude account usage selected for the active account.
- Docs: update `docs/guide/guides/chats-and-windows.md` (footer usage paragraph) only if a user-visible sentence is now wrong or missing, in both English and `docs/guide/zh-CN/`. Note in the PR that `docs/features/archive/706-agent-usage.md`'s in-memory decision is superseded; don't rewrite the archived spec.

Non-goals: no app/UI change, no new protocol field, no change to fetch timing, intervals or the Keychain flow, no usage history, no unrelated refactors. `db/app_state.rs` and `daemon/boot.rs` are core paths, so the PR gets `core-review` automatically; keep the core diff minimal and pick the risk label from what the diff does.

## Validation

Name the changed behaviors and the check that covers each before running anything (AGENTS.md → Test Scope). Run `runner-daemon` tests covering usage and `_app_state` and any affected boot tests, `cargo fmt --all --check`, `git diff --check` and workspace Clippy with `-D warnings`. If `session::manager::tests::launch_definition::claude_*` fails locally, check whether `CLAUDE_CONFIG_DIR` is set in your environment; those tests leak it and pass with it unset. They are unrelated to this change. Coders and reviewers do not run the app. The reviewer decides from the diff whether a live check is needed. If one is, name it in the PR for Jason, since this crew has no qa slot. Drive macOS, Windows and Linux CI green.

## Roles and authorization

The coder implements and validates, then hands the full diff to the reviewer through Runner. The reviewer reviews the code only and returns findings or a clean review naming any live checks. Once the review is clean, Jason authorizes the crew to commit, push this branch and open one PR against `main` that closes #984. Fold this brief into the implementation commit. Stop at the open PR with CI green and report the PR URL, final SHA, changes and validation through Runner. Do not merge, release, archive the mission, or remove the branch or worktree.
