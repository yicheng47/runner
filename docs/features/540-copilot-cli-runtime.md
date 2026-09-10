# 540 — GitHub Copilot CLI runtime

> Tracking issue: [#540](https://github.com/yicheng47/runner/issues/540)
> Priority: P1.

## Motivation

Several Runner users have asked for GitHub Copilot support. What a Copilot subscriber can run in a terminal is GitHub Copilot CLI: the `@github/copilot` npm package, command `copilot`, 1.0.83 as of 2026-09-10. It is an interactive coding agent with file edits, shell, MCP, custom agents, skills, and a plan mode, gated on a Copilot Pro/Business/Enterprise seat. It is a TUI on a PTY, so it fits Runner's runtime adapter the way claude-code, codex, and TRAE do, and it opens Runner to teams whose only model access is a Copilot seat.

Probed from `copilot --help` on 2026-09-10 (1.0.83, via `npx @github/copilot@1.0.83 --help`):

- `--session-id <id>`: "Resume an existing session or task by ID, or set the UUID for a new session." Caller-assigned keys, like claude-code and pi, so no post-spawn capture thread.
- `-i, --interactive <prompt>`: start interactive mode and run this prompt. That is the first turn.
- `--model <model>` (`auto` lets Copilot pick) and `--effort none | minimal | low | medium | high | xhigh | max`.
- Permissions are flags: `--allow-tool`, `--deny-tool`, `--allow-all-tools`, `--allow-all-paths`, `--allow-all` / `--yolo`, `--add-dir <dir>`. Default behaviour prompts per tool.
- `--no-auto-update` disables the background CLI update. `-C <dir>` sets cwd. `--additional-mcp-config <json|@file>` augments `~/.copilot/mcp-config.json` per session.
- `--mouse` / `--no-mouse` toggle "mouse support in alt screen mode": the TUI is alt-screen with mouse capture, the same shape as claude-code 2.1+.
- No system-prompt flag. Persona goes through custom agents (`~/.copilot/agents/*.agent.md`, `--agent <name>`) or custom instructions, not argv.

The `~/.copilot` directory (relocatable with `COPILOT_HOME`) holds `session-state/<session-id>/events.jsonl`, `settings.json`, `mcp-config.json`, `permissions-config.json`, `agents/`, `skills/`, and `copilot-instructions.md`.

## Scope

### In scope

- **Runtime catalog entry.** `copilot` in `RUNTIME_DEFINITIONS` with command `copilot`, display name `GitHub Copilot CLI`, `native_fork: false` (no fork flag; revisit if `--session-id` plus a copied `session-state` directory proves to be a fork), `skills_dirs: [".copilot/skills"]`. Project skills under `.github/skills` load from cwd on their own.
- **Model and effort.** `model` emits `--model <value>`; `effort` emits `--effort <level>` with Copilot's seven-level enum, which is claude-code's plus `none` and `minimal`. The effort dropdown for copilot rows shows that enum. Runtime defaults read `model` from `~/.copilot/settings.json` when present; no effort default.
- **First turn.** The composed body rides `-i <body>` instead of a positional, so `first_turn_argv("copilot", body)` returns `["-i", body]`. Persona folds into the first turn like claude-code and codex; `system_prompt_args` stays empty.
- **Session key and resume.** Fresh spawns pre-assign a UUID and pass `--session-id <uuid>`; resume passes the same flag. The conversation-exists probe checks `~/.copilot/session-state/<uuid>/events.jsonl`, honouring `COPILOT_HOME` from the spawn environment. Missing directory falls back to a fresh session with the same id, which Copilot then creates.
- **Permission modes.** Default → no flags. AcceptEdits → `--allow-tool=write`. Bypass → `--yolo`. Auto is unmapped in v1: Copilot's `--assisted-approval` judge is behind `--experimental` and takes precedence over `--allow-all-tools`, so the dropdown hides Auto for copilot rows. `strip_permission_flags` knows `--allow-tool` (value-bearing), `--yolo`, `--allow-all`, `--allow-all-tools`. `infer_permission_mode` recognises the three pairs. Mission permission mode (#527) Bypass → `--yolo`.
- **Always-on args.** `--no-auto-update` on every spawn, mirroring codex's `check_for_update_on_startup=false`, so the Settings → Agents Update button (#533) is the one update path.
- **Mission bus.** Mission slots get `--add-dir <mission dir>` so the shell tool can append to the event log under app data; `runner` stays on PATH through the existing spawn environment.
- **Settings → Agents row.** Detect `copilot` on PATH with the executable override, show `copilot --version`, wire Update to `copilot update`. Enabled by default when detected on both platforms; Copilot CLI is a Node CLI with native Windows support, including PowerShell.
- **MCP registration.** `mcp_defaults` writes Runner's server into `~/.copilot/mcp-config.json` under the same guard as the other clients (only when the CLI is detected, once).
- **Skills pane (#73).** Catalog from `~/.copilot/skills`; global on/off out of scope until a Copilot-side override is probed.
- **Terminal fixture.** Capture a Copilot CLI transcript for `crates/runner-app/tests/fixtures`. Because it runs alt-screen with mouse capture, host scrollback is empty and wheel events become SGR reports, so it follows the claude-code 2.1+ handling rather than codex's.
- **Docs.** README supported-agents table, `docs/arch` runtime enumerations.

### Out of scope

- Managing Copilot custom agents, plugins (`--plugin-dir`), or instructions from Runner. A generated per-runner `*.agent.md` passed as `--agent` is the natural home for `runner.system_prompt` and is recorded as the follow-up once the first-turn fold is proven to work.
- `--acp`. Runner's contract is the PTY.
- Remote control and export (`--remote`, `--remote-export`, `--share*`). Runner passes nothing and leaves the user's settings in charge.
- Plan and autopilot modes (`--mode`, `--plan`, `--autopilot`); users can put them in runner args.
- Installing the CLI or handling authentication and the subscription. The Agents row says "not detected" with the npm install line.

## Implementation Phases

### Phase 1 — adapter

- Add the `copilot` definition to `crates/runner-backend/src/router/runtime.rs` and cover `runtime_definitions`, `runtime_display_name`, `model_effort_args` (model, effort, both, neither), `first_turn_argv` (`-i` shape, empty on resume), `permission_mode_args` and `mode_match_pairs` for Default/AcceptEdits/Bypass, `strip_permission_flags`, `infer_permission_mode`, `mission_permission_mode_args`, `mission_bus_sandbox_args`, and `resume_plan` fresh vs resume.
- Add `"copilot"` to `runtime_defaults` reading `~/.copilot/settings.json`.
- Add the conversation-exists probe beside `claude_code_conversation_exists`, resolving `COPILOT_HOME` first.
- Extend `trailing_runtime_args` with `--no-auto-update` for copilot.
- Add `"copilot"` to the spawn-path runtime matches for the first-turn warning and `agent_session_key` handling; no launch gate, no capture thread.

### Phase 2 — UI

- Start Chat runtime picker and runner create/edit form list `GitHub Copilot CLI`, command prefilled `copilot`, effort dropdown with the seven levels, permission dropdown without Auto.
- Settings → Agents row per #533: detect, version, update, executable override, MCP registration.
- Skills pane catalog from `~/.copilot/skills`.

### Phase 3 — smoke

- Direct chat: persona and first turn arrive once through `-i`; the pane paints; wheel scroll reaches the TUI, not host scrollback.
- Crew with a copilot slot: launch prompt lands once, `runner msg read/post` and `runner signal ask_human` succeed from the shell tool without a path prompt (the `--add-dir` grant), Bypass mission mode runs unattended.
- Close and relaunch Runner: the session resumes by id with history. Delete the `session-state/<id>` directory and confirm a clean fresh start.
- Repeat the direct-chat and mission smoke on JASONPC under ConPTY.

### Phase 4 — docs

- README supported-agents table gains a GitHub Copilot CLI row with the subscription note.
- `docs/arch` runtime enumerations and the permission-mode table gain the copilot column.

## Verification

- [ ] `runtime_list` includes `copilot` with command `copilot`.
- [ ] `model = gpt-5.4`, `effort = high` produce `--model gpt-5.4 --effort high`.
- [ ] A fresh spawn carries `--session-id <uuid> --no-auto-update -i <body>` in that order after the runner's own args, and the row's `agent_session_key` is set before the process starts.
- [ ] Resume carries `--session-id <uuid>` and no `-i`.
- [ ] AcceptEdits rows spawn with `--allow-tool=write`, Bypass rows with `--yolo`, Default rows with neither; the dropdown offers no Auto for copilot.
- [ ] A mission with a copilot slot completes a `runner msg` round trip without a permission prompt for the mission directory.
- [ ] Settings → Agents shows the Copilot CLI version, `copilot update` runs in the update pane, and Runner's MCP server appears in `~/.copilot/mcp-config.json` after the first detection.
- [ ] The Copilot CLI terminal fixture renders without stray escapes in the runner-app tests, and wheel input reaches the TUI.
