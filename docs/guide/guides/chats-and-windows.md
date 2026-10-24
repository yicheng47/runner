# Chats, tabs and windows

Arrange separate sessions into split tabs, organize them in the sidebar, and use multiple windows without losing track of which terminal you are controlling.

## Know the three surfaces

| Surface | What it holds |
| --- | --- |
| Window | A real operating-system window |
| Tab | A group of panes, represented by one entry in the Projects sidebar |
| Pane | One slot holding a single chat or shell session |

**⌘N** / **Ctrl+N** fills the focused empty pane or starts a chat in a new tab. **⌘T** / **Ctrl+Shift+T** does the same for a shell. A new tab does not start a crew; use a [mission](../concepts/missions.md) when agents need a shared feed.

## Split and navigate

Use **⌘D** / **Ctrl+D** to split right and **⇧⌘D** / **Ctrl+Shift+D** to split down. Choose **New chat** in an empty pane or select a chat from the sidebar. Drag pane headers to reorder the layout; the layout picker offers common arrangements. Splitting is bounded by the available window size.

Use **⌘[** / **Ctrl+[** and **⌘]** / **Ctrl+]** to move focus between panes. **⌘1–9** / **Ctrl+1–9** selects the corresponding visible sidebar row: a tab or mission in either view. **⇧⌘[** / **Ctrl+Shift+[** and **⇧⌘]** / **Ctrl+Shift+]** move through recently viewed pages.

Drag files from Finder or File Explorer onto a chat pane, a drawer terminal or a mission slot to insert their paths at the prompt, quoted as when you paste files. The terminal highlights while the files are over it and takes focus when you drop them; nothing runs until you press Return.

For shell splits, Runner follows the shell's reported working directory. It installs that reporting for zsh and bash on macOS. Windows shells without their own directory-reporting prompt use the original launch directory for a split, even after `cd`.

## Organize the sidebar

Use the sidebar context menus to rename and organize tabs, group them into folders, and pin work you return to often. [Projects](../concepts/projects.md) group work by directory; folders organize sidebar entries. Status markers show activity, attention and stopped sessions. Use them to find the terminal to inspect, rather than treating an idle marker as a completed review.

Use the list-filter button beside Search to switch between **Projects** (the default) and **Active**. Runner remembers your choice. Active lists **Chats → Missions → Terminals**, hides empty groups, and shows each group's visible count. Click a group's heading to collapse or expand it; Runner remembers which groups are collapsed. Active includes running chats, terminals and missions with live members, including idle agents and shells. Within each group, rows keep the order their tabs and missions were created, oldest first, and new work joins the end; status changes, unread output, renames and restarts never move a row. The second line shows the project and relative folder, or the working directory for work outside a project. Renaming edits only the title, keeping this location visible.

Active has one row per tab, like Projects. A tab with any agent pane appears under Chats; a tab of only terminals appears under Terminals. Split tabs use the split icon and tab name, derived from the first chat or terminal pane unless you name the tab. Selecting a row opens its tab on the pane that needs attention, or on the tab's last focused pane when none does. Mission members roll up into one mission row. A stopped chat or mission stays only for unread output or an error; selection alone does not keep it visible. A finished unread response stays until viewed, and an error stays until cleared. Every running terminal is listed, including idle shells; an exited terminal stays only for unread output or an error. Terminal activity uses Runner's existing output-based Busy estimate; drawer terminals are excluded. Active keeps context menus; drag and reorder remain in Projects.

The footer shows live weekly Codex and Claude usage when those agents are installed and enabled. Select it to open Usage, including other supported providers. When only other usage providers are available, a **Usage** label keeps the popover accessible. The separate gear opens Settings. An available Runner update appears beside the Runner name.

**⌘K** / **Ctrl+K** opens the command palette to find chats, missions, roles, crews and commands. **⌘S** / **Ctrl+S** toggles the sidebar.

## Use another window

Press **⇧⌘N** / **Ctrl+Shift+N** to open another Runner window. If a chat or mission is already being driven in another window, Runner shows an **Open in another window** overlay. Use its handoff controls to focus or bring the subject here. A terminal has one owning window at a time; the overlay prevents two windows from sending competing input.

Close the window with **⇧⌘W** / **Alt+F4**. Its layout is saved, and sessions can keep running. On Windows, closing the last window follows **Settings → General → When Runner quits**. On macOS, quitting the application is separate from closing a window.

On Windows, launching Runner while it is running, from the Start menu, a shortcut or the taskbar, brings the running app forward instead of starting a second copy: its main window reopens if you closed it and restores if it is minimized. A launch while Runner is quitting waits for it to exit and then starts; if Runner is still running after 10 seconds, the launch starts another copy anyway. The development build and the installed Runner are separate apps and can run side by side.

**⌘W** / **Ctrl+Shift+W** acts on the focused drawer, pane or chat tab. It is different from closing the whole window. Use **Stop** when you specifically want to halt an agent, and **Archive** when you want a chat out of active lists.

For alerts while the app is in the background, open **Settings → Notifications**. Chat events are enabled by default; mission events are opt-in. Clicking a notification opens its chat or mission. When the active window shows another computer, local alerts can still appear; clicking one returns a window to this computer and opens the local item.

See [Keyboard shortcuts](../reference/keyboard-shortcuts.md) for all defaults and customization.

## Work on another computer

Once signed in, choose a computer in the sidebar’s picker. **Settings → General → Local only** hides the remote surfaces and keeps Runner on this computer; see [Remote access](remote-access.md#local-only). Chat creation, pane splits and resizing, drawer shells, chat lifecycle and sidebar organization use the same controls on that host. Remote missions use the same feed, answer cards, recipient composer, member tabs and terminal drawer. Start mission selects an existing host crew; creating or editing a crew stays on the host. Available actions depend on the host’s version. The command palette searches that computer’s chats and missions; Role and Crew open the host’s read-only lists, with chat and mission launch actions. Creating/editing/deleting roles or crews, Settings and Archived are available after returning to this computer.

New chat uses the host’s agents and defaults. In New chat and Start mission, the Working directory Browse menu lists host projects and recent folders, then lets you browse folders on that computer. Choosing a folder fills the field; Escape or clicking outside leaves it unchanged. Local windows keep their OS folder picker. Returning to this computer restores its own tabs, selection and sidebar state; remote sidebar choices do not change saved local preferences.
