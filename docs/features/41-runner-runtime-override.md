# 41 — Runner runtime override (per slot / per chat)

> Tracking issue: [#305](https://github.com/yicheng47/runner/issues/305)

## Motivation

A runner template bundles two different things: a **persona** (handle, display name, system prompt, working dir, env) and an **engine** (`runtime` + command + args + model + effort). The persona is runtime-agnostic — "@architect with this brief" reads the same whether claude-code or codex executes it. But because the runtime is baked into the runner row, running the same persona on two engines means duplicating the runner with only the runtime changed. This shows up acutely when driving Runner over MCP: templates multiply as prompt-identical, runtime-different copies.

Feature 25 (shipped) solved the inverse case — runtime without a persona (bare direct chats). This feature solves the remaining half: treat `runner.runtime` as a **default**, overridable at the point of use.

Terminology note: the concept is named `runtime` throughout the codebase (`Runner.runtime`, `RuntimeDefinition` registry in `router/runtime.rs`, `runtime_list`, `sessions.agent_runtime`) and in specs 25/30/37. "Agent" appears only in prose. This spec keeps that naming: runner = persona, runtime = engine.

## Scope

### In scope (v1)

- **Slot-level override.** New nullable `slots.runtime_override` column. The crew slot editor gets a runtime dropdown defaulting to "Runner default (<runtime display name>)". Mission spawns resolve the effective runtime as `slot.runtime_override ?? runner.runtime`.
- **Direct-chat override.** In the Start Chat modal's Runner mode, an optional runtime dropdown with the same "Runner default" sentinel. `session_start_direct` gains `runtime: Option<String>`.
- **Spawn resolution.** When an override is active, the spawn command comes from the runtime registry (and feature 37's executable settings once that lands), not `runner.command`. The session row records the effective runtime (existing `agent_runtime` path), so respawn/resume keeps working per session.
- **Roster display.** Wherever a slot or session shows its runner, show the effective runtime badge when it differs from the runner's default.
- **MCP parity.** `slot_create` / `slot_update` accept `runtime_override`; `session_start_direct` accepts `runtime`.

### Non-goals

- Mission-level runtime swapping. No runtime pickers in the Start Mission modal, ever — starting a mission stays zero-decision. The runtime choice is crew configuration (edit the slot), not a launch-time knob.

### Out of scope (deferred)

- Per-runtime profiles on a runner (distinct args/model per runtime). If overriding with dropped flags proves too lossy, revisit.
- Changing the runtime of a live session.

### Key decisions

1. **Override lives on the slot, not a new runner variant.** The whole point is to stop multiplying runner rows; the slot is already the per-crew identity layer (`slot_handle`), so the engine choice sits beside it.
2. **On override, runtime-specific fields reset to registry defaults.** `command`, `args`, `model`, and `effort` on the runner are flags for its default runtime (`claude-opus-*` means nothing to codex). When the effective runtime differs from `runner.runtime`, spawn uses the registry command with default args; persona fields (`system_prompt`, `working_dir`, `env`) carry over.
3. **Keep the name `runtime`.** No rename to "agent" — the data model, registry, commands, and prior specs all say runtime, and "agent" is overloaded with the runner/persona concept itself.

### To be decided

- Whether model/effort should optionally carry across runtimes when the effective runtime understands them (e.g. a neutral effort scale mapped per adapter), or stay reset-on-override permanently.
- Whether the sidebar/chat header shows the override badge always or only on mismatch.

## Implementation Phases

### Phase 1 — backend

- Migration: nullable `slots.runtime_override` (validated against the runtime registry on write).
- Effective-runtime resolution in `SessionManager::spawn` (mission path) and `session_start_direct_impl` (chat path), including the reset-to-registry-defaults rule.
- `slot_create` / `slot_update` / `session_start_direct` parameter plumbing, Tauri + MCP.

### Phase 2 — frontend

- Runtime dropdown in the crew slot editor with the "Runner default" sentinel.
- Optional runtime dropdown in Start Chat modal Runner mode.
- Effective-runtime badge on slot rows and chat headers on mismatch.

### Phase 3 — verification

- Mission with one runner in two slots, one slot overridden → each session spawns its own runtime, both resume correctly after app restart.
- Direct chat with an override → spawns override runtime with the runner's persona fields, registry-default flags.
- No override anywhere → byte-identical spawn behavior to today.

## Verification

- [ ] `slots.runtime_override` migration applies; existing slots unaffected.
- [ ] Slot editor and Start Chat modal offer the override with a working "Runner default" sentinel.
- [ ] Overridden spawns use registry command/args and drop runner `model`/`effort`; persona fields carry over.
- [ ] Session rows record the effective runtime; respawn/resume uses it.
- [ ] MCP `slot_create`/`slot_update`/`session_start_direct` accept the new params.
- [ ] Invalid runtime names rejected with a clear error.
- [ ] `tsc --noEmit` clean; `cargo fmt + clippy + test` clean.
