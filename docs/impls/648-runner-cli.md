# General `runner` CLI — implementation plan

Plan for feature [648](../features/648-runner-cli.md) ([#648](https://github.com/yicheng47/runner/issues/648), milestone 0.11; with pi it makes 0.11.0). The spec carries the principles, the command reference, the tool table, the eight decisions and the six phases; this file is the mission sequence, what reading the code changed, and what has landed.

## Status (2026-09-18)

Spec and command set designed with Jason on 2026-09-18; the identity model (decision 7: a caller is the person at the app or a roster handle, never a location) and the release plan were decided the same day, and `peek` was dropped from the reserved verbs. The spec landed on `main` in `756a60f`. Later that day Jason decided that the MCP integration is removed in 0.11.0 itself rather than a release later (decision 8: two ways in at once would be confusing) and that TRAE gets a skills root so no runtime is cut off; the spec, this plan and the mission 1 brief were updated on the branch. Mission 1 is briefed in [`briefs/648-m1-command-tree.md`](./briefs/648-m1-command-tree.md) on branch `feat/648-runner-cli` and has not started.

## Missions

| Mission | Scope | Crew | State |
| --- | --- | --- | --- |
| 1 | Spec phase 1: a shared socket client for both binaries, the `runner <noun> <verb>` tree over every tool, references, `--json` / `-q` / tables, exit codes 0–3, the caller handle (`RUNNER_HANDLE` inside, `--as` outside), `mission_post` and `mission_signal` with `from` (a clean rename of the `_human_` tools, no aliases), `mission_resume`, `session_list`, runtime-only `session_start_direct`, the parity test, `status` as the health check with `status busy\|idle` removed, arch §9. | codex peer | Briefed, not started |
| 2 | Spec phases 2 and 3: `mission feed --follow` with `--types` and `--from`, `runner help agents`, the embedded `runner` skill (template, three roots, `.runner-managed` marker, startup refresh, `runner-dev` for debug builds, installed once by default), and TRAE's skills root (`skills_dirs`, the Skills pane caption). Ends with Jason's smoke: the Runner MCP entry switched off, then Claude Code, Codex, pi and TRAE each find the skill and drive a mission through the CLI. | codex peer | Not started |
| 3 | Spec phase 4, only after mission 2's smoke passes: the upgrade step that unregisters the `runner` entry Runner wrote from every client, the end of `initialize_mcp_defaults`, the Runner row and config snippet out of Settings → MCP, and the `runner-mcp` bin target, `install_mcp_cli`, the stale sidecar and its packaging gone. From the nightly that carries it, this repo's own mission driving and the mission-watch skill run on the CLI. | codex peer | Not started |
| 4 | Spec phase 5, design first: the two Settings → General rows (the `runner` command and the agent skill) drawn in `design/specs/648-runner-cli.pen` for Jason's sign-off, then `cli_install` install and uninstall on both platforms and the rows. Inline if the frame turns out small. | Jason + inline, or codex peer | Not started |
| 5 | Spec phase 6: vision §4.9, both READMEs, the mission-watch skill switched to `mission feed --follow --json`, Jason's smoke on macOS and JASONPC, archive the spec and this plan. One JASONPC session also covers pi's mission 3 checklist ([539](./539-pi-runtime.md)). | Jason + inline | Not started |

Arch §9 moves into mission 1 instead of the docs phase: the command tree is how the system works the moment mission 1 merges, and arch describes today.

## Decisions that bind

The spec's decisions 1–8 and its principles. The ones a crew is most likely to get wrong:

- **Inside a mission nothing changes.** `msg post`, `msg read`, `signal` and `ask` append to the event log directly, with no socket, no tokio runtime and no dependency on the app accepting connections. Only the control commands and the outside path use `mcp.sock`.
- **Identity is a handle or the person, never a location (decision 7).** Inside a mission the handle is `RUNNER_HANDLE`. Outside, `--as <handle>` becomes the tool's `from`, validated against the mission's roster by the tool, and no handle means `human`. `mission answer` is the person's verb and takes no `--as`. With a `from` handle the signal tool refuses the person's and the app's own types (`human_said`, `human_response`, `human_question`, `mission_goal`); without one it keeps today's whitelist.
- **A clean rename, no aliases.** `mission_post_human_message` and `mission_post_human_signal` become `mission_post` and `mission_signal`, as the `role_*` cutover did (#604). With decision 8 the CLI is the socket's only client.
- **The order inside 0.11.0 is fixed (decision 8).** Command tree, then the skill on all five runtimes and Jason's smoke of an agent driving a mission with no MCP entry, then the removal. `runner-mcp` keeps working until mission 3, because it is how missions 1 and 2 are launched and watched.
- **One binary.** The control commands live in the same `runner` the mission sessions already have on PATH. No second binary, no scoping by packaging.
- **No new dependencies without a reason in the handoff.** `clap`, `tokio` and `rmcp` are already in `cli/Cargo.toml`; tables are hand-rolled.

## What reading the code changed (2026-09-18)

- **The mission-wide Resume is UI code.** `mission_workspace/actions.rs::resume_open_mission` lists the mission's sessions and calls `ops::session::session_resume` for each stopped one. `mission_resume` is therefore a new backend op with the same loop, not a wrapper. The UI keeps its own loop in mission 1, because its per-session transitions drive the pane state; sharing the op is a follow-up.
- **`ops::session::session_list` already exists and is mission-scoped.** The new `session_list` tool lists direct chats from `repo::session::list_recent_direct`, so its op needs another name.
- **`mission_list` omits archived missions** and `MissionListArgs` has no include-archived flag, so prefix resolution for `mission unarchive` and for `mission show` on an archived mission needs a decision in mission 1.
- **The CLI crate has no lib target and does not depend on the backend,** and the tool registry is `pub(crate)`. The parity test needs a shared list of tool names (a constant in `runner-core` that the backend's registry test and the CLI's command map both assert against) rather than a cross-crate import.
- **`mission answer` takes the `human_question` event id.** `mission_status` already reports it as `pending_asks[].question_id`, which is the id the router matches a `human_response` on.
- **The sidecar is `<app data>/bin/runner`** (`cli_install::install_binary`), and a debug build of the CLI resolves the dev app's socket through `cfg!(debug_assertions)`. Crews never run the dev app, so a crew's live evidence is limited to the not-running path; the wire proof is Jason's smoke against `make run`.
- **Outside a mission with no `--mission`, a mission-scoped command is a usage error (exit 2),** replacing today's soft notice and exit 0. A general CLI that reports success after doing nothing is a trap for scripts. `Partial` env stays exit 2.

## Risks

- **The skill is the only way an agent finds Runner once MCP is gone.** MCP tools sit in an agent's tool list; a skill fires only when its description matches what the user said. Mission 2's smoke across four runtimes is the gate for mission 3, and the skill's description is tuned there, not after release.
- **The upgrade step edits three or four user config files.** It goes through the writer Settings → MCP already uses, touches only the `runner` entry of clients in `initialized_mcp_clients`, runs once, and leaves a config it cannot parse alone. A hand-written entry pointing at the deleted bridge will show as a failed server in that agent; the release notes say so.
- **Windows is unproven for the skill path.** The skill carries an absolute sidecar path with spaces and agents there run it under PowerShell or Git Bash; JASONPC proves it in mission 5, which is after the removal lands on `main` but before 0.11.0 is cut.
- **TRAE's skills root comes from its own manual** (`traecli doc skills`, 0.120.52: `~/.trae/skills/<name>/SKILL.md`), not from a live run; mission 2 proves a TRAE session loads the Runner skill.

## Log

- 2026-09-18 — Jason: remove MCP in the same release, and TRAE needs skills too. Checked TRAE CLI 0.120.52 on this machine: its manual documents user-level skills at `~/.trae/skills/` (also `~/.coco/skills/` and `~/.trae-cn/skills/`), project-level `.trae/skills/` with `.agents/skills/` read as a compatibility path, and `disable-model-invocation` as the only per-skill switch. Spec gained decision 8 and a removal phase (now six phases); the tool rename lost its aliases; this plan went from four missions to five.
- 2026-09-18 — spec, identity decision and roadmap landed on `main` (`756a60f`) with the docs housekeeping. Plan and mission 1 brief written on `feat/648-runner-cli` after reading `cli/src/`, `mcp/server.rs`, `mcp/tools/{mission,session}.rs`, `ops/{mission,session}.rs` and the mission workspace's resume action.
