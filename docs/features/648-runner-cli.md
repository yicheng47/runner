# 648 — A general `runner` CLI

> Tracking issue: [#648](https://github.com/yicheng47/runner/issues/648)
> Priority: P1, milestone 0.11. Platforms: macOS and Windows.
> Design: `design/specs/648-runner-cli.pen`, one frame for the Settings → General rows (the `runner` command and the agent skill), drawn before Phase 4.
> Related: [562](./562-mission-spawn.md) (missions as containers) builds its outside-coordinator commands (`spawn`, `ps`, `wait`, `done`) on this CLI.

## Motivation

`runner` today is only the command agents use inside a mission (`cli/src/main.rs`): `signal`, `msg post`, `msg read`, the deprecated `status`, and `help`. It needs the four `RUNNER_*` variables a mission session gets (`cli/src/env.rs`); in a direct chat it prints a notice and exits 0. `cli_install` puts it in `$APPDATA/runner/bin/`, which is on PATH only inside terminals Runner spawns.

Everything else is MCP-only: 39 tools over projects, crews, roles, slots, missions, and sessions (`crates/runner-backend/src/mcp/tools/`), reached through the `runner-mcp` stdio proxy to the app's `mcp.sock`. That has four costs:

- **Setup per client.** On first run Runner writes a `runner-mcp` entry into each available agent's config (`initialize_mcp_defaults` in `app_store/mcp_defaults.rs`). An agent installed later, a client Runner doesn't know, or another machine each need their own entry, and colleagues' configs point into the app. pi has no MCP at all, so a pi session can't reach Runner.
- **Stale schemas.** Clients cache the tool list for the whole session. After a Runner update, a long session keeps old schemas until `/mcp` reconnects (2026-09-17: `slot_create` still asked for `runner_id` after #604).
- **No following.** A tool call returns once, so it can't follow a feed. The mission-watch skill works around this by tailing `events.ndjson` in a Monitor.
- **No scripts or people.** A shell script, a cron job, or a person at a terminal can't start a mission without an MCP client. #193 closed cronjobs on the premise that `mission_start` "over MCP/CLI" lets any external scheduler fire missions; the CLI half of that doesn't exist.

Orca's coordinator surface is a CLI plus a skill (`orca orchestration …`, with the guide served by the binary), not MCP. #562 needs an agent outside the mission (Claude Code in a terminal, a Runner chat, a script) to spawn, wait, and stop agents from a shell.

## Decision: the CLI first, discovered through a skill, MCP as a compatibility layer

- The CLI becomes the main surface for agents, scripts, and people at a terminal. Every MCP tool gets a command, and new capabilities land in the CLI first.
- MCP stays, with the same tools over the same socket, for clients that prefer tools and for existing configs. It gets nothing the CLI lacks. Whether to retire `runner-mcp` is decided after 0.11, based on how colleagues actually use it.
- **Transport.** The CLI is an MCP client of `mcp.sock`, as `runner-mcp`'s `proxy_call_tool` already is (`cli/src/mcp.rs`). It connects per command, calls one tool, prints the result, and exits. There is no new protocol and no backend change for the wrapped commands, and the socket's tool registry stays the single source of truth. Because every command fetches from the running app, a CLI call never sees stale schemas.
- **Discovery.** Agents learn that the CLI exists from a `runner` skill embedded in the app, not from MCP registration. New installs get the skill by default and no longer register `runner-mcp`. Existing registrations are left alone, and Settings → MCP keeps its toggles. TRAE has no skills directory, so it keeps MCP registration as its default.
- **Trust.** There is no new boundary: anything that can reach `mcp.sock` today can already do all of this.

## Scope

### In scope

- **One binary, two modes.**
  - **Inside a mission PTY** (all four `RUNNER_*` set): `signal`, `msg post`, and `msg read` are unchanged, and every command that takes a mission defaults to the caller's own.
  - **Anywhere else:** the control commands below act as the human, the same identity MCP calls carry today (`mission_post_human_message`, `mission_post_human_signal`). The in-mission commands keep their off-bus notice.
- **Command tree.** Nouns and verbs mirror the tools one to one. The primary id is positional, every other input field is a `--kebab-case` flag, and a role, crew, or project accepts an id or a handle wherever the tool does.

  | Command | Tool |
  |---|---|
  | `runner project list \| show \| create \| rename \| delete` | `project_*` |
  | `runner role list \| show \| create \| update \| delete` | `role_*` (`show` accepts a handle through `role_get_by_handle`) |
  | `runner crew list \| show \| create \| update \| delete` | `crew_*` |
  | `runner crew slot list \| add \| update \| remove \| lead \| reorder` | `slot_*` |
  | `runner mission list [--summary] \| show \| status \| start \| stop \| archive \| unarchive \| pin \| rename \| project` | `mission_*` |
  | `runner mission post <id> "<text>" [--to <handle>]` | `mission_post_human_message` |
  | `runner mission signal <id> <type> [--payload <json>]` | `mission_post_human_signal` |
  | `runner mission feed <id> [--limit] [--oldest-first] [--since <offset>] [--follow]` | `mission_feed` |
  | `runner chat start --role <role> \| --runtime <runtime> [--cwd] [--project]` | `session_start_direct` |
  | `runner session resume \| restart <id>` | `session_resume`, `session_restart` |
  | `runner call <tool> [<json>]` | any tool, arguments passed through |

  `runner call` is the escape hatch: a tool added to the app works from the CLI the same day, before it gets a hand-written command.
- **Output.** Commands print readable tables and key-value blocks by default. `--json` prints the tool's JSON result verbatim, which is what agents should use. Exit codes: `0` success, `1` the tool returned an error (message on stderr), `2` a usage error, `3` Runner is not running ("Runner is not running. Open Runner and retry.", the proxy's existing wording).
- **`runner mission feed --follow`.** Prints the requested window, then polls `mission_feed` from the last `next_offset` (oldest first, every 500 ms) and prints each new event as it lands. It exits on Ctrl-C, when the mission is archived, or with code 3 if the app goes away. It reads nothing from disk, so it keeps working when #562 moves crewless logs. With `--json` it prints one event per line (NDJSON), which a Claude Code Monitor or a shell pipe can consume directly.
- **`runner help agents`.** A compact guide printed by the binary itself, so it always matches the installed version (Orca's pattern). It covers the command tree, `--json`, and the common flows: find a crew, start a mission, follow its feed, post to the lead, archive. `runner help` and `runner <noun> --help` remain the full reference.
- **Embedded `runner` skill.**
  - **Content.** A `SKILL.md` template compiled into the app. Frontmatter `name: runner`, and a description that fires on Runner, missions, crews, roles, or handing work to another agent: "Operate Runner, the local cockpit for CLI coding agents, through the `runner` CLI: start, follow and steer missions, list crews and roles, start chats. Use when the user mentions Runner, a mission, a crew or a role, or asks to hand work to another agent such as Codex, Claude Code or pi." The body is a stub, as Orca's is. It says what Runner is in two lines. It names the executable: `runner` when it is on PATH, else the absolute sidecar path Runner writes into the file at install (quoted, because it contains a space on macOS). It points to `runner help agents` for the version-matched guide. And it states four rules: prefer `--json`; exit code 3 means ask the user to open Runner; use `--help` rather than guessing commands; inside a mission, `msg` and `signal` talk to the crew while the control commands act as the human.
  - **Where.** `~/.claude/skills/runner/` for Claude Code, and `~/.agents/skills/runner/` for Codex, Copilot and pi, which all read that root (`RuntimeDefinition::skills_dirs`). Two folders cover four runtimes.
  - **Ownership.** Each installed folder carries a `.runner-managed` marker. On startup Runner rewrites a marked folder whose content or sidecar path has changed, so the skill follows app updates. A `runner/` folder without the marker belongs to someone else: it is never touched, and Settings reports it.
  - **Default.** On first run the skill is installed for every available agent that reads one of the two roots. Like `initialized_mcp_clients`, this happens once, so a removal is respected. One toggle in Settings → General installs or removes both folders. Per-runtime visibility stays with the Skills pane's existing toggles (`skillOverrides`, `[[skills.config]]`).
  - **Debug builds** install `runner-dev`, pointing at the debug sidecar, so a dev app never overwrites the release skill.
  - **Runner's own sessions** are unchanged. Mission slots keep the coordination preamble, and direct chats find the global skill like any other session.
- **Install `runner` command.** One row in Settings → General with an **Install** button, for people at a terminal and for scripts. Agents don't need it, since the skill carries the absolute path. The row shows the installed path once done, or a warning if something else already owns the name.
  - **macOS:** the button links `~/.local/bin/runner` to the sidecar `$APPDATA/runner/bin/runner` and says so if `~/.local/bin` is not on the login shell's PATH. The link targets the sidecar and never `Runner.app/Contents/MacOS/`: on a case-insensitive volume `MacOS/runner` resolves to the GUI binary `Runner`, and launching it bootstraps a second app that kills every live session (2026-09-18).
  - **Windows:** the button adds the sidecar directory to the user PATH (`HKCU\Environment`) and broadcasts the environment change.
  - **Both:** it never replaces a `runner` it did not create. Debug builds install as `runner-dev` and reach the debug app's socket, as `runner-mcp` already does through `cfg!(debug_assertions)`.
- **Docs.**
  - arch §9 describes the two modes and the command tree.
  - vision §4.9 says the CLI is the main external surface, agents find it through the skill, and MCP is the compatibility layer.
  - `README.md` and `README.zh-CN.md` gain a CLI section together.
  - The mission-watch skill in the memory repo switches from its hand-rolled tail to `runner mission feed --follow --json`.

### Out of scope

- #562's commands: `spawn`, `ps`, `wait`, `stop <handle>`, `done`, and `mission new` without a crew. They land with #562 on this CLI.
- New MCP tools, freezing the MCP surface in code, or retiring `runner-mcp`.
- Launching Runner from the CLI. The not-running error tells the user to open it.
- Remote hosts. The CLI talks to the local app only.
- Removing existing `runner-mcp` registrations from agent configs.
- A skill for TRAE, which has no skills directory.
- Shell completions.

## Implementation Phases

### Phase 1 — the command tree over the socket

- Move `endpoint`, `connect_app`, and the call-tool path out of `cli/src/mcp.rs` into a module both binaries use. `runner-mcp` keeps its behavior byte for byte.
- `cli/src/main.rs`: the clap tree above beside the in-mission commands, the mode switch on `env::resolve()`, `runner call`, table and `--json` output, and the exit codes.
- Tests:
  - every command builds the right tool name and argument JSON, checked without a socket;
  - a parity test fails when the app's tool registry (the list `mcp/server.rs` asserts) has a tool with no command;
  - the not-running path exits 3 with the message;
  - `--json` passes the result through unchanged;
  - the in-mission commands still behave as before with and without `RUNNER_*`.

### Phase 2 — following and the agent guide

- `mission feed --follow` with the cursor loop and its three exits; `help agents`.
- Tests: the follow loop prints each event exactly once across polls that return nothing, one event, and several events; an archived mission ends the loop.

### Phase 3 — the embedded skill and the new default

- `crates/runner-backend/src/agent_skill.rs`: the template, rendering with the sidecar path, and install, remove, and status for the two roots with the marker; the startup refresh.
- `app_store`: `initialize_skill_defaults` beside `initialize_mcp_defaults`. For new installs, `initialize_mcp_defaults` stops registering Claude Code, Codex, and Copilot. Clients already in `initialized_mcp_clients` are untouched, and TRAE still registers.
- Tests:
  - install writes both folders with the marker and the rendered path;
  - a startup refresh rewrites a stale marked folder and leaves an unmarked `runner/` alone;
  - removal deletes only marked folders, and a restart after removal does not reinstall;
  - a fresh settings file registers MCP for TRAE only, while an existing one keeps its registrations.

### Phase 4 — design, then the install actions

- Draw the Settings → General rows (the command and the skill) in `design/specs/648-runner-cli.pen` and stop for sign-off.
- `crates/runner-backend/src/cli_install.rs`: install and uninstall for both platforms, detection of an existing `runner`, and the `runner-dev` name for debug builds. `crates/runner-app/src/surfaces/settings_page.rs`: the two rows.
- Tests: the link target is the sidecar path; an existing foreign `runner` is left untouched and reported; reinstalling is a no-op.

### Phase 5 — docs and smoke

- The arch, vision, README, and mission-watch updates above.
- Smoke on macOS:
  - install from Settings; from a plain Terminal.app shell, `runner crew list`, `runner mission start --crew <crew> --goal …`, `runner mission feed <id> --follow`, and `runner mission post <id> "…" --to lead`, then `runner mission archive <id>`;
  - quit Runner mid-follow and see exit code 3;
  - run `runner mission status` inside a mission PTY and get the caller's mission;
  - with no Runner MCP entry anywhere, ask a fresh Claude Code session in Terminal.app to "start a mission with the codex solo crew to …": it loads the skill, reads `runner help agents`, then starts and follows the mission. Repeat from Codex and from pi.
- Smoke on Windows (JASONPC): the same from PowerShell after the install.

## Verification

- [ ] Every tool in the registry has a CLI command, and `runner call` reaches any tool by name.
- [ ] Inside a mission, `signal`, `msg post`, and `msg read` behave exactly as before, and mission commands default to the caller's mission.
- [ ] `--json` output equals the tool result; the default output is readable without it.
- [ ] Exit codes are 0, 1, 2, and 3 as documented, and Runner not running always gives 3 with the message.
- [ ] `mission feed --follow` prints every new event once, in order, and exits on archive, on Ctrl-C, and when the app quits.
- [ ] `runner help agents` prints the guide for the installed version.
- [ ] Install links the sidecar (never the app bundle), refuses a foreign `runner`, and debug builds install `runner-dev` against the debug app.
- [ ] A fresh install puts the skill in both roots for the available agents and registers MCP for TRAE only; an existing install keeps its MCP registrations.
- [ ] Claude Code, Codex, and pi sessions with no Runner MCP entry find and use the CLI through the skill.
- [ ] An app update refreshes a marked skill; an unmarked `runner/` folder is never touched; a removal survives restarts.
- [ ] `runner-mcp` behaves unchanged against the same socket.
