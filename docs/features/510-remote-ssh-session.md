# Remote SSH session — direct chats and terminals on another host

Tracking issue: [#510](https://github.com/yicheng47/runner/issues/510). Status: planned. Priority P2.

## Motivation

Every session Runner opens is a local process: the runtime command from the runner template or the runtime picker, spawned by `portable-pty` in a local cwd. The people who use Runner daily mostly do their real work on a remote dev box over SSH, so the agent CLI ends up in an external terminal and Runner only ever sees the local half of the day. The same is true for the odd home-lab box or a Windows PC reached from the Mac.

The cheap observation is that a PTY is the primitive `ssh` itself uses (arch §5). A direct chat whose child is `ssh -t <host> -- … exec claude` gets the terminal renderer, byte-flow idle detection, tabs, panes, the drawer, scrollback, the mouse, and relaunch-on-start for free, because none of those care what process sits on the slave end. What breaks is the handful of places where the spawn path touches the local filesystem: the cwd existence check, the runtime executable lookup, the claude-code conversation-file probe on resume, and codex rollout capture. Those step aside for a remote row and nothing else changes.

This narrows one vision non-goal rather than dropping it. §6 "Remote runners / SSH / multi-host coordination bus" becomes "multi-host coordination bus": mission slots stay local, the `runner` CLI and the NDJSON bus stay on this machine, and the coordination story does not grow a transport. A remote direct chat is a terminal that happens to run somewhere else.

## Scope

### In scope

- **Host on Start Chat.** The Runtime mode of the Start Chat modal (`crates/runner-app/src/surfaces/start_chat.rs`) gains an optional **Host** field beside the working directory. Empty means local, today's behaviour. Non-empty is an `ssh` target as `ssh` itself accepts it: `devbox`, `jason@10.0.0.4`, `build-01.internal`. The working-directory field becomes a plain text path when a host is set, since the local folder picker cannot browse the remote; the placeholder reads `~` and an empty value means the login directory. Runner mode does not get a host in v1: a runner template's system prompt and hooks are composed locally and their delivery over ssh is a separate question.
- **Host on New terminal.** The sidebar `+` menus, the empty-pane path, and the drawer `+` (469) get a **New terminal on host…** sibling next to **New terminal** that opens a small host + directory prompt, then spawns the remote login shell in the same slot the local one would take.
- **Spawn shape.** The PTY child is `ssh -t <host> -- <remote command>`, where the remote command is `cd '<cwd>' && exec <argv>` with every argument POSIX single-quoted, or just the runtime command with no `cd` when cwd is empty. `-t` is required because `ssh` does not allocate a remote TTY when a command is given. The `--session-id` / `--resume` plan (`router::runtime::resume_plan`), model and effort flags, and the skill allowlist `--settings` from 73 travel inside that argv unchanged; they are flags to the remote CLI. `TERM` is forwarded by ssh; window size follows the local PTY through `SIGWINCH`. Runner's own env (`RUNNER_HANDLE`, `RUNNER_*` bus variables, the bundled CLI dir on `PATH`) is not sent: direct chats are off-bus already, and the `runner` CLI does not exist on the host.
- **Auth stays with ssh.** Keys, agents, `ProxyJump`, ports, and usernames live in `~/.ssh/config`. Host-key confirmations and password prompts render in the PTY and the user answers them there, the same way they would answer a claude permission prompt. Runner never stores a credential and never runs `ssh-keygen`, `ssh-copy-id`, or `sshpass`.
- **Session row.** Migration adds `sessions.remote_host TEXT` (NULL for every existing row). `cwd` holds the remote path as typed. `agent_runtime` and `agent_command` keep their meaning (`claude-code`, `codex`, `shell`, and the runtime command), so every list, filter, and resume branch that switches on runtime keeps working; only the argv wrapper is new. Rows and pane headers show `host:cwd` where they show `cwd` today, in the same truncation.
- **Local-only steps bypassed for remote rows.** In `spawn_direct_inner` and `resume_with_fresh_fallback` (`crates/runner-backend/src/session/manager/spawn.rs`): `resolve_runner_executable` (37's local path overrides), the cwd `is_dir` rejection, the claude-code conversation-file probe under `~/.claude/projects`, and codex `spawn_capture`. Each already lives behind a runtime or condition check; a remote row adds one more guard, not a parallel spawn path.
- **Resume and relaunch.** claude-code resumes natively with `--resume <uuid>` on the host, because the uuid was self-assigned at spawn and the conversation file lives there. A failed remote resume falls through the existing `resume_failed` heuristic, which wipes the key and starts fresh next time. codex and shell rows respawn fresh at the recorded host and cwd, which is what a local codex row without a captured key does today. Relaunch-on-start claims remote rows exactly like local ones; a host that is down surfaces as the exited card.
- **Failure surface.** ssh exiting before the remote command ran (bad host, refused key, timeout) shows the existing exited overlay (`SessionOverlay`) with ssh's exit code and its last stderr lines, and **Restart** respawns in place. No retry loop.
- **Recent hosts.** The last few hosts used are remembered in settings and offered as suggestions in the Host field. No Settings page, no host CRUD.
- **Fork disabled** on remote rows (60's copy-on-write of local session dirs cannot reach the host); the button renders disabled with a "Local chats only" tooltip.

### Out of scope

- Mission slots on a remote host, the `runner` CLI on the host, or any bus transport over ssh. The vision non-goal keeps the coordination bus local.
- Remote projects in the sidebar tree, project-scoped cwd resolution, or `project::resolve_cwd` for remote paths. Remote rows live in Recent.
- Opening file links (458), pasting file paths (55), or pasting images (#79) into a remote session: the files are on this machine and the paths would be meaningless there. Link detection may still highlight; activation is a no-op with a tooltip.
- Per-host runtime executable overrides. The remote CLI is whatever the remote login shell finds on `PATH`.
- Installing or updating agent CLIs on the host, mosh, port forwarding, or agent forwarding beyond what the user's ssh config already does.
- Runner-backed (template) remote chats, hooks, and system-prompt delivery over ssh.

## Design

Feature-scoped Pencil file `design/remote-ssh-session.pen`, designed first per the repo convention and reviewed before any code:

1. Start Chat, Runtime mode, with the Host field empty and filled (working directory switches from picker to text).
2. The New terminal on host prompt.
3. A remote chat in a pane: header and sidebar row carrying `host:cwd`, Fork disabled.
4. The exited card for an ssh failure.

## Implementation Phases

1. **Design.** Pencil frames above; stop for review.
2. **Backend.** Migration `sessions.remote_host`; an ssh argv wrapper next to the PATH helpers in `crates/runner-backend/src/session/launch.rs` with unit tests for quoting (spaces, quotes, JSON in `--settings`, empty cwd); `session_start_runtime` and `session_start_shell` accept a host; the four bypass guards; resume and relaunch carry the host.
3. **App.** Host field on Start Chat Runtime mode; New terminal on host in the sidebar, empty pane, and drawer; recent-hosts suggestions; `host:cwd` labels; Fork and file links disabled on remote rows.
4. **Docs.** Narrow vision §6; add a paragraph to arch §5 on the ssh wrapper and the bypassed local checks; archive this spec on close.

## Verification

- A claude-code chat on a remote host starts from the modal, renders the full TUI, shows busy and idle from byte flow, resizes with the pane and the window, and after quit and relaunch resumes into the same conversation by uuid.
- A codex chat and a login shell on a remote host start, and after relaunch respawn fresh at the recorded host and cwd.
- A remote terminal works in a pane and in the drawer; a host-key confirmation and a password prompt are answered by typing in the terminal.
- An unknown host, a refused key, and a killed remote CLI each show the exited card with the right exit code, and Restart retries in place.
- Quoting tests cover a cwd with spaces, an argument containing single quotes, and the `--settings` JSON from 73.
- Local chats and terminals are byte-for-byte unchanged in argv and env; `make verify` passes on macOS and the Windows nightly still spawns local sessions and can spawn a remote one through `ssh.exe`.
