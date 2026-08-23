# Native terminal as a tab type and as pane content

Tracking issue: [#356](https://github.com/yicheng47/runner/issues/356). Status: planned, post-`v0.6.0`, **design first**. Priority P2.

## Motivation

Runner's premise is that you stop scattering agent work across terminal windows — but there's no way to run a quick `git status`, `ls`, or `cargo test` without leaving the app or burning a chat on it. Today the options are bouncing out to Ghostty or starting a chat, and neither fits a ten-second command — or a long-running `pnpm dev` you want to watch next to the agent working on it.

This is **not** a VS Code-style bottom drawer. A terminal is a first-class citizen of the existing surface hierarchy (window → tab → pane, per `AGENTS.md`), not a toggleable panel bolted onto the chrome. Two things:

1. **Terminal tab.** A third kind of tab beside chat tabs and mission tabs. It runs the user's `$SHELL` in a real PTY, shows up in the sidebar like any other tab, and is switched to, renamed, pinned, reordered, and closed like any other tab.
2. **Terminal pane.** A pane in a split can hold a terminal instead of a chat. The empty-pane placeholder gets a second action beside **New chat**: **New terminal**. So a `Cols2` tab can be an agent on the left and a shell on the right, in the same cwd.

## Behavior

### Starting a terminal

- **New terminal tab**: `⌘T` (new `new-terminal` registry entry — free in `keymap.rs`, and the Terminal.app / Ghostty chord for a new tab), the command palette ("New terminal"), the sidebar `+` menu beside "New chat", and the project context menu ("New terminal in project"). No modal: the shell spawns immediately in a fresh single-pane tab in the current window, and focus lands in it.
- **New terminal pane**: the empty-pane placeholder ("No chat in this pane") offers **New terminal** beside **New chat**. Spawns immediately into that pane.
- **cwd**: for a pane, the cwd of the tab's focused sibling session — the shell lands where the agent is working. For a tab, the active project's directory, else `settings.default_working_dir`, else `$HOME`. `cd` covers the rest; there is no cwd picker.
- **Process**: `$SHELL` (fallback `/bin/zsh`) as an interactive login shell, matching Terminal.app and Ghostty on macOS so `PATH` and rc files are what the user expects. No agent adapter: no system prompt, no hooks, no MCP config, no permission-mode args, no conversation-key capture, no bundled `runner` CLI on `PATH`.

### While it runs

- Same terminal element as chats (`crates/runner-app/src/terminal/`): same `alacritty_terminal` model, renderer, IME, paste, resize, scrollback, selection, ⌘-hover links, and terminal theme / font settings. No second terminal implementation.
- Keys: app chords (`⌘N`, `⌘T`, `⌘W`, `⌘K`, `⌘[`/`⌘]`, `⌘1`–`9`, zoom) keep working over a terminal pane; everything else goes to the PTY. `⌘C` copies the selection like it does in a chat pane; `⌃C` goes to the shell.
- Pane header: the title, a **Terminal** badge where a chat shows **Chat** and the runtime badge, and the close button. No busy/idle status dot.
- Sidebar row: a terminal icon instead of the chat icon; the title; no runtime badge. Terminals never contribute to working / unread / needs-you attention state — a streaming `pnpm dev` is not "working", and its going quiet is not "done".
- The chat side panel (runner, model, session key) is hidden for a terminal pane.
- Title: defaults to `Terminal`; rename works like any chat. Following the shell's OSC title (`zsh: ~/repos/runner`) is a follow-up, not v1.

### Ending

- Shell exits (`exit`, `⌃D`, crash): the pane shows the ended overlay with terminal copy — "Shell exited" — and **Restart** / **Close**. Restart spawns a fresh shell in the same cwd into the same pane; Close removes the pane (or the tab, if it was the only pane).
- **Close** (pane × , `⌘W`, sidebar context menu) on a running terminal kills the shell and removes the row. Terminals are never archived: they don't appear in Settings → Archived, and there is no "archive" step between running and gone.
- Quit and relaunch: a terminal tab (or pane) comes back in the ended state — "Shell exited", **Restart** — with its position, title, and split intact. Runner never auto-respawns a shell on launch: auto-resume (spec 45) is resume-only, and a shell has nothing to resume.

## Non-goals

- A bottom drawer, a toggleable panel, or any surface outside window → tab → pane.
- Terminals inside the mission workspace (Feed / slot tabs), including "open a shell in this mission's worktree" (spec 61). Natural follow-up; not this spec.
- A shell-path setting, per-terminal profiles, or a `"shell"` entry in `RUNTIME_DEFINITIONS` / Settings → Agents / the chat runtime picker / crew slot dropdowns.
- Exposing terminals over MCP or the CLI.
- Auto-respawning shells on launch, or restoring scrollback across relaunch.
- OSC title tracking, bell, notifications on command completion.

## Decisions

- **No new `NodeType`.** A terminal is a direct session row whose `runtime == "shell"`. The node reconciler already wraps every uncovered direct session in a single-pane `Tab` node (`crates/runner-backend/src/repo/node.rs:720`), so a terminal tab is a `Tab` whose only pane is a shell session, and a terminal pane is a `PaneLeaf` pointing at one. The sidebar and pane header derive their treatment from the member session's runtime. One code path; a terminal tab that later gets split into terminal + chat doesn't change type; `NodeType::Terminal` would have forced every tab-typed branch (`sidebar.rs:631`, pinning, reorder, folders, `⌘1`–`9`) to grow a third arm for no behavior gain.
- **`"shell"` stays out of the catalog.** It is already the documented fallback runtime name (`ops/session.rs:37`), and every non-catalog branch already does the right thing — `resume_plan` falls through to `ResumePlan::fresh()` (`router/runtime.rs:730`), `system_prompt_args` / `permission_mode_args` / `mission_bus_sandbox_args` are empty, `sessions_root_for("shell")` is `None` so codex capture stays off. Putting it in `RUNTIME_DEFINITIONS` would leak it into every runtime picker.
- **No modal.** The start-chat modal's Runner / Runtime mode toggle is not extended with a third mode. A terminal has nothing to configure that `cd` doesn't cover.
- **Close deletes, never archives.** Archive exists so a chat's conversation key survives for a later resume; a shell has no key and no conversation. Keeping the rows around would only pollute Settings → Archived.
- **Keep the row across relaunch.** Dropping it at quit would leave the sibling chat's split with an empty pane; keeping it costs nothing (the quit-kill path already marks it stopped) and comes back with one click.

## Design (to do in `design/runner.pen` before the brief)

- Empty-pane placeholder with two actions: **New chat** (primary) and **New terminal** (secondary), plus the icon/copy change from "No chat in this pane" to something that covers both.
- Terminal pane header: **Terminal** badge in the Chat badge's slot, no status dot, close button — in focused and unfocused states inside a `Cols2` split beside a chat pane.
- Sidebar row for a terminal tab: terminal icon, title, no runtime badge, no attention states — in RECENT and PINNED, selected and not.
- "Shell exited" overlay: **Restart** (primary) / **Close**.
- Sidebar `+` menu and project context menu with the new entries.

## Implementation Phases

1. **Backend: shell spawn and close.** `ops::session::session_start_shell(state, project_id, cwd, cols, rows)`: resolves `$SHELL` (fallback `/bin/zsh`), builds the runner via `runtime_direct_runner("shell", Some(shell_path), None, None)` (`session/manager/mod.rs:1360` already accepts an explicit command for a non-catalog runtime; `session_start_runtime` at `ops/session.rs:733` passes `None` and so rejects `"shell"`), spawns through `spawn_runtime_direct` with `-l`-style login-shell args and no agent injection. `display_name` "Terminal". `ops::session::session_close(state, id)`: kill if running, `repo::node::remove_session`, delete the row — one transaction. `session_resume` for a `"shell"` row is a fresh spawn in the row's cwd. `mark_running_for_resume_on_launch` (`repo/session.rs:339`) skips `"shell"` rows so the launch resumer never tries them.
2. **Pane + tab entry points.** A `StartRequest::Shell` path in `start_chat.rs` that bypasses the modal: `ChatTarget::NewTab` → spawn, `reload_tabs`, `activate_session` (the reconciler creates the tab node); `ChatTarget::Pane` → spawn, `assign_to_active`, `persist_active_tab`. **New terminal** button in the placeholder (`panes.rs:1533`); `new-terminal` action (`⌘T`) in `keymap.rs` and the Shortcuts pane; command-palette, sidebar `+` menu (`sidebar.rs:233`), and project context menu (`sidebar.rs:2907`) entries.
3. **Treatment.** Pane header branches on `SessionRow.runtime == "shell"`: Terminal badge, no status, side panel hidden (`panes.rs:1311`, `:573`). Sidebar row icon and badge (`sidebar.rs:631`). Shell sessions excluded from `working` in the sidebar row derivation and from `record_session_completion` (`ops/node.rs:418`) so they never stamp `last_completed_at`. Ended overlay copy and button labels for shells (`panes.rs:1499`); close-pane and sidebar close route to `session_close` instead of archive for `"shell"` rows.

## Verification

- `cargo test -p runner-backend`: `session_start_shell` spawns the resolved `$SHELL` with login-shell args and no runtime-specific argv, settings, hooks, or env injection; the row has `runtime == "shell"`, `runner_id == "runtime:shell"`, and the requested cwd; `ensure_active_sessions` wraps it in a single-pane `Tab` node; `session_close` removes the node slot and the row in one step; `mark_running_for_resume_on_launch` skips a running shell row; `record_session_completion` never stamps a tab whose only member is a shell.
- `cargo test -p runner-app`: the empty-pane placeholder renders both buttons; a shell session's pane header shows the Terminal badge and no status; the sidebar row for a shell-only tab carries the terminal icon and no attention state; `⌘T` dispatches `new-terminal`.
- `make clippy` and `make fmt`.
- Manual smoke (Jason, per the crews-never-run-the-dev-app rule): `⌘T` opens a shell in the project directory with the user's normal `PATH`; split a chat tab and **New terminal** lands in the agent's cwd; `pnpm dev` streams without the tab ever showing working/unread; `exit` shows "Shell exited" → Restart brings a fresh shell in place; `⌘W` on a running shell kills it and leaves nothing in Settings → Archived; quit with a shell open and relaunch shows the tab in the ended state with its split intact.
