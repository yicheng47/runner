# Native terminal as a pane option

Tracking issue: [#356](https://github.com/yicheng47/runner/issues/356). Status: planned, design settled in `design/runner.pen`.

## Motivation

Runner's premise is that you stop scattering agent work across terminal windows — but there is still no way to run `git status`, `git diff --stat`, or a long `pnpm dev` without leaving the app or burning a chat on it. Today the options are bouncing out to Ghostty or starting a chat with an agent you do not want, and neither fits a ten-second command or a process you want to watch next to the agent editing the code it serves.

The shape that earns its keep is the smallest one: **a pane can hold a terminal instead of a chat**. Everything a split already does — build it from the layout picker, focus it, resize it, close it — keeps working, and the thing in the cell is a `$SHELL` in a real PTY instead of an agent. No new surface, no drawer, no new level in the hierarchy.

A larger design was explored and deliberately postponed: giving a node several switchable tabs, each with its own split, with the tab bar replacing the chat header. It is recorded in `design/runner.pen` under `Spec — Node → Tab → View · DEFERRED` plus two backup chat frames, and nothing in this spec has to be undone to get there later. It was dropped from v1 because the terminal never needed it (a terminal is a pane, and panes already exist), and because it multiplies the sessions live across a restart — see **Relaunch** below for why that is expensive.

## Scope

- **Spawn**: `ops::session::session_start_shell(state, project_id, cwd, cols, rows)` resolves `$SHELL` (fallback `/bin/zsh`), builds the runner through `runtime_direct_runner("shell", Some(shell_path), None, None)` (`crates/runner-backend/src/session/manager/mod.rs:1360` already accepts an explicit command for a non-catalog runtime; `session_start_runtime` passes `None` and so rejects `"shell"` today), and spawns via `spawn_runtime_direct` with login-shell args and no agent injection: no system prompt, no hooks, no MCP config, no permission-mode flags, no conversation-key capture, no bundled `runner` CLI on `PATH`.
- **Entry points**: the empty-pane placeholder gains **New terminal** beside **New chat**; `⌘T` opens a terminal in the active node — into the focused empty pane when there is one, otherwise splitting the focused pane. No modal: a terminal has nothing to configure that `cd` does not cover.
- **cwd**: the cwd of the focused sibling session, else the active project's directory, else `settings.default_working_dir`, else `$HOME`. The shell lands where the agent beside it is working.
- **Pane chrome**: the pane header is replaced by a 26 px **identity line** — icon · name · status dot · `⋯`, with `×` alone at the far right. It renders only when the node is split, exactly as the current header does (`grouped` gates it, `crates/runner-app/src/surfaces/panes.rs:1212`).
- **Actions**: `⋯` opens Stop (`⌘.`) · Rename… · Archive chat. `×` closes the pane. `⌘W` keeps its current meaning on the focused pane.
- **Rename a pane**: `⋯` → Rename… puts the name into an inline field on the identity line; double-clicking the name does the same. Enter commits, Escape reverts, and an empty value restores the default. This is the per-pane rename colleagues have been asking for, and it costs nothing extra here because the identity line is already the one place a pane names itself.
- **Lifecycle**: a shell that exits shows a **Shell exited** card (exit code · shell · cwd) with **Restart** and **Close**, the shell twin of the agent's **Chat paused** card. A pane emptied by Archive shows **New chat** / **New terminal** in place.
- **Relaunch**: terminal panes come back live — a fresh shell at the row's recorded cwd, no command replay, no scrollback. Chat panes stay behind the existing resume-on-launch setting.
- **Everything above the split is unchanged**: the chat header renders exactly as it does today (`WorkspaceHeader`, `crates/runner-app/src/surfaces/panes.rs:257`) — split glyph, auto-composed title, group kebab, layout picker and side-panel toggle. The layout picker keeps its current scope because a node is still one arrangement.

## Out of scope

- Multiple tabs per node, a tab bar, and tab-scoped group actions — the deferred design named above.
- A bottom drawer, a toggleable panel, or any surface outside the existing window → tab → pane hierarchy.
- Terminals inside the mission workspace (Feed / slot panes). Natural follow-up; the same identity line and the same session parenting apply when it happens.
- A shell-path setting, per-terminal profiles, or a `"shell"` entry in `RUNTIME_DEFINITIONS` / Settings → Agents / the chat runtime picker / crew slot dropdowns.
- Exposing terminals over MCP or the CLI.
- OSC title tracking, bell, notifications on command completion, scrollback across relaunch.

## Decisions

- **A terminal is a pane, not a tab type.** No new `NodeType`: a terminal is a direct session row whose `runtime == "shell"`, sitting in a pane of the node's existing layout. The sidebar, pinning, reorder, drag-drop and `⌘1`–`9` never learn a third case. Shell rows must always be placed into a pane on creation, because the node reconciler wraps any uncovered direct session in its own single-pane tab — an unplaced shell would sprout a sidebar row.
- **`"shell"` stays out of the catalog.** It is already the documented fallback runtime name, and every non-catalog branch does the right thing: `resume_plan` falls through to `ResumePlan::fresh()`, the system-prompt / permission-mode / mission-bus arg builders return empty, and `sessions_root_for("shell")` is `None` so codex capture stays off. Adding it to `RUNTIME_DEFINITIONS` would leak it into every runtime picker.
- **One identity line, not a second header.** The current pane header carries six things — icon, name, CHAT chip, status dot, "idle", Stop, `⋯` — at `PANE_HEADER_HEIGHT` (34, `crates/runner-app/src/main.rs:109`). In a 3-pane split that is three bars repeating what the title above already says. The line keeps four: the icon distinguishes chat from terminal so the CHAT chip goes, the dot says the status so the word goes, and Stop moves into `⋯`. Height drops to 26.
- **`⋯` and `×` sit apart, and both are always visible.** They are not equally forgiving — `⋯` opens something dismissible, `×` takes a pane out of the split — so the flex spacer separates them and the destructive one owns its corner alone. Neither is hover-only: a paused agent must always be archivable and a running one always stoppable without hunting for the affordance. `×` is not duplicated inside `⋯`.
- **`×` is a layout action; Archive is a session action.** `×` removes the pane and the split re-flows; a chat's session is untouched and stays in the sidebar, reopenable. Archive ends the session and leaves the pane in place, empty. Acting on the layout changes the geometry and spares the session; acting on the session spares the geometry.
- **Terminals collapse the two.** A shell has no conversation to preserve, so `×` kills the process and drops the pane, and Archive does not apply. Confirm before killing only when a foreground process is still running; a shell at its prompt closes silently. Terminals are never archived and never appear in Settings → Archived.
- **Rename renames the session, not the cell.** It writes `sessions.title` through the existing `ops::session::session_rename`, which is the same field the sidebar and `session_label` already read, so a renamed pane keeps its name when it is closed and reopened, when it moves to another pane, and across relaunch. Because the node's header title is composed from its panes' labels, renaming a pane also renames what the header shows — which is the behaviour people expect and the reason a separate pane-only label would be wrong. Terminals rename identically; their default is the shell name.
- **An empty pane keeps a stub identity line** — dashed icon, "Empty", and a `×`, with no `⋯` because there is no session to act on. Without it the Archive path dead-ends in a pane no mouse can remove.
- **Terminals never contribute to attention.** No status dot, and excluded from `working` / unread / needs-you derivation and from `record_session_completion`. A streaming `pnpm dev` is not "working", and its going quiet is not "done".
- **Relaunch respawns shells, not chats.** A shell has no conversation to lose, so respawning is free and lossless and the pane is usable the moment the window opens — Zed's terminal behaviour. Resuming a chat costs an agent process and replays a conversation, so it stays behind the setting. The cost is the reason: auto-resume runs sequentially with a 300 ms stagger (`AUTO_RESUME_STAGGER_MS`, `crates/runner-app/src/bootstrap.rs:11`) and claude-code spawns serialise behind a 1500 ms launch gate (`CLAUDE_LAUNCH_GATE_GRACE`, `crates/runner-backend/src/session/manager/mod.rs:107`), so N agents cost roughly N × 1.8 s; shells skip both.
- **A missing cwd on respawn splits by runtime.** The hard error `spawn_direct_inner` raises today is right for an agent and wrong for a shell: its stated reason is that portable-pty silently substitutes `$HOME`, which strands an autonomous process in the wrong repo and breaks codex rollout capture. Neither applies to a human at a prompt. A chat pane keeps the hard failure and returns on its ended card; a terminal pane falls back — to the nearest existing ancestor of the recorded cwd, else the project directory, else `$HOME` — resolved by us before the fork, never by portable-pty's implicit substitution, and never silently: Runner writes the substitution into the terminal as its first lines via `feed_output`, the way a terminal would.
- **Titles**: a pane the user renamed keeps its name across relaunch; one auto-named after its command shows the shell name again, because the command is not replayed.

## Design

Settled in `design/runner.pen`:

- `Spec — Split panes, slimmed chrome (64) · v1` — the identity line, its states (focused · idle, focused · working, unfocused, paused, hover, terminal, renaming), one/two/three-pane surfaces, the `⋯` menu, the empty-pane placeholder, the missing-cwd notice, and the relaunch rules.
- `Runner chat — terminal as a pane option (64) · v1` plus the state frames: one pane paused, terminal pane exited, a pane emptied by Archive, the `⋯` menu open, and after quit and relaunch.
- Deferred: `Spec — Node → Tab → View · DEFERRED` and the two `DEFERRED backup` chat frames.

## Implementation phases

1. **Backend: spawn and close.** `session_start_shell` as scoped above; `ops::session::session_close(state, id)` kills if running, removes the node slot, and deletes the row in one transaction; `session_resume` on a `"shell"` row is a fresh spawn at the row's cwd. Exclude shells from `record_session_completion` and from sidebar attention derivation.
2. **Relaunch.** Exempt `"shell"` rows from the resume-on-launch gate in `consume_launch_claims` (`crates/runner-app/src/bootstrap.rs:196`) so they always come back while chats stay behind the setting; resolve a missing cwd per the decision above and write the notice into the terminal before the shell's first output.
3. **Pane chrome.** Replace the pane header with the identity line (`crates/runner-app/src/surfaces/panes.rs:1286`–`1340`), branching on `runtime == "shell"` for the icon and for hiding the status dot and the chat side panel. Wire `⋯` (Stop · Rename… · Archive chat) and `×` (close pane), and the empty-pane stub line.
4. **Entry points and lifecycle cards.** **New terminal** in the empty-pane placeholder, the `new-terminal` action bound to `⌘T` in `keymap.rs` and the Shortcuts pane, the command palette entry, and the **Shell exited** card (Restart / Close) alongside the existing **Chat paused** card (`crates/runner-app/src/ui/session_overlay.rs:158`, `crates/runner-app/src/surfaces/chat_lifecycle.rs:52`).

## Verification

- `cargo test -p runner-backend`: `session_start_shell` spawns the resolved `$SHELL` with login-shell args and no runtime-specific argv, settings, hooks, or env injection; the row has `runtime == "shell"` and the requested cwd; `session_close` removes the node slot and the row together; a running shell row is exempt from the resume-on-launch gate and resumes as a fresh spawn at its cwd; a shell whose cwd no longer exists resolves to the nearest existing ancestor rather than `$HOME`; `record_session_completion` never stamps a tab whose only member is a shell.
- `cargo test -p runner-app`: the empty-pane placeholder renders both actions; a shell pane's identity line shows the terminal icon, no status dot, and no chat side panel; `⋯` lists Stop · Rename… · Archive chat and never Close pane; Rename… opens an inline field on the identity line, Enter commits to `sessions.title`, Escape reverts, and an emptied value restores the default; `×` closes the pane without archiving the session; an emptied pane keeps its stub line and `×`; a single-pane node renders no identity line; `⌘T` dispatches `new-terminal`.
- `make clippy` and `make fmt`.
- Manual smoke (Jason, per the crews-never-run-the-dev-app rule): `⌘T` opens a shell in the agent's cwd with the user's normal `PATH`; `pnpm dev` streams without the node ever showing working or unread; `exit` shows **Shell exited** → **Restart** brings a fresh shell in place; renaming a pane updates both the identity line and the composed header title, and survives a relaunch; `×` on a chat pane re-flows the split and leaves the chat in the sidebar; Archive leaves the pane empty with both actions; quit with a split of one chat and one shell, relaunch, and the shell is live at its cwd while the chat waits on **Resume**.
