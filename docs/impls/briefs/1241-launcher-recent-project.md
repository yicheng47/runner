# #1241 — The new-tab launcher starts in the recent tab's project

Jason asked for a codex trio to build [#1241](https://github.com/yicheng47/runner/issues/1241) on 2026-11-09. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1241-launcher-recent-project` on `feat/1241-launcher-recent-project`, based on `origin/main` at `ac0a5dca`. The [spec](../../features/1241-launcher-recent-project.md) holds the behaviour and wins on detail; the issue has the background.

## Problem

1. ⌘N and **New tab** open the launcher page with `ProjectScope::Root` (#1211), so starting work in the current project means a trip to the picker.
2. After a choice, `StyledSelect::choose` keeps focus on the picker, and `SelectState::handle_key` reopens a focused, closed select's menu on ↑/↓. The launcher's `capture_key_down` acts only while its search field is focused, so the agent and role rows cannot be reached by keyboard until you click back into the search.

## Read first

- `crates/runner-app/src/surfaces/start_chat/modal.rs`: `open_new_tab_modal`.
- `crates/runner-app/src/surfaces/start_chat/launcher.rs`: `open_launcher_page`, which records the origin and sets the sidebar's active project from its scope; `retarget_page`; `choose_project`; both `ProjectField::new` calls, local and remote; the `capture_key_down` handler; `ChatLauncher::focus`.
- `crates/runner-app/src/surfaces/start_chat/launcher/mission.rs`: `LauncherOrigin`, `leave_launcher_page`, `recently_viewed_tab_id`.
- `crates/runner-app/src/surfaces/sidebar/activation.rs`: `active_tab_project_id`.
- `crates/runner-app/src/surfaces/sidebar/view.rs` (`launcher_row_scope`) and `sidebar/rows_render.rs` (a project row's `selected`).
- `crates/runner-app/src/ui/select.rs`: `StyledSelect::choose`, `on_key_down`, `SelectState::handle_key`.

## Deliverables

1. **Starting project.** ⌘N / Ctrl+N and **New tab** in the sidebar and command palette open the page with the recent tab's project in an editable picker, with that project's folder. The recent tab is the active tab, or, while the page already shows, the tab it opened from: the origin's tab, or `recently_viewed_tab_id` when that tab is gone. A tab in Recents, or no tab, gives **No project**. The same rule applies on a remote computer with that host's tab.
2. **No sidebar jump.** The dashed row stays in Recents, and the sidebar does not select the preselected project: when the row has nowhere to show (the Active view or a collapsed Recents), **New tab** is selected and the project row is not. A collapsed project stays collapsed. What starts still files under the picker's project.
3. **Other entries unchanged.** The Recents **+**, a project's **+**, ⇧⌘M, starting from a crew, and the landing with no tab left behave as they do today.
4. **Focus.** After a project is chosen in the picker, with the mouse or the keyboard, or its menu is dismissed with Esc, focus returns to the launcher's search field, local and remote. Other `StyledSelect` uses keep their focus behaviour; if the select needs a new hook, make it opt-in.
5. **Docs.** Update `docs/guide/concepts/projects.md` and `docs/guide/guides/chats-and-windows.md` and their `zh-CN` counterparts (run `python3 script/check-guide-translations.py`), and the expected result of `WS-CREATE-17` in `docs/tests/regression/workspace.md`, which says ⌘N and the top row start with No project from a tab in A.

**Non-goals:** no layout or UI change, no new setting, no change to **Change**'s folder picker or to a split pane's launcher.

## Tests

- Rework `new_tab_entries_start_in_recents_with_no_project_and_mission_entries_keep_the_active_one`: from a tab in alpha, the shortcut and the sidebar row show alpha, unlocked, with alpha's folder, the row in Recents and the sidebar's active project `None`; the Recents **+** shows No project; the shortcut on a locked page shows the opening tab's project, unlocked, with the row in Recents.
- From a tab in Recents, No project. With the page already showing, the opening tab's project. The existing no-tab-left test still holds.
- With the row hidden (the Active view or a collapsed Recents), New tab is selected and the project row is not.
- Choosing a project leaves the search field focused, and ↓ moves the selected row; Esc on the open project menu does the same. Cover the remote launcher if the harness in `launcher/mission.rs` allows.
- The regression tests for deliverables 1, 2 and 4 must fail without the fix; say which in the handoff.
- Gates: `runner-app` tests, workspace Clippy with warnings denied, `cargo fmt --all --check`, and the guide translation check. Drive macOS, Windows and Linux CI green.

## Live checks (qa)

Use the macOS development app from this worktree, and take the QA lock as AGENTS.md describes. No real agents are needed: start a **Terminal** wherever a check starts something. Use two owned test projects, A and B, each pointing at a fresh folder under `/tmp`, created through the development CLI or app and deleted at cleanup, plus a terminal tab in A and one in Recents.

- **L1, starting project.** From A's tab press ⌘N, then use the sidebar's top **New tab**: the picker shows A, editable, with A's folder; the dashed row is at the top of Recents; A's row is not highlighted. Start a Terminal: it files under A. Reason: deliverables 1 and 2.
- **L2, fallbacks.** ⌘N from the Recents tab shows No project. With the page showing from A's tab, change the picker to B and press ⌘N: the picker returns to A. Reason: deliverable 1's other cases.
- **L3, sidebar stays put.** Repeat L1's ⌘N with Recents collapsed, with A collapsed, and in the Active view: no dashed row, the top **New tab** is selected, A's row is not highlighted, and A stays collapsed. Reason: deliverable 2.
- **L4, other entries (WS-CREATE-17).** Run the updated `WS-CREATE-17`: a project's **+** locks to it with the row under it, and the Recents **+** shows No project. Reason: `open_launcher_page` and the row placement are shared with these entries.
- **L5, arrows after the picker.** Choose B with the mouse, then press ↓ until **Terminal** is selected and press Enter: the selection moves through the rows and a terminal starts. Open the picker again, press Esc, then ↓: the rows move, not the project menu. Reason: deliverable 4.

The verdict gives pass or fail per check with screenshot evidence, and the PR description records it.

## Handoffs

The coder implements and commits, then hands the candidate to `reviewer`. When the review is clean, the coder hands the reviewed commit to `qa` with L1 to L5. Any failure, and any change after review, goes back through `reviewer` before qa reruns the affected checks.

The PR uses `Fixes #1241` and carries `risk-medium`, `frontend-only` and `needs-live-test` until qa's checks pass. The spec and this brief fold into the implementation commit.

## Authorization

The codex trio may commit, push this branch and open one PR against `main`. It does not merge.
