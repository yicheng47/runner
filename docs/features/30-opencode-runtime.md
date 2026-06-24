# 30 — OpenCode runtime

> Tracking issue: [#233](https://github.com/yicheng47/runner/issues/233)
> Priority: P2.

## Motivation

Runner currently treats `claude-code` and `codex` as first-class runtimes. OpenCode is another terminal-first AI coding agent with a TUI entrypoint (`opencode`), provider/model flexibility, and native session commands, so supporting it directly would make Runner less tied to the two current agent CLIs and let users put OpenCode into direct chats, runner templates, and crews without hand-maintained custom command rows.

This should be a real runtime adapter, not just a new string in a dropdown. OpenCode's CLI shape is close enough to fit Runner's existing runtime registry, model override, first-turn delivery, and session resume concepts, but its permission model and session ids do not map exactly to the existing Codex / Claude Code behavior.

## Scope

### In scope

- **Runtime catalog entry.** Add `opencode` to the frontend runtime catalog and backend runtime definitions with default command `opencode` and display name `OpenCode`.
- **Direct chat runtime picker.** Make OpenCode selectable in the Start Chat runtime path. The command should spawn the interactive TUI, not `opencode run`, because Runner needs a live PTY the user can keep typing into.
- **Runner templates.** Make OpenCode selectable in create/edit runner flows, with the default command prefilled as `opencode`.
- **Model override.** Map Runner's `model` field to OpenCode's `--model <provider/model>` flag. Keep model suggestions minimal (`default` only) because OpenCode's documented format is provider/model and the provider catalog is user-config dependent.
- **First-turn delivery.** Use OpenCode's documented TUI `--prompt <text>` flag for the composed first user turn. Do not rely on the positional prompt path used by Claude Code / Codex unless local validation shows `--prompt` cannot handle Runner's multi-line launch payloads.
- **Working directory.** Keep using Runner's PTY cwd as the project directory. Only pass OpenCode's optional `[project]` positional if local testing proves the TUI ignores the process cwd.
- **No effort dropdown in v1.** OpenCode has model/provider and run-mode variant/thinking concepts, but the interactive TUI does not expose the same stable reasoning-effort enum as the current Codex / Claude Code adapters. Store no effort-specific args for OpenCode until there is a confirmed TUI mapping.
- **No permission-mode dropdown in v1.** OpenCode permissions are config/env based (`permission` rules / `OPENCODE_PERMISSION`) rather than the simple CLI flags Runner currently owns for Codex / Claude Code. Do not add `opencode` to `RUNTIMES_WITH_PERMISSION_MODE` until a safe, narrow mapping exists.
- **Mission smoke support.** OpenCode runner templates should be usable in crews. If mission bus commands hit OpenCode's `external_directory` permission guard because Runner's event log lives outside cwd, solve it with the narrowest documented config/env override for the mission directory, not a global bypass.
- **Session key display behavior.** OpenCode sessions without an exact captured native id should show `session_key = NULL` and behave as non-resumable, matching the current UI semantics.

### Out of scope

- Installing OpenCode, detecting whether it is installed, or hiding the runtime when it is missing.
- OpenCode desktop or IDE integration.
- Editing OpenCode provider/config files inside Runner.
- Curated provider/model catalogs.
- Mapping Runner's permission modes to OpenCode before the config/env behavior is locally validated.
- Native OpenCode resume unless Runner can capture the exact OpenCode session id for the spawned PTY.

## Implementation Phases

### Phase 1 — runtime registry and adapter

- Add `opencode` to `src/components/ui/runtimes.ts` and `src-tauri/src/router/runtime.rs`.
- Add OpenCode unit coverage for `runtime_definitions`, `runtime_display_name`, model args, unsupported effort, unsupported permission mode, and unsupported permission inference.
- Extend `model_effort_args` so OpenCode emits only `--model <value>` when `model` is set and ignores `effort`.
- Extend first-turn handling so OpenCode emits `--prompt <body>` for fresh spawns and suppresses it on resume, following the same replay-avoidance rule as existing runtimes.
- Keep `system_prompt_args` empty for OpenCode; Runner's persona / launch text should ride the first-turn path.

### Phase 2 — direct chat and runner-template UI

- Surface OpenCode in the Start Chat runtime dropdown with display name `OpenCode`.
- Surface OpenCode in runner create/edit runtime selectors.
- Ensure OpenCode rows hide the permission-mode control and effort control, while keeping the free-text model input available.
- Verify the chat metadata panel and sidebar display runtime identity and `session_key = NULL` correctly for OpenCode direct chats.

### Phase 3 — mission behavior

- Start a mission with a crew that contains an OpenCode runner and verify the PTY paints correctly, accepts human input, and receives the launch prompt once.
- Verify the bundled `runner` CLI is on PATH for OpenCode mission sessions, matching the existing mission behavior for Codex / Claude Code.
- Test `runner msg read`, `runner msg post`, and `runner signal ask_human` from inside an OpenCode session. If OpenCode asks about the event-log directory, add a narrowly scoped mission-dir permission override through documented OpenCode config/env support.
- Do not use OpenCode's dangerous skip-permissions path for normal mission support.

### Phase 4 — native resume

- Investigate where OpenCode records the exact session id for the spawned TUI session. Candidate sources are OpenCode's `session` CLI commands and local session storage, but implementation should be based on an exact id captured for the current PTY, not "last session" heuristics.
- Persist the captured id in `sessions.agent_session_key` only when the match is exact.
- Add `resume_plan("opencode", Some(key))` using OpenCode's `--session <id>` flag.
- Avoid `--continue` for Runner-managed resumes because it is ambiguous when multiple sessions exist in the same cwd.
- If capture is not reliable, keep OpenCode non-resumable rather than presenting a broken Resume affordance.

### Phase 5 — docs and validation

- Update architecture docs where they enumerate first-class runtimes or runtime-specific prompt/resume behavior.
- Add implementation notes if OpenCode requires a mission-dir permission override.
- Keep tests focused on adapter behavior and one end-to-end manual smoke path; do not build a generic custom-runtime system as part of this feature.

## Verification

- [ ] OpenCode appears in `runtime_list` with command `opencode`.
- [ ] Start Chat runtime mode can spawn an OpenCode direct chat in a chosen cwd.
- [ ] Creating and editing a runner can select OpenCode and prefill command `opencode`.
- [ ] OpenCode runner rows show the model input but do not show permission-mode or effort controls.
- [ ] A direct chat with an OpenCode runner delivers the runner persona / first turn through `--prompt`.
- [ ] Setting `model` on an OpenCode runner emits `--model <provider/model>`.
- [ ] Setting `effort` on an OpenCode runner emits no effort args.
- [ ] Permission mode selection is not available for OpenCode, and backend permission helpers emit no OpenCode args.
- [ ] A mission with an OpenCode slot starts, paints the TUI, and receives the launch prompt once.
- [ ] Mission bus commands work from OpenCode, or a narrow mission-dir permission override is documented and tested.
- [ ] Resume is hidden or disabled while `agent_session_key` is NULL.
- [ ] If native resume is implemented, `resume_plan("opencode", key)` uses `--session <key>` and never `--continue`.
- [ ] `pnpm exec tsc --noEmit` passes.
- [ ] `pnpm run lint` passes.
- [ ] `cargo test --workspace` passes, or narrower runtime/session tests pass with the skipped scope documented.

## References

- OpenCode homepage: <https://opencode.ai/>
- OpenCode docs: <https://opencode.ai/docs>
- OpenCode CLI docs: <https://opencode.ai/docs/cli/>
- OpenCode config docs: <https://opencode.ai/docs/config/>
- OpenCode permission docs: <https://opencode.ai/docs/permissions/>
