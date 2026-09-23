# 644 — Antigravity CLI runtime

> Tracking issue: [#644](https://github.com/yicheng47/runner/issues/644)
> Priority: P1, milestone 0.11.x. Platforms: macOS first; Windows after a JASONPC smoke.
> Probed 2026-09-23 against the installed `agy` on macOS, which updated itself from 1.2.8 to 1.2.9 in the middle of the probe. Every claim below comes from `--help`, the docs bundled under `~/.gemini/antigravity-cli/builtin/skills/`, agy's own logs, or a live session recorded in a PTY. This spec supersedes the issue body where they differ.

## Motivation

Antigravity CLI (`agy`) is Google's terminal coding agent: the TUI surface of Antigravity, sharing its agent engine, and the migration path from Gemini CLI. It runs Gemini models by default plus Claude and GPT-OSS, and signs in with a Google account, so supporting it puts Runner in front of users on a Google AI plan who never run Claude Code or Codex. It is a TUI on a PTY and fits the adapter the way Copilot ([540](./archive/540-copilot-cli-runtime.md)) and pi ([539](./archive/539-pi-runtime.md)) do. The inventory in 540 is the checklist; this spec lists only the values that are agy's.

## Probe evidence

### CLI surface

- **Flags** (`agy --help`, 1.2.9): `--add-dir` (repeatable), `--agent`, `-c` / `--continue`, `--conversation <id>`, `--dangerously-skip-permissions`, `--effort low|medium|high`, `-i` / `--prompt-interactive`, `--log-file`, `--mode accept-edits|plan`, `--model`, `--project`, `--new-project`, `--remote-control`, `--sandbox`, and the print-mode set (`-p` / `--print` / `--prompt`, `--output-format`, `--input-format`, `--json-schema`, `--print-timeout`, `--disable-slash-commands`). `--version` works but is not listed. There is no `-m`.
- **Subcommands:** `agent(s)`, `changelog`, `help`, `install`, `mcp` (`add`, `remove`, `list`, `enable`, `disable`), `mic-serve`, `models`, `plugin(s)` (`list`, `import`, `install`, `uninstall`, `enable`, `disable`, `validate`, `link`), `remote-control`, `update`.
- **Parsing is Go's `flag` package.** `-mode=accept-edits`, `--mode accept-edits` and `-version` are all accepted. A bad value is ignored silently: `--mode bogus` starts normally with no log line.
- **Version and updates.** `agy --version` prints a bare `1.2.9`. agy updates itself in the background at launch with no prompt and no opt-out flag; `~/.gemini/antigravity-cli/updater/update_status.json` records the result. A running session keeps its version, and the update applies at the next launch. Releases are frequent (1.2.6 to 1.2.9 in a week).
- **Sign-in.** Every launch shows "You are currently not signed in. Signing in…" for about a second while agy reads the keyring, then the banner.

### Session key and resume

- **agy assigns the conversation id; Runner cannot.** `--conversation <unknown uuid>` prints `warning: conversation "<id>" not found` to the terminal, logs `Conversation <id> not found, ignoring --conversation flag`, and starts fresh.
- **The conversation is created lazily, when the first message is sent.** With `-i`, the log shows `Created conversation <uuid>` about 3 s after spawn. Launched with no prompt, no conversation exists until the user types.
- **`--log-file <path>` is a per-session channel.** It replaces the default `~/.gemini/antigravity-cli/log/cli-<timestamp>.log`, and it carries `Created conversation <uuid>` for a fresh session and `Resuming conversation <uuid>` for a resumed one.
- **Resume works from any cwd.** `--conversation <id>` resumed the conversation in a different directory from the one that created it. The store is global.
- **On disk.** Each conversation is a SQLite file at `~/.gemini/antigravity-cli/conversations/<uuid>.db`. The readable transcript is JSONL at `~/.gemini/antigravity-cli/brain/<uuid>/.system_generated/logs/transcript.jsonl`, with `step_index`, `source`, `type` (`USER_INPUT`, `PLANNER_RESPONSE`, …), `status`, `created_at` and `content`. `~/.gemini/antigravity-cli/cache/last_conversations.json` maps each cwd to its most recent conversation, which is what `-c` uses.
- **No command-line fork.** Only the in-TUI `/fork`.

### Folder trust

- **An untrusted cwd opens a "Do you trust the contents of this project?" dialog** before anything else. Trust is per exact path: with `/Users/jason` trusted, `/Users/jason/repos/yicheng47/runner` still showed the dialog. The list is `trustedWorkspaces` in `~/.gemini/antigravity-cli/settings.json`.
- **The `-i` first turn does not wait for the dialog.** In an untrusted folder with the dialog still on screen and unanswered, agy sent the prompt, the model answered, and the conversation recorded both steps. For a no-tool prompt nothing else happened; what tools would do behind the dialog was not tested.

### Model and effort

- **`agy models`** needs the network and a signed-in account. It lists ids with the effort baked in: `gemini-3.8-flash-{high,medium,low}`, the same for `3.7` and `3.6`, `gemini-3.1-pro-{high,low}`, `claude-sonnet-4-6`, `claude-opus-4-6-thinking`, `gpt-oss-120b-medium`.
- **`--model` takes the full id or a base alias.** `--model gemini-3.1-pro --effort high` resolves to `gemini-3.1-pro-high`. `--effort` on its own applies to the default model.
- **Every invalid combination silently runs the default model** (Gemini 3.8 Flash High on this account), and the only trace is a line in agy's log: a leveled id plus `--effort`; `--effort` on a Claude or GPT-OSS model; a level the model lacks (`gemini-3.1-pro` has no `medium`); an unknown id.
- The footer shows the resolved model and level, `Gemini 3.8 Flash · high`.

### Permissions

- **Settings.** `toolPermission` in `settings.json`: `request-review` (default), `proceed-in-sandbox`, `always-proceed`, `strict`, plus `action(target)` allow and deny rules. The issue's "three levels" missed `proceed-in-sandbox`.
- **Flags.** `--mode accept-edits` logs `overriding agent mode to accept-edits`. `--dangerously-skip-permissions` auto-approves everything. Under `request-review`, a `view_file` read ran without a prompt.

### Hooks

- **Five events:** `PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation`, `Stop`. None fires when a permission prompt or a question is on screen, so hooks cannot report Approval needed.
- **Locations.** Global `~/.gemini/config/hooks.json`, workspace `.agents/hooks.json`, and plugins. Commands run through `sh -c` (`cmd /c` on Windows) with the directory that holds `hooks.json` as the working directory.
- **`--add-dir <dir>` loads `<dir>/.agents/hooks.json` for one launch, alongside the global file.** The log said `loaded 2 named hooks from 2 hooks.json file(s)`, the hooks fired, and they inherited the spawn environment: a variable set only on the `agy` process reached the hook command.
- **The added directory is visible to the model.** It appears in `workspacePaths` and in the agent's workspace list. With the probe folder named `rh`, the model spent turns guessing that it was a "red herring" workspace and wandered.
- **`PreToolUse` must answer `{"decision":"ask"}`; `{}` denies.** With a hook that printed `{}`, every tool call was refused ("Access was denied by a pre-tool hook") and the model flailed through a blocked search and a blocked write. With `{"decision":"ask"}` the read ran unprompted under `request-review`. `{"decision":""}` and empty output were inconclusive: the model did not reach a tool within 55 s.
- **Payloads are camelCase.** Every event carries `conversationId`, `workspacePaths`, `transcriptPath`, `artifactDirectoryPath` and `modelName` (the leveled id). `PreInvocation` and `PostInvocation` add `invocationNum` and `initialNumSteps`; `PreToolUse` adds `stepIdx` and `toolCall: {name, args}`; `Stop` adds `executionNum`, `terminationReason`, `fullyIdle` and `error`. Observed tool names: `run_command`, `view_file`, `write_to_file`, `invoke_subagent`, `search_web`; the tool list in the system prompt also names `ask_question`. A finished no-tool turn ended with `terminationReason: "NO_TOOL_CALL"` and `fullyIdle: true`, not the documented `model_stop`.
- **Order within a turn:** `PreInvocation` → `PreToolUse` → `PostToolUse` → `PostInvocation` → the next `PreInvocation` → … → `Stop`. A denied tool has no `PostToolUse`.
- **The dev Mac also runs Orca's hooks.** `~/.gemini/config/hooks.json` holds Orca's `orca-status`, whose `PreToolUse` always answers `{"decision":"ask"}`. Any agy session on this machine, Runner's included, runs it.

### MCP, skills, rules

- **MCP.** `agy mcp add runner <command> [args…]` writes `~/.gemini/config/mcp_config.json` as `{"mcpServers":{"runner":{"args":[…],"command":"…","disabled":false}}}`, omitting `args` when empty. It accepts a 0-byte file, which is what the dev Mac has. `~/.gemini/antigravity/mcp_config.json` belongs to the Antigravity desktop app and `~/.gemini/settings.json` to Gemini CLI; neither is agy's.
- **Skills.** The conversation's customization record lists the personal roots `~/.gemini/antigravity-cli/skills` and `~/.gemini/skills`; the project root is `.agents/skills` (also `.agent/`, `_agents/`, `_agent/`). Built-in skills sit in `~/.gemini/antigravity-cli/builtin/skills/`. Plugins are enabled and disabled in `~/.gemini/config/config.json`.
- **Rules.** `AGENTS.md` and `GEMINI.md` load from the cwd up to the repository root, capped at 24 KB per file.

### Terminal

- DECSET `1049` alternate screen, `2004` bracketed paste, hidden cursor. **No mouse reporting.** It pushes kitty keyboard flags (`CSI > 1 u`), queries them (`CSI ? u`), sets modifyOtherKeys (`CSI > 4;2 m`), and queries DA1, synchronized output (`?2026$p`, `?2027$p`) and kitty graphics. Runner's terminal answers none of the kitty queries, so agy falls back to plain key encoding, as other runtimes do.
- **No OSC 0/2 title at any point**, before or after a turn.
- Runner's `encode_scroll` turns the wheel into Up/Down arrows on an alternate screen without mouse reporting, so every wheel tick reaches agy as an arrow key.

## What a new runtime touches

Sites and their shapes follow the [540 inventory](./archive/540-copilot-cli-runtime.md#what-a-new-runtime-touches); these are agy's values. *Exhaustive* marks the `match` arms the compiler forces.

### Spawn path (`crates/runner-backend`)

| Site | agy |
| --- | --- |
| `model.rs` `Runtime` | `Antigravity`, wire name `antigravity`; round-trip test. *Exhaustive.* |
| `router/runtime.rs` `RUNTIME_DEFINITIONS` | Display `Antigravity CLI`, command `agy`, `native_fork: false`, `skills_dirs: [".gemini/antigravity-cli/skills", ".gemini/skills"]`. Appended. |
| `model_effort_args` | `--model <id>`; `--effort <level>` only when the chosen catalog model lists that level (decision 3). Never `--effort` without `--model`. |
| `first_turn_argv` | `["-i", body]`, as Copilot. |
| `system_prompt_args` | Empty; the persona folds into the first turn (decision 6). |
| `permission_mode_args` and the strip, infer and mission functions | Default → nothing; AcceptEdits → `--mode accept-edits`; Bypass → `--dangerously-skip-permissions`; Auto hidden. The strip set covers `mode` (value-bearing) and `dangerously-skip-permissions` in all four Go spellings: `-x`, `--x`, `-x=v`, `--x=v`. *Exhaustive on `(runtime, mode)`.* |
| `mission_bus_sandbox_args` | `--add-dir <mission dir>`, as Codex and Copilot. |
| `trailing_runtime_args` | Always `--log-file <app data>/antigravity/logs/<runner session id>.log`; in phase 3, also `--add-dir <app data>/antigravity-hooks`. |
| `resume_plan` | Fresh: no key flag, `assigned_key: None`, and the key is captured from the log. Resume: `--conversation <key>` as a trailing flag, `assigned_key` set, `resuming: true`. |
| Conversation probe | `antigravity_conversation_exists(key)` checks `~/.gemini/antigravity-cli/conversations/<key>.db`. |
| `session/agy_capture.rs` (new) | Tails the per-session log for the first `Created conversation <uuid>` and persists it as `agent_session_key` (decision 1). |
| `session/agy_trust.rs` (new) | Seeds the spawn cwd into `trustedWorkspaces` before every spawn (decision 2). |
| `session/manager/spawn.rs` | Trust preseed, capture start, the conversation-missing arm, the first-turn warning gate, the Windows batch fallback list. *Exhaustive in two places.* |
| `session/codex_capture.rs` `sessions_root_for` | `Antigravity => None`. *Exhaustive.* |
| `runtime_defaults.rs` | None in v1. Corrected 2026-09-23: `/model` persists to `settings.json` as a display label (`"model": "Gemini 3.8 Flash (High)"`), not an id `--model` accepts, so reading it needs a label-to-alias map. *Exhaustive.* |
| `ops/runtime.rs` catalog | Description; `default_enabled: true` on macOS, Windows after its smoke; the static model list with per-model efforts (decision 3). |
| `ops/runner.rs`, `ops/slot.rs`, `mcp/tools/session.rs`, `runtime_status.rs` | Runtime lists, error strings and tests gain `antigravity`. |
| `runtime_status/models.rs` `DISCOVERY_RUNTIMES` | Unchanged in v1. `agy models` needs network and sign-in and prints leveled ids that need grouping; that is a follow-up. |

### Settings, identity, status (`runner-app`, `ops/mcp.rs`, `skills.rs`)

| Site | agy |
| --- | --- |
| `ops/mcp.rs` `McpClientId` | `Antigravity`, path `~/.gemini/config/mcp_config.json`, entry `{"command":…,"args":[…],"disabled":false}` under `mcpServers`; a 0-byte or missing file reads as `{}`. *Exhaustive in four matches.* |
| `app_store/mcp_defaults.rs` | Gone since [#648](https://github.com/yicheng47/runner/issues/648) removed Runner's MCP registration; `mcp_removal.rs` gains only a compile arm, since Runner never registered with agy. `McpClientId::Antigravity` makes agy's servers a Settings → MCP catalog column. *Exhaustive.* |
| `surfaces/settings/agents.rs` | Catalog-driven. For [#533](./533-agent-cli-updates.md): `agy --version` prints a bare semver and `agy update` is the update command. The badge is mostly moot because agy updates itself. |
| `skills.rs`, `surfaces/settings/skills.rs` | Two personal roots, no global toggle (the TRAE shape). |
| `session/title.rs` | No change: agy sets no title, so the tab keeps Runner's name. |
| `session/agy_status.rs` (new), `pty_runtime.rs` `HookStatusWatcher::Antigravity` | Phase 3 (decision 5). |
| `assets.rs`, `chat_icon.rs` and the icon tests | An Antigravity mark, designed first in `design/specs/644-antigravity-runtime.pen` and signed off before code. *Exhaustive in `chat_icon.rs`.* |
| `surfaces/runners/logic.rs` | `[Default, AcceptEdits, Bypass]` with agy's wording. |
| `surfaces/panes.rs` `header_fork_state` | agy is not forkable. |

### Docs, fixture

`README.md` and `README.zh-CN.md` together (the agents row, the MCP sentence), the runtime enumerations in `docs/arch/arch.md` and `docs/arch/windows.md`, and a fixture `crates/runner-terminal/fixtures/agy-first-turn.ndjson` recorded from a live session: alternate screen with no mouse reporting.

## Scope

### In scope

- The catalog entry and every site above.
- Model and effort from a static catalog that can only emit combinations agy accepts.
- First turn on `-i`; persona folded into it.
- Session key captured from a per-session `--log-file`; resume with `--conversation`; the conversation-exists check on `conversations/<id>.db`.
- Permission modes Default, AcceptEdits and Bypass. Mission Bypass ([#527](./archive/527-mission-permission-mode.md)) maps to `--dangerously-skip-permissions`; chats assert nothing.
- Trust preseed for every spawn cwd.
- `--add-dir <mission dir>` for mission slots.
- Settings → Agents row, MCP registration, Skills pane roots.
- Provider mark, designed on the canvas first.
- Hook status on macOS for Working, Idle and Response failed.
- Terminal fixture, README rows, `docs/arch`.

### Out of scope

- Print mode, `--sandbox`, `--remote-control`, `--project` / `--new-project`, `--agent`, `/boost` and the subagent surfaces; users can put flags in runner args.
- Native fork (only `/fork` exists).
- Plugins, rules and global skill toggles.
- The Antigravity desktop app and its `~/.gemini/antigravity/` config.
- Installing the CLI, signing in, and controlling its self-update.
- Model discovery through `agy models` (a follow-up).
- Approval needed and Answer needed from hooks: agy has no event for either.

## Decisions

1. **The key comes from a per-session `--log-file`.** Every spawn passes `--log-file <app data>/antigravity/logs/<runner session id>.log`, and a capture thread tails it for the first `Created conversation <uuid>`. That is deterministic per session, needs no hooks, and works on Windows from day one. `cache/last_conversations.json` is rejected because it is keyed by cwd and two slots in one worktree collide; watching `conversations/` is rejected because the directory is global and races across sessions. The log line is agy's internal log rather than a contract, so a fixture test pins it, and a miss fails soft (no key, no resume), as Codex capture does. A blank chat keeps tailing until the first message or session end. On resume, Runner first checks `conversations/<key>.db`. If it is missing, Runner spawns fresh with the first turn and captures a new key. If agy still falls back to a fresh conversation (its own "not found" path), the next `Created conversation` line replaces the key. The file is deleted when the session row is.
2. **Seed folder trust before every spawn, like `codex_trust` and `copilot_trust`.** Add the exact cwd to `trustedWorkspaces` in `~/.gemini/antigravity-cli/settings.json` with a `preserve_order` edit that touches no other key, and create the file when it is absent. The dialog would park an unattended slot. Because agy runs the `-i` turn behind an unanswered dialog, the preseed also keeps the first turn from racing trust.
3. **The catalog only emits combinations agy accepts.** agy silently swaps in its default model on any bad pair, so the catalog is the guard. Gemini entries use base aliases with `supported_efforts`: the three Flash generations take `low`, `medium` and `high`; `gemini-3.1-pro` takes `low` and `high`. `claude-sonnet-4-6`, `claude-opus-4-6-thinking` and `gpt-oss-120b-medium` take none, and `--effort` is never passed with them. With the model left on Default, the effort picker offers only Default, because the account's default model decides which levels exist.
4. **Permission flags.** AcceptEdits is `--mode accept-edits`, Bypass is `--dangerously-skip-permissions`, and Auto is unmapped. The user's `toolPermission` governs Default rows. `--mode plan` is not a Runner mode; when Runner asserts a mode it strips any `--mode` from runner args, as it does Copilot's `--allow-tool`.
5. **Hook status registers no `PreToolUse`.** A `PreToolUse` reply is a permission decision: `{}` denied every tool, and whether `"ask"` overrides `--dangerously-skip-permissions` is unprobed. Runner therefore registers only `PreInvocation`, `PostToolUse`, `PostInvocation` and `Stop`, each answering `{}`, and never takes part in a decision. Working comes from `PreInvocation` and `PostToolUse`; Idle from `Stop` with `fullyIdle: true`; Response failed from `Stop` with a non-empty `error`, once a live failure shows its shape. The hooks ride `--add-dir <app data>/antigravity-hooks`, a Runner-owned folder holding `.agents/hooks.json` and a reporter script driven by per-session env vars through the shared `hook_feed` transport. It writes nothing into `~/.gemini`. The cost is that the folder shows in the model's workspace list, so it gets a self-explaining name and phase 3 checks for distraction. The fallback, if that cost proves real, is Orca's shape: one named entry in the global `hooks.json` that no-ops without Runner's env vars, at the price of a shell on every event of every agy session on the machine.
6. **The persona folds into the first turn in v1.** Two native channels for a follow-up: an `AGENTS.md` in a Runner-owned `--add-dir` folder, if added directories load rules (unprobed), or a `PreInvocation` hook answering `injectSteps: [{"ephemeralMessage": …}]`.
7. **Self-update is left alone.** There is no opt-out flag; an update lands at the next launch and never mid-session.
8. **Enabled by default on macOS; Windows after a smoke.** The docs place the Windows binary under `%LOCALAPPDATA%\Antigravity\`; that path, ConPTY behaviour and `cmd /c` hooks are all unverified.

## Open items

These did not get a clean answer on 2026-09-23. None blocks phase 1; each is closed in the phase that needs it.

- Whether `--dangerously-skip-permissions` skips the trust dialog. The preseed makes it moot for Runner; record it anyway.
- Whether a `PreToolUse` `"ask"`, such as Orca's global hook, overrides `--dangerously-skip-permissions`. If it does, Bypass missions on a machine with Orca installed will stop on prompts. Phase 4.
- What agy does with Up and Down, which is every wheel tick: prompt history or transcript scroll. If it is history, a wheel over an agy pane rewrites the input line, and the pane needs a wheel policy. Phase 2 fixture.
- A live `ask_question` payload, and whether the transcript tail can show Answer needed. Phase 3.
- Where `/model` persists the user's default, for `runtime_defaults.rs`.
- Whether an added directory's `AGENTS.md` loads as rules (decision 6).
- The meaning of `{"decision":""}` and of empty output from `PreToolUse` (moot under decision 5).

## Implementation phases

### Phase 0 — design

The Antigravity mark in `design/specs/644-antigravity-runtime.pen`, beside the shipped marks at 12/14/16/24 px, with the Agents row and the three permission modes. Stop for sign-off.

### Phase 1 — adapter (`runner-backend`)

Enum, definition, argv, permission flags, resume plan, log capture, conversation probe, trust preseed, catalog, and the test loops. `make verify` green. A direct chat and a mission slot start in a never-trusted directory, take their first turn once, and resume by id after relaunch.

### Phase 2 — settings and identity (`runner-app`, `ops/mcp.rs`)

MCP client and default registration, the Agents row, the Skills pane, the mark and icon tests, permission copy, the fixture, the wheel check, docs and READMEs.

### Phase 3 — hook status (macOS)

The Runner-owned hooks folder, `agy_status.rs`, the watcher, env injection and the decision 5 mapping, with its smoke checklist in [`docs/tests/644-antigravity-smoke.md`](../tests/644-antigravity-smoke.md).

### Phase 4 — smoke (macOS, then JASONPC)

Direct chat: the first turn arrives once, the pane paints, the tab keeps Runner's name. Crew with an agy slot: the launch prompt lands once; `runner msg read/post` and `runner signal ask_human` work from `run_command`; Bypass runs unattended, with and without Orca's global hook present. Relaunch resumes by id; deleting `conversations/<id>.db` leads to a fresh start with a new key. Windows: direct chat and mission under ConPTY with baseline status.

### Phase 5 — follow-ups, each its own issue

Model discovery from `agy models`; the persona on a native channel; hooks on Windows; `/usage` for the [706](./706-agent-usage.md) usage popover.

## Verification

- [ ] `runtime_list` includes `antigravity` with command `agy`; `Runtime::parse("antigravity")` round-trips.
- [ ] `gemini-3.1-pro` with `high` produces `--model gemini-3.1-pro --effort high`; `claude-opus-4-6-thinking` never carries `--effort`; the picker offers no `medium` for Pro and no effort for Default.
- [ ] A fresh spawn carries `--log-file <app data>/antigravity/logs/<session>.log -i <body>` and no `--conversation`; the key appears on the row within seconds of the first message.
- [ ] A blank chat has no key until the first message, then gets one.
- [ ] Resume carries `--conversation <key>` and no `-i`; a missing `conversations/<key>.db` gives a fresh spawn with the first turn and a new key.
- [ ] AcceptEdits rows spawn with `--mode accept-edits`, Bypass rows with `--dangerously-skip-permissions`, Default rows with neither; the strip removes `-mode=plan`, `--mode plan` and `-dangerously-skip-permissions`.
- [ ] A never-trusted cwd is added to `trustedWorkspaces` before spawn with every other key unchanged, and the session shows no trust dialog.
- [ ] A server copied in Settings → MCP lands in `~/.gemini/config/mcp_config.json` with `disabled: false`, including when the file starts at 0 bytes (Runner no longer registers itself since #648).
- [ ] Hook status: a finished turn shows Idle, a running turn Working; tools still run under `request-review`, `accept-edits` and bypass with Runner's hooks loaded, which proves Runner sends no `PreToolUse` decision.
- [ ] The agy fixture renders without stray escapes, and wheel input behaves as phase 2 decided.
- [ ] Every runtime-enumerating test lists `antigravity`.
