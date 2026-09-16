# 540 — GitHub Copilot CLI runtime

> Tracking issue: [#540](https://github.com/yicheng47/runner/issues/540)
> Shipped 2026-09-16: adapter and surfaces in [#607](https://github.com/yicheng47/runner/pull/607), hook status, Agents marks and Skills pane fixes in [#609](https://github.com/yicheng47/runner/pull/609), the terminal fixture `copilot-first-turn.ndjson` with this archive. Windows is enabled but not yet smoke-tested there (README footnote); Windows hook status is [#610](https://github.com/yicheng47/runner/issues/610). Phase 5 follow-ups get their own issues.
> Priority: P2. Platforms: macOS and Windows; hook status lands on macOS first, as it did for Claude Code and Codex.
> Revived 2026-09-16. Parked on 2026-09-10 behind [539](../539-pi-runtime.md) because pi reaches Copilot models; Jason reopened it for GitHub's own CLI agent. Every claim below was re-probed on 2026-09-16 against the installed `copilot` 1.0.83 on macOS with a live session, not only `--help`, so the earlier audit is superseded where they differ.

## Motivation

Several Runner users have asked for GitHub Copilot support. What a Copilot subscriber can run in a terminal is GitHub Copilot CLI: the `@github/copilot` npm package, command `copilot`, 1.0.83 as of 2026-09-16. It is an interactive coding agent with file edits, shell, MCP, custom agents, skills, plugins and a plan mode, gated on a Copilot Pro/Business/Enterprise seat. It is a TUI on a PTY, so it fits Runner's runtime adapter the way claude-code, codex, and TRAE do, and it opens Runner to teams whose only model access is a Copilot seat.

This is the fourth agent runtime and the first added after [#347](./347-hook-based-session-status.md), [#587](./587-terminal-provided-titles.md), [#590](./590-runtime-model-discovery.md), [#593](../593-provider-chat-icons.md) and [#596](../596-chat-permission-posture.md) landed, so it is also the first time the full per-runtime surface is written down in one place. The inventory below is that list; keep it current so the next runtime is a checklist rather than an archaeology dig.

## What a new runtime touches

Everything keyed on `Runtime` in code, grouped by the surface a user sees. Sites marked *exhaustive* are `match` arms the compiler forces; the rest are lists, tables, and tests that only a grep finds.

### Chat and mission — spawn path (`crates/runner-backend`)

| Site | What changes for Copilot |
| --- | --- |
| `model.rs` `Runtime` enum, `ALL`, `key()` | `Copilot` with wire name `copilot`; round-trip test. *Exhaustive.* |
| `router/runtime.rs` `RUNTIME_DEFINITIONS` | Display `GitHub Copilot CLI`, command `copilot`, `native_fork: false`, `skills_dirs: [".copilot/skills", ".agents/skills"]`. Append, never prepend — index 0 is the Create Runner default. |
| `router/runtime.rs` `model_effort_args` | `--model <id>`; `--effort <level>` with `none / minimal / low / medium / high / xhigh / max`, forwarded verbatim. |
| `router/runtime.rs` `first_turn_argv` | `["-i", body]` — Copilot takes its first turn on a flag, not a positional. Empty on resume. |
| `router/runtime.rs` `system_prompt_args` | Empty; persona folds into the first turn like claude-code and codex (see decision 6 for the follow-ups). |
| `router/runtime.rs` `permission_mode_args`, `mode_match_pairs`, `strip_permission_flags`, `infer_permission_mode`, `mission_permission_mode_args` | Default → nothing; AcceptEdits → `--allow-tool=write`; Bypass → `--yolo`; Auto → nothing and hidden. Strip set: `--allow-tool` (value-bearing, both `--allow-tool=write` and `--allow-tool write`), `--yolo`, `--allow-all`, `--allow-all-tools`, `--allow-all-paths`, `--allow-all-urls`. *Exhaustive on `(runtime, mode)`.* |
| `router/runtime.rs` `mission_bus_sandbox_args` | `--add-dir <mission dir>` so the shell tool can append to the event log without a path prompt. |
| `router/runtime.rs` `trailing_runtime_args` | `--no-auto-update` always, plus `--plugin-dir <app data>/copilot-hooks` when hooks are supported (phase 3). |
| `router/runtime.rs` `resume_plan`, `supports_native_fork`, `fork_plan` | Fresh: pre-assign a UUID and pass `--session-id <uuid>` with `assigned_key` set before spawn (claude-code shape, no capture thread). Resume: the same flag. No fork in v1. |
| `router/runtime.rs` conversation probe | `copilot_conversation_exists(key)` → `$COPILOT_HOME` or `~/.copilot`, then `session-state/<key>/events.jsonl`; absent falls back to a fresh session with the same id, which Copilot then creates. |
| `router/runtime.rs` tests | Every `for runtime in ["claude-code", "codex", "trae"]` loop and the permission matrix gain `copilot`. |
| `runtime_defaults.rs` | `~/.copilot/settings.json` → `model` and `effortLevel` (JSONC: the file may carry `//` comments). *Exhaustive.* |
| `session/manager/spawn.rs` | First-turn warning gate and Windows batch fallback list gain Copilot; the conversation-missing check gets a `(Copilot, Some(key))` arm; `codex_capture_prompt_marker` and the three capture sites stay codex/trae only; new trust preseed beside `codex_trust` (decision 2). *Exhaustive in two places.* |
| `session/codex_capture.rs` `sessions_root_for` | `Copilot => None`. *Exhaustive.* |
| `session/copilot_trust.rs` (new) | Seed the spawn cwd into `config.json` `trustedFolders` before every spawn, preserving the file's comment header (decision 2). |
| `ops/runtime.rs` catalog | Entry with description, `default_enabled: true` on both platforms (native Windows binary), the static model list from `copilot help config` with `auto` first, the seven-level effort list; `catalog_matches_supported_runtime_order_and_defaults` and the selectable-catalog test. |
| `ops/runner.rs`, `ops/slot.rs`, `mcp/tools/session.rs` | The create/update permission-baking test gets a Copilot case; the valid-runtime error string and its test list `copilot`; the MCP `runtime` doc comment lists it. |
| `runtime_status.rs` | `status_list_includes_all_catalog_runtimes` and any table that indexes `.runtimes[n]`. |
| `runtime_status/models.rs` `DISCOVERY_RUNTIMES` | Unchanged in v1 (static catalog). Follow-up: a `models/copilot.rs` that parses the `model` bullet list out of `copilot help config` — offline, no auth, and exactly what the CLI accepts. |

### Settings → Agents, MCP, Skills (`runner-app` + `ops/mcp.rs`, `skills.rs`)

| Site | What changes for Copilot |
| --- | --- |
| `ops/mcp.rs` `McpClientId` | `Copilot` with `copilot_path()` → `~/.copilot/mcp-config.json`; `ALL`, key `copilot`, label, path string, `McpIntegrationStatus.copilot`; `copilot_status_at` / `copilot_write_at` reuse the Claude JSON writer under the `mcpServers` key with Copilot's entry shape `{"type":"local","command":…,"args":[],"tools":["*"]}` (probed via `copilot mcp add`). *Exhaustive in four matches.* |
| `app_store/mcp_defaults.rs` | The default-registration match gains `Copilot => ("copilot", &status.copilot)`. *Exhaustive.* |
| `app_settings.rs` | Nothing structural: enabled/disabled sets are keyed by runtime name and `runtime_default_enabled` reads the catalog. Test coverage only. |
| `surfaces/settings/agents.rs` | Catalog-driven; verify no hardcoded row count. The version caption and Update button are [#533](../533-agent-cli-updates.md) and are not implemented yet; when they land, `copilot --version` prints `GitHub Copilot CLI 1.0.83.` (first semver token parses) and `copilot update` is the update command, so the runtime definition should name it. |
| `surfaces/settings/mcp.rs` | Registration copy hint and presentation tests name the new client and path. |
| `cli_install.rs` | Doc comment on `MCP_DEST_BIN_NAME` lists the clients. |
| `skills.rs` `skill_catalog` | Generic over `skills_dirs`; Copilot's two personal roots appear on their own. `copilot skill --help` confirms `~/.copilot/skills/` and `~/.agents/skills/`. |
| `ops/skills.rs` `set_global_enabled_at` | Stays "Claude Code and Codex only" in v1; the error text should say so. `copilot plugins disable --skill <name>` exists and is the follow-up once its persistence is probed. |
| `surfaces/settings/skills.rs` | Runtime switcher is catalog-driven; Copilot shows up with its two roots and no global toggle, the TRAE shape. |

### Tab title and session status

| Site | What changes for Copilot |
| --- | --- |
| `session/title.rs` `decoration` | Add `"github copilot"` and `"copilot"` to the chrome list. Copilot titles are `GitHub Copilot` at startup, then `<first prompt text> - GitHub Copilot`, then a generated topic such as `Run Shell Command Echo - GitHub Copilot`; the suffix must strip and the bare product name must not become a tab name. Tests with the three observed shapes. |
| Title-spinner baseline (`runner-terminal` `TitleStatus`) | Copilot never emits a braille title, so the spinner heuristic never arms; the byte detector is the only baseline. Its footer animates while working (`● Working · 625 B esc interrupt`), so byte activity reads Working correctly. Whether the idle footer animates too is a verification item. |
| `session/copilot_status.rs` (new) + `pty_runtime.rs` `HookStatusWatcher::Copilot` | The hook adapter (phase 3). Injection is `--plugin-dir <app data>/copilot-hooks`, a Runner-owned plugin with `plugin.json` and `hooks/hooks.json` whose commands call a Runner-owned reporter script with the per-session `RUNNER_COPILOT_STATUS_PATH` and `RUNNER_COPILOT_STATUS_GENERATION` from the spawn environment; the feed and watcher are the shared `hook_feed` transport. Event mapping is in decision 3. |
| `session/manager/spawn.rs` `apply_runtime_args` | Insert the two env vars whenever hooks are supported. A runner's own `--plugin-dir` stays additive; there is no invocation opt-out, and user `disableAllHooks` falls back to the baseline by emitting no reports. |
| `ops/session.rs` `preferred_title` | No code change: a runner-backed chat keeps its handle because Copilot has no system-prompt flag either ([#603](https://github.com/yicheng47/runner/pull/603)); update the comment that says "codex or trae". |

### Chat identity and forms (`runner-app`)

| Site | What changes for Copilot |
| --- | --- |
| `assets.rs`, `chat_icon.rs` | A bundled Copilot mark with its tint, per [#593](../593-provider-chat-icons.md); `pane_identity_icon`, `sidebar_tab_icon`, `command_palette.rs` and `settings/archived.rs` tests gain the row. *Exhaustive in `chat_icon.rs`.* Designed 2026-09-16: `cmp/MarkCopilot` (`QGeFI`) on `design/runner.pen` beside the three existing marks, the goggles from lobe-icons `githubcopilot.svg`, tinted GitHub's Copilot Purple `#8534F3` fixed in both themes (decision 9). |
| `surfaces/runners/logic.rs` `permission_modes`, `permission_mode_description` | `[Default, AcceptEdits, Bypass]` with Copilot's wording; the `(runtime, mode)` description match is exhaustive. |
| `surfaces/runners/forms.rs`, `start_chat.rs`, `ui/select.rs` | Catalog-driven; existing tests only. |
| `surfaces/panes.rs` `header_fork_state` | The disabled-fork message names the forkable runtimes; Copilot is not one in v1. |

### Docs, fixture, CI

| Site | What changes for Copilot |
| --- | --- |
| `README.md` + `README.zh-CN.md` | Supported-agents row, the MCP registration sentence, the hook-status sentence. Both files in one PR. |
| `docs/arch/arch.md` | Runtime enumerations at §1.1, §3 (runner row and `runtime TEXT` comment), §4 (MCP settings), §5 (TUIs, first turn, resume, fork), §5.10 (status sources); the `--settings` sentence gains the `--plugin-dir` sibling. `docs/arch/windows.md` gets the Windows default. |
| `docs/impls/347-hook-status/` | A Copilot adapter slice with its own smoke checklist under `docs/tests/`. |
| `crates/runner-terminal/fixtures/copilot-session.ndjson` + snapshot | Recorded from a real session; alt-screen with mouse capture, so it follows the claude-code 2.1+ scrollback rules, not codex's. |
| `docs/features/README.md`, this file | Index line; archive on close. |

Rough size: about thirty Rust files, of which ten are compiler-forced match arms; six docs; one fixture; one Pencil component. Phases 1 and 2 are the TRAE-sized catalog work; phase 3 is the part TRAE never got.

## Probe evidence (2026-09-16, copilot 1.0.83, macOS)

- **Per-invocation hooks work.** `--plugin-dir <dir>` mounts a plugin whose `hooks/hooks.json` runs with no trust step and no change to `~/.copilot`; `copilot plugin list` shows it under "External Plugins (via --plugin-dir)". Hooks inherit the spawn environment (a probe env var reached the reporter), so one static plugin serves every session through per-session env vars. The user's own `~/.copilot/hooks/*.json` kept running alongside. Unknown event names are ignored silently.
- **Event vocabulary is Claude Code's, with snake_case payloads under PascalCase keys.** Observed live: `SessionStart` (`source: new | resume`, `initial_prompt`), `UserPromptSubmit` (`prompt`), `PreToolUse` (`tool_name` mapped to Claude names: `Bash`, `Edit`, `AskUserQuestion`; `tool_input`), `PermissionRequest` (camelCase: `hookName`, `sessionId`, `toolName`, `toolInput`, `permissionSuggestions`), `Notification` (`notification_type`, `title`, `message`), `PostToolUse` (`tool_result`), `Stop` (`stop_reason: end_turn`, `stop_hook_active`, `transcript_path`), `SessionEnd` (`reason: complete | user_exit`). Documented but not exercised: `PostToolUseFailure`, `ErrorOccurred` (`error_context`, `recoverable`), `PreCompact`, `SubagentStart` / `SubagentStop`, and `Notification` types `agent_idle`, `agent_completed`, `shell_completed`. No `tool_use_id` on any payload; no interrupt event; no `StopFailure`.
- **Surfaced prompts are distinguishable.** A file write in manual mode produced `PreToolUse(Edit)` → `PermissionRequest` → `Notification(permission_prompt)` 7 ms later, when the dialog was on screen, then `PostToolUse` after Enter. `echo` was auto-approved and produced `PermissionRequest` with no notification, so `PermissionRequest` alone is not a wait, exactly as on Claude Code. `ask_user` produced `PreToolUse(AskUserQuestion)` then `Notification(elicitation_dialog)` with the question text.
- **`--session-id` pre-assigns and resumes.** A fresh `-p` run with a chosen UUID created `session-state/<uuid>/events.jsonl`; a second run with the same flag recalled the earlier turn and its `SessionStart` said `source: resume`. Sessions keep `workspace.yaml` with `cwd`, `name`, `client_name`.
- **`events.jsonl` is a usable transcript tail.** It carries `assistant.turn_start` / `turn_end`, `tool.execution_start` / `complete`, `hook.start` / `hook.end`, `session.shutdown`, so correlation of a wait to its resolution can follow the Claude adapter's transcript path.
- **Folder trust is a launch gate that `--yolo` does not skip.** An untrusted cwd shows "Confirm folder trust" with 1 Yes / 2 Yes and remember / 3 No before the first turn, even with `--yolo`. "Remember" writes the absolute path into `config.json` under `trustedFolders`. `config.json` starts with two `//` comment lines and says it is managed automatically.
- **Terminal shape.** `DECSET 1049` alt screen, mouse `1003` + SGR `1006`, bracketed paste; no `1007`, no kitty keyboard. Title sequence: `GitHub Copilot`, then the first prompt text with the ` - GitHub Copilot` suffix, then a generated topic with the same suffix. No braille frames.
- **MCP config shape.** `copilot mcp add runner -- <path>` writes `~/.copilot/mcp-config.json` as `{"mcpServers":{"runner":{"tools":["*"],"type":"local","command":"<path>","args":[]}}}`. The file does not exist until the first server is added.
- **Also observed, not used.** `open-sessions-state.json` flips a per-session `working` flag on prompt submit and turn end for interactive sessions only; it is one file for all sessions and lags the hooks, so it is not a status source.
- **Model list.** `copilot help config` prints the accepted `model` values (26 on 1.0.83, Claude, GPT, Gemini, Grok, Kimi and MAI families) and `settings.json` carries `model` and `effortLevel`.

## Scope

### In scope

- **Runtime catalog entry** and every site in the inventory above.
- **Model and effort.** `--model` and `--effort` with Copilot's seven levels; defaults from `~/.copilot/settings.json`.
- **First turn** on `-i <body>`; persona folds into the first turn.
- **Session key and resume** via caller-assigned `--session-id`; conversation probe on `session-state/<id>/events.jsonl`, honouring `COPILOT_HOME`.
- **Permission modes** Default / AcceptEdits / Bypass; Auto hidden. Mission permission mode ([#527](./527-mission-permission-mode.md)) Bypass → `--yolo`; chats assert nothing ([#596](../596-chat-permission-posture.md)).
- **Folder trust preseed** so unattended mission slots never sit on the trust dialog.
- **Always-on args** `--no-auto-update`; `--add-dir <mission dir>` for slots.
- **Settings → Agents row** with detection, override, enable toggle, and MCP registration into `mcp-config.json` under the existing one-time guard.
- **Skills pane** catalog from the two personal roots, no global toggle.
- **Provider mark** in every identity surface, designed on the canvas first.
- **Hook status adapter** on macOS with the [#347](./347-hook-based-session-status.md) vocabulary; Windows stays baseline-only like the other two runtimes.
- **Title filter** for the `- GitHub Copilot` suffix.
- **Terminal fixture** and docs.

### Out of scope

- Managing Copilot custom agents, plugins, instructions, or `permissions-config.json` from Runner.
- `--acp`, remote control and export (`--remote*`, `--share*`), plan and autopilot modes; users can put them in runner args.
- Native fork. `--session-id` plus a copied `session-state` directory might be one; unprobed.
- Global skill on/off for Copilot.
- Installing the CLI or handling authentication and the subscription; a Not found row keeps its caption.
- Model discovery through the CLI ([#590](./590-runtime-model-discovery.md) shape); v1 ships the static list.

## Decisions

1. **Hooks ride `--plugin-dir`, not the user's config.** A Runner-owned plugin directory under app data, regenerated at startup, is the Copilot equivalent of claude-code's `--settings` JSON and codex's `-c hooks.<Event>` overrides: per-invocation, additive, no trust step, nothing persisted into `~/.copilot`. If the runner's own args already pass `--plugin-dir` Runner still adds its own (the flag repeats); an explicit `disableAllHooks` in the user's settings silently disables the bridge and the session falls back to baseline, which is the [#347](./347-hook-based-session-status.md) bridge-loss rule.
2. **Seed folder trust before every spawn, mirroring `codex_trust`.** `--yolo` does not bypass the dialog, so a mission slot would hang on it. Write the cwd into `config.json` `trustedFolders` with a JSONC-preserving edit: keep the leading `//` header lines byte-for-byte, parse the remainder with `preserve_order`, append if absent, write back. Only that key is touched; everything else in the file is Copilot's (the structured-parser and preserve-unrelated-edits rules in [`AGENTS.md`](../../../AGENTS.md)). Whether a trusted parent covers a child directory is unverified; seed the exact cwd.
3. **Adapter event mapping**, ported from the Claude adapter with Copilot's differences: Working from `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PreCompact`; `Approval needed` from `Notification(permission_prompt)`, correlated to the pending `PreToolUse` by tool name and input because there is no `tool_use_id`, cleared by the owning `PostToolUse` / `PostToolUseFailure` or the transcript's `tool.execution_complete`; `Answer needed` from `PreToolUse(AskUserQuestion)` raised immediately, confirmed by `Notification(elicitation_dialog)`, cleared the same way; Idle from `Stop`, corrected by the next work event, with `Notification(agent_idle)` as secondary confirmation only; `Response failed` from `ErrorOccurred` with `recoverable: false` once a fixture proves the shape; subagent events ignored for the main-agent rule; `SessionStart` owns the generation like the others. Interrupts are not published, so they recover through Runner's input layer as on Claude Code. `PermissionRequest` alone never raises attention.
4. **Permission flags.** AcceptEdits is `--allow-tool=write` (Copilot's `write` kind covers create and modify, not shell), Bypass is `--yolo` (tools, paths and URLs), Auto is unmapped because `--assisted-approval` is experimental. The user's `defaultPermissionMode` in `settings.json` governs Default rows.
5. **Static model catalog in v1**, seeded from `copilot help config` with `auto` first and the Copilot-hosted Claude, GPT, Gemini, Grok, Kimi and MAI ids. The discovery follow-up parses the same help text at discovery time, which is offline and needs no auth.
6. **Persona folds into the first turn in v1.** Two clean native channels exist for a follow-up, both writing only under Runner-owned directories: `COPILOT_CUSTOM_INSTRUCTIONS_DIRS` pointing at a per-session directory holding the persona as `AGENTS.md`, or `--agent runner` with a generated `<mission dir>/.github/agents/runner.agent.md`, which `--add-dir <mission dir>` already loads as trusted configuration. Pick one after the fold is proven, then drop the persona section from Copilot bodies the way [539](../539-pi-runtime.md) does for pi.
7. **Enabled by default on both platforms.** Copilot CLI ships native binaries for Windows through WinGet and the npm package needs Node 22; the existing Windows batch first-turn fallback covers an npm `.cmd` shim. Windows hook status stays the agreed [#347](./347-hook-based-session-status.md) follow-up.
8. **No `--session-id` capture thread, no launch gate.** Keys are caller-assigned; nothing in the probes suggested an OAuth refresh race like claude-code's.
9. **The mark is purple, `#8534F3`, fixed in both themes.** Jason, 2026-09-16, on the canvas. GitHub draws the goggles monochrome and retired the standalone Copilot logo in 2025, but its brand toolkit assigns Copilot this exact purple ("purple is thoughtfully injected to highlight Copilot products and features"); blue is GitHub's Security theme, not Copilot's. Theme foreground like Codex was not taken: two grey marks in a dim rail are two blobs, and the goggles at 12 px are a visor with two dots, the same weak-shape case that gave Trae its green under [#593](../593-provider-chat-icons.md). Purple was also the one hue the app did not use yet.

## Implementation Phases

### Phase 0 — design (done 2026-09-16)

`cmp/MarkCopilot` (`QGeFI`) sits beside the Claude, Codex and Trae marks on `design/runner.pen`, and the frame `Spec — GitHub Copilot CLI runtime (540) · v1` (`OBtYk`) shows the mark at 12/14/16/24 px beside the shipped marks, the rail before and after, the Agents row with the new words, the three permission modes with their dropdown copy, the tint decision, and the light theme. The `Settings — Agents` frame (`n1krgH`) shows the four rows in catalog order with each provider mark at 16 px before the name, which the Agents pane now draws; otherwise the pickers, permission dropdown and Skills pane reuse existing components.

### Phase 1 — adapter (`runner-backend`)

Everything in the spawn-path table: enum, definitions, argv composition, resume plan, conversation probe, trust preseed, runtime defaults, catalog entry, and the test loops. `make verify` green; a direct chat and a mission slot start, take their first turn once through `-i`, and resume by id after relaunch.

### Phase 2 — settings and identity (`runner-app`, `ops/mcp.rs`)

MCP client and default registration, Agents row check, Skills pane check, provider mark and icon tests, permission dropdown and descriptions, title filter, docs and README rows, the terminal fixture.

### Phase 3 — hook status (macOS)

The Runner-owned plugin directory, `copilot_status.rs`, `HookStatusWatcher::Copilot`, the env injection, and the mapping in decision 3, as a slice under `docs/impls/347-hook-status/` with its own smoke checklist. Ship Working / Idle first, then the two waits, then `Response failed` once `ErrorOccurred` is captured live.

### Phase 4 — smoke (macOS, then JASONPC)

Direct chat: persona and first turn arrive once; the pane paints; wheel scroll reaches the TUI, not host scrollback; the tab title becomes the generated topic without the suffix. Crew with a Copilot slot: launch prompt lands once, `runner msg read/post` and `runner signal ask_human` work from the shell tool without a path prompt, Bypass mission mode runs unattended in a never-trusted directory. Relaunch: resume by id with history; delete `session-state/<id>` and confirm a fresh start. Windows: direct chat and mission under ConPTY, baseline status only.

### Phase 5 — follow-ups, each its own issue

Model discovery from `copilot help config`; persona on a native channel (decision 6); global skill toggle via `copilot plugins disable --skill`; native fork probe; Windows hooks with the other two runtimes.

## Verification

- [ ] `runtime_list` includes `copilot` with command `copilot`; `Runtime::parse("copilot")` round-trips.
- [ ] `model = gpt-5.4`, `effort = high` produce `--model gpt-5.4 --effort high`.
- [ ] A fresh spawn carries `--session-id <uuid> --no-auto-update --plugin-dir <app data>/copilot-hooks -i <body>` after the runner's own args, and the row's `agent_session_key` is set before the process starts.
- [ ] Resume carries `--session-id <uuid>` and no `-i`; a missing `session-state/<uuid>` falls back to a fresh session with the same id.
- [ ] AcceptEdits rows spawn with `--allow-tool=write`, Bypass rows with `--yolo`, Default rows with neither; the dropdown offers no Auto; a chat strips all six flags.
- [ ] Spawning into a never-trusted directory adds it to `trustedFolders` and the session shows no trust dialog; the two comment lines and every other key in `config.json` are unchanged.
- [ ] A mission with a Copilot slot completes a `runner msg` round trip without a permission prompt for the mission directory.
- [ ] Settings → Agents shows the row on both platforms; Runner's server appears in `~/.copilot/mcp-config.json` as a `local` entry with `tools: ["*"]` after the first detection, and the file is created when absent.
- [ ] `provider_title` turns `Run Shell Command Echo - GitHub Copilot` into `Run Shell Command Echo` and returns nothing for `GitHub Copilot`.
- [ ] Hook status: a file write in Default mode shows `Approval needed` while the dialog is up and clears on Enter; `ask_user` shows `Answer needed`; a completed turn shows Idle; `echo` under auto-approval never shows a wait; a session with `disableAllHooks` shows estimated status.
- [ ] The Copilot fixture renders without stray escapes in the `runner-terminal` tests and wheel input reaches the TUI.
- [ ] Every runtime-enumerating test in `router/runtime.rs`, `ops/mcp.rs`, `ops/slot.rs`, `runtime_status.rs`, `chat_icon.rs`, `command_palette.rs`, `archived.rs`, `panes.rs` and `sidebar/tests.rs` lists `copilot`.
