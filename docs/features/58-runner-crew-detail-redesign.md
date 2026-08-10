# Runner & crew detail redesign

Tracking issue: [#393](https://github.com/yicheng47/runner/issues/393). Status: planned.

## Motivation

Both detail pages are MVP drafts implemented straight from the historical MVP canvas (`design/runner-mvp-design.pen`, frames `ocAFJ` and `CUKjM`) and both bury their primary content under prompt prose.

**Crew detail** (`src/pages/CrewEditor.tsx`, `/crews/:crewId`): the slot roster — who is actually in this crew — renders last, below the Purpose, Default goal, and Team conventions sections. With real prose in those sections the slots land below the fold; the page reads as a text editor with a roster appendix. Each `SlotRow` also packs handle, LEAD badge, runtime select, model-override chip, source-runner attribution, a line-clamped system-prompt preview, and the effective command line into one dense row, so the load-bearing facts (who, which engine, who leads) don't pop.

**Runner detail** (`src/pages/RunnerDetail.tsx`, `/runners/:handle`): the full default system prompt renders as an unbounded `<pre>` dump at the top of the two-thirds column. Any real prompt is hundreds of lines, pushing "Crews using this runner" and the working-dir info far below the fold. The "Chat now" card duplicates the header's Chat now button and exists mostly to display the working directory; identity (display name) floats as a lone paragraph under the breadcrumb.

## Scope

Presentation-only redesign of the two pages. Same data, same commands, no backend or API changes.

Design first, in Pencil, per repo convention: new feature-scoped file (e.g. `design/58-runner-crew-detail.pen`) with one frame per page; review before any code. The MVP canvas stays untouched as the historical record.

Direction to explore in the design pass (not binding until frames are approved):

- **Crew detail**: slots become the hero, directly under the header/toolbar. Prose config (purpose, goal, conventions) demoted to a secondary presentation — collapsed sections, a side column, or a config tab — collapsed by default when content is long. Slot rows restructured so identity and role read at a glance (avatar/handle/LEAD first-class), with runtime/model overrides and command detail tucked behind hover or disclosure affordances. Keep drag-reorder, set-lead, override editing, and remove flows functionally intact.
- **Runner detail**: identity block (handle, display name, runtime, avatar) plus activity as the hero. System prompt shown clamped with expand/collapse instead of a full dump. Consolidate the "Chat now" card into the header action + a Details row for working dir. "Crews using this runner" stays one glance away.

## Non-Goals

- New capabilities on either page (no new fields, no new slot operations).
- Changes to `RunnerEditDrawer`, `AddSlotModal`, or `StartMissionModal` beyond what the new layouts require for visual consistency.
- List pages (`Runners.tsx`, `Crews.tsx`) — separate surfaces, already covered by feature 56.

## Implementation Phases

1. **Design** — Pencil frames for both pages in a feature-scoped `.pen`; iterate with the user until approved.
2. **Crew detail** — reorder sections (slots first), restructure `SlotRow` per the approved frame, keep reorder/lead/override/remove behaviors and their tests green.
3. **Runner detail** — new layout: identity hero, clamped prompt block, card consolidation.

## Verification

- `pnpm exec tsc --noEmit` and `pnpm run lint` clean after each phase.
- Existing Vitest suites (`CrewEditor.test.tsx`, `RunnerRuntimeModelReset.test.tsx`) still pass; extend where row restructuring moves behavior.
- Manual pass with a worst-case fixture: a runner with a multi-hundred-line system prompt, a crew with 5+ slots and long purpose/goal/conventions prose — slots visible without scrolling on the crew page, prompt collapsed by default on the runner page.
