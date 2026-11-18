# #1385 — Keep remote workspace and terminal control across Settings detour

Jason approved `hydrogen trio` on 2026-11-18. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1385-remote-settings-keep-control` on `fix/1385-remote-settings-keep-control` (base `origin/main` `09c259b8`, dedicated `target/`). Sibling worktrees `.worktrees/fix-1377-daemon-ui-stall-isolation` (#1377) and `.worktrees/test-flaky-process-probe-tests` are active; never inspect or edit them, and do not run heavy daemon-spawning test suites concurrently with a sibling run.

## Problem and outcome

In [#1385](https://github.com/yicheng47/runner/issues/1385), opening this computer's Settings while controlling a remote terminal — via the sidebar footer gear, the host popup's **Remote access settings**, or update/usage/notification Settings actions — calls `NativeRoot::open_this_computer_settings` (`crates/runner-app/src/surfaces/remote_client/bridge.rs`), which invokes `return_to_this_computer` and sets `self.remote_client.workspace = None`. That immediately destroys `RemoteWindow`, drops every `RemotePane` and `AttachmentLifetime`, releases terminal control on the host (`{computer} has left`), and causes **Back** in Settings (`leave_settings`) to land on this computer instead of the remote workspace. Even manually switching back to the remote computer rebuilds `RemoteWindow` with an empty `take_sessions`, losing terminal control when the host or another viewer is present and forcing a cold reconnect/reattach.

Settings is a short detour for configuring **this computer**. Opening Settings from a remote window must keep the window's active `RemoteWindow` — its open route, selected tab/mission/split pane, `RemotePane`s, `AttachmentLifetime`s, and remote mission attachments — alive underneath the Settings takeover, and leaving Settings (**Back** or Escape) must return straight to that remote workspace with the same terminals attached and controlled, without reconnecting or taking control again.

## Read first

- Issue [#1385](https://github.com/yicheng47/runner/issues/1385).
- Remote/Settings transitions (`crates/runner-app/src/surfaces/`): `remote_client/bridge.rs` (`open_this_computer_settings`, `return_to_this_computer`, `remote_unobstructed`, `sync_remote_state`), `settings/page/mod.rs` (`enter_settings_pane`, `enter_settings_route`), `app_shell/actions.rs` (`leave_settings`, `open_settings`, `page_navigation_state`, and `AppRoute::Settings` shortcut guards), and `app_shell/mod.rs` (`render_settings_takeover`, `settings_confirm_overlay`, and overlay/shortcut visibility while `route == AppRoute::Settings`).
- Remote lifecycle & local escapes (`crates/runner-app/src/`): `surfaces/settings/remote.rs` (`leave_other_computers_when_signed_out`, `Forget` / `Reset remote access` on the currently shown computer), `surfaces/settings/archived.rs` (restoring/opening a local item from Settings while a remote workspace is underneath), `notifications.rs` (clicking a local chat/mission alert vs a Settings alert), and `surfaces/windowing/mod.rs` (`current_subjects`, `sync_window_activation`).
- Existing tests & docs: `crates/runner-app/src/surfaces/app_shell/settings_button.rs` (`remote_isolation_tests`), `crates/runner-app/src/surfaces/remote_client/tests.rs` (`remote_access_settings_row_opens_this_computers_remote_pane` and command-palette `REMOTE-WS-04` isolation), `docs/guide/guides/remote-access.md` (and `docs/guide/zh-CN/guides/remote-access.md`), and `docs/tests/regression/remote.md` (`REMOTE-WIN-01`).

## Deliverables

1. **Keep `RemoteWindow` and terminal attachments live across Settings:**
   - Opening this computer's Settings from a remote window (`open_this_computer_settings` — footer gear, host popup's **Remote access settings**, update/usage/notification Settings entry points — and `OpenSettings` / `⌘,` if no remote command-palette isolation rule conflicts; keep remote Command Palette free of host Settings per `REMOTE-WS-04`) enters `AppRoute::Settings` and renders `render_settings_takeover` + `settings_confirm_overlay` **without** calling `return_to_this_computer` or dropping `self.remote_client.workspace`.
   - Every Settings pane continues to read and mutate **this computer's** `AppStore`, `DaemonClient`, and local settings — never the remote host.
   - While `self.route == AppRoute::Settings`, treat the covered remote workspace as obstructed/inactive for keyboard actions, popovers/modals/overlays, and `mark_remote_sessions_viewed` (`remote_unobstructed()` must be `false` while on `AppRoute::Settings`), removing stale `&& self.remote_client.is_local()` assumptions on `AppRoute::Settings` guards in `app_shell/{mod.rs,actions.rs,sidebar.rs}` so typing/shortcuts in Settings never leak into the hidden remote workspace or open the remote command palette.

2. **Clean return on `leave_settings` and clean teardown when invalidated in Settings:**
   - When leaving Settings (`leave_settings` via **Back** or Escape) while `self.remote_client.is_active()`, restore the pre-Settings route, re-focus the active remote terminal, mission surface, or remote root cleanly, and mark unobstructed active remote sessions viewed if the window is active — keeping the exact remote tab/mission, split focus, and terminal control with zero detach/reattach or host `{computer} has left` transition.
   - If while inside Settings the user signs out (`leave_other_computers_when_signed_out`), resets remote access, or forgets the remote computer this window was showing, tear down `self.remote_client` via `return_to_this_computer` while remaining on `AppRoute::Settings` (preserving `REMOTE-LOCAL-05`), so leaving Settings afterward lands cleanly on this computer's local route.
   - If an action inside Settings explicitly opens a **local** workspace item (such as opening a restored local chat/mission from Settings → Archived) or a local chat/mission notification is clicked while in Settings, call `return_to_this_computer` before navigating so the window switches cleanly to this computer.

3. **Docs and regression suite updates:**
   - Update `docs/guide/guides/remote-access.md` and `docs/guide/zh-CN/guides/remote-access.md` (where they currently say the gear switches the window back to this computer before opening Settings) and verify parity with `python3 script/check-guide-translations.py`.
   - Update `docs/tests/regression/remote.md` (`REMOTE-WIN-01`) to reflect that opening Settings from a remote window opens this computer's Settings and going back returns to the remote computer with open terminals and control intact.

## Validation and live checks

- **Automated:** Add deterministic `runner-app` tests (`VisualTestContext::run_until_parked` / `wait` helpers only, zero fixed sleeps; loop every new async/UI test ≥ 20× and state the count) covering:
  1. Controlling a remote tab (including a split pane) and a remote mission → opening Settings via the footer gear and via the host popup's **Remote access settings** → verifying `AttachmentLifetime` / host `Controller` is never dropped while in Settings and hidden unread remote sessions are not marked viewed → leaving Settings (**Back** / Escape) → verifying immediate return to the remote tab/mission with focus and terminal control preserved and zero extra attach/take-control calls.
  2. Signing out or forgetting that host while in Settings tears down the remote workspace cleanly while staying on Settings, and **Back** then lands on this computer's local route; restoring/opening an item from Settings → Archived returns the window to this computer.
  3. Existing `remote_isolation_tests` (`settings_button.rs`) and `remote_client::tests` updated and green.
  Run `cargo test -p runner-app`, `make clippy`, `cargo fmt --all --check`, `python3 script/check-guide-translations.py`, and `git diff --check`.
- **Live QA (`@qa` after clean review):** Acquire `script/qa-lock acquire --timeout 540` (note sibling mission `#1377` may also use `qa-lock`; wait cleanly if held), preserve pre-existing dev sessions, and record screenshots in `docs/tests/runs/1385-remote-settings-keep-control-mac.md` on the **same-Mac two-host relay fixture** (`docs/tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos` — two private-`HOME` `.app` wrappers enrolled with the hosted relay via Chrome's signed-in Runner account; no local relay and no second physical computer):
  1. Open Host from Client, take control of a running chat tab (confirm Host shows **Controlled from {client}**), click the Settings gear on Client, change or inspect a local setting, and click **Back**: verify Host never flashes **{client} has left**, Client returns directly to the remote tab still in control, and typing immediately arrives on Host (`REMOTE-TERM-01` / `#1385` repro).
  2. Repeat via the host button popup's **Remote access settings** → **Back**, then revoke/forget only run-owned disposable enrollments and remove `/private/tmp` wrappers.

## Authorization

Approved crew: `hydrogen trio` (`coder`, `reviewer`, `qa`). Fold this brief into the implementation commit, push `fix/1385-remote-settings-keep-control`, and open a PR against `main` (`Fixes #1385`) carrying `frontend-only` (assuming changes stay in `runner-app` and `docs/`), `risk-medium`, `reviewed` (`@reviewer` on final head), and `needs-live-test` only if Windows native live QA remains unrun. Do not merge.
