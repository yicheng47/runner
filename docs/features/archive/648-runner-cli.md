# 648 — A general `runner` CLI

> Tracking issue: [#648](https://github.com/yicheng47/runner/issues/648)
> Status: shipped 2026-09-20 in [v0.11.0](https://github.com/yicheng47/runner/releases/tag/v0.11.0) (PRs [#650](https://github.com/yicheng47/runner/pull/650), [#652](https://github.com/yicheng47/runner/pull/652), [#654](https://github.com/yicheng47/runner/pull/654), [#664](https://github.com/yicheng47/runner/pull/664), [#665](https://github.com/yicheng47/runner/pull/665), [#667](https://github.com/yicheng47/runner/pull/667); record [`648-runner-cli/`](../../impls/archive/648-runner-cli/README.md); missions `01M2SW0NRV9W8PTFWR673JZH8S`, `01M2T4KVFBTP3MG9SMVJRG6433`, `01M2W85MNTBMWMTBPXMZJQBQQN`, `01M2YAW36SAP7B8QT8E01J9Q3Y`, `01M2YMFE6BK849GNR1GH3TC682`). The Windows checks and the README screenshot continue as [#668](https://github.com/yicheng47/runner/issues/668).
> Priority: P1, milestone 0.11. With pi ([539](./539-pi-runtime.md)) it makes 0.11.0; the rest of the milestone ships in 0.11.0 if ready, otherwise in 0.11.x. Platforms: macOS and Windows.
> Design: `design/specs/648-runner-cli.pen`: `Spec 648 — Settings · General · Command line` (`TPSwV`, the section in place) and `Spec 648 — Command line · row states` (`dJlVA`, every state of both rows, the Skills pane badge and the two command palette items). Drawn 2026-09-19 before Phase 5.
> Related: [562](../562-mission-spawn.md) (missions as containers, milestone 0.12) builds its coordinator commands (`spawn`, `ps`, `wait`, `stop`, `done`) and the seats an outside agent takes on this CLI. The two stay separate issues: this one is a surface over tools that exist, 562 changes the mission model.
> Command set designed with Jason on 2026-09-18: the principles, the command tree and eight decisions below. The identity model (decision 7), the release plan, and the removal of the MCP integration in the same release (decision 8, with TRAE gaining a skills root so no runtime is cut off) were decided the same day, after 539's mission 2 merged.

## Motivation

`runner` today is only the command agents use inside a mission (`crates/runner-cli/src/main.rs`): `signal`, `msg post`, `msg read`, the deprecated `status`, and `help`. It needs the four `RUNNER_*` variables a mission session gets (`crates/runner-cli/src/env.rs`); an agent direct chat has only `RUNNER_HANDLE` and therefore exits 2 as a partial environment, while a plain off-bus shell prints a notice and exits 0. `cli_install` puts it in `$APPDATA/runner/bin/`, which is on PATH only inside terminals Runner spawns.

Everything else is MCP-only: 39 tools over projects, crews, roles, slots, missions, and sessions (`crates/runner-backend/src/mcp/tools/`), reached through the `runner-mcp` stdio proxy to the app's `mcp.sock`. That has four costs:

- **Setup per client.** On first run Runner writes a `runner-mcp` entry into each available agent's config (`initialize_mcp_defaults` in `app_store/mcp_defaults.rs`). An agent installed later, a client Runner doesn't know, or another machine each need their own entry, and colleagues' configs point into the app. pi has no MCP at all, so a pi session can't reach Runner.
- **Stale schemas.** Clients cache the tool list for the whole session. After a Runner update, a long session keeps old schemas until `/mcp` reconnects (2026-09-17: `slot_create` still asked for `runner_id` after #604).
- **No following.** A tool call returns once, so it can't follow a feed. The mission-watch skill works around this by tailing `events.ndjson` in a Monitor.
- **No scripts or people.** A shell script, a cron job, or a person at a terminal can't start a mission without an MCP client. #193 closed cronjobs on the premise that `mission_start` "over MCP/CLI" lets any external scheduler fire missions; the CLI half of that doesn't exist.

Orca's coordinator surface is a CLI plus a skill (`orca orchestration …`, with the guide served by the binary), not MCP. #562 needs an agent outside the mission (Claude Code in a terminal, a Runner chat, a script) to spawn, wait, and stop agents from a shell.

## Decision: the CLI replaces the MCP integration, discovered through a skill

- The CLI becomes the one external surface for agents, scripts, and people at a terminal. Every tool in the app's registry gets a command, and new capabilities land as commands.
- **The MCP integration is removed in 0.11.0, the release that ships the CLI** (Jason, 2026-09-18: two ways in at once would be confusing). Removed: the `runner-mcp` stdio bridge, Runner's registration of itself in each agent's config (the first-run default and the toggles), and the pinned Runner row in Settings → MCP. On the first launch of 0.11.0, Runner removes the `runner` entry it wrote from every client's config. Settings → MCP stays as the catalog of the user's other servers.
- **Transport.** The socket and its tool registry stay: they are the CLI's transport, an implementation detail no agent is configured to talk to. The CLI is an MCP client of `mcp.sock`, as `runner-mcp`'s `proxy_call_tool` was (`crates/runner-cli/src/mcp.rs`). It connects per command, calls one tool, prints the result, and exits. There is no new protocol, and the socket's tool registry stays the single source of truth. Because every command fetches from the running app, a CLI call never sees stale schemas.
- **Discovery.** Agents learn that the CLI exists from a `runner` skill embedded in the app, installed by default for every available agent. All five runtimes read a skills root: TRAE CLI documents user-level skills at `~/.trae/skills/<name>/SKILL.md` (its manual, `traecli doc skills`, 0.120.52), which Runner had not recorded, so TRAE gains a `skills_dirs` entry and the Skills pane gains its catalog.
- **Trust.** There is no new boundary: anything that can reach `mcp.sock` today can already do all of this.

## Principles

- **Grammar.** `runner <noun> <verb> <target> [--flags]`. Nouns: `project`, `role`, `crew`, `mission`, `chat`, `session`. Verbs are the same everywhere: `list`, `show`, `create`, `update`, `delete`, plus lifecycle verbs where the noun has a lifecycle.
- **References.**
  - Roles are named by handle, which is unique.
  - Crews and projects are named by id or by exact name. Names are not unique, so an ambiguous name exits 2 and lists the matching ids.
  - Missions and sessions are named by id or by a unique id prefix, as git short SHAs are. An ambiguous prefix exits 2 and lists the matches. `session_list` exposes direct chats only, so a mission session can be resumed or restarted only by its full 26-character id; the backend validates that pass-through id.
  - Archived missions are absent from `mission_list`. Any full 26-character mission id therefore passes through after `mission_get` validates that the mission exists; the requested tool then decides whether its operation is valid for an archived mission. A missing name, prefix or full id remains an unresolvable reference and exits 2.
  - The CLI resolves references with the matching list tool before calling the target tool. Full mission ids and full mission-session ids are the documented pass-through cases because their list tools omit archived missions and mission sessions respectively; the backend validates the target or refuses the operation.
- **Context.** Inside a mission PTY, mission-scoped commands use the caller's mission and handle. Outside, they take `--mission <id>`: agent shells do not keep environment variables between calls, so context is a flag, not an exported variable.
- **Identity.** A caller is the person at the app or a handle in the mission's roster; where a call comes from does not decide who it is. Inside a mission the CLI fills the handle from `RUNNER_HANDLE`. Outside a mission the caller acts for the user: posts and answers with no handle appear as the person, which is `human` on the bus. `--as <handle>` exists for a slot the caller holds and must never be used to speak as another agent's slot; #562 adds the seats an outside agent can take. `human` is never a location: `ask_human`, `human_question` and `human_response` keep meaning the person at the app.
- **Output.**
  - stdout carries data and stderr carries messages.
  - By default, each noun prints a curated table or key-value block: list views include only identifying and operational columns, role prompts are blocks, mission snapshots split sessions, pending asks and warnings into readable sections, and feed events are one line each in chronological order. Every table cell collapses whitespace, truncates long values with an ellipsis at a fixed width, and prints null as `-`. `--json` prints the tool's JSON result verbatim, which is what agents should use.
  - `-q` prints only the id of the object a command created or changed: `id=$(runner mission start --crew peer --goal "…" -q)`.
- **Exit codes.** `0` success. `1` the tool refused, with its message on stderr. `2` a usage error or an unresolvable reference. `3` Runner is not running: "Runner is not running. Open Runner and retry.", the proxy's existing wording. `4` is reserved for #562's `wait` timing out. `5` the connection to Runner's socket was denied although the socket is there, which is what a command sandbox does (Codex's default Seatbelt profile returns `EPERM`): "Runner cannot be reached from this process: connecting to its socket was denied, which usually means a command sandbox. Run the same command again outside the sandbox." It says nothing about whether Runner is running, since a stale socket file gives the same answer; the agent runs the command again outside the sandbox and gets the real one. Found in the four-runtime gate on 2026-09-19, where a sandboxed Codex was told Runner was not running while it was.
- **Defaults follow the shell.** `mission start`, `chat start`, and `project create` use the current directory exactly unless `--project` or `--cwd` (`--path` for projects) is given. Relative paths are joined to that directory and normalized lexically (`.` is removed and `..` is resolved) without requiring the path to exist. MCP defaults to the project's directory; a shell tool acts where it stands.
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
runner crew update <crew> [--name <name>] [--purpose <text>] [--goal <text>] [--conventions-file <path | ->]
runner crew delete <crew>
runner crew add <crew> <role> [--as <handle>] [--runtime <runtime>] [--model <model>] [--effort <effort>]
runner crew set <crew> <handle> [--as <new handle>] [--runtime <runtime>] [--model <model>] [--effort <effort>]
runner crew remove <crew> <handle>
runner crew lead <crew> <handle>
runner crew order <crew> <handle> <handle>…

# missions
runner mission list [--crew <crew>]
runner mission show [<mission>]
runner mission start --crew <crew> [--goal <text> | --goal-file <path | ->] [--title <title>]
                     [--project <project> | --cwd <dir>]
runner mission stop | resume | archive | unarchive [<mission>]
runner mission rename <mission> <title>
runner mission pin | unpin [<mission>]
runner mission move [<mission>] (--project <project> | --unfile)
runner mission feed [<mission>] [--follow] [--since <offset>] [--limit <n>] [--oldest-first]
                    [--types <kind,…> | --all] [--from <handle>]
runner mission answer <mission> <question id> <choice>

# chats and sessions
runner chat start (<role> | --runtime <runtime>) [--model <model>] [--effort <effort>]
                  [--project <project> | --cwd <dir>]
runner session list
runner session show | stop | archive | resume | restart <session>

# mission-scoped: the caller's mission inside one, --mission <id> outside
runner msg post [--to <handle>] <text>
runner msg read [--since <ulid>] [--from <handle>]
runner signal <type> [--payload <json>]
runner ask <question> [--context <text>]
runner ask --human <prompt> --choices <a,b,…>

# reserved for #562 (coordinator and worker commands)
runner spawn | ps | wait | stop <handle> | done
```

What each command calls:

| Command | Tool | Notes |
|---|---|---|
| `status` | none | Connects to the socket, reads the app version from the handshake, and checks the install state on disk |
| `project …` | `project_list`, `project_get`, `project_create`, `project_rename`, `project_delete` | `--path` fills `cwd` |
| `role …` | `role_list`, `role_get_by_handle`, `role_create`, `role_update`, `role_delete` | `show` uses the handle-native getter; `update` and `delete` resolve the handle to an id first; create maps the runtime to its registry executable because `role_create` requires `command`; the id-native `role_get` remains available through `runner call` |
| `crew list \| show \| create \| update \| delete` | `crew_*`, and `slot_list` for `show` | `--conventions-file` fills `system_prompt_addendum` |
| `crew add` | `slot_create`, then `slot_update` when `--effort` is given | `slot_create` takes no effort |
| `crew set \| remove \| lead \| order` | `slot_update`, `slot_delete`, `slot_set_lead`, `slot_reorder` | Handles are resolved to slot ids through `slot_list` |
| `mission list` | `mission_list_summary` | Crew name, pending asks, live flag, activity |
| `mission show` | `mission_status` | Covers everything `mission_get` returns, plus sessions, statuses, asks and warnings |
| `mission start` | `mission_start` | `--goal` fills `goal_override`; the title defaults to the goal's first line, else the crew name and the date |
| `mission stop \| archive \| unarchive \| rename \| pin \| unpin \| move` | `mission_stop`, `mission_archive`, `mission_unarchive`, `mission_rename`, `mission_pin`, `mission_set_project` | |
| `mission resume` | `mission_resume` | New tool, below |
| `mission feed` | `mission_feed` | `--types` and `--from` filter client-side; `--follow` is below |
| `mission answer` | `mission_signal` | Posts `human_response` with `{question_id, choice}` |
| `chat start` | `session_start_direct` | Runtime-only chats need the tool change below |
| `session list` | `session_list` | New tool, below |
| `session show` | `session_get` | Persisted row plus the live `AgentStatus` and raw activity; direct chats resolve by id/prefix, mission sessions by full id |
| `session stop` | `session_stop` | Stops a direct chat or mission slot and leaves the row resumable |
| `session archive` | `session_archive` | Stops then archives a direct chat; mission sessions are refused |
| `session resume \| restart` | `session_resume`, `session_restart` | Direct chats resolve through `session_list`; a full mission-session id passes through because that list intentionally omits mission sessions |
| `msg post`, outside a mission | `mission_post` with `from` (decision 7) | Inside a mission: unchanged, appends to the log as the caller's handle |
| `signal`, outside a mission | `mission_signal` with `from` | Inside a mission: unchanged |
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
7. **The socket carries the caller.** `mission_post_human_message` and `mission_post_human_signal` become `mission_post` and `mission_signal` with an optional `from` handle, validated against the mission's roster; absent, the caller is `human`. It is a clean rename with no aliases, as the `role_*` cutover was (#604): with decision 8 the CLI is the socket's only client.
8. **The MCP integration goes in the same release.** The order inside 0.11.0 is fixed: the command tree, then the skill on all five runtimes with Jason's smoke of an agent finding and using the CLI with no MCP entry, and only then the removal. The removal is one upgrade step: for each client in the legacy `initializedMcpClients` set, unregister the `runner` entry only when its command is this installation's `<app data>/bin/runner-mcp` path and it has no arguments, then record completion once every client was removed, absent, not initialized by Runner, or safely skipped because its command points elsewhere. The end of `initialize_mcp_defaults`, the Runner row and its config snippet out of Settings → MCP, and the `runner-mcp` bin target, installer and stale sidecar file are gone. Another installation's entry and a hand edit are left alone and reported in the log. A config Runner cannot read or parse is left untouched for that launch and reported as deferred; a second-read race and a write failure also leave the step pending for retry. A running agent session keeps its already-started bridge process until it ends, so nothing breaks mid-session.

## Scope

### In scope

- The principles, command tree, and decisions above, with `--json`, `-q`, and the exit codes on every command.
- **Five backend additions**, each small:
  - the `from` handle on `mission_post` and `mission_signal` (decision 7), validated against the roster, renamed from the `_human_` tools with no aliases;
  - a `mission_resume` tool over the existing mission-wide Resume;
  - a `session_list` tool for direct chats (id, role or runtime, title, status, project, cwd);
  - `session_start_direct` accepting `runtime` without `role_id`, for the runtime-only chats the app already starts, while preserving a role's explicit runtime override;
  - nothing for `crew add --effort`, which the CLI covers by calling `slot_update` after `slot_create`.
- **`runner mission feed --follow`.** Prints the requested window, then polls `mission_feed` from the last `next_offset` (oldest first, every 500 ms) and prints each new event as it lands. It checks `mission_get` every two seconds, exits 0 on Ctrl-C or when the mission is archived, and exits 3 if the app goes away. It reads nothing from disk, so it keeps working when #562 moves crewless logs. `--types` and `--from` filter client-side. Human output and every follow stream hide `session_status` (including its legacy `runner_status` spelling) and `inbox_read` unless `--all` is passed or `--types` names them; one-shot `--json` remains the verbatim tool result unless a filter flag is present. With `--follow --json` it prints and flushes one event per NDJSON line, with no cursor line, which a Claude Code Monitor or a shell pipe can consume directly.
- **`runner help agents`.** A compact guide printed by the binary itself, so it always matches the installed version (Orca's pattern). It covers the command tree, `--json`, `-q`, the exit codes, and the common flows: find a crew, start a mission, follow its feed, answer a question, post to the lead, archive. `runner help` and `runner <noun> --help` remain the full reference.
- **Embedded `runner` skill.**
  - **Content.** A `SKILL.md` template compiled into the app. Frontmatter `name: runner`, and a description that fires on Runner, missions, crews, roles, or handing work to another agent: "Operate Runner, the local cockpit for CLI coding agents, through the `runner` CLI: start, follow and steer missions, list crews and roles, start chats. Use when the user mentions Runner, a mission, a crew or a role, or asks to hand work to another agent such as Codex, Claude Code or pi." The body is a stub, as Orca's is. It says what Runner is in two lines. It names the executable: `runner` when it is on PATH, else the absolute sidecar path Runner writes into the file at install (quoted, because it contains a space on macOS). It points to `runner help agents` for the version-matched guide. And it states four rules: prefer `--json`; exit code 3 means ask the user to open Runner, and exit code 5 means a sandbox blocked the command, so run it again outside the sandbox before telling the user anything; use `--help` rather than guessing commands; inside a mission, mission commands carry the caller's own handle; outside a mission the caller acts for the user, so posts and answers appear as the person, and must never pass `--as` to speak as a slot it was not given. #562 adds the seats an outside agent can take.
  - **Where.** `~/.claude/skills/runner/` for Claude Code, `~/.agents/skills/runner/` for Codex, Copilot and pi, which all read that root (`RuntimeDefinition::skills_dirs`), and `~/.trae/skills/runner/` for TRAE CLI. Three folders cover five runtimes.
  - **TRAE's skills root.** `RuntimeDefinition::skills_dirs` for TRAE becomes `[".trae/skills"]` (TRAE also reads `~/.coco/skills/` and `~/.trae-cn/skills/`, which Runner does not list). The Skills pane lists it through the generic catalog with a caption and no global toggle, the pi shape: TRAE's only per-skill switch is `disable-model-invocation` in the skill's own frontmatter.
  - **Ownership.** Each installed folder carries a `.runner-managed` marker. On startup Runner rewrites a marked folder whose content or sidecar path has changed, so the skill follows app updates. A `runner/` folder without the marker belongs to someone else: it is never touched, and Settings reports it.
  - **Default and control.** One switch, "Runner skill for agents", in the Command line section of Settings → General, beside the `runner` command row, on by default (`runnerSkillEnabled`). On: the skill is installed in every root that a detected agent reads, at launch and again whenever runtime detection changes, so an agent installed later gets it. Detection alone decides, not the switch in Settings → Agents (Jason, 2026-09-19): that switch is about Runner launching an agent, while the skill is about an agent in any terminal finding Runner, and the shared `~/.agents/skills` root already reached a switched-off Codex whenever pi was on. A root no detected agent reads is left alone, so Runner creates no folder for an agent the user does not have. Off: it is removed from all three roots and stays removed. The switch is the truth: while it is on, a folder deleted by hand comes back at the next launch. It replaces mission 2's once-per-root record (`initializedSkillRoots`), which is no longer consulted. The row's description states the rule once ("Installs a skill for every detected agent, so it can find Runner and drive it through the runner command."), and there is no status line in the normal case or when the switch is off: a list of agent names was redundant. A line appears only for an exception: a foreign `runner/` folder, which is named and left alone, or no supported agent detected yet. It is not in Settings → Skills (Jason, 2026-09-19, reversing the pinned row of 2026-09-18): every other row there hides a skill from one agent, this installs and removes folders, and the skill is the agent half of the CLI, so it sits with the command, as Orca's "Orca CLI" section in its General pane does. In the Skills pane the managed folder stays an ordinary row per agent with a "Managed by Runner" badge, and per-runtime visibility stays with the existing toggles (`skillOverrides`, `[[skills.config]]`).
  - **Debug builds** install `runner-dev`, pointing at the debug sidecar, so a dev app never overwrites the release skill. Its frontmatter description leads with "Development build of Runner (the `make run` app, separate data from the installed app)", keeps the CLI/cockpit purpose, and narrows its trigger to dev, development build, `make run`, or `runner-dev`, so an installed release skill and development skill do not compete for ordinary Runner requests. Its body always names the quoted absolute development sidecar, including for `help agents`, because a bare `runner` outside a development-spawned session may belong to the installed app.
  - **Runner's own sessions.** Mission slots keep the coordination preamble and their per-slot shim. Every direct chat, terminal, resume and fork puts the spawning app's sidecar folder first on PATH (after the mission shim when one exists), so a release skill loaded inside a development session still resolves bare `runner` to the development sidecar. PATH does not control which skills a runtime loads. The socket remains a compile-time debug/release choice; there is no environment override.
- **Three session controls.** `session_get`, `session_stop`, and `session_archive` are thin socket tools over the existing session row/status, kill, and archive paths. `session list` adds the manager's current activity. These make a bad status inspectable and let CLI-driven direct chats be stopped and archived without the window.
- **Install `runner` command.** For people at a terminal and for scripts; agents don't need it, since the skill carries the absolute path. The command is `runner` (checked on 2026-09-19: no Homebrew formula or cask owns the name, and the crates.io, npm and PyPI packages called `runner` are tiny). It is installed by default where that needs no prompt, under the contract the skill uses: once, recorded in settings, never over something Runner did not create, and a removal is respected (Jason, 2026-09-19).
  - **macOS targets, in order.** `~/.local/bin/runner` when that folder is on the login shell's PATH, which Runner already resolves (`runtime_shell_env`); else `/usr/local/bin/runner` when that folder is on the login PATH and writable without escalation (Intel Macs where Homebrew owns it). Runner links into no other folder: the other writable entries on a typical PATH belong to a version manager or a package manager, and a link there vanishes on an upgrade. The default waits for the login shell discovery to succeed and does nothing, unrecorded, when it has not.
  - **macOS, neither target available.** The default does nothing. The explicit Install links `/usr/local/bin/runner` through the macOS administrator prompt, which is what VS Code and Zed do (Zed's `install_cli` tries a plain symlink to a fixed `/usr/local/bin/zed`, then `osascript … with administrator privileges`; neither detects a PATH). `/usr/local/bin` is in `/etc/paths`, so the link is on every Mac's PATH. A dismissed prompt is not an error. Runner never edits a shell profile and never prompts at launch.
  - **The link targets the sidecar** `$APPDATA/runner/bin/runner` and never `Runner.app/Contents/MacOS/`: on a case-insensitive volume `MacOS/runner` resolves to the GUI binary `Runner`, and launching it bootstraps a second app that kills every live session (2026-09-18). The sidecar is rewritten in place on update, so the link survives updates.
  - **Windows:** the default adds the sidecar directory to the user PATH (`HKCU\Environment`, keeping the value's type) and broadcasts the environment change; user scope, no elevation. Uninstall removes exactly that entry.
  - **Ownership.** Runner owns a link only when it is a symlink to this app's sidecar; on Windows, only the PATH entry equal to its sidecar directory. Anything else under the name is foreign: never replaced, and reported. The row also warns when another `runner` earlier on the login PATH shadows the link.
  - **Where the action lives.** The first row of the Command line section in Settings → General (Install or Uninstall, the installed path, the foreign and shadowed warnings), above the skill switch, and the same two actions in the command palette, as VS Code and Zed have them. `runner status` prints the command's state.
  - **Debug builds** install as `runner-dev` on macOS and reach the debug app's socket, as the sidecar already does through `cfg!(debug_assertions)`. On Windows a directory on PATH cannot rename the binary, so a debug build installs nothing there and its row says so.
- **Docs.**
  - arch §9 carries the command reference, drops `status busy|idle`, describes the two modes, and states the identity rule: a caller is the person or a handle.
  - vision §4.9 says the CLI is the external surface and agents find it through the skill; every mention of Runner's own MCP server as a way in goes, in the vision, arch §9.5 and both READMEs.
  - `README.md` and `README.zh-CN.md` gain a CLI section together.
  - The mission-watch skill in the memory repo switches from its hand-rolled tail to `runner mission feed --follow --json`.

### Out of scope

- #562's commands: `spawn`, `ps`, `wait`, `stop <handle>`, `done`, seats without a session (how an outside agent gets a handle), and `mission start` without a crew. They land with #562 on this CLI; the names are reserved here, and the caller handle they need is decision 7.
- Socket tools beyond the backend additions above, and replacing MCP as the socket's wire protocol.
- Removing a `runner` MCP entry the user wrote by hand, and anything about the user's other MCP servers: Settings → MCP keeps its catalog, toggles and editing for them.
- Toggling TRAE skills from Runner.
- Launching Runner from the CLI. The not-running error tells the user to open it.
- Remote hosts. The CLI talks to the local app only.
- Shell completions.

## Implementation Phases

### Phase 1 — the command tree over the socket

- Move `endpoint`, `connect_app`, and the call-tool path out of `crates/runner-cli/src/mcp.rs` into code both binaries use. `runner-mcp` keeps working until Phase 4 removes it.
- `crates/runner-cli/src/main.rs`: the clap tree beside the in-mission commands, the mode switch on `env::resolve()`, the caller handle (`RUNNER_HANDLE` inside a mission, `--as` outside, else none), reference resolution, `runner call`, `status`, table, `--json` and `-q` output, and the exit codes. Remove the `status busy|idle` alias and its help text.
- Backend: `mission_post` and `mission_signal` with the `from` handle, renamed from the `_human_` tools, `mission_resume`, `session_list`, and runtime-only `session_start_direct`, each with its tool test, and the registry list in `mcp/server.rs` updated.
- Tests:
  - every command builds the right tool name and argument JSON, checked without a socket;
  - a parity test fails when the app's tool registry has a tool with no command;
  - references: a role handle, a crew by exact name, an ambiguous crew name, a mission id prefix, and an ambiguous prefix each resolve or exit 2 as specified;
  - `-q` prints only the id; `--json` passes the result through unchanged;
  - the not-running path exits 3 with the message;
  - `msg post` and `signal` behave exactly as before inside a mission, and call `mission_post` and `mission_signal` with `--mission` outside, passing `--as` through as `from` and nothing when absent; a `from` not in the roster is refused by the tool and exits 1;
  - `ask` builds the `ask_lead` and `ask_human` payloads, and `mission answer` builds `human_response`.

### Phase 2 — following and the agent guide

- `mission feed --follow` with the cursor loop and its three exits; `--types` and `--from` filtering; `help agents`.
- Tests: the follow loop prints each event exactly once across polls that return nothing, one event, and several events; an archived mission ends the loop and a disconnected app exits 3; the filters and default noise suppression drop the right events, `--all` restores them, and follow JSON is parseable flushed NDJSON.

### Phase 3 — the embedded skill on all five runtimes

- `crates/runner-backend/src/agent_skill.rs`: the template, rendering with the sidecar path, and install, remove, and status for the three roots with the marker; the startup refresh.
- `router/runtime.rs`: TRAE's `skills_dirs` becomes `[".trae/skills"]`; `surfaces/settings/skills.rs` gains the TRAE caption; the tests that expect an empty TRAE catalog change.
- `app_store`: `initialize_skill_defaults` beside `initialize_mcp_defaults`, which this phase leaves alone.
- Every spawn, resume, and fork carries the spawning app's sidecar folder on PATH, with a mission shim still ahead of it.
- `session_get`, `session_stop`, and `session_archive`, their CLI commands, and `session list` activity.
- Tests:
  - install writes the three folders with the marker and the rendered path;
  - a startup refresh rewrites a stale marked folder and leaves an unmarked `runner/` alone;
  - removal deletes only marked folders, and a restart after removal does not reinstall (mission 2's once-per-root record; Phase 5 replaces it with the switch);
  - the TRAE catalog lists `~/.trae/skills` and nothing else.
- Jason's smoke gates Phase 4: with the Runner entry switched off in Settings → MCP, a fresh Claude Code, Codex, pi and TRAE session each find the skill and start and follow a mission through the CLI.

### Phase 4 — remove the MCP integration

- `app_store/mcp_defaults.rs`: `initialize_mcp_defaults` goes; one upgrade step checks only clients in `initialized_mcp_clients`, unregisters only a `runner` entry that matches this installation's bridge path with no arguments through the existing per-client write path, records completion after every client reaches a final outcome, and logs a mismatched command without touching it. A config that cannot be read or parsed is left untouched for that launch and logged as deferred; a second-read race or write failure leaves completion unset so the next launch retries.
- `surfaces/settings/mcp.rs` and `ops/mcp.rs`: the pinned Runner row, `mcp_integration_status`, `mcp_set_integration`'s UI callers and `mcp_config_snippet` go; the remaining removal-only backend operation is named `remove_runner_entry`, and the catalog of the user's other servers stays.
- `crates/runner-cli/`: the `runner-mcp` bin target, `mcp_main.rs` and the stdio proxy in `mcp.rs` go, keeping the shared socket client; `cli_install::install_mcp_cli` goes and startup deletes a stale `<app data>/bin/runner-mcp`; the bundle scripts and workflows stop packaging and signing it.
- Tests: the upgrade step removes exactly the entries Runner wrote, runs once after only final outcomes, leaves a hand-written entry alone, and leaves an unparseable config untouched for the current launch while keeping the step pending for retry; a fresh settings file registers nothing; the Settings catalog renders without the Runner row; the stale sidecar is deleted.

### Phase 5 — design, then the install actions

- Draw the Command line section of Settings → General (the `runner` command row and the Runner skill switch with its status line) and stop for sign-off. Drawn 2026-09-19 in `design/specs/648-runner-cli.pen`, frames `TPSwV` and `dJlVA`.
- `crates/runner-backend/src/cli_install.rs`: target selection from the login PATH, install, uninstall and status for both platforms, the escalated macOS path, ownership and shadow detection, and the `runner-dev` name for debug builds. `app_store`: the once-only default beside `initialize_skill_defaults`, run again when the login shell discovery finishes. `app_store/skill_defaults.rs`: the skill follows `runnerSkillEnabled` in place of the once-per-root record. `crates/runner-app/src/surfaces/settings_page.rs`: the Command line section with both rows; `surfaces/settings/skills.rs`: the "Managed by Runner" badge; `surfaces/command_palette.rs`: the two command actions; `cli`: the command's state in `runner status`.
- Tests: each macOS target is chosen or skipped from a given login PATH and folder state; the link target is the sidecar path; a foreign `runner` is left untouched and reported; a shadowing `runner` is reported; reinstalling is a no-op; the default runs once, waits for discovery, and does not reinstall after a removal; uninstall removes only what Runner owns; the Windows PATH edit adds and removes exactly one entry and keeps the value's type; no test touches a real home, `/usr/local/bin`, the registry or `osascript`. The skill switch: on installs for the roots of the detected agents whatever their switch in Settings → Agents says, and follows a later detection; off removes all three and survives a restart; on restores a folder deleted by hand; a foreign folder is reported and untouched; a settings file written by mission 2's build decodes with the switch on.

### Phase 6 — docs and smoke

- The arch, vision, README, and mission-watch updates above.
- Smoke on macOS:
  - `runner` is on PATH after the first launch with no click when `~/.local/bin` is on the login PATH, else after Install from Settings or the command palette; from a plain Terminal.app shell in a repo, `runner crew list`, `runner mission start --crew <crew> --goal …` (the mission's cwd is that repo), `runner mission feed <id> --follow`, `runner msg post --mission <id> --to lead "…"`, then `runner mission archive <id>`;
  - `runner mission answer` resolves a pending ask card;
  - quit Runner mid-follow and see exit code 3;
  - run `runner mission show` inside a mission PTY and get the caller's mission;
  - with no Runner MCP entry anywhere, ask a fresh Claude Code session in Terminal.app to "start a mission with the codex solo crew to …": it loads the skill, reads `runner help agents`, then starts and follows the mission. Repeat from Codex and from pi.
- Smoke on Windows (JASONPC): the same from PowerShell after the install.

## Verification

- [ ] Every tool in the registry has a CLI command, and `runner call` reaches any tool by name.
- [ ] References resolve by handle, exact name, id, and unique prefix; ambiguity exits 2 with the candidates.
- [ ] Inside a mission, `signal`, `msg post`, and `msg read` behave exactly as before, and mission commands default to the caller's mission and handle; outside, `--mission` routes them to `mission_post` and `mission_signal`, `--as` becomes `from`, and no handle means the person.
- [ ] `--json` output equals the tool result; `-q` prints only the id; the default output is readable without either.
- [ ] Exit codes are 0, 1, 2, 3 and 5 as documented; Runner not running always gives 3 with the message, and a connection denied by a sandbox gives 5, never 3.
- [ ] `mission start`, `chat start`, and `project create` default to the current directory.
- [ ] `runner status` reports the app, and `status busy|idle` is gone from the binary, the help, and arch §9.
- [ ] `mission resume`, `session list`, and a runtime-only `chat start` work through their new tools.
- [ ] `mission feed --follow` prints every new event once, in order, and exits on archive, on Ctrl-C, and when the app quits.
- [ ] `runner help agents` prints the guide for the installed version.
- [ ] A fresh install puts the skill in the three roots for the available agents and registers no MCP entry anywhere.
- [ ] The first launch of 0.11.0 on an existing install removes the `runner` entry Runner wrote from each client's config, once, leaves hand-written entries alone, and retries a config it could not read or parse on the next launch.
- [ ] Settings → MCP shows the user's other servers and no Runner row; `runner-mcp` is absent from the bundle and from `<app data>/bin/`.
- [ ] TRAE's skills appear in the Skills pane from `~/.trae/skills`.
- [ ] Claude Code, Codex, pi, and TRAE sessions with no Runner MCP entry find and use the CLI through the skill.
- [ ] An app update refreshes a marked skill; an unmarked `runner/` folder is never touched; switching the skill off removes it and survives restarts, and while the switch is on a folder deleted by hand returns at the next launch.
- [ ] The `runner` command is installed by default where no prompt is needed (`~/.local/bin` or a writable `/usr/local/bin` on the login PATH; the user PATH on Windows), once, and a removal is respected; elsewhere Install from Settings or the command palette links `/usr/local/bin` through the administrator prompt. It links the sidecar (never the app bundle), refuses a foreign `runner`, reports a shadowing one, and debug builds install `runner-dev` against the debug app.
- [ ] Settings → General has a Command line section with the `runner` command row and one Runner skill switch; the Skills pane shows the managed folder as an ordinary row with a "Managed by Runner" badge.
