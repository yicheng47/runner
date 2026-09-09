# Mission-level permission mode, bypass by default

Tracking issue: [#527](https://github.com/yicheng47/runner/issues/527). Status: planned. Priority P1.

## Motivation

A permission prompt inside a mission slot is never a real control point. Nobody is watching that PTY, so the prompt is a silent stall: the lead waits on a handoff that never arrives, and the human sees nothing. The byte-flow `IdleDetector` (`crates/runner-backend/src/session/pty_runtime.rs`) reports the stuck slot as idle, which is what a finished slot looks like too. Spec [52](./52-hook-based-session-status.md) closed the hook-based status route, so the feed will not learn to show this.

Auto mode does not remove the stall. claude-code's `--permission-mode auto` runs a classifier that still stops for a human on some actions; on 2026-09-09 it denied a `git push` twice in one session. codex's `on-request` asks to escalate whenever the sandbox blocks a command. In a direct chat the human answers; in a mission slot nobody does.

Today permission mode is a per-runner property. `ops::runner::create` / `update` write the chosen mode onto the row's `args` at create time through `router::runtime::permission_mode_args` (`crates/runner-backend/src/router/runtime.rs:258`), default Auto (`ops::runner::default_permission_mode`). Missions inherit whatever each runner carries; a slot runtime override starts from the default mode (`session::manager::resolve_runtime_override`, `crates/runner-backend/src/session/manager/mod.rs:1328`). There is no mission-level knob. The Start Mission modal's Advanced disclosure (`crates/runner-app/src/surfaces/start_mission.rs:482`) still renders the MVP placeholder text.

The controls that actually protect a crew are elsewhere: worktree isolation ([403](./403-mission-worktree-isolation.md)), the coder/reviewer loop, and the no-commit / no-push rules in the crew prompts. A permission prompt adds nothing to those; it only removes the mission's ability to finish.

## Behavior

### Mission permission mode

Start Mission gains a **Permission mode** select inside the Advanced disclosure, replacing the placeholder text:

- **Bypass** (default for missions) — every slot spawns with the runtime's bypass flags.
- **Auto** — every slot spawns with the runtime's auto flags (today's effective behavior for a default runner).
- **Runner default** — slots spawn with whatever their runner row carries, exactly as today.

The choice is persisted on the mission row (`missions.permission_mode`) so resume, fork, and the auto-resume path honor it; a mission never changes mode after start. The mission metadata panel shows the effective mode next to the crew link.

### Per-slot override

Slots already carry `runtime_override`, `model_override`, and `effort_override` (`crates/runner-backend/src/model.rs:85`). Add `permission_override` with the same optional semantics: when set, it wins over the mission mode for that slot. The slot editor's override group gains the select. A reviewer slot that should stay read-only is the motivating case.

Resolution order at spawn: slot override → mission mode → runner row. "Runner default" at the mission level means the last step; the runner's own args are the floor, never stripped.

### Runtime mappings

Applied through the existing `router::runtime::apply_permission_mode` (strip the runtime's permission flags, append the canonical pair), so a runner row carrying its own flags converges to one shape.

- claude-code Bypass: `--permission-mode bypassPermissions` (unchanged).
- trae Bypass: `--permission-mode bypass_permissions` (unchanged).
- **codex Bypass at the mission level: `--ask-for-approval never --sandbox danger-full-access`.** Today's codex Bypass pair keeps `--sandbox workspace-write`. With `never`, codex does not ask to escalate out of the sandbox, so a command that needs the network or writes outside the tree (`cargo` fetching crates, `gh pr create`, `git push`) fails silently. The runner-level Bypass mapping is left as is for now; whether it should change too is a decision for the impl plan (it affects direct chats a human is watching).
- shell and unknown runtimes: no-op, as today.

### Direct chats

Unchanged. A direct chat has a human in front of it; the runner row's mode stays the contract.

## Non-goals

- Detecting a permission prompt from the terminal grid. Spec 52 and #455 closed that route; this spec removes the prompt instead of finding it.
- A global "bypass everything" default for runners. Runner-level modes stay per runner; only missions get the new default.
- Sandboxing beyond what the runtimes offer. Worktree isolation (403) is the blast-radius control and is not changed here.

## Design

`design/runner.pen` — Start Mission modal frame (`EzvqL`) gets the Advanced disclosure opened with the select; the slot editor frame gains the fourth override row. Pencil-first per the post-cutover rule; frames to be added before the impl plan.

## Implementation Phases

1. **Backend.** Migration `0021_mission_permission_mode.sql`: `missions.permission_mode TEXT NULL` and `slots.permission_override TEXT NULL`. `mission_start` takes an optional `permission_mode` (MCP tool schema too); slot spawn resolves slot override → mission mode → runner row and calls `apply_permission_mode`. Codex full-access mapping for the mission-level Bypass. Tests: resolution order, codex argv shape, resume carries the mode.
2. **UI.** Start Mission → Advanced: Permission mode select, Bypass preselected; slot editor override row; mission metadata panel line. Manager tests for the argv; a `VisualTestContext` test pins the disclosure at two rem sizes.
3. **Optional — permission-prompt signal.** For slots resolved to Auto or Runner default, inject a claude-code `Notification` hook with matcher `permission_prompt` through the `--settings` composer Runner already uses (`router::runtime::claude_settings_args`), posting a needs-you signal to the feed. codex 0.150+ has Claude-compatible hooks; verify before promising it there. Narrower than spec 52: fires only when a human is actually required.

## Verification

- Start a Peer Coding Crew mission with the default (Bypass) on the runner repo; have the coder run `git push` to a scratch branch and `gh pr view`. No prompt appears in either slot; the codex slot reaches the network.
- Same mission with Auto: the claude slot stops on the push, proving the mode is honored per mission.
- A slot override of Runner default on the reviewer slot, mission at Bypass: the reviewer spawns with the runner row's flags, the coder with bypass. Check the argv in the session's metadata.
- Quit and relaunch with auto-resume on: resumed slots carry the same flags.
- `make verify` green; MCP `mission_start` schema shows the new optional field.
