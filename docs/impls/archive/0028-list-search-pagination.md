# Runner and crew list search + pagination

## Status

Planned. Tracks issue [#220](https://github.com/yicheng47/runner/issues/220) / spec [29](../features/29-runner-crew-list-pagination-search.md). Design: `design/runner-crew-list-search.pen` (screens: Runners default / filtered / no matches, Crews page 2, plus the "Pager — windowing states" reference strip).

## Problem

`Runners.tsx` and `Crews.tsx` render every row in one vertical stack with no way to narrow or page the list. Both pages load their full arrays up front (`runner_list_with_activity`, `crew_list`), refresh on `runner/changed` / `slot/changed` / `crew_*` events, and — on Runners — patch live activity counters into the loaded array by id (`Runners.tsx:86-108`). Any list controls must sit downstream of that state so refreshes and activity events keep working untouched.

## Key Decisions

1. **Client-side, no new IPC** (spec-decided). Filter and paginate the already-loaded arrays with `useMemo`; the existing fetch/refresh/event paths stay the single source of truth.
2. **Shared pieces, scoped small:**
   - `src/lib/listControls.ts` — pure helpers: `buildSearchDoc(fields)` (join non-null fields, lowercase), `matchesQuery(doc, query)` (normalized substring), `pageWindow(current, total)`, `clampPage`, and `PAGE_SIZE = 8`.
   - `src/hooks/useListControls.ts` — one hook owning `query`/`page` state; returns `pageItems`, `filteredCount`, `totalCount`, `pageCount`. Query change resets to page 1; item-array change clamps the page (covers delete/refresh shrinking the list).
   - `src/components/ui/SearchInput.tsx` — search icon + input + `esc` hint chip + `×` clear (clear only visible with a query). Escape while focused clears.
   - `src/components/ui/Pager.tsx` — chevron arrows + numbered pages + ellipsis.
3. **Pager windowing** (per the design's states strip): at most 5 page numbers; first and last always visible; `…` collapses hidden ranges; arrows step ±1 and render disabled (50% opacity) at the boundaries.
   - `total ≤ 5` → `[1..total]`
   - `current ≤ 3` → `[1, 2, 3, 4, …, total]`
   - `current ≥ total − 2` → `[1, …, total−3, total−2, total−1, total]`
   - else → `[1, …, current−1, current, current+1, …, total]`

   Exactly 5 numbers whenever `total > 5`. Active page = raised box (`bg-raised` + `border-line-strong`, semibold `text-fg`); inactive = plain `text-fg-2`; buttons 28×28, `rounded`. Pure function, unit-tested.
4. **Count copy**: `${pageItems.length} of ${totalCount} runners` (visible-on-page of unfiltered total), matching the design states — `5 of 48 runners`, `1 of 23 runners`, `0 of 23 runners`. Mono 11px `text-fg-2`, right end of the toolbar.
5. **Search documents** (what the query matches against):
   - Runner: `handle`, `display_name`, `runtime`, `command`, `args.join(" ")`, `model`, `effort`, `working_dir`, `system_prompt`.
   - Crew: `name`, `purpose`, `goal`, `system_prompt_addendum`, and each member's `slot_handle`, `runner_handle`, `runtime`.

   Docs are memoized per list array; activity patches replace the array but rebuilding docs at local scale is negligible.
6. **State rendering rules**:
   - True-empty (`totalCount === 0`) keeps the existing `EmptyStateCard`; the toolbar and pager don't render at all.
   - No matches (`query` set, 0 results): inline panel per design — `search-x` icon, `No runners match "{query}"`, field-coverage hint line, secondary "Clear search" button. Pager hidden.
   - ≥1 match: pager always renders, even at one page (single `1`, both arrows dimmed), per the filtered design state.
7. **Footer placement**: the pager wrapper gets `mt-auto` inside the page's flex column — pinned low when the page is short (matching the design frames), natural flow when the list is long and scrolls.
8. **Accessibility**: input labeled `Search runners` / `Search crews`; page buttons `aria-label="Page N"` with `aria-current="page"` on the active one; arrows are real disabled buttons at bounds.

## Non-Goals

Backend pagination/search commands, sorting, fuzzy matching or query syntax, sidebar list pagination, cross-page global search (all spec-excluded).

## Implementation Phases

### Phase 1 — shared utilities + controls

`listControls.ts` with vitest coverage for `pageWindow` (all four window shapes, boundary currents) and `clampPage`; `useListControls`; `SearchInput` and `Pager` components styled per the design tokens.

### Phase 2 — Runners page

Toolbar row between the header and the card stack (`Runners.tsx:163-232` render path); wire `useListControls` over `runners` with the runner search doc; no-match panel; pager footer. Card actions (open, Chat spawn, menu delete) and the activity listener are untouched — verify delete/refresh clamp.

### Phase 3 — Crews page

Same wiring over `crews` with the crew search doc (`Crews.tsx:104-150`); card actions and member pills untouched.

### Phase 4 — polish + validation

Disabled/hover styles against all four themes (tokens only — no hardcoded colors), copy pass, `pnpm test`, `pnpm exec tsc --noEmit`, `pnpm run lint`.

## Verification

- [ ] Runners: typing part of a handle/runtime/command/model/effort/cwd/system-prompt narrows the list; count updates; page resets to 1.
- [ ] Crews: name/purpose/goal/addendum/slot handle/member handle/runtime all match.
- [ ] Pager renders the exact windows from the design strip: all-shown (≤5), start `1 2 3 4 … N`, middle `1 … c−1 c c+1 … N`, end `1 … N−3 N−2 N−1 N`; arrows disabled at bounds.
- [ ] Clicking a page number jumps to it; arrows step ±1; only current-page cards render.
- [ ] No-match panel appears only with a query; "Clear search" restores the full list; true-empty pages unchanged.
- [ ] Deleting a runner/crew on the last page clamps rather than showing an empty page.
- [ ] Card actions work from filtered and later pages; live activity text stays current after filtering.
- [ ] Esc (focused) and `×` clear the query.
- [ ] `pnpm test`, `pnpm exec tsc --noEmit`, `pnpm run lint` clean.

## Relevant Code

- `src/pages/Runners.tsx:26-108` — list state, refresh, event listeners, activity patching; `:163-232` — render path the toolbar/pager slot into; `:256-392` — `RunnerCard` (unchanged).
- `src/pages/Crews.tsx:104-150` — header/list render; crew card + member pills (unchanged).
- `src/lib/types.ts` — `Runner`/`RunnerWithActivity`, `CrewListItem`/`CrewMemberPreview` field lists the search docs are built from.
- `src/components/ui/Button.tsx` — secondary variant the "Clear search" button and pager active-box styling mirror.
- `src/index.css` — theme tokens (`raised`, `line-strong`, `fg-2/3`); controls must stay token-only so Codex Light / Catppuccin themes work.

## References

- Issue #220 — feat: add pagination and search to runners and crews.
- Spec `docs/features/29-runner-crew-list-pagination-search.md`.
- Design `design/runner-crew-list-search.pen` — `cmp/SearchField`, `cmp/Pager`, the four page states, and the pager windowing strip.
