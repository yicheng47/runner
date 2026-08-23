# 56 — Backend pagination for Runners and Crews, honest bottom pager

Tracking: [#377](https://github.com/yicheng47/runner/issues/377). Status: shipped (#377 closed 2026-07-31; native `list_controls.rs` / `ui/list.rs`).

Design: `design/runner-crew-list-search.pen` — the "56" row below the Crews frames: frame `csK4O` ("Runners — 56 · bottom pager, honest scroll" — pager stays bottom-pinned behind a 1px hairline aligned with the sidebar footer's divider, list region above scrolls with a visible scrollbar, a card cut at the fold reads as scrollable rather than hidden) plus deltas note `nmAx8`. The single-page state uses the same footer with page 1 selected and both arrows disabled; it has no separate frame. `cmp/Pager` itself is unchanged.

## Motivation

Two problems on the Runners/Crews list pages (direction set after an earlier scrolling-list draft — pagination stays, per product call):

1. **The bottom band reads as a block hiding rows.** The card list is `min-h-0 flex-1 overflow-y-auto` with a hidden scrollbar, so it claims all remaining viewport height and clips the trailing card mid-border; the `Pager` is then pushed below it by `mt-auto` + `pt-3` on top of the page's `pb-8`. The clipped card sliver directly above a thick dark band with no separating edge makes it look like runners are hidden behind the pagination block.
2. **Pagination is client-side slicing.** `runner_list_with_activity` / the crews list return every row; `useListControls` filters and slices in the frontend. Fine at 6 runners, but it means search and paging semantics live in the wrong layer, and every list render pays for the full table.

## Scope

- **Backend pagination.** The list commands gain `page`, `page_size`, and `query` parameters and return `{ items, total_count, filtered_count }`, with `LIMIT`/`OFFSET` and the search filter applied in SQL. Search must move with pagination: filtering the current page client-side would silently drop matches on other pages. Runner search covers handle and display name only, so matches always correspond to visible identity text instead of hidden command/config fields. Crew search covers the fields its empty-state copy advertises: name, purpose, goal, system prompt, slot handle, runner handle, and runtime.
- **Honest bottom pager.** The pager stays bottom-pinned and centered exactly where it is today, including the single-page state so the footer and its aligned divider remain visible. A 1px `$border` hairline separates it from the list, so a card cut at the fold reads as a scrollable region above a fixed footer rather than content masked by a black block — the hairline aligns to the same y as the sidebar footer's divider (the Settings row's top border), so the two read as one continuous line. The list column scrolls with a **visible** auto-hiding scrollbar instead of the current hidden one.
- **Frontend plumbing.** `useListControls` becomes the IPC driver: debounced query, page state, clamp-on-shrink (deleting the last row of the last page steps back a page). Counter stays `N of M` from `filtered_count` / `total_count`.
- **Drop the `esc` keycap hint from the search field.** The badge is noise; Esc keeps its clear-the-query behavior, and the `×` clear button remains the visible affordance.
- Runners and Crews get identical pagination and list-layout treatment; their search fields follow the visible identity/context copy documented above.

## To Be Decided

- Viewport-adaptive `page_size` (fit as many whole cards as the window allows) vs. fixed 8: adaptive kills the short-window scroll case entirely but adds resize→refetch churn. Spec assumes fixed 8 until dogfooding says otherwise.
- Whether `runner_list` (bare, non-activity) keeps its unpaginated shape for internal callers — assumed yes; only the two list-page commands paginate.

## Implementation Phases

1. **Backend** — paginated query for runners-with-activity and crews (SQL `LIKE` filter + `LIMIT`/`OFFSET` + count queries in one command), unit tests for filter coverage, paging math, and the empty-page clamp.
2. **Frontend** — `useListControls` drives the IPC (debounce, page clamp); Runners/Crews render returned items; pager stays visible for non-empty lists and gains the hairline separator; visible auto-hiding scrollbar on the list column.
3. **Validation** — `cargo test`, `tsc`, lint, vitest; manual pass below.

## Verification

- [ ] Six runners, default window: all six visible or reachable with no half-clipped card; the page-1 pager and aligned hairline remain visible with both arrows disabled and no thick dead band above the window bottom.
- [ ] More than 8 runners: pager appears bottom-pinned behind its hairline; a card cut at the fold shows a visible scrollbar above the separator; page 2 shows the remainder; deleting the last row of page 2 steps back to page 1.
- [ ] Search narrows across *all* pages (a match on what was page 2 appears while page 1 is open), counter reads `filtered of total`.
- [ ] Short window: full page of cards scrolls with a visible scrollbar; nothing renders below the pager but page padding.
- [ ] The pager hairline sits at the same y as the sidebar footer's divider (one continuous line across the window); the search field shows no `esc` keycap and Esc still clears the query.
- [ ] Crews page behaves identically.
