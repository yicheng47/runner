# 648 — A general `runner` CLI

> Tracking issue: [#648](https://github.com/yicheng47/runner/issues/648)
> Priority: P1, milestone 0.11. Platforms: macOS and Windows.
> Design: `design/specs/648-runner-cli.pen`, one frame for the Settings → General rows (the `runner` command and the agent skill), drawn before Phase 4.
> Related: [562](./562-mission-spawn.md) (missions as containers) builds its coordinator commands (`spawn`, `ps`, `wait`, `stop`, `peek`, `done`) on this CLI.
> Command set designed with Jason on 2026-09-18: the principles, the command tree and six decisions below.

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
- **Transport.** The CLI is an MCP client of `mcp.sock`, as `runner-mcp`'s `proxy_call_tool` already is (`cli/src/mcp.rs`). It connects per command, calls one tool, prints the result, and exits. There is no new protocol, and the socket's tool registry stays the single source of truth. Because every command fetches from the running app, a CLI call never sees stale schemas.
- **Discovery.** Agents learn that the CLI exists from a `runner` skill embedded in the app, not from MCP registration. New installs get the skill by default and no longer register `runner-mcp`. Existing registrations are left alone, and Settings → MCP keeps its toggles. TRAE has no skills directory, so it keeps MCP registration as its default.
- **Trust.** There is no new boundary: anything that can reach `mcp.sock` today can already do all of this.

## Principles

- **Grammar.** `runner <noun> <verb> <target> [--flags]`. Nouns: `project`, `role`, `crew`, `mission`, `chat`, `session`. Verbs are the same everywhere: `list`, `show`, `create`, `update`, `delete`, plus lifecycle verbs where the noun has a lifecycle.
- **References.**
  - Roles are named by handle, which is unique.
  - Crews and projects are named by id or by exact name. Names are not unique, so an ambiguous name exits 2 and lists the matching ids.
  - Missions and sessions are named by id or by a unique id prefix, as git short SHAs are. An ambiguous prefix exits 2 and lists the matches.
  - The CLI resolves every reference with the matching list tool before calling the target tool, so no tool needs to change for this.
- **Context.** Inside a mission PTY, mission-scoped commands use the caller's mission and handle. Outside, they take `--mission <id>`: agent shells do not keep environment variables between calls, so context is a flag, not an exported variable.
- **Identity.** Outside a mission the CLI acts as the human, the identity MCP calls carry today (`mission_post_human_message`, `mission_post_human_signal`). #562 adds the coordinator seat: an outside caller takes the `lead` seat when the mission has no lead slot, and stays the human otherwise.
- **Output.**
  - stdout carries data and stderr carries messages.
  - By default, commands print tables and key-value blocks. `--json` prints the tool's JSON result verbatim, which is what agents should use.
  - `-q` prints only the id of the object a command created or changed: `id=$(runner mission start --crew peer --goal "…" -q)`.
- **Exit codes.** `0` success. `1` the tool refused, with its message on stderr. `2` a usage error or an unresolvable reference. `3` Runner is not running: "Runner is not running. Open Runner and retry.", the proxy's existing wording. `4` is reserved for #562's `wait` timing out.
- **Defaults follow the shell.** `mission start`, `chat start`, and `project create` use the current directory unless `--project` or `--cwd` (`--path` for projects) is given. MCP defaults to the project's directory; a shell tool acts where it stands.
- **Text inputs.** Long text (a goal, a role prompt, crew conventions) comes from a flag or a file. `-` reads stdin, so `cat brief.md | runner mission start --crew peer --goal-file -` works without shell quoting.
- **Clearing a field.** On `update` and `set`, an empty value clears an optional field (`--model ''`), matching the backend's rule that blank means NULL or inherit.

## Command reference

```
# meta
runner status                                 is Runner running; version, socket, CLI and skill install state
runner help [agents | <noun>]                 runner --version
runner call <tool> [<json>]                   any registered tool, arguments passed through

# projects
runner project list | show <project>
runner project create <name> [--path <dir>]
runner project rename <project> <name>
runner project delete <project> [--force]

# roles
runner role list | show <handle>
runner role create <handle> --runtime <runtime> [--name <display name>] [--model <model>] [--effort <effort>]
                   [--permission <mode>] [--prompt <text> | --prompt-file <path | ->]
                   [--arg <arg>]… [--env KEY=VALUE]… [--cwd <dir>]
runner role update <handle> [the same flags]
runner role delete <handle>

# crews; slots are addressed by their handle inside the crew
runner crew list | show <crew>                show prints the crew and its slot table
runner crew create <name> [--purpose <text>] [--goal <text>] [--conventions-file <path | ->]
runner crew update <crew> [the same flags]
runner crew delete <crew>
runner crew add <crew> <role> [--as <handle>] [--runtime <runtime>] [--model <model>] [--effort <effort>]
runner crew set <crew> <handle> [--as <new handle>] [--runtime] [--model] [--effort]
runner crew remove <crew> <handle>
runner crew lead <crew> <handle>
runner crew order <crew> <handle> <handle>…

# missions
runner mission list [--crew <crew>]
runner mission show <mission>
runner mission start --crew <crew> [--goal <text> | --goal-file <path | ->] [--title <title>]
                     [--project <project> | --cwd <dir>]
runner mission stop | resume | archive | unarchive <mission>
runner mission rename <mission> <title>
runner mission pin | unpin <mission>
runner mission move <mission> (--project <project> | --unfile)
runner mission feed <mission> [--follow] [--since <offset>] [--limit <n>] [--oldest-first]
                    [--types <kind,…>] [--from <handle>]
runner mission answer <mission> <question id> <choice>

# chats and sessions
runner chat start (<role> | --runtime <runtime>) [--model <model>] [--effort <effort>]
                  [--project <project> | --cwd <dir>]
runner session list
runner session resume | restart <session>

# mission-scoped: the caller's mission inside one, --mission <id> outside
runner msg post [--to <handle>] <text>
runner msg read [--since <ulid>] [--from <handle>]
runner signal <type> [--payload <json>]
runner ask <question> [--context <text>]
runner ask --human <prompt> --choices <a,b,…>

# reserved for #562 (coordinator and worker commands)
runner spawn | ps | wait | stop <handle> | peek <handle> | done
```

What each command calls:

| Command | Tool | Notes |
|---|---|---|
| `status` | none | Connects to the socket, reads the app version from the handshake, and checks the install state on disk |
| `project …` | `project_list`, `project_get`, `project_create`, `project_rename`, `project_delete` | `--path` fills `cwd` |
| `role …` | `role_list`, `role_get_by_handle`, `role_create`, `role_update`, `role_delete` | `update` and `delete` resolve the handle to an id first |
| `crew list \| show \| create \| update \| delete` | `crew_*`, and `slot_list` for `show` | `--conventions-file` fills `system_prompt_addendum` |
| `crew add` | `slot_create`, then `slot_update` when `--effort` is given | `slot_create` takes no effort |
| `crew set \| remove \| lead \| order` | `slot_update`, `slot_delete`, `slot_set_lead`, `slot_reorder` | Handles are resolved to slot ids through `slot_list` |
| `mission list` | `mission_list_summary` | Crew name, pending asks, live flag, activity |
| `mission show` | `mission_status` | Covers everything `mission_get` returns, plus sessions, statuses, asks and warnings |
| `mission start` | `mission_start` | `--goal` fills `goal_override`; the title defaults to the goal's first line, else the crew name and the date |
| `mission stop \| archive \| unarchive \| rename \| pin \| unpin \| move` | `mission_stop`, `mission_archive`, `mission_unarchive`, `mission_rename`, `mission_pin`, `mission_set_project` | |
| `mission resume` | `mission_resume` | New tool, below |
| `mission feed` | `mission_feed` | `--types` and `--from` filter client-side; `--follow` is below |
| `mission answer` | `mission_post_human_signal` | Posts `human_response` with `{question_id, choice}` |
| `chat start` | `session_start_direct` | Runtime-only chats need the tool change below |
| `session list` | `session_list` | New tool, below |
| `session resume \| restart` | `session_resume`, `session_restart` | |
| `msg post`, outside a mission | `mission_post_human_message` | Inside a mission: unchanged, appends to the log as the caller's handle |
| `signal`, outside a mission | `mission_post_human_signal` | Inside a mission: unchanged |
| `msg read` | none | Inside a mission only, as today; an outside caller reads with `mission feed` |
| `ask` | inside: appends `ask_lead` or `ask_human` like `runner signal` | Builds the payload so agents never hand-write JSON |
| `call` | any | Passes the JSON object through as the tool's arguments |

## Decisions

1. **The working directory defaults to `$PWD`** for `mission start`, `chat start`, and `project create`, unless `--project` or an explicit directory is given.
2. **`runner status` is the app health check.** The `status busy|idle` alias, deprecated since #124, is removed; it has printed a deprecation notice since then, and status is inferred.
3. **One set of messaging commands.** `msg post` and `signal` work inside a mission as today and outside with `--mission`. There is no separate `mission post` or `mission signal`.
4. **Slots are crew verbs addressed by handle**: `crew add`, `set`, `remove`, `lead`, `order`. No command asks for a slot id.
5. **Shortcut commands over signals.** `ask` and `mission answer` build the signal payloads (`ask_lead`, `ask_human`, `human_response`), and #562 adds `done` and `spawn` the same way. `signal` stays for everything else.
6. **One `mission show`**, backed by `mission_status`.

## Scope

### In scope

- The principles, command tree, and decisions above, with `--json`, `-q`, and the exit codes on every command.
- **Four backend additions**, each small:
  - a `mission_resume` tool over the existing mission-wide Resume;
  - a `session_list` tool for direct chats (id, role or runtime, title, status, project, cwd);
  - `session_start_direct` accepting `runtime` without `role_id`, for the runtime-only chats the app already starts;
  - nothing for `crew add --effort`, which the CLI covers by calling `slot_update` after `slot_create`.
- **`runner mission feed --follow`.** Prints the requested window, then polls `mission_feed` from the last `next_offset` (oldest first, every 500 ms) and prints each new event as it lands. It exits on Ctrl-C, when the mission is archived, or with code 3 if the app goes away. It reads nothing from disk, so it keeps working when #562 moves crewless logs. With `--json` it prints one event per line (NDJSON), which a Claude Code Monitor or a shell pipe can consume directly.
- **`runner help agents`.** A compact guide printed by the binary itself, so it always matches the installed version (Orca's pattern). It covers the command tree, `--json`, `-q`, the exit codes, and the common flows: find a crew, start a mission, follow its feed, answer a question, post to the lead, archive. `runner help` and `runner <noun> --help` remain the full reference.
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
  - arch §9 carries the command reference, drops `status busy|idle`, and describes the two modes.
  - vision §4.9 says the CLI is the main external surface, agents find it through the skill, and MCP is the compatibility layer.
  - `README.md` and `README.zh-CN.md` gain a CLI section together.
  - The mission-watch skill in the memory repo switches from its hand-rolled tail to `runner mission feed --follow --json`.

### Out of scope

- #562's commands: `spawn`, `ps`, `wait`, `stop <handle>`, `peek`, `done`, the coordinator seat, and `mission start` without a crew. They land with #562 on this CLI; the names are reserved here.
- New MCP tools beyond the three backend additions, freezing the MCP surface in code, retiring `runner-mcp`, or removing existing registrations.
- A skill for TRAE, which has no skills directory.
- Launching Runner from the CLI. The not-running error tells the user to open it.
- Remote hosts. The CLI talks to the local app only.
- Shell completions.

## Implementation Phases

### Phase 1 — the command tree over the socket

- Move `endpoint`, `connect_app`, and the call-tool path out of `cli/src/mcp.rs` into a module both binaries use. `runner-mcp` keeps its behavior byte for byte.
- `cli/src/main.rs`: the clap tree beside the in-mission commands, the mode switch on `env::resolve()`, reference resolution, `runner call`, `status`, table, `--json` and `-q` output, and the exit codes. Remove the `status busy|idle` alias and its help text.
- Backend: `mission_resume`, `session_list`, and runtime-only `session_start_direct`, each with its tool test, and the registry list in `mcp/server.rs` updated.
- Tests:
  - every command builds the right tool name and argument JSON, checked without a socket;
  - a parity test fails when the app's tool registry has a tool with no command;
  - references: a role handle, a crew by exact name, an ambiguous crew name, a mission id prefix, and an ambiguous prefix each resolve or exit 2 as specified;
  - `-q` prints only the id; `--json` passes the result through unchanged;
  - the not-running path exits 3 with the message;
  - `msg post` and `signal` behave exactly as before inside a mission, and call the human tools with `--mission` outside;
  - `ask` builds the `ask_lead` and `ask_human` payloads, and `mission answer` builds `human_response`.

### Phase 2 — following and the agent guide

- `mission feed --follow` with the cursor loop and its three exits; `--types` and `--from` filtering; `help agents`.
- Tests: the follow loop prints each event exactly once across polls that return nothing, one event, and several events; an archived mission ends the loop; the filters drop the right events.

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
  - install from Settings; from a plain Terminal.app shell in a repo, `runner crew list`, `runner mission start --crew <crew> --goal …` (the mission's cwd is that repo), `runner mission feed <id> --follow`, `runner msg post --mission <id> --to lead "…"`, then `runner mission archive <id>`;
  - `runner mission answer` resolves a pending ask card;
  - quit Runner mid-follow and see exit code 3;
  - run `runner mission show` inside a mission PTY and get the caller's mission;
  - with no Runner MCP entry anywhere, ask a fresh Claude Code session in Terminal.app to "start a mission with the codex solo crew to …": it loads the skill, reads `runner help agents`, then starts and follows the mission. Repeat from Codex and from pi.
- Smoke on Windows (JASONPC): the same from PowerShell after the install.

## Verification

- [ ] Every tool in the registry has a CLI command, and `runner call` reaches any tool by name.
- [ ] References resolve by handle, exact name, id, and unique prefix; ambiguity exits 2 with the candidates.
- [ ] Inside a mission, `signal`, `msg post`, and `msg read` behave exactly as before, and mission commands default to the caller's mission; outside, `--mission` routes them to the human tools.
- [ ] `--json` output equals the tool result; `-q` prints only the id; the default output is readable without either.
- [ ] Exit codes are 0, 1, 2, and 3 as documented, and Runner not running always gives 3 with the message.
- [ ] `mission start`, `chat start`, and `project create` default to the current directory.
- [ ] `runner status` reports the app, and `status busy|idle` is gone from the binary, the help, and arch §9.
- [ ] `mission resume`, `session list`, and a runtime-only `chat start` work through their new tools.
- [ ] `mission feed --follow` prints every new event once, in order, and exits on archive, on Ctrl-C, and when the app quits.
- [ ] `runner help agents` prints the guide for the installed version.
- [ ] A fresh install puts the skill in both roots for the available agents and registers MCP for TRAE only; an existing install keeps its MCP registrations.
- [ ] Claude Code, Codex, and pi sessions with no Runner MCP entry find and use the CLI through the skill.
- [ ] An app update refreshes a marked skill; an unmarked `runner/` folder is never touched; a removal survives restarts.
- [ ] Install links the sidecar (never the app bundle), refuses a foreign `runner`, and debug builds install `runner-dev` against the debug app.
- [ ] `runner-mcp` behaves unchanged against the same socket.
