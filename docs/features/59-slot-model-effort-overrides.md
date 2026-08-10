# Per-slot model and effort overrides

Tracking issue: [#397](https://github.com/yicheng47/runner/issues/397). Status: planned.

## Motivation

Slot-level agent customization is incomplete and asymmetric. A runner template pins `model` and `effort`; a direct chat can override both at start (impl 0043 companion work); but a crew slot can only override the runtime, and the model chip is gated on that: `SlotRuntimeSelect` must be set before `SlotModelOverrideEditor` even renders (`CrewEditor.tsx`), and the backend enforces the same gate — `resolve_runtime_override` (`src-tauri/src/session/manager/mod.rs:1214`) early-returns when `runtime_override` is empty, and the mission spawn hardcodes `None` for effort (`src-tauri/src/session/manager/spawn.rs:248`). Net effect: to give one slot a different model you must first "override" the runtime (possibly to the same engine), and no slot can ever run at a different effort than its template. The natural mental model — a slot describes the complete agent configuration for that seat, with the runner as the default — breaks exactly at model/effort.

## Scope

Make model and effort independently overridable per slot, with the runner template as the fallback at every level.

- **Schema**: add `effort_override TEXT NULL` to `slots`; expose on `Slot`/`SlotWithRunner` (`src/lib/types.ts`), `slot_update`, and the `slot_update` MCP tool alongside `model_override`.
- **Resolution**: rework `resolve_runtime_override` so model/effort overrides apply without a runtime override — effective config = runner template, then runtime override (which resets model/effort to the new engine's defaults, as today), then model/effort overrides on top. `pinned` semantics unchanged: only a set `runtime_override` pins the session's engine; model/effort-only overrides don't pin.
- **Spawn**: mission spawn passes `slot.effort_override`; `model_effort_args` already emits per-runtime flags and ignores what a runtime doesn't support.
- **UI** (`CrewEditor.tsx` slot rows): model and effort chips always visible, no runtime-override gate. Model keeps `ModelField` with the effective runtime's catalog; effort is a small select (runtime default / the runtime's effort steps, mirroring the chat-level override control). Chips render dim when inheriting, accent when overridden — same pattern as the runtime chip. Clearing the runtime override keeps model/effort overrides only if they are valid for the runner's own runtime; on engine change, model resets (today's behavior) and effort resets with it.

## Non-Goals

- Per-slot system-prompt, env, args, or permission-mode overrides — the runner template stays the home for persona and engine config.
- Per-mission (as opposed to per-slot) agent overrides.
- Changing direct-chat override behavior or the runner editor.

## Implementation Phases

1. **Backend** — migration, repo/commands/MCP plumbing for `effort_override`, resolver rework + spawn wiring, rust tests (model-only override without runtime override; effort flows to spawn args; engine-change reset semantics; pinning unchanged).
2. **Frontend** — ungated model chip + new effort chip on slot rows, reset-on-engine-change behavior, vitest coverage (extend `RunnerRuntimeModelReset.test.tsx` patterns).
3. **Docs** — refresh feature 41's archived assumptions if referenced by arch docs.

## Verification

- `cargo test --workspace`: resolver and spawn-arg tests above.
- Vitest: chip visibility ungated, override reset on runtime change, dim/accent state mapping.
- Manual: crew with one runner template and two slots — slot A default, slot B model+effort overridden, no runtime override; mission spawn shows the flag difference in the PTY command lines; overriding runtime resets both; clearing runtime override restores template inheritance.
