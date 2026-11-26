# Sessions

A session is one agent or shell running in a real terminal. A direct chat has its own session; every live mission member has one too.

The runtime supplies its own terminal interface, authentication, commands and conversation history. Runner hosts the terminal, tracks its state and records the conversation identity needed to resume it. A [role](roles.md) is a template; a session is a particular run.

## Read the state

Working and idle indicators help you find active agents. Supported runtimes report those states through hooks; others use terminal activity as a baseline. An idle indicator is not proof that a task passed its checks. Some runtimes also report when they need an approval or answer; [Supported agents](../reference/supported-agents.md) lists the differences.

Stopped and crashed sessions have no running agent to receive input. Read the terminal and any error before choosing the next action.

Click a chat pane’s **Title ▾**—or press **⌘I** / **Ctrl+Shift+I**—to inspect the session’s metadata without giving up workspace width: role or runtime, model and effort, its **worktree** (branch, `from <base>` and path copy) or **cwd**, any non-default launch command, config, `session_key` with copy, and a collapsible scrollable **System prompt** for role chats.

## Open terminal links

Hover a URL or file path to highlight it, then ⌘-click or Ctrl-click on macOS, or Ctrl-click on Windows, to open it. Web URLs (`http://` and `https://`) open in your browser. Local `file://` URLs (plain text or terminal hyperlinks targeting this machine or `localhost`, such as `file:///C:/...` on Windows) and file paths—including Windows drive-letter paths (`C:\...` and `C:/...`), UNC paths (`\\server\share\...`), and `:line[:column]`, `(line,column)`, or `#Lline` suffixes—open in a tab in the side panel at that line when the path is inside the folder its Files view shows, and otherwise in the editor picked with **Open in**; see [Read a file in the side panel](../guides/chats-and-windows.md#read-a-file-in-the-side-panel).

When a runtime wraps an underlined URL onto indented rows or a hyperlink inside a table cell, hovering any fragment highlights the complete link; modifier-clicking any fragment opens the same destination. Links remain usable after reopening the session.

## Terminal images and pixel mouse

Local terminal panes display inline images from programs that use the Kitty graphics protocol, such as `kitten icat`, `chafa`, and terminal-hosted browsers or device simulators. Images stay clipped inside their pane underneath overlays such as the command palette and find bar, scroll with scrollback, and survive closing and reopening Runner while the session stays running in `runnerd`.

Terminals also report real pixel window and cell sizes (`CSI 14 t` and `CSI 16 t`) and send zero-based SGR-Pixels mouse reports (mode 1016) and hover motion without a pressed button (mode 1003) to programs that request them. Hold Shift to bypass mouse reporting for local selection and scrollback.

## Choose the right action

| Action | Effect |
| --- | --- |
| Stop | Ends the process and leaves the session available to resume |
| Resume | Continues a stopped agent's recorded conversation where supported |
| Restart a mission slot | Starts a fresh conversation for that member |
| Fork a chat | Creates another chat from the conversation, for runtimes that support forking |
| Archive a chat | Stops it and removes it from active chat lists; find it in Settings → Archived |

Archiving preserves the agent's saved conversation and history. Restore the chat in **Settings → Archived**, then resume it to continue that conversation in a fresh terminal.

Shells have no Stop, Resume or Restart controls. Interrupt commands with Ctrl+C. Closing a terminal or exiting its shell with any exit code closes the pane rather than archiving it. Terminal pane ×, the close shortcut, sidebar terminal close and mission-drawer shell close act immediately without confirmation, locally and remotely, even while a command runs. Archiving a chat still asks for confirmation, and Archive all keeps its chat/terminal counts. Closing a window saves its layout and can leave its sessions running.

## Claude Code accounts

When several [Claude Code accounts](../guides/claude-accounts.md) are configured, new sessions and resumes use the active account. Running sessions keep their current account. A paused chat identifies the previous and active accounts; resuming continues its original transcript, and a fork saves a new conversation under the active account. Explicit `CLAUDE_CONFIG_DIR` environment pins take precedence.

## Quit, relaunch and recover

Runner's background service, `runnerd`, owns processes independently of the app window. **Settings → General → When Runner quits** offers **Ask**, **Keep running** and **Stop sessions**. Reopening after **Keep running** attaches to the live terminals.

If the processes were stopped for shutdown or an update, **Resume running agents on launch** controls their automatic recovery. Manually stopped chats stay stopped until you resume them. Terminal panes resume after a planned stop regardless of the agent-resume setting. Recovery depends on the agent's saved conversation identity and history; it cannot restore an unsaved conversation from terminal pixels alone.

If an agent exits while it resumes, Runner keeps its conversation and Resume tries it again. Runner starts a fresh conversation only when the runtime's saved conversation is gone. For Codex, if the recorded conversation is gone but the chat's original conversation is still on disk, Runner resumes the original instead.

If the daemon crashes, Runner reconnects and offers recovery for affected chats; shells without a planned launch-resume claim close their panes. Use **Settings → About → Open log folder** (**Reveal logs in Finder** on macOS) when a failure repeats. See [Troubleshooting](../guides/troubleshooting.md) for a missing conversation or a failed launch.
