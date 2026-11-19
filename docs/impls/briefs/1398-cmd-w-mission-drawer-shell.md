# Brief: #1398 — Close focused mission drawer shell on ⌘W

## Starting point

- **Issue:** https://github.com/yicheng47/runner/issues/1398 (`bug`, `P1`, `agent-ready`)
- **Branch / worktree:** `fix/1398-cmd-w-mission-drawer-shell` at `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1398-cmd-w-mission-drawer-shell`
- **Base commit:** `4c3f06c5057f4b5b546eaded89a75f51b15c2e04` (`origin/main`)
- **Parallel isolation:** Sibling worktrees exist at `.worktrees/fix-1395-remote-project-drag` (PR #1397) and `.worktrees/test-flaky-process-probe-tests`. Stay strictly inside this worktree, never assume unmerged peer branches landed, and never share `CARGO_TARGET_DIR` (if `coder` works while `qa` holds a candidate build, use `CARGO_TARGET_DIR=target/coder` inside this worktree).

## Problem and expected behavior

In a mission with the terminal drawer open and one of its shells focused, pressing ⌘W (Ctrl+Shift+W on Windows/Linux) hides the whole drawer while leaving the shell running and its tab open.

Expected behavior:

1. With focus in a mission terminal drawer shell, ⌘W / Ctrl+Shift+W closes the active drawer shell, matching the shell tab's `×` (`request_close_terminal_drawer_shell` / `close_terminal_drawer_shell`).
2. If other shells remain, focus moves to the drawer's next active shell.
3. Closing the last shell hides the drawer and returns focus to the mission (clearing any stale `drawer_was_focused`, on both member tabs and Feed).
4. If the drawer is open and focused with `active_shell() == None`, fall back safely to hiding the drawer.
5. Both **local** and **remote** mission workspaces must match this behavior, respecting remote `"close"` capability checks.
6. Non-drawer behavior stays unchanged: with focus elsewhere in the mission, ⌘W keeps its current behavior (`CloseTarget::None` locally; `close_mission_session_tab` remotely via `close_visible_tab`), the drawer hide button still hides without closing shells, and ⌥F12 / Alt+F12 still toggles visibility without closing shells.

## Read first

1. `crates/runner-app/src/surfaces/windowing/windows.rs` (`close_tab` — `CloseTarget::MissionDrawer` calls `workspace.hide_terminal_drawer`).
2. `crates/runner-app/src/surfaces/mission_workspace/input.rs` (`close_visible_tab` — remote mission close calls `self.hide_terminal_drawer` when `drawer_focused` is true).
3. `crates/runner-app/src/surfaces/front_page/actions.rs` (`close_remote_front_page` dispatches remote `CloseTab` to `workspace.close_visible_tab`).
4. `crates/runner-app/src/surfaces/mission_workspace/drawer.rs` (`close_terminal_drawer_shell`, `request_close_terminal_drawer_shell`, `hide_terminal_drawer`, post-close focus and layout persistence).
5. `crates/runner-app/src/surfaces/mission_workspace/attach.rs` (`drawer_focused`, `focus_active_mission_terminal`, `drawer_was_focused`) and `crates/runner-app/src/surfaces/root/mod.rs` (`close_target`).
6. Guide pages: `docs/guide/reference/keyboard-shortcuts.md`, `docs/guide/zh-CN/reference/keyboard-shortcuts.md`, `docs/guide/guides/chats-and-windows.md`, and `docs/guide/zh-CN/guides/chats-and-windows.md`.

## Non-goals

- Do not change chat terminal drawer ⌘W behavior (`CloseTarget::ChatDrawer`).
- Do not change ⌘W when focus is outside the mission drawer, or change the drawer hide button / ⌥F12 toggle.
- Do not add confirmation dialogs that tab `×` does not have, and do not touch daemon protocol, CLI, or storage (keep `frontend-only`).

## Deliverables and tests

- Route local `CloseTarget::MissionDrawer` (`windows.rs`) and remote `close_visible_tab` (`mission_workspace/input.rs`) through a shared focused-shell close method on `MissionWorkspace`, matching tab `×` behavior, clearing `drawer_was_focused` when the last shell closes, persisting layout, and restoring focus cleanly.
- Verify remote mission drawer closing for multi-shell, last-shell, and hosts without `"close"` capability.
- Update the four guide pages (EN + zh-CN `keyboard-shortcuts.md` and `chats-and-windows.md`) and any shortcut copy mentioning "Hide focused mission drawer".
- Add/update deterministic tests covering local and remote mission drawer ⌘W (multi-shell close + next-shell focus, last-shell close + hide + mission focus on Feed and Session tabs, hide button and non-drawer ⌘W non-regressions). Follow `AGENTS.md`'s non-flaky test rule (`h.wait` / `wait_until`, no fixed sleeps; run new async/app tests 20x and report the count).
- Run `cargo fmt --all -- --check`, relevant `cargo test -p runner-app`, and `make clippy` before review handoff.

## Role and QA boundaries

- `coder` and `reviewer` never launch the app. `reviewer` reviews frozen local commits, checks test determinism/scope, applies `reviewed` on the final PR head, and confirms the live-QA matrix.
- `qa` acquires `script/qa-lock acquire --timeout 540` before building/launching, holds it until full teardown (preserving any pre-existing user dev sessions), records results in `docs/tests/runs/1398-cmd-w-mission-drawer-shell-mac.md` and the PR description, and releases `script/qa-lock`.
- Required live checks (macOS):
  1. **Local mission drawer ⌘W (2 shells -> 1 -> hidden):** In a scratch mission, open drawer, add a second shell (`+`), focus shell 2, press ⌘W -> shell 2 closes (tab gone, process stopped), drawer stays open, and focus moves to shell 1 (type echo to confirm focus). Press ⌘W again -> last shell closes, drawer hides, focus returns to mission.
  2. **Local non-regressions:** With 2 shells open, click drawer hide button -> hides without closing shells (reopening shows both). With focus outside the drawer (feed/composer), press ⌘W -> drawer stays open and shells stay alive.
  3. **Remote mission drawer ⌘W (same-Mac two-host relay fixture):** Per `docs/tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos`, open a host mission from Client with 2 drawer shells, focus a shell, press ⌘W -> closes active shell on Client and Host and moves focus to remaining shell; press ⌘W on last shell -> hides drawer and returns focus to remote mission. Tear down enrollments, pairings, wrappers, and daemons to baseline.
- Keep `needs-live-test` if any check fails or is blocked; remove it only once all 3 checks pass on the final head SHA (disclosing macOS live coverage vs Windows/Linux CI).

## Authorization

- **Approved crew:** `neon trio` (explicitly requested by Jason).
- **Git / PR mode:** Authorized to commit, fold this brief into implementation commit(s) before final push (no standalone brief commit in PR history), push `fix/1398-cmd-w-mission-drawer-shell`, open a PR against `main` (`Fixes #1398`, `frontend-only`, accurate `risk-*`, `reviewed`, `needs-live-test` as applicable), and drive CI green on all platforms.
- **Stop gate:** Stop at an open, reviewed, QA-verified, CI-green PR. Do not merge, delete branches, or cut a nightly/release.
