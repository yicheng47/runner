# 29 — Runner and crew list pagination and search

> Tracking issue: [#220](https://github.com/yicheng47/runner/issues/220)
> Design: `design/runner-crew-list-search.pen` — search toolbar + pager components, Runners default/filtered/no-match states, Crews page 2 state.

## Motivation

The Runners and Crews pages currently render every row in one vertical stack. That is fine with the seed data, but it gets slow to scan once the user has dozens of runner templates or experimental crews. The user needs two basic list affordances before these surfaces feel operational at scale: search to narrow the list by identity/context, and pagination so the page stays visually manageable.

This should make the list pages behave like work surfaces, not inventories the user has to manually sweep. The common workflows are "find the runner I already made", "open the crew for this project", and "check whether a runner/crew exists before creating another one."

## Scope

### In scope

- **Runners page search.** Add a compact search input above the runner cards. Match against runner handle, display name, runtime, command, model, effort, working directory, and system prompt text.
- **Crews page search.** Add the same control above crew cards. Match against crew name, purpose, default goal, system prompt addendum, slot handles, member runner handles, and member runtimes.
- **Client-side pagination for v1.** Keep using `runner_list_with_activity` and `crew_list`; filter and paginate the already-loaded arrays in the frontend. This avoids new IPC/SQL contracts until there is evidence that the local lists are large enough to need backend paging.
- **Shared list control behavior.** Use the same page size, empty-result state, page count, and next/previous controls on both pages. Reset to page 1 when the search query changes or when refreshed data makes the current page invalid.
- **Result count copy.** Show visible count vs total count near the controls, e.g. `12 of 48 runners`, and distinguish `0 matching runners` from the existing "No runners yet" empty state.
- **Stable card behavior.** Existing card actions remain unchanged: opening, chat start, edit/delete menus, and live runner activity updates continue to work on the filtered/paginated view.
- **Keyboard and accessibility basics.** Search input has a clear label/placeholder, Escape clears the query when focused, and pagination buttons expose disabled states and accessible labels.

### Out of scope

- Backend pagination/search commands for v1.
- Sorting controls. Preserve the existing list order.
- Fuzzy search, saved filters, tags, or advanced query syntax.
- Pagination for sidebar mission/chat lists or detail-page slot lists.
- Cross-page global search.

## Implementation Phases

### Phase 1 — shared list utilities/control

- Add a small shared hook or utility for normalized text matching and pagination calculations, scoped to the list pages rather than a broad data-grid abstraction.
- Add a compact search + pagination control component if it removes duplication between `Runners.tsx` and `Crews.tsx`.
- Default page size to a fixed value that keeps card stacks scannable; prefer a simple constant over a user setting in v1.

### Phase 2 — Runners page

- Build a runner search document from `RunnerWithActivity` fields.
- Render filtered/paginated runners while preserving the existing loading, failed-load, and true-empty states.
- Update live activity events against the full runner array so currently hidden rows stay current when the filter changes.
- Verify delete/create/refresh paths clamp pagination correctly.

### Phase 3 — Crews page

- Build a crew search document from `CrewListItem` fields and member previews.
- Render filtered/paginated crews while preserving the existing true-empty state and card interactions.
- Verify create/delete/refresh paths clamp pagination correctly.

### Phase 4 — polish and validation

- Tune empty-result copy and button disabled styling against the existing dark card UI.
- Add focused tests where the local test stack makes it cheap, especially the filtering utility and page clamping behavior.
- Run frontend typecheck and lint.

## Verification

- [ ] On Runners, entering part of a handle filters the list and shows the matching count.
- [ ] On Runners, entering a runtime, command, model, effort, or working directory filters matching runner cards.
- [ ] On Crews, entering part of a crew name, purpose, goal, slot handle, runner handle, or runtime filters matching crew cards.
- [ ] Search with no matches shows an empty-result state without replacing the true-empty "No runners/crews yet" state.
- [ ] Pagination shows only the current page, disables previous/next at boundaries, and resets/clamps after search, create, delete, and refresh.
- [ ] Existing runner card actions still work from filtered and later pages.
- [ ] Existing crew card actions still work from filtered and later pages.
- [ ] `pnpm exec tsc --noEmit` passes.
- [ ] `pnpm run lint` passes.
