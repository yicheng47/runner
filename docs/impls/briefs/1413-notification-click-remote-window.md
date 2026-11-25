# #1413 — A local notification click never takes over a remote window

Jason approved `neon trio` on 2026-11-24. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1413-notification-click-remote-window` on `fix/1413-notification-click-remote-window` (base `origin/main` `3edbffa4`, its own `target/`). The root checkout stays on `main`. Other worktrees under `.worktrees/` belong to other work; never inspect or edit them.

## Problem and outcome

While a window is viewing a remote host, clicking a system notification for a local chat, mission member, question or idle mission disconnects that window from the host and switches it back to this computer.

Cause, in `notifications::open` (`crates/runner-app/src/notifications.rs`): only the `displaying` candidate filters on `root.remote_client.is_local()`. The fallbacks (`holding`, the `"main"` window for a chat target, `cx.active_window()` otherwise, then `windows.first()`) can all pick a window that is connected to a remote host, and `open` then calls `root.return_to_this_computer(window, cx)` on it before opening the target.

Outcome: a local notification click opens its target in a window that is already on this computer, never in a remote one. Pick the most recently focused local window, keeping today's preference order among local windows (one displaying the target, one holding the chat, `"main"`, the active window, any window). When every open window is on a remote host, open a new local window (`open_new_runner_window` in `surfaces/windowing/windows.rs`, the ⇧⌘N path) and open the target there. A remote window stays connected and keeps its route, tabs and terminal control. Notifications keep firing while you work in a remote window (`visible()` already treats remote windows as not showing local content; keep that).

Also check the other notification paths in `open` against the same rule and fix any that disconnect a remote window: the `usage:` path (`open_this_computer_settings`, which #1385 made safe across a Settings detour; confirm it does not disconnect) and the `remote-paired:` path (`enter_settings_route(Some("remote"))` on the active window).

## Read first

- Issue #1413.
- `crates/runner-app/src/notifications.rs`: `open`, `visible`, `Target`.
- `crates/runner-app/src/surfaces/windowing/windows.rs`: `open_new_runner_window`, `handle_reopen`.
- `crates/runner-app/src/surfaces/remote_client/bridge.rs`: `NativeRoot::return_to_this_computer`, `open_this_computer_settings`.
- `crates/runner-app/src/surfaces/remote_client/tests.rs`: `remote_windows_do_not_suppress_local_notifications_and_clicks_return_to_local_content` and `settings_archived_and_local_notification_return_to_this_computer`, which assert today's behavior and must change.
- `crates/runner-app/src/notifications/tests.rs` for the existing window-selection tests.

## Deliverables

1. **Window selection.** Exclude windows whose `remote_client` is active from every candidate in `open`; when no local window exists, open a new local window and open the target in it. Remove the `return_to_this_computer` call from this path. A notification for a target that no longer exists still does nothing harmful to any window.
2. **Tests.** Update the two tests above to the new behavior, and cover: the only window is remote (a new local window opens with the target; the remote window stays connected on its route); a remote `"main"` window plus a local window (the local one is used); a remote active window plus a local one, for mission targets; the usage path with a remote window (it stays connected). Follow the repo's rule on racing background work: check results through `h.wait(…)`, no fixed sleeps. If a new test starts background work, run it under `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'` and state the count.
3. **Guide.** If `docs/guide/` describes what clicking a notification does while remote, update it and its zh-CN page; otherwise say none needed in the PR.

Non-goals: changing when notifications fire, notification content, remote-host notifications, or the remote client's own return-to-this-computer button.

## Validation and live checks

- **Automated:** `cargo test -p runner-app notifications`, `cargo test -p runner-app remote_client`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`; CI green on macOS, Windows and Linux.
- **Live QA (`@qa` after a clean review).** Remote control is involved, so run on the [same-Mac two-host relay fixture](../../tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos), taking `script/qa-lock acquire --timeout 540` first. Each check exercises the changed window selection through a real macOS notification click:
  1. **Only window is remote:** client window A on host B; on A's own computer, let a small local test chat finish unviewed so its notification posts; click it. A new local window opens on that chat; window A is still on host B with its tab and terminal control intact.
  2. **A local window exists:** open a second, local window, return focus to the remote window A, let another local chat finish, click. The local window comes forward on the chat; A stays on host B.
  3. **No remote windows (regression):** with only local windows, a click still opens the chat as before.
  4. **Usage alert while remote:** if a usage alert can be triggered safely on the fixture, click it while A is remote; Settings → Usage opens and A stays on host B. If it cannot be triggered without touching real account limits, record it as not run with the reason.

  Record the run in `docs/tests/runs/1413-notification-click-remote-window-mac.md`. Windows and Linux notification clicks are not live-tested here; state that gap in the PR.

## Authorization

Approved crew: `neon trio` (coder, reviewer, qa). Commit on `fix/1413-notification-click-remote-window`, folding this brief into the implementation commit, and open a PR against `main` with `Fixes #1413`, `frontend-only`, `risk-medium` and `needs-live-test`. `@reviewer` adds `reviewed` on the final head; `@qa` removes `needs-live-test` only when checks 1–3 pass. Drive CI green, then stop. Do not merge.
