# Qoder CLI as a first-class runtime

## Status

Implemented. Tracking issue [#341](https://github.com/yicheng47/runner/issues/341). Capabilities were live-probed against qodercli v1.1.4 on 2026-07-24; its installed bundle and help output were inspected during implementation to pin session-path encoding and unprobed follow-up flags. This is the first runtime added since [0033](0033-runtime-executable-discovery-and-overrides.md) shipped the Agents settings pane, so it doubles as the proof that adding a runtime is now mostly a catalog edit.

## Problem

`RUNTIME_DEFINITIONS` (`router/runtime.rs:37`) has exactly two entries. Qoder's CLI is a near-clone of claude-code's — self-assigned `--session-id`, `--resume <uuid>`, positional first-turn prompt, `--permission-mode`, `~/.qoder/projects/<cwd-slug>/<uuid>.jsonl` session files in the same layout as `~/.claude/projects` — so it takes the cheapest adapter path Runner has. What makes it non-trivial is not the adapter; it's that per-runtime behavior is spread across ~12 `match` arms in Rust plus five hand-ported `Record` maps in TypeScript, several of which have inverted-polarity defaults that silently give a new runtime the *wrong* behavior rather than no behavior.

Three specific traps, all confirmed in the current tree:

1. **`runtime_purges_on_resume` (`output.rs:798`) is a denylist**: `!matches!(runtime, Some("claude-code"))`. A new runtime silently inherits codex's purge-and-full-reset on resume. Qoder repaints its conversation like claude-code, so it needs an explicit exception or resumed panes lose their scrollback.
2. **`runtimeClearsOnResize` is duplicated** in Rust (`output.rs:770`) and TypeScript (`RunnerTerminal.tsx:168`) with no shared source. Changing one without the other makes the local xterm pre-clear and the backend ring purge disagree.
3. **Declaration order is load-bearing.** `RUNTIME_OPTIONS[0]` is the default in the Create Runner form (`CreateRunnerModal.tsx:43`), and a `runtime_status.rs` test indexes `.runtimes[0]`. Both lists must be appended to, never prepended.

## Key Decisions

1. **Mirror claude-code, not codex.** Qoder accepts a caller-supplied `--session-id` at spawn, so it uses the self-assign path and needs **no** post-spawn session-key capture. This deliberately avoids `codex_capture.rs` entirely — that module is named and typed for codex (`CodexCaptureContext`, `SessionHandle.codex_capture`), and a second self-assigning runtime would force a refactor we don't need here.
2. **Share the conversation-file guard, not the project slug encoder.** Claude Code and Qoder both store `<uuid>.jsonl` under an agent dotdir's `projects/<cwd-slug>/`, so the existence check is shared while thin runtime wrappers provide the dotdir and encoder. Qoder's installed v1.1.4 bundle replaces every non-ASCII-alphanumeric UTF-16 code unit with `-`; over 200 units it keeps the first 200 and appends `-${abs(djb2(original)).toString(36)}`. Claude Code keeps Runner's previously verified `/` and `.` replacement. The resume guard at `spawn.rs:1179-1197` accepts both runtimes without assuming their slugs are identical.
3. **Ship only the permission mode that was probed.** `--permission-mode auto` is verified. Qoder's help declares `default`, `accept_edits`, `bypass_permissions`, `dont_ask`, and `auto`, but only `auto` was live-probed; notably the additional values are snake_case rather than claude-code's camelCase. An invalid value makes the CLI refuse to start, so: `Default` → no flags, `Auto` → `--permission-mode auto`, and the other modes stay omitted until probed. `default_permission_mode()` is `Auto` (`commands/runner.rs:59`) and `runtime_direct_runner` applies it unconditionally, so the verified mode is exactly the one every direct chat needs.
4. **No model or effort flags in v1.** Qoder's help declares `-m` / `--model` and `--reasoning-effort <level>`, but accepted values and interactive behavior remain unprobed; the effort flag also differs from claude-code's `--effort`. The lookups degrade correctly — `runtimeSupportsEffort()` returns false, `modelSuggestions()` returns `[]`, and `model_effort_args` falls through to `Vec::new()`. Adding verified mappings later is additive and needs no migration.
5. **Keep-ring on resume, clear-on-resize — both flagged for smoke-test confirmation.** Qoder is a full-repaint TUI that restores the prior conversation on `--resume`, which is claude-code's profile on both axes. Both settings are single-line changes if the smoke test contradicts them, and the verification list below makes that an explicit check rather than an assumption.
6. **The Agents settings pane needs no per-runtime code — but does need one fix.** `runtime_status.rs` and `commands/runtime.rs` are fully catalog-driven (verified: no hardcoded runtime names outside `#[cfg(test)]`), and `AgentsPane.tsx` renders entirely from `display_name`/`command` off the backend rows. Discovery, override, validation, and the not-found pre-spawn error all come free with the catalog entry. The one hardcode is the loading skeleton at `AgentsPane.tsx:81` — `[0, 1].map(...)` renders exactly two placeholder rows and will be short by one. Make it render from the previous row count or a constant derived from the catalog.
7. **Register Runner MCP in Qoder's user config.** The [official Qoder CLI MCP documentation](https://docs.qoder.com/en/cli/mcp-servers) stores user-scoped servers in `~/.qoder/settings.json` under the same `mcpServers` JSON shape used by Claude Code. Settings → MCP gets a Qoder toggle and manual snippet; writes preserve every unrelated setting and replace or remove only `mcpServers.runner`. Existing Qoder sessions need `/mcp reload`; new sessions discover it on startup.
8. **No launch gate.** `enter_claude_launch_gate` (`spawn.rs:70`) exists for claude-code's OAuth refresh-token race under concurrent spawns. Whether qoder has the same problem is unknown; adding a 1500ms serialization gate speculatively would slow every multi-slot mission for a hypothetical bug. Left off, with a smoke-test item that would catch it.

## Goals

- Qoder is selectable everywhere claude-code and codex are: direct-chat runtime picker, runner form, and crew slot runtime override.
- Settings → Agents shows a Qoder row with its detected `qodercli` path, an override field, and the same six states as the other runtimes.
- Settings → Agents owns the Direct-mode default runtime; the standalone Chat settings pane is removed.
- Settings → MCP can register or unregister Runner in Qoder's user config.
- A qoder direct chat spawns, takes its first turn via positional argv, stops, and resumes into the same conversation with scrollback intact.
- A qoder mission slot receives its worker preamble and responds to inbox nudges.
- A missing `~/.qoder/projects/<slug>/<uuid>.jsonl` falls back to a fresh spawn instead of an error loop.

## Non-Goals

- Model and effort flag mapping (unprobed; additive later).
- `accept_edits` / `bypass_permissions` / `dont_ask` permission modes (declared but unprobed; see decision 3).
- Generalizing `codex_capture.rs` into a runtime-agnostic capture framework — qoder doesn't need it, and no third self-assigning runtime is on the table.
- Generating `src/components/ui/runtimes.ts` from the Rust catalog. It stays a hand-port; this impl adds one more entry to the existing maps. (The duplication is real and worth its own issue — out of scope here.)

## Implementation Phases

### Phase 1 — Rust catalog and adapter (`src-tauri/src/router/runtime.rs`)

- **Append** to `RUNTIME_DEFINITIONS` (`:37`): `{ name: "qoder", display_name: "Qoder", command: "qodercli" }`. Append — see Problem trap 3.
- `resume_plan` (`:612`): add a `"qoder"` arm mirroring claude-code exactly — `Some(k) if is_uuid(k)` → `args: ["--resume", k]`, `prepend: false`, `assigned_key: Some(k)`, `resuming: true`; otherwise self-assign a fresh UUID via `["--session-id", id]`.
- `first_turn_argv` (`:492`): add `"qoder"` to the `"claude-code" | "codex"` arm. Mandatory — without it a qoder lead spawns with no persona and no mission goal.
- `permission_mode_args` (`:198`) and `mode_match_pairs` (`:372`): `("qoder", Auto)` → `["--permission-mode", "auto"]`. No `AcceptEdits`/`Bypass` arms (decision 3).
- `strip_permission_flags` (`:250`): `"qoder"` → `&[("--permission-mode", true)]`, so a mode change round-trips.
- Generalize the conversation-file existence check per decision 2; keep the claude-code wrapper and add a qoder wrapper pointing at `.qoder` with Qoder's own project-slug encoder.
- `src-tauri/src/commands/mcp.rs`: add Qoder to the MCP client enum, integration status, toggle command, and manual snippets. Read and write `~/.qoder/settings.json`; preserve unrelated JSON and mutate only `mcpServers.runner`.
- Tests: extend the both-runtimes loops at `:1414` and `:1436` to include qoder; add resume-plan self-assign/resume arms; add the permission matrix arm at `:960` asserting qoder yields the auto flag and nothing for the unprobed modes.

### Phase 2 — Rust spawn and output policy

- `session/manager/spawn.rs:1179-1197`: the `conversation_missing` and `effective_prior_key` matches must accept `"qoder"` alongside `"claude-code"`, using the qoder guard.
- `session/manager/spawn.rs:550, 936, 1421`: add `"qoder"` to the first-turn-not-delivered warning gate so the diagnostic stays honest.
- `session/manager/output.rs:770` `runtime_clears_on_resize`: add `Some("qoder")`.
- `session/manager/output.rs:798` `runtime_purges_on_resume`: add `Some("qoder")` to the `matches!` so it keeps its ring like claude-code (decision 5 — confirm in smoke test).
- Do **not** touch `enter_claude_launch_gate` or any `codex_capture` site.
- Tests in `session/manager/tests.rs`: mirror the claude-code ring-keep-on-resume coverage (`:2836-2922`) for qoder, and add a qoder case to the `resolve_runtime_override` matrix (`:4073`).

### Phase 3 — Frontend

- `src/components/ui/runtimes.ts`: **append** to `RUNTIME_OPTIONS` (`:19`) — `{ value: "qoder", label: "qoder", defaultCommand: "qodercli", description: "Qoder CLI" }`. Add `"qoder"` to `RUNTIMES_WITH_PERMISSION_MODE` (`:40`) and a `PERMISSION_MODES_BY_RUNTIME` entry (`:159`) with default + auto only. Add `MODE_MATCH_PAIRS_BY_RUNTIME` (`:214`) and `PERMISSION_STRIP_KEYS_BY_RUNTIME` (`:307`) entries mirroring Phase 1. Leave `EFFORT_OPTIONS_BY_RUNTIME` and `MODEL_SUGGESTIONS_BY_RUNTIME` without a qoder key (decision 4).
- `src/components/RunnerTerminal.tsx:168` `runtimeClearsOnResize`: add `"qoder"`, in lockstep with Phase 2 (Problem trap 2).
- `src/components/settings/AgentsPane.tsx:81`: replace the hardcoded `[0, 1]` skeleton with something that doesn't assume two runtimes (decision 6).
- `src/components/settings/McpPane.tsx`: add the Qoder registration toggle and manual-config copy action.
- `StartChatModal`, `RuntimeSelect`, `AddSlotModal`, and `CrewEditor` already read the backend catalog or `RUNTIME_OPTIONS`. Move the existing Direct-mode default-runtime control from the now-misplaced Settings → Chat pane into Settings → Agents, keep its stored preference wired to `StartChatModal`, remove the standalone Chat pane, and use the runtime display name alone for generated Direct-chat titles.
- Do **not** add qoder to the codex session-key polling in `MissionWorkspace.tsx:448` / `RunnerChat.tsx:527` — qoder's key is assigned at spawn, so there is nothing to poll for.

### Phase 4 — Docs and verification

- `docs/arch/arch.md`: runtime mentions at `:52`, `:82`, `:156`, `:404`, `:439`, `:684` — update the enumerations and record qoder's keep-ring/clear-on-resize policy alongside the others.
- `README.md`: the runtime enumerations in the tagline and feature copy.
- `src-tauri/src/mcp/tools/session.rs:15-21`: the `runtime` doc comment lists the valid names as examples — add qoder.
- Full check matrix: `cargo fmt --check`, `cargo clippy --workspace`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.

## Verification

Automated:

- [ ] `resume_plan("qoder", None)` self-assigns a UUID via `--session-id`; with a prior UUID it emits `--resume <uuid>`, `prepend: false`.
- [ ] `first_turn_argv("qoder", body)` returns the positional body; suppressed on resume.
- [ ] Permission round-trip: apply Auto → `--permission-mode auto`; switch to Default → flag stripped.
- [ ] Qoder project slugs replace every non-alphanumeric UTF-16 code unit and truncate over 200 units with the CLI's djb2/base36 suffix.
- [ ] `runtime_purges_on_resume("qoder")` is false; `runtime_clears_on_resize("qoder")` is true; the TS mirror agrees.
- [ ] Unknown-runtime slot-override rejection still lists qoder among valid names.
- [ ] Settings → Agents persists the Direct-mode default runtime, and Start Chat preselects it.
- [ ] A generated Direct-chat title is the runtime display name (`Codex`, `Claude Code`, or `Qoder`) without a `Chat with` prefix.
- [ ] Qoder MCP registration creates `~/.qoder/settings.json` when needed, preserves unrelated settings and servers, and removes only `mcpServers.runner` when disabled.
- [ ] Settings → MCP renders a Qoder toggle that calls `mcp_set_integration` with the `qoder` client id.

Manual (the human smoke-tests — these decide two of the key decisions):

- [ ] Direct chat: spawn, converse, stop, resume — conversation restored, no blank grid, **no doubled or missing scrollback** (confirms the keep-ring choice in decision 5).
- [ ] Resize a qoder pane mid-conversation — frame repaints cleanly, no shredded box-drawing in scrollback (confirms clear-on-resize).
- [ ] First-turn prompt auto-submits in interactive mode; if the positional doesn't submit, fall back to `-i/--prompt-interactive`.
- [ ] Mission slot: worker preamble lands; the inbox nudge submit chord (80ms Enter) lands in its input box.
- [ ] Start three or more qoder sessions at once — if any fail to authenticate, the claude-code launch gate may be needed after all (decision 8).
- [ ] Delete the session `.jsonl` and resume — falls back to a fresh spawn without an error loop.
- [ ] Settings → Agents shows a Qoder row with the detected `qodercli` path; the loading skeleton renders three rows, not two.
- [ ] Settings → MCP toggles Qoder registration; a new Qoder session sees Runner tools, while an existing session sees them after `/mcp reload`.
