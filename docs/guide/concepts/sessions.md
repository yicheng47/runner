# Sessions

A session is one agent or shell running in a real terminal. A direct chat has its own session; every live mission member has one too.

The runtime supplies its own terminal interface, authentication, commands and conversation history. Runner hosts the terminal, tracks its state and records the conversation identity needed to resume it. A [role](roles.md) is a template; a session is a particular run.

## Read the state

Working and idle indicators help you find active agents. Supported runtimes report those states through hooks; others use terminal activity as a baseline. An idle indicator is not proof that a task passed its checks. Some runtimes also report when they need an approval or answer; [Supported agents](../reference/supported-agents.md) lists the differences.

Stopped and crashed sessions have no running agent to receive input. Read the terminal and any error before choosing the next action.

## Open terminal links

Hover a URL or file path to highlight it, then ⌘-click or Ctrl-click on macOS, or Ctrl-click on Windows, to open it. URLs open in your browser; file paths, including `:line[:column]` suffixes, open in your configured editor.

When a runtime wraps an underlined URL onto indented rows, hovering any fragment highlights the full URL; modifier-clicking any fragment opens the same destination.

## Choose the right action

| Action | Effect |
| --- | --- |
| Stop | Ends the process and leaves the session available to resume |
| Resume | Continues a stopped agent's recorded conversation where supported |
| Restart a mission slot | Starts a fresh conversation for that member |
| Fork a chat | Creates another chat from the conversation, for runtimes that support forking |
| Archive a chat | Stops it and removes it from active chat lists; find it in Settings → Archived |

A shell can restart in its directory, but has no agent conversation to resume. Shell terminals close rather than archive. Closing a window saves its layout and can leave its sessions running; closing a pane or chat tab is a separate action, so read any confirmation before accepting it.

## Claude Code accounts

When several [Claude Code accounts](../guides/claude-accounts.md) are configured, new sessions and resumes use the active account. Running sessions keep their current account. A paused chat identifies the previous and active accounts; resuming continues its original transcript, and a fork saves a new conversation under the active account. Explicit `CLAUDE_CONFIG_DIR` environment pins take precedence.

## Quit, relaunch and recover

Runner's background service, `runnerd`, owns processes independently of the app window. **Settings → General → When Runner quits** offers **Ask**, **Keep running** and **Stop sessions**. Reopening after **Keep running** attaches to the live terminals.

If the processes were stopped for shutdown or an update, **Resume running agents on launch** controls their automatic recovery. Manually stopped sessions stay stopped until you resume them. Recovery depends on the agent's saved conversation identity and history; it cannot restore an unsaved conversation from terminal pixels alone.

If the daemon crashes, Runner reconnects and offers recovery for affected sessions. Use **Settings → Diagnostics → Open log folder** when a failure repeats. See [Troubleshooting](../guides/troubleshooting.md) for a missing conversation or a failed launch.

## Workspace files

The right panel provides [workspace files and read-only previews](../guides/files-and-previews.md) alongside its existing information views. File tabs do not start new sessions or modify files.
