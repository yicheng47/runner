# #1415 — Drop worktree path from sidebar row tooltips

Jason approved `neon trio` on 2026-11-21. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1415-sidebar-worktree-tooltip` on `feat/1415-sidebar-worktree-tooltip` (base `origin/main` `e0dfbe82`, its own `target/`). Other worktrees under `.worktrees/` belong to other work; never inspect or edit them.

## Problem and outcome

Hovering a sidebar row for a chat or mission in a linked or missing git worktree currently shows a multi-line tooltip containing both the branch name (or missing status) and the full checkout path (`"{checkout_label}\n{path_display}"`). Because `compact_path` only shortens `$HOME`, paths on external volumes or remote hosts expand to the 320 px tooltip max width and wrap across multiple lines, spilling over the terminal pane while repeating the branch name already encoded in `.worktrees/<branch>`.

Outcome: sidebar row tooltips for linked and missing worktrees show only the full branch/commit label (or missing-worktree status) without the checkout path line, matching multi-checkout tooltips. The full checkout path remains in the Info panel.

## Read first

- Issue #1415.
- `crates/runner-app/src/surfaces/sidebar/location.rs`: `resolve_row_location`, specifically the missing-worktree tooltip (around lines 185–208) and the linked-worktree tooltip (around lines 224–256), plus `pane_tooltip_branch` and multi-checkout tooltips (lines 129–159).
- `crates/runner-app/src/surfaces/sidebar/active.rs` and `crates/runner-app/src/surfaces/sidebar/rows_render.rs` to confirm how `location.tooltip` is attached to sidebar rows.
- `crates/runner-app/src/surfaces/sidebar/tests.rs` (around lines 2070–2220) and any other tests asserting sidebar worktree tooltips.

## Deliverables

1. **Tooltip content.** In `crates/runner-app/src/surfaces/sidebar/location.rs`, drop the checkout path line from `RowLocation.tooltip`:
   - Linked worktree (branch or detached commit): `tooltip: Some(checkout_label.clone())` (single line, no `\n{path_display}`).
   - Missing worktree: `tooltip: Some(status.clone())` (single line, no `\n{path_display}`).
   - Keep main-checkout behavior (`tooltip: None`) and multi-checkout tooltips (`<pane>: <branch>` per pane) unchanged.
2. **Tests.** Update existing assertions in `crates/runner-app/src/surfaces/sidebar/tests.rs` (and any other affected tests) and ensure linked branch, detached commit, outside worktree, missing worktree, and multi-checkout tooltips are all covered cleanly.
3. **Docs check.** Confirm whether any `docs/guide/` page mentions the path in the sidebar row tooltip; update both English and `zh-CN` pages only if one does (`python3 script/check-guide-translations.py`).

Non-goals: changing the Info panel's checkout path display, changing `tooltip_content` max width or styling in `crates/runner-app/src/ui/tooltip.rs`, or changing sidebar row layout/truncation.

## Validation and live checks

- **Automated:** run `cargo test -p runner-app` (or scoped sidebar tests plus `runner-app` tests), `make clippy`, `cargo fmt --all -- --check`, and `git diff --check`.
- **Live QA (`@qa` after a clean review, macOS development app, QA lock):**
  1. Hover a sidebar row for a chat or mission in a linked worktree (in both Projects and Active views) and verify the tooltip shows only the full branch name on a single compact line with no cwd/path below it.
  2. Verify a main-checkout row still shows no worktree tooltip.
  3. Record results with screenshot evidence in `docs/tests/runs/1415-sidebar-worktree-tooltip-mac.md`.

## Authorization

Approved crew: `neon trio`. Fold this brief into the implementation commit, push, and open a PR against `main` with `Fixes #1415`. `@reviewer` adds `reviewed` on the final head, and `needs-live-test` is handled as `AGENTS.md` describes. Do not merge.
