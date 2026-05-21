# 22 — Collapsed rail mission + chat switcher

> Tracking issue: [#184](https://github.com/yicheng47/runner/issues/184)

## Motivation

The collapsed sidebar rail (52px) is the default state for users who
want maximum screen real estate for the mission workspace. Today the
rail surfaces five chrome affordances — Search · Runners · Crews ·
Expand · Settings — but **nothing** for the user's actual work: there
is no way to see which missions are active, which chats are open, or
to jump between them without expanding the sidebar first.

That hurts the high-velocity workflow the rail was designed for. A
power user with three active missions and two long-running chats has
to expand → click → collapse on every context switch. The collapse
toggle becomes a friction point instead of a layout pick.

The fix is to push the missions/chats list partially onto the rail
itself, in a form that fits its 52px width. Two interaction patterns
already live in users' muscle memory from VS Code and Discord:

- **Pinned items as stacked dots** — small per-item slots that
  surface a fixed number of high-priority subjects at a glance,
  letting the user one-click into them.
- **Overflow popover** — a "More…" button that anchors a wider
  flyout listing everything that didn't make it onto the rail.

We adopt the hybrid: **pinned slot + overflow popover**. The pinned
slot is small enough to never crowd the rail; the overflow popover
holds everything else, with the same context-menu + pin/rename/archive
affordances the expanded sidebar already exposes.

## Scope

### In scope (v1)

- **Rail layout.** Three new sections inserted between the existing
  Crews row and the bottom Expand/Settings cluster:
  1. **Pinned missions** — vertical stack of icon-button slots, one
     per pinned mission. Each slot is 36×36; renders the mission's
     accent (status pill color) as a 6×6 dot at the bottom-right
     corner of a Lucide `target` icon. Hover-tooltip shows the
     mission title + cwd hint.
  2. **Pinned chats** — same shape, one slot per pinned direct chat.
     Lucide `message-square` icon; tooltip shows `@<handle>` +
     started_at.
  3. **Overflow button.** A 36×36 `more-horizontal` button that
     anchors a flyout popover listing everything not pinned. The
     popover layout matches the expanded sidebar's mission/chat
     rows (label + meta + context-menu trigger).
- **Pinned cap.** The rail shows at most **4 pinned missions** and
  **4 pinned chats** at any time (8 slots total, ~288px of vertical
  rail real estate). Beyond the cap, the most-recently-pinned win
  and the spillover lives in the overflow popover.
- **Active row indicator.** When the current route's mission/chat
  matches a rail slot, the slot gets a left-edge accent bar (same
  treatment the expanded sidebar uses for the active row).
- **Status dot.** Each mission slot's dot tracks the mission's
  status (`running`, `paused`, `idle`, `errored`) via the same color
  mapping the expanded sidebar uses. Chat slots get a single neutral
  dot — chats don't carry the same status surface.
- **Overflow popover.**
  - Anchored to the right edge of the rail, opens on click (not
    hover — hover would dismiss every time the user moves toward
    a row).
  - Width: 280px. Same chrome as the expanded sidebar lists
    (sections "MISSION" + "CHAT" with their counts, status pills,
    context-menu trigger per row).
  - Closes on outside-click + Escape, and on row activation.
- **Pinning UX.** The existing right-click `Pin` / `Unpin` action
  on a mission or chat row (in both the expanded sidebar and the
  overflow popover) is what promotes/demotes items into the rail.
  No new dedicated UI — the rail is just a *consequence* of the
  pinned set.
- **Expanded-sidebar parity.** The rail surfaces don't replace
  anything in the expanded sidebar — the user who expands the
  sidebar gets the same lists they have today. The rail is purely
  additive.

### Out of scope (deferred)

- **Drag-and-drop reordering** on the rail. Pin order = pinned-at
  timestamp, descending. Manual reorder is a follow-up.
- **Drag-to-rail to pin.** No new drop target — pinning goes through
  the existing right-click menu.
- **Unpinned items in the rail.** Only pinned items get a dedicated
  slot. Avoids a "5+ missions running" rail blowout.
- **Custom rail width / pinned cap.** Caps are hard-coded for v1;
  user-configurable cap is a settings follow-up.
- **Folder grouping on the rail.** When spec 17 (sidebar folders)
  lands, the rail will need a follow-up to expose folders too;
  out of scope here.

### Key decisions

1. **Hybrid (pinned + overflow), not pure-flyout.** The pure-flyout
   pattern (two icon buttons → popover lists) keeps the rail
   compact but loses the at-a-glance "what am I working on right now"
   surface. The hybrid trades a small amount of vertical rail
   real estate for that surface, which is the rail's whole point.
2. **Pin = promote, not Pin = mark-as-favorite.** Reuse the existing
   `pinned_at` column on missions + the `pinned` boolean on
   sessions. We don't introduce a separate "rail slot" concept;
   the rail is a *view* of the pinned set with a cap.
3. **Click-to-open overflow, not hover.** Hover popovers are
   fiddly with the 52px target — moving the cursor into the
   popover dismisses if the path leaves the trigger. Click is
   reliable and matches the rail's other affordances (Search,
   Settings).
