# #1227 — ⌘W on a remote single-chat tab asks before archiving

Jason asked for a claude duo to fix [#1227](https://github.com/yicheng47/runner/issues/1227) on 2026-11-09. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1227-remote-tab-close-confirm` on `fix/1227-remote-tab-close-confirm`, based on `origin/main` at `0c4d2e479d3226c4d416e560cf42fa93265574c8`. Two trios are fixing #1224 and #1225 at the same time in their own worktrees.

## Problem

In a window showing another computer, ⌘W (Ctrl+Shift+W on Windows and Linux) on a tab holding a single chat archives the chat at once, with no confirmation. The same shortcut confirms on a local tab and on a chat pane in a remote split. From reading the code, not yet confirmed with a test: `close_tab` in `surfaces/windowing/windows.rs` sends every remote close to `close_remote_front_page`, which reaches `request_close_chat_pane` (and its remote confirmation branch) only for a tab with more than one pane. A single-chat tab falls to `request_archive_all`, whose remote path archives immediately because `archive_confirmation_counts` returns `None` when there are no terminals. The issue has the line references.

## Outcome

⌘W on a remote single-chat tab opens the same archive confirmation a local tab gets, with Enter to confirm and Esc to cancel, and sends the `archive` request only after confirmation. Unchanged: closing a remote terminal stays immediate (`immediate-terminal-close`), remote splits, the drawer and the mission workspace behave as today, and sidebar **Archive** for chats keeps skipping the dialog, locally and remotely.

## Read first

- `crates/runner-app/src/surfaces/front_page/actions.rs`: `close_remote_front_page`.
- `crates/runner-app/src/surfaces/chat/close.rs`: `request_close_chat_pane`, `archive_chat_pane_target`, the remote branch of `request_archive_all`, and the local `request_close_single_pane_tab` for the behavior to match.
- `crates/runner-app/src/surfaces/sidebar/archive.rs`: `archive_confirmation_counts`, which must not change.
- `crates/runner-app/src/surfaces/remote_client/tests.rs`: the `immediate-terminal-close` test and its fake host.

## Deliverables

1. In `close_remote_front_page`, send a tab whose only session is an agent chat to the remote confirmation, most likely `request_close_chat_pane`. Confirm that a confirmed close archives that chat and leaves the tab state the same as a confirmed local close would. Do not change `archive_confirmation_counts` or `request_archive_all`.
2. A remote test beside `immediate-terminal-close`: ⌘W on a single-chat remote tab opens the dialog and sends no `archive`; Esc cancels with no `archive`; confirming sends exactly one `archive` for that chat. The test must fail without the fix; say so in the handoff.
3. Check `docs/guide/reference/keyboard-shortcuts.md` and `docs/guide/guides/remote-access.md`; update one only if it describes ⌘W on a remote tab differently from the new behavior.

**Non-goals:** no change to local close paths, sidebar Archive, remote terminal close or the dialog's design.

## Validation

`runner-app` tests (including the remote client tests), workspace Clippy with warnings denied and `cargo fmt --all --check`. Drive macOS and Windows CI green.

## Live check (Jason)

Crew slots do not contact the hosted service, so neither slot runs the app. Jason runs this check himself, and the PR keeps `needs-live-test` until it passes:

- **L1:** in a window showing another computer, select a tab holding one chat (no split, no drawer terminal) and press ⌘W. The archive confirmation opens; Esc keeps the chat, and Enter archives it. Closing a remote terminal tab with ⌘W is still immediate.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. Fold review fixes into the commit and re-review until clean. The PR uses `Fixes #1227`, names L1 for Jason, and carries `risk-low`, `frontend-only` and `needs-live-test`.

## Authorization

The claude duo may commit, push this branch and open one PR against `main`. It does not merge.
