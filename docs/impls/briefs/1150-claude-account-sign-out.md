# #1150 — Sign out of a Claude Code account

Jason asked for a codex trio to build [#1150](https://github.com/yicheng47/runner/issues/1150) on 2026-11-09. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1150-claude-account-sign-out` on `feat/1150-claude-account-sign-out`, based on `origin/main` at `22436acd`.

## Problem and outcome

Runner can sign a Claude Code account in but not out; Remove only forgets it. Add **Sign out** to each signed-in account's **⋯** menu, above Remove. It always asks first, runs `claude auth logout` with the account's `CLAUDE_CONFIG_DIR`, and leaves the row **Not signed in** with **Log in**. The spec, [`docs/features/1150-claude-account-sign-out.md`](../../features/1150-claude-account-sign-out.md), holds the scope, the exact copy and the decisions Jason signed off (sign out is allowed for the active and the native account with a note, a line counts running sessions, Remove is unchanged). The design is `runner/desktop/specs/1150-claude-account-sign-out.pen` in `yeelinks_design`; where the two differ, the spec wins.

## Read first

- The spec above.
- `crates/runner-core/src/protocol/claude_accounts.rs`: `ClaudeAccountAction`, `ClaudeSessionAccount`.
- `crates/runner-daemon/src/claude_accounts.rs`: `action`, `command`, `login`, `auth_status_for_job`, the Cancel branch, and the tests that use a fake `claude` script (`login_uses_the_selected_folder_and_cancel_stops_only_its_process`).
- `crates/runner-app/src/surfaces/settings/claude_accounts.rs`: `menu_items`, `account_menu` (index-based dispatch), `row`, `AccountSharingOverlay`, `account_error`, and its tests.
- `crates/runner-app/src/ui/overlay.rs`: `ConfirmDialog`.
- `docs/guide/guides/claude-accounts.md` and its `zh-CN` counterpart.

## Deliverables

1. `ClaudeAccountAction::SignOut { id }` and its daemon handling, as the spec's "Signing out" section describes, with daemon tests using a fake `claude`.
2. The menu item, the confirmation (notes, running line, Danger button, busy state), the running-session count, the dispatch that holds with and without the conditional items, the new `accounts.keep_folder` copy, and English and Simplified Chinese strings, with app tests.
3. The guide pages, in English and Chinese.

**Non-goals:** listed in the spec. No change to Remove, Log in, sharing, Make main or the usage popover beyond what the refreshed status already does.

## Validation

`runner-daemon` and `runner-app` tests, workspace Clippy with warnings denied and `cargo fmt --all --check`. Drive macOS and Windows CI green.

## Live checks (qa, macOS development app)

**Never run a real `claude auth logout`, and never sign out an account folder that existed before the test.** They are Jason's real sign-ins, shared with the installed Runner and his terminals. All checks run against a stub:

- Before launching, record the development app's **Settings → Agents → Claude Code → Executable** value. After launch, point it at a stub script in `/tmp/1150-qa/`. The stub answers: `--version` with the installed `claude --version` output; `auth status` with `{"loggedIn":true,"email":"qa@example.test","orgName":"QA","subscriptionType":"pro"}` when a marker for its `CLAUDE_CONFIG_DIR` (`native` when unset) exists in `/tmp/1150-qa/state`, otherwise `{"loggedIn":false}` with exit 1; `auth logout` removes the marker, appends the config dir (or `native`) and the args to `/tmp/1150-qa/calls.log` and exits 0; `auth login` prints `https://claude.ai/oauth/authorize?fixture=1`, waits two seconds, creates the marker and exits 0; anything else runs `exec sleep 3600`, which stands in for a running session.
- Add test accounts with **Add account** and folders under `/tmp/1150-qa/`, and create their markers by hand. To make the native account read as signed in, create the `native` marker. Do not use the existing non-native accounts.
- At cleanup, switch the active account back to the one recorded at the start, remove the test accounts, restore the Executable value, and archive any test sessions.

Checks:

- **L1 Menu:** a signed-in test account's ⋯ shows Sign out under the separator, directly above Remove; it is absent on a signed-out account and while one is signing in. Every other item still does what its label says, including Stop using {main}'s settings on a linked account.
- **L2 Sign out:** the confirmation shows the spec's title and body with the account's email and folder. Cancel leaves it signed in. Sign out logs that folder in `calls.log`, the row turns Not signed in with Log in and no email line, and the account leaves the usage popover's account dropdown. Log in then signs it in again.
- **L3 Running sessions:** with one session running on the account, the confirmation shows the one-session line; with none, the line is absent.
- **L4 Active account:** Sign out on the active test account shows the active note; afterwards the row keeps Active with Log in.
- **L5 Native account:** Sign out on the native account shows the `~/.claude` warning, and `calls.log` records `native` (no `CLAUDE_CONFIG_DIR`).
- **L6 Remove:** Remove on a non-protected test account is still immediate, and the note under the list shows the new wording.

## Handoffs

The coder implements, commits locally and hands each candidate to `reviewer`. After a clean review it hands the same commit to `qa` with this section. qa takes the qa lock and reports pass or fail per check with screenshots. Failures go back to the coder, and a fix is re-reviewed before qa reruns only the affected checks. The PR uses `Fixes #1150` and records the live results. Expect `core-review` from the protocol change, and add `risk-medium`; the PR is not `frontend-only`.

## Authorization

The codex trio may commit, push this branch and open one PR against `main`. It does not merge.