4. **Cap at 4 per type.** Sized for the typical power-user case
   (2–3 active missions, 3–4 long-running chats). Above the cap,
   the overflow popover is one click away; we'd rather have a
   useful at-a-glance surface than a giant scrollable rail.
5. **Status dot on missions, neutral on chats.** Mission status is
   the most useful at-a-glance signal — "is `log-pipeline-fix` still
   running?" is the question the rail should answer. Chat dots
   don't carry that meaning today; promoting them to a status
   surface would invent new state.

## Implementation phases

### Phase 1 — pinned slot rendering

- Extend `Sidebar.tsx` collapsed-rail branch (currently lines
  ~556–595): insert the pinned-missions stack, pinned-chats stack,
  and overflow button between the existing Crews row and the
  bottom Expand/Settings cluster.
- New rail components:
  - `RailMissionSlot({ mission, active, onClick, onContextMenu })`
    — 36×36 icon button with status dot, left-edge accent bar
    when `active`, title-attribute tooltip.
  - `RailChatSlot({ session, active, onClick, onContextMenu })` —
    same shape, neutral dot.
  - `RailOverflowButton({ count, onClick })` — `more-horizontal`
    icon, badge showing overflow count when > 0.
- Filter `missions` and `directSessions` by `pinned_at` /
  `pinned`, slice to 4 each, sort by pinned_at desc.
- Right-click on a rail slot opens the existing
  `MissionContextMenu` / `RowContextMenu` (after #181's merge) at
  the cursor position — same flow as the expanded sidebar's
  right-click.

### Phase 2 — overflow popover

- New `RailOverflowPopover` component, anchored to the right edge
  of the rail's overflow button. Width 280px, max-height
  `100vh - 100px`, scroll inside.
- Renders two stacked sections: "MISSION" and "CHAT", each with
  the section's count and the same row layout as the expanded
  sidebar (label, meta, context-menu trigger). Reuses
  `RuntimeRow` / `SessionRow` directly.
- Outside-click and Escape close. Row activation navigates +
  closes.

### Phase 3 — active-route highlight

- Pull `currentMissionId` and `currentChatSessionId` (already
  threaded through the Sidebar in the expanded view) into the
  rail's slot components so the left-edge accent bar lights up
  for the active row.
- Verify the highlight survives mission/chat status changes (a
  mission that flips from running → paused should keep the
  active highlight, just change its status dot).

### Phase 4 — verification

- **Visual smoke:**
  1. Pin 1 mission + 2 chats → rail shows the slots in order;
     overflow button hidden (no overflow).
  2. Pin 5 missions → rail shows top 4 by pinned_at desc; overflow
     button shows badge "1".
  3. Click overflow → popover opens listing the 5th mission + all
     unpinned items, grouped by type.
  4. Click a rail slot → app navigates to the mission/chat;
     active highlight moves to that slot.
  5. Right-click a rail slot → context menu (`Pin/Unpin`,
     `Rename`, `Archive`) opens; Unpin removes the slot from the
     rail and pushes the next-most-recent pinned item up.
  6. Collapse → expand → collapse: rail state survives, pinned
     order stable.
- **Cross-spec compatibility:**
  - Spec 12 (multi-window): new windows pick up the same pinned
    set via the existing `sidebar_state` channel; rail renders
    identically in spawned windows.
  - Spec 17 (sidebar folders): when folders land, the overflow
    popover will need to mirror their grouping; flagged as a
    follow-up in spec 17's scope.
- **No backend changes.** Pinning already exists; this is a pure
  frontend rearrangement.

## Verification

- [ ] Pinned missions render as 36×36 slots in the collapsed
      rail, capped at 4.
- [ ] Pinned chats render as 36×36 slots in the collapsed rail,
      capped at 4.
- [ ] Overflow button appears with a badge when total pinned
      exceeds the cap or any unpinned items exist.
- [ ] Overflow popover anchors to the rail, lists unpinned +
      overflow items grouped by MISSION / CHAT, closes on
      outside-click + Escape + row activation.
- [ ] Mission status dot tracks `running` / `paused` / `idle` /
      `errored` colors.
- [ ] Active-route highlight (left-edge accent bar) lights up
      the matching rail slot.
- [ ] Right-click on a rail slot opens the row context menu and
      Pin/Unpin/Rename/Archive flow through correctly.
- [ ] Pinned order = `pinned_at` desc, stable across
      collapse/expand toggles.
- [ ] `pnpm exec tsc --noEmit` clean; no backend changes.
