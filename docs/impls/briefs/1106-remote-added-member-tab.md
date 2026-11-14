# Give remote members added while away their tab (#1106)

## Starting point

- Issue: `yicheng47/runner#1106` (`bug`, `P2`, milestone 0.20)
- Branch: `fix/1106-remote-added-member-tab`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1106-remote-added-member-tab`
- Base: `origin/main` at `60a63523`
- Parallel isolation: `.worktrees/feat-1141-git-changes` and `.worktrees/feat-1288-agy-bypass-and-trust` run other missions. Stay inside this worktree with its own Cargo `target/`; do not touch the root checkout or sibling worktrees.

## Problem and outcome

A worker added to a running mission with `runner mission add-role` gets no member tab in a remote viewer that had already opened that mission, and stopping and reopening the mission does not bring it back. The host shows it normally.

The host is not at fault. A throwaway test that ran the real `ops::mission::roles::add_role` against a two-slot crew and then `companion::core::DaemonCore::overview()` listed the members `a`, `b` and `extra-writer`, and `session_list` agreed. The overview is pushed on the `session/` and `event/appended` events the add produces.

The loss is in the viewer's app. `MissionWorkspace::sync_remote` (`crates/runner-app/src/surfaces/mission_workspace/remote.rs`) rebuilds `open_tabs` from the saved presentation's `open_handles` when it switches back to a mission it showed before (`restored = true`). It appends members that are new since the last sync only when `!changed`, so a member that joined while the window showed another tab or mission is never added. `SavedPresentation` keeps the open handles but not which members existed when it was saved, so it cannot tell a closed tab from a new member. The saved presentations live in the window for the session, so every later return, including after the mission stops, restores the same tabs. A throwaway test in `remote_tests.rs` (open the mission, select `RemoteItem::Tab("remote-tab")`, push an overview with an extra member, select the mission again) ended with `open_tabs == ["member"]` while `members()` returned `["coder", "extra-writer"]`.

Locally a mission reopens with every session as a tab (`attach.rs`), and sessions that appear while it is shown are appended (`events.rs`). While a remote mission stays shown, pushed members are already appended (`pushed_roster_and_shell_order_preserve_client_tab_order_focus_and_drawer_presentation`).

Outcome: returning to a remote mission opens a tab for every member that joined since the window last showed it, as the live push path does, while a tab the person closed stays closed and the restored order, active tab, selection, drawer and rail are kept (REMOTE-MIS-03). Confirm with the reproduction that no other remote surface, such as the rail, the composer recipients or the pane arrangement, still drops the added member, and fix any that does in the same change.

## Read first

- `crates/runner-app/src/surfaces/mission_workspace/remote.rs`: `sync_remote`, `SavedPresentation`, `MissionPresentation::switch_mission`, `members`
- `crates/runner-app/src/surfaces/mission_workspace/remote_tests.rs`: `pushed_roster_and_shell_order_preserve_client_tab_order_focus_and_drawer_presentation` and its `setup`/`overview` fixtures
- `crates/runner-app/src/surfaces/mission_workspace/attach.rs` and `events.rs`: how local missions open and add tabs
- `crates/runner-daemon/src/companion/core.rs` `overview`, only to confirm the host side needs no change

## Deliverables and boundaries

- Fix in `crates/runner-app/src/surfaces/mission_workspace/`. The daemon, the companion protocol and the CLI stay unchanged; if the reproduction shows otherwise, stop and tell the operator before widening scope.
- Add `runner-app` tests: a member added while the window shows another tab or mission gets a tab on return; one added while the mission is stopped and reopened does too; a tab the person closed before leaving stays closed on return; a member removed while away loses its tab.
- No guide change is expected, since the guide already says remote missions show their members. Update `docs/guide/guides/remote-access.md` only if the fix changes behaviour it describes.

## Validation and handoffs

- **Automated**: the new tests, `cargo test -p runner-app`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` and `cargo fmt --all --check`.
- **Handoffs**: `coder` hands `reviewer` an unpushed local commit by SHA. `reviewer` reviews that snapshot and names the live checks in its clean review under `AGENTS.md` Test Scope; `qa` takes the reviewed candidate from `coder`.
- **Live QA (`qa`, macOS)**: take `script/qa-lock acquire --timeout 540` first. Use two development instances on this Mac under the standing two-instance remote procedure in `docs/tests/full-smoke-test.md` (Jason, 2026-11-13), with disposable enrollments and the teardown it requires. On the host, start a small mission from a two-slot crew in a fresh `/tmp` folder with an inexpensive model, low effort and tiny prompts. Checks, each with its reason:
  1. In the viewer, open the mission, select another tab or chat, add a role on the host with `add-role --handle extra-writer`, then return to the mission. The new member has a tab, a rail card and the joined notice. This is the reported bug.
  2. With the mission shown in the viewer, add another role. Its tab appears live. This is the push path the fix sits beside.
  3. Close one member tab in the viewer, leave and return; then stop and resume the mission on the host. The closed tab stays closed and the arrangement holds (REMOTE-MIS-03), since the fix changes how a saved arrangement is restored.
  4. Stop the mission on the host and reopen it in the viewer. Every member, including the added ones, is listed. This is the issue's step 5.
- Record the run in `docs/tests/runs/1106-remote-added-member-tab-mac.md` with screenshot evidence per check, and record the results in the PR description. Archive only QA-created sessions and missions, tear down both instances as the procedure says, and release the lock.

## Authorization

Jason approved the `agy trio` (`coder`, `reviewer`, `qa`) for #1106 on 2026-11-14. The crew may commit, push `fix/1106-remote-added-member-tab`, open a PR against `main` with `Closes #1106` and its labels (`frontend-only` if the change stays in `runner-app`, a risk label, `needs-live-test` until qa passes every check, and `reviewed` from `reviewer` on the final head), and drive CI green. Fold this brief into the implementation commit. Stop at the open, reviewed, QA-passed, CI-green PR; do not merge or clean up.
