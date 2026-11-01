# Chats, tabs and windows

Arrange separate sessions into split tabs, organize them in the sidebar, and use multiple windows without losing track of which terminal you are controlling.

## Know the three surfaces

| Surface | What it holds |
| --- | --- |
| Window | A real operating-system window |
| Tab | A group of panes, represented by one entry in the Projects sidebar |
| Pane | One slot holding a single chat or shell session |

**⌘N** / **Ctrl+N** shows the launcher as a workspace page, with **New chat** selected in navigation. It creates no tab or project-tree row until you start a chat, terminal or mission. The page has no pane header, split menu or ×; the Runner mark and “Start something new” stay on every step. Esc on the default page returns to the previous tab, and selecting any sidebar row leaves the launcher. **⌘T** / **Ctrl+Shift+T** starts a shell in an available empty pane or a new tab. Use a [mission](../concepts/missions.md) when agents need a shared feed.

Each pane is an island: a rounded card with its own 40 px header, separated from other panes and the expanded sidebar by 6 px. The outer inset stays 8 px, including the left edge when the sidebar is collapsed. The header holds the runtime, name, account when available, status, menu, split control and ×. The side panel is a separate island. Its toggle lives at the top-right of the pane area and hides while a shell is focused.

Use the **Stop chat** icon at the right of a chat’s header, before the menu, while it runs, or the **Resume chat** icon once stopped. The button stays visible and keeps its place as the status changes; the status shows state only. Starting and resuming states have no lifecycle button. In narrow panes, detail hides first, then the name truncates, then the status label hides; the action buttons stay visible. Stopped chats show a centered **Chat stopped** card with one **Resume** button and its current shortcut; × archives. Short panes drop the badge, shorten or hide the reason, and hide the title only if needed to retain Resume. **⇧⌘X** / **Ctrl+Shift+X** and **⇧⌘R** / **Ctrl+Shift+R** act on the focused chat. The menu offers Stop or Resume, Fork chat and Rename; × archives with confirmation when the chat is running. Terminals offer Rename only. Closing a terminal pane with ×, the close shortcut or its sidebar action never asks for confirmation, even while a command runs; this also applies remotely. Any shell exit closes its pane, including a failed command used to exit the shell. Planned quit/update recovery still restores terminal panes.

## Split and navigate

Use **⌘D** / **Ctrl+D** to split right and **⇧⌘D** / **Ctrl+Shift+D** to split down. Search for an agent or role in the empty pane’s launcher, choose **Terminal** for a shell, or select a chat from the sidebar. In a split tab, drag the six-dot grip before a pane’s runtime mark to reorder the layout; the layout picker offers common arrangements. Splitting is bounded by the available window size. New windows have a fixed minimum based on the saved sidebar width and zoom when they open, with enough height for the sidebar’s fixed controls. Later zoom or sidebar changes do not update an open window’s minimum.

Use **⌘[** / **Ctrl+[** and **⌘]** / **Ctrl+]** to move focus between panes. **⌘1–9** / **Ctrl+1–9** selects the corresponding visible sidebar row: a tab or mission in either view. **⇧⌘[** / **Ctrl+Shift+[** and **⇧⌘]** / **Ctrl+Shift+]** move through recently viewed pages.

Drag files from Finder or File Explorer onto a chat pane, a mission drawer terminal or a mission slot to insert their paths at the prompt, quoted as when you paste files. The terminal highlights while the files are over it and takes focus when you drop them; nothing runs until you press Return.

For shell splits, Runner follows the shell's reported working directory. It installs that reporting for zsh and bash on macOS. Windows shells without their own directory-reporting prompt use the original launch directory for a split, even after `cd`.

## Organize the sidebar

Use the sidebar context menus to rename and organize tabs, group them into folders, and pin work you return to often. [Projects](../concepts/projects.md) group work by directory; folders organize sidebar entries. Status markers show activity, attention and stopped sessions. Use them to find the terminal to inspect, rather than treating an idle marker as a completed review.

