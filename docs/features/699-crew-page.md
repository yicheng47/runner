# 699 — Crew page redesign

> Tracking issue: [#699](https://github.com/yicheng47/runner/issues/699)
> Priority: P1, 0.12 release blocker. Platforms: macOS and Windows.
> History: split from [393](./393-role-page.md) on 2026-09-22. The role page goes first, and this page follows in the same layout language. It is built against the roster model that [#562](./562-mission-spawn.md) settles.

## Motivation

The crew page is still the MVP draft (frame `CUKjM` in `design/runner-mvp-design.pen`). It was carried over as-is through the GPUI cutover and the #604 rename, and it buries its primary content under prompt prose:

- **Slots last**: the slot roster is the crew's actual substance, but it renders last (`crates/runner-app/src/surfaces/crews/editor.rs`). It sits below Purpose, and below Default goal and Team conventions (`editor_sections.rs`). With real prose in those sections the slots land below the fold.
- **Crowded slot rows**: each slot row (`slots.rs`) packs everything into one block: the handle, the LEAD badge, the runtime badge with its override, "from @role", a one-line prompt preview and the `$ command` summary.

## Scope

Redesign the page in Pencil first, in `design/specs/699-crew-page.pen`, then implement to match. The page shows the same data and runs the same commands, with no backend changes.

- **Slots on top**: the slots sit directly under the header.
- **Prose tucked away**: purpose, default goal and team conventions move to a secondary presentation, such as collapsed sections, a side column or a tab, collapsed by default when long.
- **Slot rows**: each row reads at a glance: handle, LEAD, role, and the effective runtime, model and effort, with overrides marked. The prompt and the command move behind a disclosure.
- **Settled model**: the page is implemented against #562's settled roster model, not the one before it.

## Non-goals

- New fields or slot operations.
- The crew list page.
- The create and add-slot forms (`crews/create.rs`, `crews/add_slot.rs`), beyond what the new layout needs for visual consistency.
- The role page ([393](./393-role-page.md)).

## Implementation phases

1. **Design**: a Pencil frame for the crew page, reviewed before any code. It starts after the role page design is approved.
2. **Crew page**: slots first and the restructured slot row. Drag-reorder, set lead, overrides and remove keep working.

## Verification

- `runner-app` tests pass, extended where the row restructure moves behavior (`surfaces/crews/tests.rs`); workspace clippy is clean.
- Manual pass with a crew of five or more slots, per-slot overrides, and long purpose, goal and conventions prose: the slots are visible without scrolling.
- On macOS and Windows, the page keeps its layout at the minimum window width.
