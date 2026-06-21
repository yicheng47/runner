# 23 — Drag-to-reorder chats and missions

> Tracking issue: [#192](https://github.com/yicheng47/runner/issues/192)

## Motivation

Today the sidebar's mission and chat lists are sorted automatically:
pinned items float to the top by `pinned_at` desc, and everything else
sorts by `started_at` desc (mission) / `started_at` asc (chat). The
user has no way to express *their* sense of priority beyond the binary
pin/unpin flip.

That hurts the workflows where two or three items matter for a session
that lasts days:

- A user with three pinned missions can't say "the
  log-pipeline-fix is the top one for this week" — pin order is
  whichever they pinned last.
- A user juggling five direct chats can't keep `@architect` above
  `@runtime-fixes` because chats sort by start time.
- Spec 22 (collapsed rail) shows the top N pinned items as slots; pin
  order = sidebar order, so the rail inherits the same problem.

The fix is **manual reorder via drag-and-drop**, the same gesture
users learn from Linear, Notion, and the macOS Finder sidebar. Pinning
stays a separate concept (a section boundary); within each section,
the user decides.

This is also the explicit follow-up that spec 22 ("Collapsed rail
mission + chat switcher") deferred — see that spec's "Out of scope"
list, item 1.

## Scope

### In scope (v1)

- **Drag a mission row** (expanded sidebar's MISSIONS list) to a new
  position. The row follows the cursor with a translucent ghost
  preview, and a drop indicator (a 2px accent line) shows where the
  row will land between siblings.
- **Drag a chat row** (expanded sidebar's DIRECT CHATS list) with
  the same gesture and visual.
- **Cross-section drag = pin/unpin.** Dragging an unpinned row above
  the pinned/unpinned boundary pins it; dragging a pinned row below
  the boundary unpins it. The boundary is a visible 1px separator
  with a "Pinned" label above it (added in this spec).
- **Persistence.** New `sort_index` REAL column on `sessions` and
  `missions`. NULL means "no manual order — fall back to legacy
  ordering". Saved on drop via a single Tauri command per row moved.
- **Fractional indexing.** Insertions use the midpoint between the
  two neighbors (e.g. drop between `1.0` and `2.0` → `1.5`). Avoids
  cascade renumbering on every drop and keeps the write small.
  Periodic background rebalance is **out of scope** for v1; we'll
  add it when fractional collisions get within ε of `Number.EPSILON`,
  which is unreachable in practice for hand-reorderable lists.
- **Keyboard reorder.** `Cmd+Shift+↑` / `Cmd+Shift+↓` on a focused
  row moves it up/down within its section. Same persistence path.
- **Touch + pointer.** Mouse, trackpad, and touch all work; touch
  uses a 200ms long-press to start dragging (so a tap-to-open
  isn't accidentally a drag).

### Out of scope (deferred)

- **Reorder on the collapsed rail.** The rail (spec 22) renders the
  top 4 pinned items per type; reordering still happens in the
  expanded sidebar, and the rail picks up the new order on next
  render. Drag-on-rail is a follow-up.
- **Cross-list drag (chat → mission list or vice versa).** They're
  different entities with different schemas; no semantic mapping.
- **Cross-crew mission drag.** Missions are scoped to a crew in the
  sidebar; v1 reorders within the current crew's mission list only.
- **Drag-to-folder.** Folders land in spec 17; folder drop targets
  are flagged as a follow-up there.
- **Background rebalancing of fractional indices.** Theoretical
  problem only; ship without it and add a `rebalance_sort_indices`
  command if the field ever runs into precision issues.
- **Undo for reorder.** Dragging is reversible by dragging back;
  no separate undo stack in v1.

### Key decisions

1. **`@dnd-kit/sortable` over hand-rolled HTML5 drag.** Hand-rolled
   HTML5 drag is fine for trivial reorder, but loses on
   accessibility (keyboard nav, screen reader announcements) and
   autoscroll (long lists need to scroll while dragging near the
   edge). `@dnd-kit/sortable` is ~25KB gz, the de-facto React
   choice, and ships with the keyboard/touch sensors and autoscroll
   we'd otherwise hand-roll. Trade: one more dep. Worth it.
2. **Cross-section drag = pin/unpin, not "separate pinned-area
   reorder only."** The alternative (only reorder within pinned, or
   only within unpinned, never across) requires a separate UI for
   pin/unpin. The unified drag is fewer affordances and matches the
   muscle memory from every other reorderable list.
3. **Fractional indexing, not integer + cascade renumber.** Integer
   indexing forces an `UPDATE` per row in the affected range on
   every drop — fine for 10 items, miserable for 100. Fractional
   indexing writes one row per drop. The "indices collide" worry
   is theoretical for hand-reorderable lists; we'll add rebalancing
   if it ever fires in practice.
4. **REAL column, NULL default, no backfill.** Existing rows keep
   their legacy ordering until first drag. NULL `sort_index` sorts
   below all assigned values (so a manually-ordered list stays
   stable as new items arrive), which we do via
   `ORDER BY sort_index IS NULL, sort_index ASC, started_at DESC`.
   Avoids a migration backfill and the "first user to upgrade sees
   their list reshuffled" failure mode.
5. **Reorder respects pin boundary; pin overrides reorder.** Within
   the pinned section, items sort by `sort_index ASC`. Within
   unpinned, same. The boundary is hard — a pinned item with
   `sort_index = 99` still sits above an unpinned item with
   `sort_index = 1`. Pinning is the dominant concept; reorder is
   the within-group refinement.
6. **Keyboard reorder uses `Cmd+Shift+↑/↓`.** Matches macOS
   conventions (Finder sidebar, Mail mailboxes). Discoverable via
   the row's right-click menu, which will gain "Move up" / "Move
   down" entries that show the shortcut.

## Implementation phases

### Phase 1 — schema + backend

- Migration `0007_sort_index.sql`:
  ```sql
  ALTER TABLE sessions ADD COLUMN sort_index REAL;
  ALTER TABLE missions ADD COLUMN sort_index REAL;
  CREATE INDEX ix_sessions_sort_index ON sessions(sort_index);
  CREATE INDEX ix_missions_sort_index ON missions(sort_index);
  ```
- Update `ORDER BY` in `src-tauri/src/commands/session.rs:302` and
  `src-tauri/src/commands/mission.rs:128` to:
  ```sql
  ORDER BY <pinned-clause>,
           sort_index IS NULL, sort_index ASC,
           started_at DESC
  ```
- New Tauri commands:
  - `session_set_sort_index(session_id, sort_index)` —
    `UPDATE sessions SET sort_index = ?, updated_at = NOW() WHERE id = ?`.
  - `mission_set_sort_index(mission_id, sort_index)` — same
    shape on missions.
- Backend tests in `commands/session.rs` and `commands/mission.rs`
  for the new sort order, including the NULL-fallback case.

### Phase 2 — frontend drag

- Add `@dnd-kit/core` + `@dnd-kit/sortable` to `package.json`.
- Wrap the MISSIONS and DIRECT CHATS lists in `Sidebar.tsx` with
  `DndContext` + `SortableContext`. Two contexts (one per list) —
  cross-list drag is out of scope.
- Per-row: replace each row's outer wrapper with `useSortable` so
  drag is initiated by the row itself, with the existing
  context-menu / pin / archive affordances continuing to work
  unchanged on the row body.
- Drop handler:
  - Compute the new `sort_index` as the midpoint of the two
    neighbors (or `prev + 1` if dropped at the end, `next - 1` if
    at the start).
  - Detect cross-section drops: if the dragged row crossed the
    pinned/unpinned boundary, call the existing
    `api.session.pin` / `api.mission.pin` first, then the new
    `set_sort_index`.
  - Persist via the new command. Optimistic UI: update the local
    list immediately, roll back on error with a toast.
- Visual: 2px accent drop indicator (`bg-accent`), translucent
  ghost (CSS `opacity: 0.4`, `transform: scale(0.98)`), no
  animation delay on the snap (matches Linear's "instant" feel).

### Phase 3 — keyboard + a11y

- Wire `@dnd-kit`'s `KeyboardSensor` with default activation
  (space-to-pick-up, arrows-to-move, space-to-drop, esc-to-cancel).
- Add Runner-specific shortcut: `Cmd+Shift+↑` / `Cmd+Shift+↓` on
  a focused row moves it one slot within its section without
  entering pick-up mode. Bound in `Sidebar.tsx`'s keyboard
  handler; calls the same drop handler with a synthesized
  position.
- Update `SessionContextMenu` and `MissionContextMenu` (after
  #181's unification) to expose "Move up" and "Move down" entries
  showing the shortcut. Disabled at the section's top/bottom.
- Screen reader announcements: `@dnd-kit` ships
  `accessibility.announcements` — provide localized strings
  ("Picked up <name>. Use arrow keys to move. Press space to
  drop.").

### Phase 4 — verification

- **Functional smoke:**
  1. Drag a mission from position 3 to position 1 → reflected
     immediately; refresh page → still in position 1.
  2. Drag an unpinned chat above the pinned boundary → row pins +
     reorders in one drop; right-click → "Unpin" is shown
     (confirming the pin took).
  3. Drag a pinned mission below the boundary → unpins + reorders
     into the unpinned section.
  4. With 50 missions, drag from the bottom to the top → smooth
     autoscroll while dragging; final position correct; only one
     `UPDATE` issued (verified via SQL log).
  5. Keyboard: focus a row, `Cmd+Shift+↓` → moves down one slot;
     repeat 10× → reaches the bottom and stops (no wraparound).
  6. Keyboard pick-up: focus a row, Space → pick up; arrow down 3 →
     ghost moves; Space → drops at position; final position correct.
  7. Touch: long-press a row on a touch-capable device → drag
     begins; tap (without long-press) → row opens normally.
- **Cross-spec compatibility:**
  - Spec 22 (collapsed rail): rail picks up the new order on next
    render — the rail filters by `pinned_at` and slices to N; a
    manual reorder of pinned items reorders the rail too.
  - Spec 17 (sidebar folders): when folders land, drag-into-folder
    is the follow-up; v1 ignores folder drop targets.
  - Spec 12 (multi-window): reorder writes propagate via the
    existing `sidebar_state` channel to other windows in the
    same way pin already does.
- **Lint / type:** `pnpm exec tsc --noEmit` clean,
  `cargo fmt && cargo clippy --all-targets --all-features` clean,
  `cargo test` for the new ORDER BY behavior.

## Verification

- [ ] `sort_index REAL` column added to `sessions` and `missions`
      with indexes; existing rows have NULL and sort as before.
- [ ] `session_set_sort_index` and `mission_set_sort_index` Tauri
      commands return success and persist the value.
- [ ] Drag with mouse, trackpad, and touch reorders rows in the
      MISSIONS and DIRECT CHATS lists.
- [ ] Cross-pinned-boundary drag pins or unpins the row as a side
      effect, in a single user gesture.
- [ ] Drop indicator (2px accent line) renders between rows during
      drag; ghost row follows the cursor.
- [ ] Autoscroll fires when the cursor is within ~40px of the
      list's top or bottom edge during drag.
- [ ] Keyboard pick-up (Space → arrows → Space) works on focused
      rows; Esc cancels.
- [ ] `Cmd+Shift+↑/↓` shortcut moves a focused row one slot
      within its section; disabled at the top/bottom boundary.
- [ ] Context menu shows "Move up" / "Move down" with the
      shortcut hint, disabled at boundaries.
- [ ] Screen reader announces pick-up, move, drop, cancel.
- [ ] Cross-spec: collapsed rail (spec 22) reflects the new
      pinned order on next render.
- [ ] `pnpm exec tsc --noEmit` clean; `cargo fmt + clippy + test`
      clean.
