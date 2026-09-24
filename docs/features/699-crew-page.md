# 699 — Crew page redesign

> Tracking issue: [#699](https://github.com/yicheng47/runner/issues/699)
> Priority: P1, 0.12 release blocker. Platforms: macOS and Windows.
> History: split from [393](./393-role-page.md) on 2026-09-22. The role page goes first, and this page follows in the same layout language. It is built against the roster model that [#562](./562-mission-spawn.md) settles.

## Motivation

The crew page is still the MVP draft (frame `CUKjM` in `design/runner-mvp-design.pen`). It was carried over as-is through the GPUI cutover and the #604 rename, and it buries its primary content under prompt prose:

- **Slots last**: the slot roster is the crew's actual substance, but it renders last (`crates/runner-app/src/surfaces/crews/editor.rs`). It sits below Purpose, and below Default goal and Team conventions (`editor_sections.rs`). With real prose in those sections the slots land below the fold.
- **Crowded slot rows**: each slot row (`slots.rs`) packs everything into one block: the handle, the LEAD badge, the runtime badge with its override, "from @role", a one-line prompt preview and the `$ command` summary.

## Scope

Redesign the page in Pencil first, in `design/specs/393-role-page.pen` beside the role page, then implement to match. The page shows the same data and runs the same commands, except for the dropped fields below; the only backend change is removing the goal fallback.

- **Profile split, like the role page**: the left column holds the crew's picture (its first four slots' avatars), name, a one-line summary (slot count and lead), Start mission and Edit, then the slots and the details. Team conventions fill the right column, with the crew's recent missions under them, taken from the mission summaries the app already loads.
- **Conventions are the crew's only prose**: purpose, default goal and team conventions were three prose fields doing overlapping jobs, and conventions alone are enough (Jason, 2026-09-24). Team conventions sit beside the slots, collapsed when long, the way the role page shows its system prompt. Edit turns the name into a field and the conventions into an editor; slot changes keep saving on their own.
- **Purpose and default goal dropped from the app**: purpose never reached an agent; it appeared on this page, the crew list card, the create form and crew search, which stop showing and asking for it. The default goal pre-filled the Start mission dialog and was the lead's goal when a mission started without one; every mission now states its own goal, and a repeatable job's goal belongs to the job (#630), not the crew. The dialog stops pre-filling it, and `ops::mission` stops falling back to `crew.goal` when a mission starts or resumes without a goal, so a goal nobody can see never reaches the lead. The database columns and the CLI's `--purpose` and `--goal` on `crew create` and `crew update` stay for now.
- **Slot rows**: each row reads at a glance: the slot's pixel avatar (`RoleAvatar` seeded with the slot handle, as the mission rail and feed draw it), handle, LEAD, role, and the effective runtime, model and effort, with overrides marked. Clicking a slot opens a popup beside it with its setup against the role's defaults, the command, a prompt preview, and Edit overrides, Set as lead, Open role and Remove. Edit overrides edits the slot's runtime, model and effort in the popup and saves to the slot only; the role is edited on its own page. This replaces the slot menu's Edit role drawer.
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
- Manual pass with a crew of five or more slots, per-slot overrides, and long conventions prose: the slots are visible without scrolling.
- On macOS and Windows, the page keeps its layout at the minimum window width.
