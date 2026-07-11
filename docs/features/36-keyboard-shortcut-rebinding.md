# 36 — Keyboard shortcut rebinding

Tracking: [#273](https://github.com/yicheng47/runner/issues/273)

## Motivation

Impl 0025 delivered the Settings → Keyboard shortcuts pane read-only (#257 v1): the static registry in `src/lib/keymap.ts` documents Runner's bindings, but every handler keeps its hardcoded keys and users can see — not change — them. The end-state interactions are already designed in `design/runner-setting.pen` (the `mOfwl` row states plus annotation note `IHwvB`): hover-reveal edit/delete per row, chip-click → "Press keys…" recording state as a neutral inset well, unbind → "Unassigned", implied restore-defaults. This feature makes those interactions real.

## Scope

- Hover-reveal edit (pencil) and delete (trash) affordances on each shortcut row.
- Edit / chip-click enters a "Press keys…" recording state that captures the next combo.
- Delete unbinds the entry ("Unassigned" chip); a restore-defaults affordance brings stock bindings back.
- Conflict detection against the effective keymap before a new binding commits.
- Persistence: overrides stored in `localStorage` keyed by registry entry id; effective keys = defaults + overrides.
- Handler indirection: window keydown listeners and RunnerTerminal's xterm custom-key bridge match events against the registry's effective bindings instead of their current hardcoded keys — this removes the drift risk impl 0025 accepted.

Out of scope: ⌘N (owned by the OS menu accelerator; native menu rebinding is a separate problem) and multi-chord bindings.

## Implementation Phases

1. **Override store** — `keymap.ts` gains a read/write override layer and effective-binding resolution; vitest covers merge, unbind, restore, and conflict queries.
2. **Matcher indirection** — one shared "does this KeyboardEvent match entry X" helper; migrate every hardcoded handler onto it (AppShell, Sidebar, App, RunnerChat, MissionWorkspace, and the RunnerTerminal bridge), deleting the per-handler key literals and their keymap pointer comments.
3. **Pane interactions** — hover icons, recording state, unbind/restore, and conflict UI per the designed row states.

## Verification

- Vitest: override store round-trips, effective-key resolution, conflict detection.
- Manual: rebind a shortcut and confirm the new combo fires (including with terminal focus, via the xterm bridge); attempt a conflicting bind and confirm the guard; unbind and restore defaults; overrides survive relaunch.
