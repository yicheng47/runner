# 1241 — The new-tab launcher starts in the recent tab's project

Tracking issue: [#1241](https://github.com/yicheng47/runner/issues/1241)

## Motivation

New work usually goes in the project of the tab you were just in, but ⌘N opens the launcher with **No project** ([#1211](https://github.com/yicheng47/runner/issues/1211)), so starting a tab there takes a trip to the picker first. Choosing a project then leaves focus on the picker: ↑/↓ reopen the project menu instead of moving through the agent and role rows, and the keyboard flow breaks until you click back into the search.

This partly reverses #1211's rule that ⌘N always starts with No project (Jason, 2026-11-09). The New tab row still stays where the page opened.

## Scope

### Starting project

| What you did | Project picker | New tab row | Sidebar |
|---|---|---|---|
| ⌘N / Ctrl+N, or **New tab** in the sidebar or command palette, from a tab in project P | P, editable, with P's folder | Recents | P's row is not highlighted; a collapsed P stays collapsed |
| The same from a tab in Recents | No project, editable | Recents | Unchanged |
| ⌘N while the launcher page shows | The project of the tab the page opened from, editable | Recents | As above |
| ⌘N with no tab in the window | No project, editable | Recents | Unchanged |
| **+** beside Recents | No project, editable | Recents | Unchanged |
| A project's **+** | That project, locked | Under the project | Unchanged |
| ⇧⌘M, or starting from a crew | The active project, editable | Recents | Unchanged |
| The last tab archived or closed | No project, editable | Recents | Unchanged |

"The tab the page opened from" is the tab the page's × returns to: the tab active when the page opened, or, if that tab is gone, the window's most recently viewed tab. The folder is the project's folder, as choosing the project in the picker gives. What starts files under the project the picker shows, as today. The same rule applies on a remote computer, using that host's tab and projects.

When the dashed row has nowhere to show (the Active view, or a collapsed Recents), **New tab** is selected in the sidebar, as today, and the preselected project's row is not.

### Keyboard focus

After a project is chosen in the picker, with the mouse or the keyboard, or its menu is dismissed with Esc, focus returns to the launcher's search field: ↑/↓ move through the agent and role rows, Enter starts the selected row, and typing searches. This applies to the local launcher and a remote host's. Other selects in the app keep their current focus behaviour.

### Non-goals

No layout change, no new setting, no change to **Change**'s folder picker, and no change to a split pane's launcher, whose project stays its tab's.

### Guide

[Projects](../guide/concepts/projects.md) and [Chats and windows](../guide/guides/chats-and-windows.md) describe ⌘N and **New tab** starting with No project; update both and their `zh-CN` counterparts.

## Implementation Phases

1. **Starting project.** `open_new_tab_modal` passes the recent tab's project instead of `ProjectScope::Root`, unlocked. `open_launcher_page` sets the sidebar's active project from the scope it receives; these entries keep the sidebar as it is for No project, so the picker's project and the sidebar's are separate.
2. **Focus.** Return focus to the launcher's search field when its project picker closes, without changing `StyledSelect` for its other uses.

## Verification

Automated tests cover each row of the table and the focus rule; `new_tab_entries_start_in_recents_with_no_project_and_mission_entries_keep_the_active_one` changes with the first two rows. Live checks in the macOS development app cover the starting project, the sidebar staying put, the unchanged entries, and arrow keys after a project choice; the [mission brief](../impls/briefs/1241-launcher-recent-project.md) lists them.
