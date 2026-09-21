# 539 — pi runtime

> Tracking issue: [#539](https://github.com/yicheng47/runner/issues/539)
> Status: shipped 2026-09-20 in [v0.11.0](https://github.com/yicheng47/runner/releases/tag/v0.11.0) (PRs [#646](https://github.com/yicheng47/runner/pull/646), [#649](https://github.com/yicheng47/runner/pull/649), the smoke test in [#667](https://github.com/yicheng47/runner/pull/667); record [`539-pi-runtime/`](../../impls/archive/539-pi-runtime/README.md); missions `01M2S4DF48DH2T66QY0TG9PXAD`, `01M2SGW74SGGMFASKK0PN99F5D`). The first-turn fixture and the JASONPC pass continue as [#668](https://github.com/yicheng47/runner/issues/668); the resume probe and `PI_CODING_AGENT_DIR` as [#666](https://github.com/yicheng47/runner/issues/666).
> Priority: P1, milestone 0.11. Platforms: macOS and Windows; hook status on both, and it needs pi 0.84.4 or newer (older pi falls back to estimated status).
> Implementation plan: [`docs/impls/539-pi-runtime.md`](../../impls/archive/539-pi-runtime/README.md).
> Design: `design/specs/539-pi-runtime.pen`, frame `Spec — pi runtime (539) · v1` (`h5rNdS`), the first spec designed in its own file (see `design/README.md`). The mark component `cmp/MarkPi` also sits on `design/runner.pen` beside the four shipped marks.
> Refreshed 2026-09-17. The 2026-09-10 draft predates hook status ([347](./347-hook-based-session-status.md), [610](./610-windows-hook-status.md)), model discovery ([590](./590-runtime-model-discovery.md)), provider marks ([593](./593-provider-chat-icons.md)), chat permission posture ([596](./596-chat-permission-posture.md)), the README feature matrix and the Copilot runtime ([540](./540-copilot-cli-runtime.md)). Every claim below was re-probed on 2026-09-17 against the installed pi 0.85.1 on macOS (ten print-mode runs, two interactive PTY captures, and the package's `dist/` source), so this file supersedes the draft where they differ. Decided the same day with Jason: the system-prompt channel is the core of the runtime and ships in the first mission, not as a follow-up.

## Motivation

pi ([earendil-works/pi](https://github.com/earendil-works/pi), npm `@earendil-works/pi-coding-agent`, command `pi`) is a terminal-first coding agent with a large following, and the one the Chinese developer community is currently building desktop shells around: V2EX and linux.do threads ask for a GUI, and several shells have appeared since July. Orca and herdr already integrate it; both had installed their status extensions into `~/.pi/agent/extensions/` on the dev Mac. It is bring-your-own-model (Anthropic, OpenAI, `openai-codex` OAuth, `github-copilot/*`, DeepSeek, MiniMax, Kimi, Qwen, local), so it gives users on a ChatGPT, Copilot or Chinese-provider plan a way into Runner.

pi is also the first runtime where Runner's prompt model works the way [arch §6](../../arch/arch.md#6-system-prompt-composition) describes it. Every other agent forced Runner to fold the persona, the team conventions and the coordination verbs into the first user turn, because Claude Code's `--append-system-prompt` is print-mode only and Codex has no flag at all; `router/runtime.rs` calls that fold a workaround. pi's interactive TUI honours `--append-system-prompt`, so those layers become a real system prompt: re-sent with every request, so they survive compaction instead of being summarized away mid-mission; no wasted first turn where the agent replies to its own persona; and live, because Runner re-supplies the current role and crew text on every spawn, so editing a role changes its existing chats at their next relaunch. Sessions are caller-assigned and forkable from the command line, and the extension API was built for hosts like Runner: `agent_settled` says pi will not continue on its own, `ui_prompt_start` / `ui_prompt_end` exist, in pi's words, "so host/status integrations can report 'waiting for user'", and `project_trust` lets a host answer the trust dialog.

## What a new runtime touches

The per-runtime inventory from [540](./540-copilot-cli-runtime.md#what-a-new-runtime-touches), re-walked on `main` at `cd2e176`. Sites marked *exhaustive* are `match` arms the compiler forces; the rest are lists, tables and tests only a grep finds. The phase column says when a site changes: the runtime with its system-prompt channel (1), hook status and rekey (2), or smoke and fixture (3).

### Chat and mission — spawn path (`crates/runner-backend`)

| Site | What changes for pi | Phase |
| --- | --- | --- |
| `model.rs` `Runtime` enum, `ALL`, `key()` | `Pi` with wire name `pi`; round-trip test. *Exhaustive.* | 1 |
| `router/runtime.rs` `RUNTIME_DEFINITIONS` | Display `pi`, command `pi`, `native_fork: true`, `skills_dirs: [".pi/agent/skills", ".agents/skills"]`. Appended after TRAE. | 1 |
| `router/runtime.rs` `model_effort_args` | `--model <value>` verbatim (pi accepts `provider/id`, a pattern, or `pattern:<thinking>`); `--thinking <level>` **lowercased**: pi rejects `High` with a warning and ignores it (probed). Levels `off / minimal / low / medium / high / xhigh / max`. *Exhaustive.* | 1 |
| `router/prompt.rs` | A split composer for runtimes with a system-prompt channel, returning `(system_prompt, first_turn)` per session kind (decision 1). The existing `compose_launch_prompt`, `compose_worker_first_turn` and `compose_direct_first_turn` are untouched, and a test pins their output byte-identical. | 1 |
| `ops/mission.rs` `start`, `ops/session.rs` `session_start_direct` / `session_resume` / restart | Compose the pi system-prompt body from the current role, crew and roster rows and write it to `<app data>/session-prompts/<session id>.md` before every spawn, resume included. `ensure_first_turn_fits` applies to the first turn only. | 1 |
| `router/runtime.rs` `system_prompt_args` | `--append-system-prompt <file>` on every spawn, resume included; empty for every other runtime as today. `trailing_runtime_args` stops suppressing it on resume for pi. *Exhaustive.* | 1 |
| `router/runtime.rs` `first_turn_argv` | `["--", body]` when there is a body: the lead's goal turn. Behind `--` so a body starting with `-` is not a flag (probed). `--` does **not** protect a body starting with `@`, which pi reads as a file reference and fails on (probed); pi's only first turn starts with `== Mission ==`, so a user goal beginning with `@` is safe. Empty for workers, direct chats and every resume. *Exhaustive.* | 1 |
| `router/runtime.rs` `trailing_runtime_args` | `-e <app data>/pi-hooks/runner-status.ts` when the extension is installed and hooks are supported, Copilot's `plugin_available` shape. | 2 |
| `router/runtime.rs` `permission_mode_args`, `mode_match_pairs`, `strip_permission_flags`, `infer_permission_mode`, `mission_permission_mode_args` | All empty / Default: pi has no approval gating (decision 7). *Exhaustive on `(runtime, mode)`.* | 1 |
| `router/runtime.rs` `mission_bus_sandbox_args` | Empty: pi has no sandbox, so `runner msg` and `runner signal` reach the mission log from its bash tool as they are. *Exhaustive.* | 1 |
| mission-slot args (beside `mission_bus_sandbox_args` in `spawn.rs`) | `--approve` for pi mission slots only (decision 6). Argv order is already safe: role args, then `--session-id`, then slot args, then the trailing args ending in `-- <body>`. | 1 |
| `router/runtime.rs` `resume_plan` | Copilot's shape: a fresh spawn pre-assigns a UUID and passes `--session-id <uuid>` with `assigned_key` set before spawn; resume passes the same flag with the prior key. No capture thread (decision 2). *Exhaustive.* | 1 |
| `router/runtime.rs` `fork_plan` | `ForkPlan::Direct` with `--fork <source> --session-id <new uuid>` (decision 3). *Exhaustive.* | 1 |
| `router/runtime.rs` conversation probe | `pi_conversation_exists(cwd, key)`: one glob, `~/.pi/agent/sessions/<slug>/*_<key>.jsonl`, where the slug is pi's own `getDefaultSessionDirPath`: strip one leading `/` or `\`, replace every `/`, `\` and `:` with `-`, then wrap the result in `--` (read from `dist/core/session-manager.js`; `/Users/jason` → `--Users-jason--`, `C:\Users\x` → `--C--Users-x--`). Uses the same resolved cwd the Claude probe uses. `sessionDir` in settings and `--session-dir` in a role's args are not honoured in v1 (decision 2). | 1 |
| `router/runtime.rs` tests | Every `for runtime in [...]` loop and the permission matrix gain `pi`. | 1 |
| `runtime_defaults.rs` | `~/.pi/agent/settings.json` (plain JSON): `defaultProvider` + `defaultModel` → `provider/model` (the model alone when no provider), `defaultThinkingLevel` → effort. *Exhaustive.* | 1 |
| `session/manager/spawn.rs` | The first-turn warning gate and the Windows batch first-turn fallback list gain `Pi` (only the lead has a body); the conversation-missing check gets a `(Pi, Some(key))` arm that keeps the key, like Copilot, because pi creates a missing session under the same id; `PI_SKIP_VERSION_CHECK=1` in the env (decision 8). Phase 2 adds the status env vars. *Exhaustive in two places.* | 1, 2 |
| `session/codex_capture.rs` `sessions_root_for` | `Pi => None`. *Exhaustive.* | 1 |
| `session/pi_status.rs` (new) | The embedded extension source, `install_extension` at startup beside `copilot_status::install_plugin` in `manager/mod.rs`, `extension_available`, and `PiStatusWatcher` over `HookFeed::start_external` with the mapping in decision 5. | 2 |
| `session/hook_feed.rs` `hooks_supported` | `Pi` on both platforms, in phase 2 only: until the extension exists the gate stays false, or phase 1 would inject `-e` for a missing file. The reporter is Node inside pi, not `sh` or PowerShell. | 2 |
| `session/pty_runtime.rs` `HookStatusWatcher` | `Pi(PiStatusWatcher)` arm, started from the env vars like Copilot. *Exhaustive.* | 2 |
| `session/claude_rekey.rs` | No code change: the extension writes the drop file this watcher already consumes for every runtime (decision 10). The doc comment names pi. | 2 |
| `ops/runtime.rs` catalog | Entry with description `pi coding agent (bring your own model provider)`, `default_enabled: true` on both platforms, only the default model option statically (the list comes from discovery), and the seven thinking levels. `catalog_matches_supported_runtime_order_and_defaults` and the selectable-catalog test. | 1 |
| `runtime_status/models.rs` `DISCOVERY_RUNTIMES` + `models/pi.rs` (new) | `pi --offline --list-models`, parsing its `provider model context max-out thinking images` table into `provider/model` options, in the shape of `models/codex.rs`. On the dev Mac it listed only the two providers with credentials; whether that is the rule is confirmed when the adapter is written. | 1 |
| `ops/role.rs`, `ops/slot.rs`, `mcp/tools/session.rs`, `runtime_status.rs` | The valid-runtime error string and its tests, the MCP `runtime` doc comment, and the status table test list `pi`. | 1 |
| `session/title.rs` | No change expected: pi set no window title in either PTY capture. The first-turn fixture confirms whether it titles later. | 3 |

### Settings → Agents, MCP, Skills

| Site | What changes for pi | Phase |
| --- | --- | --- |
| `surfaces/settings/agents.rs` | Catalog-driven; nothing structural. `pi --version` prints a bare `0.85.1`. `pi update self` is the command #533 will wire. | 1 |
| `ops/mcp.rs`, `app_store/mcp_defaults.rs`, `surfaces/settings/mcp.rs` | None: pi has no MCP client, so it gets no `McpClientId` and no MCP column. | — |
| `skills.rs` `skill_catalog` | Generic over `skills_dirs`; pi's two personal roots appear on their own. | 1 |
| `surfaces/settings/skills.rs` | A pi caption naming the two roots; `supports_global_skill_toggle` stays false (pi has no per-skill off switch for plain directories), the TRAE shape. *Exhaustive in the caption match.* | 1 |

### Chat identity and forms (`runner-app`)

| Site | What changes for pi | Phase |
| --- | --- | --- |
| `assets.rs`, `chat_icon.rs` | The bundled pi mark, one path in `viewBox -2 -2 28 28`, tinted `theme::text()` like Codex (decision 11); `pane_identity_icon`, `sidebar_tab_icon`, `command_palette.rs`, `settings/archived.rs`, `panes.rs`, `sidebar/tests.rs` and `roles/tests.rs` gain the row. *Exhaustive in `chat_icon.rs`.* | 1 |
| `surfaces/roles/logic.rs` `permission_modes`, `permission_mode_description` | `&[]`, which hides the permission control on the role form and slot overrides. *Exhaustive.* | 1 |
| `surfaces/roles/forms.rs`, `start_chat.rs`, `ui/select.rs`, `app_settings.rs` | Catalog-driven; test lists only. | 1 |
| `surfaces/panes.rs` `header_fork_state` | pi is forkable; the disabled-fork message's runtime list is unchanged. | 1 |
| `ops/session.rs` `preferred_title` | No behaviour change: a role-backed chat keeps its handle under [#587](./587-terminal-provided-titles.md)'s identity rule. The comment's "there is no system-prompt flag" aside gains the pi exception. | 1 |

### Docs, fixture, CI

| Site | What changes for pi | Phase |
| --- | --- | --- |
| `README.md` + `README.zh-CN.md` | A pi column in Supported agents, both files in one PR; hooks and Needs you cells update in phase 2. | 1, 2 |
| `docs/arch/arch.md` | Runtime enumerations (§1.1, §3, §5, §5.10); §5's first-turn paragraph gains pi's delivery; §6.2's table gets the pi delivery per session kind, and §6.3's claim that the launch prompt is written to stdin on `mission_goal` is corrected while the section is open (it has been the spawn-time first turn since the GPUI cutover). `docs/arch/windows.md` gets pi's Git Bash requirement. | 1 |
| `docs/tests/539-pi-hooks-smoke.md` | Smoke checklist for decision 5, macOS and JASONPC. | 2 |
| `crates/runner-terminal/fixtures/pi-first-turn.ndjson` + snapshot | Recorded from a real session: inline, no alternate screen, codex's scrollback rules. | 3 |
| `crates/runner-app/tests/pi_runtime_smoke.rs` | Real-binary smoke in the shape of `copilot_runtime_smoke.rs`, skipped when `pi` is not installed. | 3 |

Rough size: about thirty Rust files, a third of them compiler-forced arms; one split composer, one new status module and one new discovery adapter; five docs; one fixture; one Pencil mark.

## Probe evidence (2026-09-17, pi 0.85.1, macOS)

- **`--session-id` accepts a Runner UUID and creates the session.** A fresh `-p` run with a v4 UUID (pi's own ids are v7) printed `Warning: No project session found with id '<id>'; creating a new session with that id.` and wrote `<timestamp>_<id>.jsonl`, whose first line is `{"type":"session","version":3,"id":"<id>","timestamp":…,"cwd":…}`. A second run with the same flag continued that conversation. The default directory is `~/.pi/agent/sessions/<slug>/`, slug as in the inventory; `--session-dir` puts the file directly in that directory.
- **`--append-system-prompt` takes a file path or inline text, is composed into the system prompt, and is not persisted.** Given a path, `before_agent_start` showed the file's contents in `systemPromptOptions.appendSystemPrompt` and the reply followed it; inline text worked the same way. `dist/core/agent-session.js` builds it from the resource loader at startup, and the session JSONL holds only `session`, `model_change`, `thinking_level_change` and `message` entries. Proof it must be re-sent: a persona holding a secret codename that never appeared in any reply; the resumed session without the flag answered "I don't have a secret codename", the same resume with the flag answered the codename. (An earlier check where the resumed reply still followed the persona was the model mimicking its own prior turn.)
- **`--fork <id> --session-id <new>` forks natively.** It created `<timestamp>_<new>.jsonl` whose header carries `parentSession`, and the child continued with the parent's context.
- **Argv edges.** `-- "--model x …"` delivered the text as a message. `-- "@nonexistent …"` failed with `Error: File not found`, so `--` ends options but not `@file` expansion. `--thinking High` printed `Warning: Invalid thinking level "High"` and ran without it.
- **Extension events, in order, for one print-mode turn:** `session_start {reason: startup}` → `before_agent_start` → `agent_start` → `turn_start` → `message_end` (user) → `message_end` (assistant, `stopReason: "stop"`, `usage`) → `turn_end` → `agent_end` → `agent_settled` → `session_shutdown {reason: quit}`. `ctx.mode` was `print`; the interactive capture reported `tui`. `ctx.sessionManager.getSessionId()` returned the assigned id in every handler.
- **Documented, not exercised:** `tool_execution_start` / `_end` (`toolCallId`, `toolName`), `session_before_compact` / `session_compact` / `session_compact_failed`, `ui_prompt_start {kind: select | confirm | input | editor | custom, title}` / `ui_prompt_end` (nested prompts coalesced into one span; added in pi 0.84.4 on 2026-08-28), assistant `stopReason: "error" | "aborted"` with `errorMessage`, and `session_start` reasons `reload | new | resume | fork` with `previousSessionFile`. Orca's extension adds two field lessons: pi tears down an open dialog on session replacement without `ui_prompt_end`, so `session_shutdown` must clear waits; and a `reload` start is not a turn boundary.
- **Project trust.** In a directory holding `.agents/skills/`, the TUI opened on "Trust project folder?" (Trust / Trust parent folder / Trust this session only / Do not trust / Do not trust this session only) before any session started. A `-e` extension received `project_trust {cwd}` with `mode: tui` first, as `dist/core/project-trust.js` shows: pre-trust extensions get the event, a `yes`/`no` answer wins and suppresses the dialog, then saved decisions, then `defaultProjectTrust`, then the dialog. The dialog itself emitted no `ui_prompt_start`, so the wait bridge cannot see it. Runner's own repository has `.agents/skills/`, so this is the common case. Print mode never prompts; `--approve` / `--no-approve` decide for one run without saving. (An earlier capture that saw no events at all was a probe bug: the script never passed its environment to the child.)
- **Terminal shape.** Regular mode is inline: no `DECSET 1049`, no mouse capture, bracketed paste on, synchronized output (`?2026`) around frames. It pushes kitty keyboard flags (`CSI > 7 u`) and queries them (`CSI ? u`); Runner leaves alacritty's `kitty_keyboard` off, so the query goes unanswered and pi should fall back to legacy keys. No OSC title at startup.
- **Models.** `pi --offline --list-models` printed a table of seven models from the two providers with credentials on this machine (DeepSeek and MiniMax), with no network. `settings.json` carries `defaultProvider` and `defaultModel`; the docs add `defaultThinkingLevel`, `defaultProjectTrust` (`ask` by default) and `sessionDir`.
- **No built-in dialogs.** pi's eight built-in tools (`read`, `bash`, `powershell`, `edit`, `write`, `grep`, `find`, `ls`) never prompt. The `ask_question` named in `pi --help`'s `--exclude-tools` example has no implementation in the installed package; it is an extension tool.
- **Quiet start.** `PI_SKIP_VERSION_CHECK=1` disables only the version check; `--offline` also stops package update checks, telemetry and installing missing project packages, and a provider call still worked under it.
- **Windows (docs, not probed).** pi runs its bash tool through Git Bash, with an optional `powershell` tool. The npm install is a `.cmd` shim, which puts it on Runner's Windows batch path.

## Scope

### In scope

- The runtime catalog entry and every site in the inventory above.
- The three prompt layers on `--append-system-prompt <file>` for every spawn; the lead's goal as the only first turn; workers and direct chats open at pi's empty prompt.
- Model and effort through `--model` and `--thinking`, defaults from `settings.json`, and model discovery from `pi --list-models`.
- Caller-assigned `--session-id` for fresh and resumed sessions, the one-glob conversation probe, native fork, and rekeying when `/new`, `/resume` or `/fork` runs inside a pane.
- Hook status on macOS and Windows through a Runner-owned extension.
- `--approve` for mission slots, nothing for chats.
- Skills pane catalog from the two personal roots, no global toggle.
- The provider mark, decided on the canvas first (decision 11).
- Terminal fixture, real-binary smoke test, README columns and arch docs.

### Out of scope

- Moving the other four runtimes off the first-turn fold: nothing to move them to until their CLIs grow an interactive system-prompt flag.
- pi extension and package management (`pi install`, `pi config`), prompt templates and themes.
- MCP: pi has no MCP client.
- A token ledger reader for pi's per-message `usage`; that is [#630](https://github.com/yicheng47/runner/issues/630).
- `--mode rpc` / `--mode json`, `--tui-mode fullscreen`, `/name` session names, and OMP.
- Per-skill on/off for pi; `sessionDir` in `settings.json` or a role's own `--session-dir` anywhere in Runner. The environment variables `PI_CODING_AGENT_DIR` and `PI_CODING_AGENT_SESSION_DIR` are the follow-up [#666](https://github.com/yicheng47/runner/issues/666).
- The #533 update button (named here, built there), installing pi, and provider authentication.

## Decisions

1. **The prompt layers ride `--append-system-prompt <file>` on every spawn; only the goal is a first turn.** Per session kind, the system-prompt body is: direct chat, the role's persona (Layer 3); worker, the coordination preamble (Layer 1), the crew's team conventions (Layer 2) and the persona (Layer 3), today's `compose_worker_first_turn` body; lead, the identity line, the team conventions, the brief, the crewmates roster and the coordination section, everything in today's launch prompt except `== Mission ==`. The lead's first turn is `== Mission ==` with the goal, so a goal that starts with `@` or `-` is never argv's first character. Workers and direct chats have no first turn: a worker waits for its inbox as it does today, and a role chat opens at pi's prompt instead of spending a turn on its persona. Runner writes the body to `<app data>/session-prompts/<session id>.md` before each spawn and passes the path; the file is removed when the session ends and swept at startup. Inline text would work (probed), but a file keeps the body out of argv, where an npm `pi.cmd` on Windows meets cmd.exe's 8,191-character line limit (the limit #610 hit with Codex hooks). pi does not persist the text, so resume writes and passes it again from the current role, crew and roster rows: the role is live, not frozen at the chat's birth, and the coordination verbs survive compaction because they arrive with every request. The other four runtimes keep the fold, and a test pins their composed bodies byte-identical.
2. **Keys are caller-assigned; fresh and resume share `--session-id`.** Copilot's plan shape, no capture thread. A missing conversation keeps its key, because pi recreates the session under the same id. Print mode writes a "creating a new session with that id" warning on every fresh start; whether the TUI shows it too is for the fixture to record. The v1 probe assumed pi's default session directory; [#666](https://github.com/yicheng47/runner/issues/666) adds `PI_CODING_AGENT_SESSION_DIR` and `PI_CODING_AGENT_DIR`, while `sessionDir` in `settings.json` and a role's own `--session-dir` remain documented, not handled.
3. **Native fork is one direct spawn.** `--fork <source> --session-id <new>` needs no headless step, so pi forks to a pane or tab exactly like claude-code under [60](./60-fork-chat-to-pane-or-tab.md). The forked session gets its own system-prompt file, composed from the same rows.
4. **Hooks ride `-e`, not the user's extension directory.** `<app data>/pi-hooks/runner-status.ts` is regenerated at startup and passed with `-e` per spawn, the counterpart of claude-code's `--settings`, codex's `-c hooks.*` and Copilot's `--plugin-dir`: additive, per invocation, nothing written into `~/.pi`. It is inert unless `RUNNER_PI_STATUS_PATH` and `RUNNER_PI_STATUS_GENERATION` are set and `ctx.mode` is `tui`. It appends one complete JSON line per event with a single `fs.appendFileSync` (`generation`, `hook_event_name`, the event's fields inline, no payload file), which `HookFeed::read_report` already accepts; the extension file doubles as the reporter whose removal means bridge loss. Explicit `-e` paths load even under a role's `--no-extensions`. A nested `pi` started from the bash tool inherits the env vars but not `-e`, so it cannot write into its parent's feed. Node runs the reporter on both platforms, so there is no `sh` or PowerShell body. `hooks_supported(Pi)` turns on in the same PR as the extension.
5. **Event mapping.** `session_start` owns the generation except for `reason: reload`. Working from `agent_start`; the `UsingTools` detail while any `toolCallId` is between `tool_execution_start` and `tool_execution_end`; `CompactingContext` from `session_before_compact` until `session_compact` or `session_compact_failed`. Idle from `agent_settled`, never `agent_end`, because pi may still retry, compact or run a queued follow-up. The outcome comes from the last assistant `message_end` before settling: `error` → Failed with `errorMessage`, `aborted` → Interrupted, anything else → Completed. `ui_prompt_start` raises a wait, `confirm` as Approval needed, `select`, `input` and `editor` as Answer needed, `custom` as Needs you, cleared by `ui_prompt_end`; `session_shutdown` clears every wait, since a replaced session never sends the matching end. pi ships no dialog of its own, so for most users a pi row never shows a wait; waits come from extensions the user installed, and the trust dialog is invisible to the bridge (decision 6 covers it for missions). The README cell says "from extensions only".
6. **Mission slots pass `--approve`; chats pass nothing.** An unattended slot in a directory with `.agents/skills` or `.pi` resources would sit on "Trust project folder?" with no status signal. `--approve` is pi's documented one-run `trustOverride`: it trusts for that run only and writes nothing to `~/.pi/agent/trust.json`. Not taken: answering `project_trust` from the status extension, which the probe shows would work; it would tie trust to the phase-2 bridge and to `ctx.mode`, while `--approve` is one argv token in phase 1. A chat has a human at the prompt, whose answer pi saves as usual, in line with [596](./596-chat-permission-posture.md). Accepted risk: trusting loads the project's `.pi` extensions and settings, the same boundary the crew's own bash tool already has in that directory, since pi has no sandbox. Signed off by Jason on 2026-09-18, who added that mission agents should bypass everything by default: the app-wide mission permission mode already defaults to Bypass ([#527](https://github.com/yicheng47/runner/issues/527)), and for pi, which has nothing to bypass, `--approve` is what that default means.
7. **No permission modes.** pi has no approval gating and no sandbox (its security doc says real isolation belongs to the OS), so the role form hides the control, every permission function returns empty or Default, and the app-wide mission permission mode leaves pi argv untouched. The README column says so.
8. **Quiet start through `PI_SKIP_VERSION_CHECK=1`**, the [#475](https://github.com/yicheng47/runner/issues/475) intent. Not `--offline`, which would also stop a project's missing pi packages from installing.
9. **Enabled by default on both platforms.** pi is a Node CLI; on Windows its npm `.cmd` shim takes the batch first-turn path, and its bash tool needs Git Bash, which the Agents row does not check.
10. **Rekey through the Claude drop file.** `/new`, `/resume` and `/fork` inside a pane replace pi's session, and `agent_session_key` would go stale, so the next relaunch would reopen the old conversation. On every `session_start` whose `getSessionId()` differs from the last id it reported, starting from `RUNNER_PI_SESSION_KEY`, the extension writes `{"session_id": "<new id>"}` atomically (temp file, rename) to `RUNNER_PI_REKEY_PATH`, which Runner sets to `claude_rekey::drop_path(app data, runner session id)`. `ClaudeSessionKeyWatcher` already runs for every runtime, checks the id is a UUID (pi's are v7), rekeys the row and emits the update. No new Rust beyond three env vars.
11. **The mark is pi's own silhouette in the theme foreground, the Codex treatment.** Decided by Jason on 2026-09-18 on the design frame. pi.dev's logo is three blocks, coral `#F09082`, blue `#4D9ABF` and yellow `#F1BE58`, and Runner paints a mark as one mask in one tint, so the frame drew it three ways: the silhouette in `$text-1`, the silhouette in coral, and the three blocks as three stacked masks. Option 1 was chosen: zero new rendering code, and the shape is the one-colour favicon pi.dev itself ships, so the mono mark is not a Runner invention. Accepted tradeoff: two neutral marks in the rail, Codex and pi, told apart by shape, the round knot against the square π. Blue would have been the coloured fallback, being the one hue no provider or status uses; coral sits beside Claude's orange and yellow is the warning colour. The geometry is a 4 × 4 grid of 6-unit cells with the eye as a reverse-wound subpath, `M0 0h18v12h-6v6H6v6H0zM6 6v6h6V6zM18 12h6v12h-6z` in `viewBox -2 -2 28 28`, so it lands on whole pixels at 12 and 24 px. A stopped row draws it at 0.45 under [#641](https://github.com/yicheng47/runner/issues/641), where every option would have looked the same.

## Implementation Phases

### Phase 0 — design (done 2026-09-18)

`design/specs/539-pi-runtime.pen`, frame `Spec — pi runtime (539) · v1` (`h5rNdS`): the pi mark beside the four shipped marks at 12/14/16/24 px, drawn three ways (A); the rail before and after (B); the Agents row with the model read from `settings.json` (C); trust and permissions as decision 6 (D); the three tint options, each with running and stopped rows on Carbon and Runner Light (E). Decision 11 records the pick. The role form without a permission control reuses existing components and needed no frame.

### Phase 1 — the runtime (`runner-backend` + `runner-app`)

Every phase-1 row in the inventory: enum, definitions, the split composer and the system-prompt file at start and resume, `--append-system-prompt` and the goal turn in argv, resume and fork plans, conversation probe, runtime defaults, catalog entry, model discovery, `--approve` for slots, `PI_SKIP_VERSION_CHECK`, empty permission arms, Skills caption, provider mark, every test loop, README columns, arch §5 and §6. `hooks_supported(Pi)` stays false. A role chat opens at pi's prompt with the persona in force; a crew starts with the lead on its goal and the workers waiting; both resume by id after relaunch with the layers re-supplied.

### Phase 2 — hook status and rekey (macOS and Windows)

`pi_status.rs` with the embedded extension, installation at startup, `HookStatusWatcher::Pi`, the env injection, decision 5's mapping, decision 10's drop file, `hooks_supported(Pi)` on, the README hooks cells, and `docs/tests/539-pi-hooks-smoke.md`.

### Phase 3 — smoke, fixture, archive

macOS, then JASONPC: direct chat with and without a role, a crew with a pi slot in Runner's own repository, relaunch and resume, edit a role's prompt then relaunch its chat, `/new` in a pane then relaunch, fork to a pane, model discovery, keyboard input (Enter, Shift+Enter, Esc, Ctrl+C, arrows) under the unanswered kitty query. Record `pi-first-turn.ndjson`, add `pi_runtime_smoke.rs`, archive this spec and the plan.

## Verification

Phase 1:

- [ ] `runtime_list` includes `pi` with command `pi` and `native_fork: true`; `Runtime::parse("pi")` round-trips.
- [ ] `model = deepseek/deepseek-v4-pro`, `effort = High` produce `--model deepseek/deepseek-v4-pro --thinking high`.
- [ ] A fresh lead spawn carries `--session-id <uuid>` before the role's args, then `--append-system-prompt <app data>/session-prompts/<id>.md`, then `-- == Mission == …` last; `agent_session_key` is set before the process starts; a goal starting with `@` or `-` reaches pi as text.
- [ ] A worker and a role chat carry `--append-system-prompt <file>` and no body; the session file shows no user turn until the human or the bus writes one.
- [ ] The system-prompt file for a worker is today's `compose_worker_first_turn` body; for the lead it is today's launch prompt minus `== Mission ==`; for a role chat it is the persona. The four other runtimes' composed bodies are byte-identical to before.
- [ ] Resume carries `--append-system-prompt <file>` and no body; a codename placed in the role's prompt after the chat was created is known to the resumed conversation.
- [ ] Deleting the session file and relaunching starts a fresh conversation under the same id, with the lead's goal re-sent, without an error.
- [ ] Fork produces a pane whose session file has `parentSession` pointing at the source and whose own system-prompt file exists.
- [ ] A pi mission slot in a directory with `.agents/skills/` starts without the trust prompt; a pi chat there shows pi's own dialog.
- [ ] The role form shows no permission control for pi; the mission permission mode leaves pi argv unchanged.
- [ ] The model picker lists the `provider/model` entries from `pi --list-models`; with no credentials it falls back to the default option.
- [ ] A mission with a pi slot completes a `runner msg` round trip.
- [ ] The pi mark draws in `theme::text()` at 12, 14, 16 and 24 px on both themes and dims to 0.45 when the session is stopped.
- [ ] Every runtime-enumerating test in `router/runtime.rs`, `ops/runtime.rs`, `ops/slot.rs`, `runtime_status.rs`, `chat_icon.rs`, `command_palette.rs`, `archived.rs`, `panes.rs`, `sidebar/tests.rs` and `roles/tests.rs` lists `pi`.

Phase 2:

- [ ] A fresh spawn carries `-e <app data>/pi-hooks/runner-status.ts`; a turn shows Working then Idle only after `agent_settled`; a tool run shows `Working · Using tools`; `/compact` shows `Compacting context`; Esc during a turn ends Interrupted; a provider error shows Response failed; an extension's `ctx.ui.confirm` shows Approval needed until answered; a nested `pi -p` from the bash tool changes nothing.
- [ ] `/new` inside a pane, then relaunch, resumes the new conversation, and the sidebar row updated without a restart.
- [ ] pi older than 0.84.4 shows estimated status with no error.
- [ ] The same on JASONPC, with no PowerShell or Git Bash reporter involved.

Phase 3:

- [ ] The pi fixture renders without stray escapes, and Enter, Shift+Enter, Esc, Ctrl+C and arrows reach pi.
