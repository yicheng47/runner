# 1421 — Unified Files & Changes Viewer + Stable Per-Tab Panel Width (Option A)

> Tracking issue: [#1421](https://github.com/yicheng47/runner/issues/1421). Scheduling and priority live on the issue and in the [roadmap](../roadmap.md). Platforms: macOS, Windows and Linux.
> Design: `runner/desktop/specs/1421-unified-files-changes.pen` in `yeelinks_design` (signed off on **Option A**, 2026-11-23, commit `01fb5c7`). Boards: `uvxUZ` (Option A section `hHBbi` + Mission proof `C2`) and `C8P7J` (1:1 unified viewer Diff ↔ Code deep-dive, Runner Light verification `TVJPT`, and width contract `O2iqVZ`).
> Builds on: [#1403](1403-right-side-panel.md) (Orca activity bar, S2 Info popover, narrow/split overlay sheets, per-tab open state), [#1141](archive/1141-git-changes.md) (Git changes & diff viewer), [#1345](https://github.com/yicheng47/runner/issues/1345) (Changes checkout/base pickers), and [#634](archive/634-file-preview.md) (Files viewer & daemon file service).

## Motivation

After [#1403](1403-right-side-panel.md) shipped, switching between `Files`, `Changes`, and `Sessions` in the right side panel's activity bar causes the panel's width to jump (`300 px` vs. `680 px` vs. `740 px`) and triggers terminal PTY resizes (`SIGWINCH`). That happens for two coupled reasons:

1. Width is derived from the **active activity-bar mode** rather than owned by the workspace tab (`Files` widens to `file_viewer_width` `440 + 240 = 680 px` only when a file tab is open; `Changes` widens to a separate `changes_viewer_width` `500 + 240 = 740 px` only when a diff is selected; `Sessions` is always `300 px`).
2. `Files` and `Changes` each host their own completely separate viewer column (`FilesView` owns tabbed code/markdown + inline diff toggle; `ChangesView` owns a separate non-tabbed single-file diff viewer), so clicking from `Changes` to `Files` (or `Sessions`) tears down your open diff and resizes the entire panel.

This feature adopts **Option A — Shared Docked Viewer + Fixed-Width Right Rail**: the right side panel separates its fixed-width navigator rail from a single unified File + Diff viewer docked beside the terminal, locking width per tab across all rail switches and coupling `Files` and `Changes` around one document viewing model.

## Design Contract

### 1. Spatial Architecture & Aligned 40 px Header Row

When **no document is open** (rail-only state), the right panel island stays compact at the tab's rail width (`300 px` default, `chat_panel_width` / `mission_rail_width`, resizable `200–480 px`) with the `40 px` Orca top icon activity bar (`Files`, `Changes` with live amber dirty dot; `Sessions` first on missions; `Collapse` on the right).

When **at least one document tab is open** in the unified viewer, the right panel island splits horizontally into two full-height columns separated by a `1 px` `$border` divider:

- **Left — Unified File/Diff Viewer (`flex_1`, `$bg` ground):** hosts one shared document tab strip, an in-place `Code | Diff` mode switch, and the active file or diff body.
- **Right — Fixed-Width Navigator Rail (`file_tree_width`, default `240 px`, resizable `160–480 px`, `$panel` ground):** hosts whichever navigator is selected (`Files` project tree, `Changes` checkout/base/commits/changed-files list, or mission `Sessions` roster).

Both columns share **one continuous `40 px` top header row** aligned with the chat pane's `40 px` `IslandHeader` (eliminating the stacked `40 px` activity bar + `38 px` file tab strip, saving `38 px` of vertical reading height):

- **Viewer header (`40 px`, directly above the viewer):** horizontally scrollable unified tab bar (with overflow list button when tabs overflow), `Code | Diff` segmented toggle (shown whenever the active file has changes or a diff target), `Pin` button (shown when the active tab is an italic preview tab), rail visibility toggle, and `Maximize` (`⤢`, `⇧⌘↩` / `Ctrl+Shift+Enter`).
- **Rail activity bar (`40 px`, directly above the right rail):** the fixed icon buttons (`Files`, `Changes` with amber dirty dot; `Sessions` first on missions) with raised active tile (`theme::raised()`) and `2 px` bottom accent indicator (`theme::accent()`), plus `Collapse` (`panel-right-close`) on the far right. Do not render a placeholder Browser button (#1260 is out of scope).
- When the user hides the rail (`tree_hidden`) while viewing a document (or maximizes with the rail hidden), `Collapse` and the `Show rail` button stay reachable at the right edge of the `40 px` viewer header. On macOS when `maximized && sidebar_collapsed`, preserve `workspace_titlebar_padding` on the left of the unified `40 px` header row.

### 2. Per-Tab Fixed Panel Width (Zero Jump on Rail Switch)

- **Width follows viewer open state, never the active rail icon.** Once a tab or mission has an open document in its unified viewer, switching the right rail between `Files`, `Changes`, and `Sessions` swaps **only** the right rail column (`240 px`). The unified viewer stays open and steady, total panel width changes by `0 px`, and the terminal PTY receives zero resize events.
- **Rail-only switching is equally stable.** When no document is open in the viewer, switching `Files ↔ Changes ↔ Sessions` stays locked at the compact rail width (`300 px`).
- **Explicit open/close widening.** Opening the first file or diff from either rail (or `⇧⌘O` / `⌘`-click terminal link) widens smoothly from `300 px` to `viewer_width + rail_width`; explicitly closing the last tab in the unified viewer shrinks smoothly back to the `300 px` rail-only width.
- **Single unified viewer width preference.** Replace the two separate persisted viewer widths (`file_viewer_width` `440 px` and `changes_viewer_width` `500 px`) with **one** unified viewer width (`file_viewer_width`, default `460 px`, range `200–1200 px`). Keep `ui-settings.json` backward-compatible: legacy `changesViewerWidth` keys deserialize cleanly without error and are not written as a conflicting second width. Dragging the panel's outer left edge while the viewer is open always resizes the unified viewer width regardless of which rail icon is active; dragging the inner divider between the viewer and right rail always trades width between the viewer and `file_tree_width` (`160–480 px`) while holding the total panel width constant, across `Files`, `Changes`, and `Sessions`.
- **Per-tab isolation & narrow/split sheet protection preserved.** Keep `#1420` default-collapsed behavior for chats/terminals (default-open on `Sessions` for wide missions), per-tab open/mode/viewer state, `KEPT_VIEWS` eviction state saving, and `#1403`'s `compute_panel_layout` auto-sheet geometry (`MAIN_MIN_WIDTH = 420 px` single-pane, `SPLIT_COL_MIN_WIDTH = 340 px` per split horizontal column, `SHEET_MIN_SCRIM_WIDTH = 48 px`, `reserved_docked_width`, `Esc` / scrim dismiss, zero PTY resize).

### 3. Unified File + Diff Viewer & Files ↔ Changes Coupling

Both `Files` and `Changes` operate on **one shared viewer per subject** (`PanelSubject::Session` or `PanelSubject::Mission`):

- **Pinned tabs and italic preview tab:**
  - Opening a file from `Files` (tree click/Enter, `⇧⌘O` finder, Markdown relative link, or `⌘`-clicked terminal path) opens or activates a **pinned** tab in `Code` (or Rendered Markdown) mode.
  - Single-clicking a changed file or stepping with `↑↓` / `j`/`k` in `Changes` opens or reuses **one italic preview tab** in `Diff` mode for the active `Changes` target (`Uncommitted changes`, `All changes`, or a specific commit). Moving through the `Changes` list swaps that single preview slot instantaneously without creating tab spam.
  - If the selected `(path, target)` is already open as a pinned tab, selecting it in `Changes` focuses that pinned tab instead of opening a duplicate.
  - Clicking the `Pin` control, double-clicking the preview tab (or double-clicking a file row in `Changes`), flipping `Diff → Code`, or opening the same path from `Files` promotes the preview tab to a permanent pinned tab.
- **In-place `Code | Diff` mode switch:**
  - Any open tab whose file has uncommitted changes against `HEAD`, or that was opened with a `Changes` diff target (`Uncommitted`, `All changes`, or a commit), displays the `Code | Diff` segmented toggle in the `40 px` viewer header.
  - Flipping `Diff → Code` shows the working-tree file (highlighted code or rendered/source Markdown) inside the **same tab** and scrolls near the current diff hunk's new-file line so reading context is never lost. For a deleted file with no working-tree file, `Code` is disabled with a tooltip (or falls back cleanly to the deleted notice).
  - Flipping `Code → Diff` shows the tab's diff in place (using its associated `Changes` target when opened from `Changes`, or `Working tree vs HEAD` when opened from `Files`).
- **Context subbar (`34–36 px`) below the unified tab bar:**
  - In **Diff mode:** abbreviated file path, comparison scope pill (`Working tree vs HEAD`, `All changes · <branch> vs <base>`, or `<sha> vs parent`), `+N −M` line counts, `Unified | Split` toggle (shared preference, still auto-maximizing if `Split` would leave either side under `360 px`), **`✓ Viewed`** toggle (wired to `runnerd`'s `changes_set_viewed` and synced live with the `Changes` rail), and `Open in <editor> ▾`.
  - In **Code / Markdown mode:** abbreviated file path, changed-hunk jump indicator when the file has changes, Markdown `Rendered | Source` toggle (for `.md` files), `⌘F` find button, and `Open in <editor> ▾`.
- **Root synchronization & live watches:**
  - `Files` and `Changes` continue sharing the subject's daemon-saved root (`files_root` / `files_set_root` / `FilesEvent::RootChanged`), live `files_watch` refresh, and `DirtyTracker` when neither tool is mounted (e.g., a mission on `Sessions`).
  - While the unified viewer has an open file or diff tab and the user switches the right rail to `Sessions` (or switches between `Files` and `Changes`), keep live updates working for the visible viewer without leaking extra watchers when the panel collapses or the tab switches away.
  - Remote sessions continue showing the compact `300 px` unavailable notice in `Files` and `Changes` with no viewer widening, while preserving S2 `Title ▾` Info and remote mission `Sessions`.

### 4. Keyboard & Focus

Preserve existing shortcuts and focus zones cleanly:

- **Viewer focus:** `⌘W` / `Ctrl+Shift+W` closes the active unified tab (shrinking to `300 px` when closing the last tab); `⌃Tab` / `⌃⇧Tab` cycles open tabs; `⌘F` opens find in code; `[` / `]` jumps hunks in diff; `← →` scrolls horizontally; `⇧⌘↩` / `Ctrl+Shift+Enter` maximizes/restores the panel; `⌘+` / `⌘-` steps viewer font size.
- **`Changes` rail focus:** `↑↓` / `j`/`k` navigates commits/files (updating the italic preview tab when a file row is selected); `Tab` / `⇧Tab` moves between Commits and Files; `Space` toggles `Viewed`; `Enter` moves focus into the viewer; `Esc` from viewer returns focus to the active rail.
- **`Files` rail focus:** `↑↓` / `←→` navigates the tree; `Enter` / `Space` opens a pinned tab; `⇧⌘O` opens the panel on `Files` and opens the finder.

### 5. Scope & Non-Goals

- **Strictly frontend-only (`crates/runner-app/`):** Zero changes to `crates/runner-core/`, `crates/runner-daemon/`, `crates/runner-cli/`, client protocol, or SQLite schema.
- **Non-goals:** Option B (main-area workspace tabs beside terminals), Option C (removing separate `Files` and `Changes` rail icons), embedded browser (#1260), inline diff comments (#1293), or editing/staging/committing files.

## Implementation Phases

1. **Unified layout & width state (`surfaces/side_panel.rs`, `surfaces/files/mod.rs`, `app_settings.rs`, `panes/header.rs`, `panes/tab.rs`, `mission_workspace/view.rs`, `mission_workspace/rail.rs`):** Unify viewer width to one preference (`file_viewer_width`, default `460 px`, backward-compatible deserialization for legacy `changesViewerWidth`), compute `widened` from the subject's unified viewer open state rather than the active `PanelViewId`, and align the `40 px` viewer header beside the `40 px` right-rail activity bar.
2. **Unified File + Diff viewer (`surfaces/files/`, `surfaces/changes/`):** Extract or unify the viewer column so both `Files` and `Changes` open into one shared per-subject tab model (pinned tabs + single italic preview tab, in-place `Code | Diff` switch, shared `Unified | Split`, `✓ Viewed` integration, `Open in`, `⌘F`, Markdown `Rendered | Source`, and `KEPT_VIEWS` state persistence).
3. **Rail-only `Files` & `Changes` navigators + Mission integration:** Keep `Files` and `Changes` rails focused on navigation/pickers/commits/lists feeding the shared viewer; ensure mission `Sessions ↔ Files ↔ Changes` keeps the viewer open at locked width and preserves `DirtyTracker` + watch hygiene.
4. **i18n, Guides, and Tests:** Add any new strings to `crates/runner-app/i18n/{en,zh-hans}.toml`; update `docs/guide/guides/chats-and-windows.md`, `docs/guide/zh-CN/guides/chats-and-windows.md`, `docs/guide/concepts/missions.md`, and `docs/guide/zh-CN/concepts/missions.md`; add deterministic unit + GPUI app tests (20x nextest stress loop on async tests).

## Verification

### Automated

- Unit & GPUI app tests covering:
  - Per-tab width stability: opening a file or diff widens once; switching `Files ↔ Changes` (chats/terminals) and `Sessions ↔ Files ↔ Changes` (missions) produces **0 px width change** and keeps the viewer open; closing the last tab shrinks back to `300 px`.
  - Single viewer width preference: resizing outer divider or inner rail divider updates the same unified `file_viewer_width` / `file_tree_width` across `Files`, `Changes`, and `Sessions`; legacy `changesViewerWidth` in `ui-settings.json` loads cleanly.
  - Preview tab vs. pinned tab mechanics: single-click / `↑↓` in `Changes` reuses one italic preview slot; `Pin` / double-click / `Files` open / `Diff → Code` promotes to pinned; already-pinned `(path, target)` focuses without duplicating.
  - In-place `Code | Diff` toggle: flips seamlessly for uncommitted and Changes-opened files, syncs `✓ Viewed` with the `Changes` rail, disables `Code` cleanly for deleted files, and preserves `Unified | Split` + auto-maximize.
  - Narrow window & split-pane auto-sheet geometry (`420 px` / `340 px` budgets) and remote unavailable states remain intact.
- Run `cargo test -p runner-app`, `cargo nextest run -p runner-app --stress-count 20 -E '<new/modified async tests>'`, `make clippy`, `cargo fmt --all -- --check`, `python3 script/check-guide-translations.py`, and `git diff --check`.

### Live Checks (macOS development app, `@qa` with `script/qa-lock`)

1. **Zero width jump across `Files ↔ Changes` (Chat & Terminal):** In a fixture git repo under `/tmp` with modified, added, and deleted files, open the right side panel (`300 px`), switch `Files ↔ Changes` while rail-only (stays `300 px`). Click a changed file in `Changes` → widens once with unified `40 px` header row (`Unified Viewer` left + `Activity Bar` right). Click `Files` icon in the right rail → verify panel width does not change (`0 px` jump), terminal does not reflow, open diff stays visible, and only the right rail swaps to the Project Tree. Click back to `Changes` → still `0 px` jump. Close the last tab → shrinks smoothly back to `300 px`. Reason: core #1421 per-tab width contract.
2. **Unified viewer coupling (`Preview` vs `Pinned`, `Code | Diff`, `Viewed`, `Split`):** Step through multiple changed files in `Changes` with `↑↓` / `j`/`k` → confirm one italic preview tab swaps cleanly without tab spam. Pin one diff, open a second file from `Files` tree, confirm both coexist in one tab strip. Flip `Diff → Code → Diff` in place on a modified file and confirm scroll/context and `✓ Viewed` sync with the `Changes` list; click `Split` when narrow → maximizes cleanly and `Restore` returns; verify deleted file disables `Code`. Reason: new unified viewer & coupling mechanics.
3. **Outer & inner divider resizing:** Drag the outer panel left edge and inner viewer/rail divider while on `Changes`, then switch the right rail to `Files` (and back) → confirm both columns keep exact widths with zero jump and persist cleanly. Reason: single unified viewer width preference.
4. **Mission workspace (`Sessions ↔ Files ↔ Changes`):** Open a mission in the fixture repo, open a changed file diff, then switch the right rail to `Sessions` → confirm the Mission Feed + Unified Diff Viewer + Sessions roster all display cleanly with zero width jump, and switching back to `Files` / `Changes` keeps the diff intact. Reason: mission rail integration and watch/dirty-dot hygiene.
5. **Split-pane auto-sheet & Light Mode:** Verify a 2-column split chat auto-sheets when widening a file/diff (leaving the `300 px` rail reserved and zero PTY squeeze) and returns cleanly on close; switch to Runner Light and verify contrast across the aligned `40 px` header, unified viewer (`$bg`), right rail (`$panel`), active `Code | Diff` control, and diff highlights. Reason: visual parity & #1403 sheet regression check.

Selected regression cases: `WS-FILES-01`, `WS-FILES-05`, `WS-CHANGES-01`, `WS-CHANGES-09`, `SET-THEME-01`.
