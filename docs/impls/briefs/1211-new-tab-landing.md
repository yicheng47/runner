# #1211 — New tab landing and launcher row

Work in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1211-new-tab-landing` on `fix/1211-new-tab-landing`, based on `origin/main` at `c0da07cf237bd0daf4d9713c350ea5a4cc6d76fe`. Issue #1211 has the report. There is no design frame: this reuses the existing row and the split panes' locked picker. This brief is the source for detail.

## Problem

Archiving the active tab shows the launcher even when other tabs exist, the dashed **New tab** row jumps between projects with the picker, and a launcher opened from a project's **+** can switch project. Jason decided on 2026-11-07 to change all three, which reverses the landing rule in #853 and the editable selection in #968.

## Outcome

| What you did | What shows | New tab row | Project picker |
|---|---|---|---|
| Archive or close the active tab, other tabs exist | The last viewed tab | None | n/a |
| Archive or close the active tab, no tabs left | Launcher | Recents | No project, editable |
| ⌘N, **New tab** in the sidebar, the palette's New chat | Launcher | Recents | No project, editable |
| **+** on Recents | Launcher | Recents | No project, editable |
| **+** on a project | Launcher | Under that project | That project, locked |
| ⇧⌘M, or starting from a crew | Launcher | Recents | Active project preselected, editable, as today |

### Landing

- Replace the clear in `PendingTabClose::apply` and in `leave_archived_mission` with a landing on `recently_viewed_tab_id`: the last viewed surviving tab, else the first. Only with no tab left does the launcher show.
- The landing behaves like selecting that tab: attached, marked viewed, focused and recorded in navigation history, on every path that closes a tab (archive, terminal close, empty-tab delete, archive all, mission archive). The route does not change.
- If the last viewed tab is open in another window, where this window would show the duplicate-chat placeholder, take the next most recent that is not. If every remaining tab is open elsewhere, take the last viewed.
- Unchanged: archiving one chat of a split leaves its pane empty, and archiving a chat that is not on screen changes nothing.
- A second close within about half a second of a landing does nothing, from ⌘W or a pane's ×. Terminal tabs close with no prompt and cannot be restored (`request_close_terminal_tab`), so a double press would otherwise kill the terminal it landed on. A later press closes normally.

### The row

- The row stays where the page opened. `launcher_row_scope` stops reading the live picker through `launcher_page_scope`, so neither a picker change nor Change directory moves it.
- Only a project's **+** draws the row under a project. Every other entry draws it in Recents.
- The chat, terminal or mission still files under the project the picker shows when it starts.
- The existing fallback stays: under a collapsed section or in the Active view there is no row, and **New tab** is selected. **+** on a collapsed project still expands it.

### ⌘N

- ⌘N always gives Recents with no project selected (Jason, 2026-11-07), including from a tab in a project and when a launcher page is already showing. On a page opened from a project's **+**, it moves the row to Recents, clears the project and unlocks the picker.
- The window's empty state is the same Recents, no project page. Set this explicitly: the page launcher is cached between visits and could hold an older project.

### The lock

- `ProjectField` already has a locked mode, used by split-pane launchers. It is set only at construction, and `choose` does nothing while locked, so a reused page needs a way to change it: `open_launcher_page` retargets a visible page, and **+** on another project must move the row and lock to that project.
- `open_launcher_page` is shared. ⇧⌘M and crew starts pass the active project through it and must not lock.
- Change directory still works when locked. A folder outside the project stays filed under it, as a split pane's does.
- If the locked project is deleted while the page is open, the page falls back to Recents, no project, unlocked.

### Remote windows

The launcher is shared, so the row, ⌘N and lock rules apply there too. A remote host's projects arrive after the page opens (`load_host`, `page_scope`), so check that a locked page still ends up on its project and that the row does not move while the catalog loads. Remote landing already goes to an existing tab: `sync.rs` picks the first tab, then the first mission. Use the last viewed tab there if that is simple, and otherwise leave it and say so in the PR.

## Read first

Under `crates/runner-app/src/surfaces/`: `PendingTabClose` and `replace_tab_rows` in `root/mod.rs`; `close.rs` in `chat/`; `leave_archived_mission` in `app_shell/actions.rs`; `open_launcher_page`, `launcher_page_scope`, `page_scope`, `launcher_page_closable` and `cached_launcher` in `start_chat/launcher.rs`; `leave_launcher_page` and `recently_viewed_tab_id` in `start_chat/launcher/mission.rs`; `open_new_tab_modal` in `start_chat/modal.rs`; `launcher_row_scope` and the New tab row in `sidebar/view.rs`; `project_field.rs`; `PaletteDestination::NewChat` in `command_palette.rs`; `remote_client/workspace/sync.rs`.

## Docs

Update the launcher paragraph in `docs/guide/guides/chats-and-windows.md` and its `docs/guide/zh-CN/` counterpart: where closing a tab lands, where the row sits and that it stays there, what ⌘N starts with, and the locked picker from a project's **+**. Fix any other guide or `docs/arch/` sentence that says closing a tab shows the launcher.

## Validation

Automated: the `runner-app` tests and workspace Clippy, plus the two-platform CI. These tests pin the old behavior and need rework: `launcher_page_row_follows_its_project_and_plus_reveals_it`, `page_left_by_closing_the_active_tab_closes_to_the_last_viewed_tab`, `page_close_row_close_and_close_tab_return_to_where_the_launcher_page_opened`, `sidebar_plus_keeps_reused_page_project_picker_and_start_scope_in_sync`, and the `PendingTabClose` tests in `root/mod.rs` and `chat/mod.rs`. Add coverage for the table above, the other-window skip, the close guard, a retargeted lock, a deleted locked project and a remote page whose catalog arrives late.

Live checks, run by `qa` on the reviewed candidate on macOS; do not expand beyond them.

1. **Archive with other tabs.** Archive the active chat after viewing another tab: that tab shows, focused, and the launcher does not. Reason: the landing changes.
2. **Close a terminal tab, and repeat ⌘W.** ⌘W on a terminal tab lands on the last viewed tab. A quick double ⌘W with a second terminal as the landing tab leaves that terminal open, and a later ⌘W closes it. Reason: the close guard protects work that cannot be restored.
3. **No tabs left.** From a tab in a project, close the last tab in the window: the launcher shows, with the row in Recents, No project and no ×. Reason: the empty state must not inherit the project.
4. **⌘N.** From a tab in a project, ⌘N shows the row in Recents with No project. Change the picker to a project and use Change directory: the row stays in Recents. Start a chat: it files under the chosen project. Reason: the row no longer follows the picker, and ⌘N no longer inherits the project.
5. **Project +.** **+** on a project shows the row under it and a disabled picker naming it. **+** on another project moves both. ⌘N from there moves the row to Recents and unlocks the picker. Start a chat from the locked page: it files under that project. Reason: the lock is new and retargets on a reused page.
6. **Archive a mission on screen.** It lands on the last viewed tab. Reason: the mission path changes.
7. **Two windows.** With the last viewed tab open in a second window, archive the active tab in the first: it lands on a different tab, with no duplicate-chat placeholder. Reason: the skip is new.

Remote is covered in process; check it live only if a paired host is available, and otherwise record the gap.

## Authorization

Jason approved this mission on 2026-11-07 with the claude trio. It may commit, push and open one PR against `main`.