Use the list-filter button beside Search to switch between **Projects** (the default) and **Active**. Runner remembers your choice. Active lists **Chats → Missions → Terminals**, hides empty groups, and shows each group's visible count. Click a group's heading to collapse or expand it; Runner remembers which groups are collapsed. Active includes running chats, terminals and missions with live members, including idle agents and shells. Within each group, rows keep the order their tabs and missions were created, oldest first, and new work joins the end; status changes, unread output, renames and restarts never move a row. The second line shows the project and relative folder, or the working directory for work outside a project. Renaming edits only the title, keeping this location visible.

Active has one row per tab, like Projects. A tab with any agent pane appears under Chats; a tab of only terminals appears under Terminals. Split tabs use the split icon and tab name, derived from the first chat or terminal pane unless you name the tab. Selecting a row opens its tab on the pane that needs attention, or on the tab's last focused pane when none does. Mission members roll up into one mission row. A stopped chat or mission stays only for unread output or an error; selection alone does not keep it visible. A finished unread response stays until viewed, and an error stays until cleared. Every running terminal is listed, including idle shells; an exited terminal closes immediately. Terminal activity uses Runner's existing output-based Busy estimate; drawer terminals are excluded. Active keeps context menus; drag and reorder remain in Projects.

The footer shows live weekly Codex and Claude usage when those agents are installed and enabled. Select it to open Usage, including other supported providers. When only other usage providers are available, a **Usage** label keeps the popover accessible. The separate gear opens Settings. An available Runner update appears beside the Runner name.

**⌘K** / **Ctrl+K** opens the command palette to find chats, missions, roles, crews and commands. **⌘S** / **Ctrl+S** toggles the sidebar.

## Use another window

Press **⇧⌘N** / **Ctrl+Shift+N** to open another Runner window. If a chat or mission is already being driven in another window, Runner shows an **Open in another window** overlay. Use its handoff controls to focus or bring the subject here. A terminal has one owning window at a time; the overlay prevents two windows from sending competing input.

Close the window with **⇧⌘W** / **Alt+F4**. Its layout is saved, and sessions can keep running. On Windows, closing the last window follows **Settings → General → When Runner quits**. On macOS, quitting the application is separate from closing a window.

On Windows, launching Runner while it is running, from the Start menu, a shortcut or the taskbar, brings the running app forward instead of starting a second copy: its main window reopens if you closed it and restores if it is minimized. A launch while Runner is quitting waits for it to exit and then starts; if Runner is still running after 10 seconds, the launch starts another copy anyway. The development build and the installed Runner are separate apps and can run side by side.

**⌘W** / **Ctrl+Shift+W** acts on the focused pane or chat tab (or hides a focused mission drawer). It is different from closing the whole window. Use **Stop** when you specifically want to halt an agent, and **Archive** when you want a chat out of active lists.

For alerts while the app is in the background, open **Settings → Notifications**. Chat events are enabled by default; mission events are opt-in. Clicking a notification opens its chat or mission. When the active window shows another computer, local alerts can still appear; clicking one returns a window to this computer and opens the local item.

See [Keyboard shortcuts](../reference/keyboard-shortcuts.md) for all defaults and customization.

## Work on another computer

Once signed in, choose a computer in the sidebar’s picker; see [Remote access](remote-access.md) to sign in. Chat creation, pane splits and resizing, shell panes, chat lifecycle and sidebar organization use the same controls on that host. Remote missions use the same feed, answer cards, recipient composer, member tabs and terminal drawer. The launcher's crew step lists the host's crews; creating or editing a crew stays on the host. Available actions depend on the host’s version. The command palette searches that computer’s chats and missions; Role and Crew open the host’s read-only lists, with chat and mission launch actions. Creating/editing/deleting roles or crews, Settings and Archived are available after returning to this computer.

New chat uses the host’s agents and roles and shows its active Claude account. **Change** opens the host’s folder browser; local windows use the OS folder picker. The launcher lists the host’s projects for chats and missions. Launcher recency is shared across projects and windows, with a separate list for this computer and each remote host. Use ↑/↓ and Enter to select a ranked or pinned row; Esc on the default page returns to the previous tab/view; in a split pane it clears a search. Returning to this computer restores its own tabs, selection and sidebar state.
