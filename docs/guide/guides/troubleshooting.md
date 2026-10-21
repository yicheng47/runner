# Troubleshooting

Start with the failing agent or command, then check Runner's settings and diagnostics. The steps below preserve existing chats and missions while you identify the problem.

## Runner does not open

Runner starts its background service before showing a window. If that fails, Runner records the reason in `runner.log` and, on Windows, shows it in a **Runner could not start** dialog. The log is in `~/Library/Logs/com.wycstudios.runner` on macOS and `%APPDATA%\com.wycstudios.runner\logs` on Windows. Include the message when you [report the problem](#collect-a-useful-bug-report).

## An agent is missing or fails to start

1. Run the agent in your usual terminal. If it cannot start or sign in there, finish its own setup first.
2. Open **Settings → Agents**. Enable the runtime and click **Refresh**. Use **Browse** to select the actual executable if Runner did not find it on `PATH`.
3. Check the chat's working directory and model choice. Select a model available to that agent and account, or use its default.
4. Read the terminal for an authentication, folder trust or provider error. Runner cannot complete the provider's login for you.

On Windows, npm shims need Node.js, and Claude Code and pi's bash tool need Git for Windows. Cursor, Grok and other newer integrations have platform-specific validation gaps listed under [Supported agents](../reference/supported-agents.md). An enabled runtime is not a guarantee that its provider supports every platform.

## The `runner` command is missing

Open **Settings → General → Command line** and inspect the installation status. On macOS, use **Install** if necessary. On Windows, open a new terminal after Runner adds the command to your user `PATH`.

Check which command your shell finds:

```sh
command -v runner
runner --version
```

In PowerShell:

```powershell
Get-Command runner
runner --version
```

If another executable named `runner` comes first, use the path shown in Settings or correct your shell's `PATH` order. Restart the agent session if it inherited an older environment.

## A sandbox blocks Runner

Exit code `5` means the calling sandbox prevented access to the local service. It does not mean Runner is closed. Ask the calling agent to retry the same command with its host's permission mechanism outside the sandbox. For code `3`, open Runner and retry. [Exit codes](drive-runner-from-agents.md#exit-codes) explains other failures.

## A stopped conversation did not resume

Check **Settings → General → Resume running agents on launch**. Only agents recorded as running at shutdown are candidates for automatic recovery; a manually stopped chat needs **Resume**.

Read the session's error and terminal output. Resume needs a saved conversation identity and the runtime's own history. If the agent changed conversations using its native commands, whether Runner tracks that change depends on the [runtime](../reference/supported-agents.md). A new chat cannot reconstruct missing provider history.

For a mission slot, **Restart** intentionally starts a fresh conversation. Use **Resume** when you want continuity. Restore archived work from **Settings → Archived** before trying to resume it.

## The mission looks idle or the feed watch ended

Open the member's terminal to check for a login, approval or unfinished prompt. Some runtimes use terminal activity to estimate working and idle; the marker is not a completion signal.

For a CLI watch, run `runner mission show <mission-id> --json` and read stderr from the follower. The follower can exit when all sessions stop, when the mission ends, or on a connection error. If work is still active, follow the resume command printed by the watcher. After resuming a stopped mission, start a new watch.

## MCP or skill changes do not appear

Start a new session after changing a registration or global skill switch. These edits affect the runtime's own files; a running process may have already loaded its configuration. Check the path and selected agent in Settings, and use **Refresh** after an external edit. Some skill catalogs deliberately omit bundled, project or plugin skills; see [MCP servers and skills](mcp-and-skills.md).

## Collect a useful bug report

Find the Runner version in **Runner → About Runner** on macOS, or **Settings → About** on either platform. The macOS About window shows the app icon, version and build identity, and copyright. Released builds include their commit identifier; local development builds show `dev`.

Open **Settings → Diagnostics → Open log folder**. Include the Runner version, operating system, agent CLI and version, the steps that failed, and the visible error in a [GitHub issue](https://github.com/yicheng47/runner/issues/new). Share only the relevant log excerpt after removing secrets and private conversation content. Keep the original sessions and logs until you have captured the evidence; deleting application data is not a first troubleshooting step.

## Linux local builds

For source builds, graphics prerequisites, packages and WSL setup see [Linux development on main](https://github.com/yicheng47/runner/blob/main/docs/arch/linux.md). If the CLI is used over SSH, start `runnerd` with `systemctl --user start runnerd` first; remote CLI calls do not auto-start it. A minimal user service resolves the account login shell for PATH discovery. Check `systemctl --user status runnerd` and the log directory inside the selected XDG data root.

Linux notification permissions are managed by your desktop. Runner cannot query a universal permission status; confirm the desktop notification daemon and portal backend are available. Clipboard text is supported by GPUI; clipboard file paths are not available. For application updates, use **Settings → Updates**. System deb/rpm installation needs a desktop polkit authentication agent and `pkexec`; RPM installation also needs DNF. Authorization cancellation, a busy package manager or dependency errors are shown in Runner; retry after resolving the cause. **Settings → Diagnostics** shows the installation log. Source builds and unmanaged installations cannot replace themselves; install a managed package first. WSL systemd services do not keep an idle distro alive.

In a normal Linux window, drag an edge or corner to resize. The middle caption button toggles maximize and restore; maximized/fullscreen windows have no resize border. New windows use the desktop's reported logical display bounds. If the compositor does not report a separate work area, panel/taskbar space may be included in those bounds. Include the desktop/session type and whether maximize, restore or edge dragging failed when reporting a window-control problem.
