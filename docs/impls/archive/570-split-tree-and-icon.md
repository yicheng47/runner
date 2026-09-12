# 570 — Split panes redesign, PR 2: the layout tree, one split icon per pane, the picker gone

Tracking issue: [#570](https://github.com/yicheng47/runner/issues/570). Spec: [570](../../features/archive/570-split-panes-redesign.md), phases 3–5. Feature, P2. Shipped 2026-09-12 in [#572](https://github.com/yicheng47/runner/pull/572) (mission `01M2AHEYTZQFK3X5JZA2TGZNDN`, claude crew; three review must-fixes on the tree reader, pane measurement, and the shortcut gate, then two of Jason's tweaks: the identity-line trigger moved left of `×`, the trigger tooltip dropped). Design: `design/runner.pen` frame `Spec — Split panes redesign (570) · v1` (`w2FDzK`), section D. PR 1 ([#571](https://github.com/yicheng47/runner/pull/571)) shipped the fade, the border removal, and the chat glyph. Branch: **`feat/570-split-tree-and-icon` already exists and is checked out** — it carries this brief; work on it, do not create another.

## What ships

Every pane has one split icon, `columns-2.svg`, opening a two-item menu: **Split Right** `⌘D` and **Split Down** `⇧⌘D`. A single-pane tab shows it in the header where the picker button is today; once split, each identity line carries it beside `⋯` and the header button goes. Each item turns *that* pane's leaf into a 50 / 50 split with a new empty pane on that side; the new pane takes focus and shows the empty stub. `⌘D` / `⇧⌘D` do the same to the focused pane. There is no pane count; an item is disabled with the tooltip **Too small to split** when either resulting pane would be under 240 × 160 px at 1× zoom, scaled with app zoom. The six-preset layout picker is gone. The layout persists as a tree. Splits hold chats only: a terminal-only tab has no split icon and the shortcuts do nothing there.

## Where the code is

- `crates/runner-app/src/pane_layout.rs`: `PresetKind` `:15`; `PaneLeaf` `:78`, `PaneSplit` `:84` (`id`, `orientation`, `sizes: [f32; 2]`, `a`, `b`), `PaneNode` `:93`; `PaneLayout` `:167` (`preset`, `root`, `focused_pane_id`, `drawer`); `PersistedLayout` `:300` (`preset`, `slots`, `sizes` by split id, `drawer`); `fresh` `:364`; `apply_preset` `:514`; `prepare_new_pane` `:526` (steps presets, bails at three); `next_split_preset` `:571`; `close_pane` `:581` (collapses through `remove_pane` `:822`, then `derive_preset` `:849` and `canonicalize_split_ids` `:870`); `serialize` `:629`; `upsert_input` `:645`; `build_preset_tree` `:790`.
- **Backend contract, do not touch:** `crates/runner-backend/src/repo/node.rs:52` `StoredLayout { slots, drawer }` — `session_ids_from_layout` `:490` and `drawer_session_ids_from_layout` `:510` feed the reconciler and `find_for_session`. The JSON must keep `slots` (leaf session ids in leaf order) and `drawer` exactly as today.
- `crates/runner-app/src/surfaces/chat.rs:1660` `pick_preset` (the persist → reload → attach → focus sequence to keep; its terminal-location branch goes); `:1699` / `:1708` `split_pane_right` / `split_pane_down`; `:1717` `split_pane`.
- `crates/runner-app/src/surfaces/panes.rs:401` the header button `layout-picker-toggle` and `layout_picker_open` (`:353`, `:411`, `:579`, `:622`); `:1338` `render_layout_picker`; `:1587` the identity line's `⋯`, a `PopoverMenu` (`ui/menu.rs:278`) with `.trigger_icon("more-horizontal.svg")` and `MenuItem` (`:19`, `.shortcut()`); `:2411` `split_panes_tooltip`; `workspace_header_icon` (title glyph, unchanged).
- `crates/runner-app/src/keymap.rs:303` / `:311` the two entries, `:937` the bindings. `sidebar.rs:3598` `sidebar_tab_icon` counts panes; unchanged.

## Fix shape

1. **Model.** `PaneLayout::split(&mut self, pane_id: &str, orientation: SplitOrientation) -> Result<String>`: the leaf becomes `PaneSplit { id, orientation, sizes: [50., 50.], a: that leaf, b: new empty leaf }` and the new pane is focused and returned. Ids: leaves `p<n>`, splits `s<n>`, `n` one past the highest number in the tab. `close_pane` keeps collapsing through `remove_pane` with no preset step after. `preset` leaves `PaneLayout`; `fresh` becomes `single(focused, visible)` plus what the legacy reader needs; `apply_preset`, `prepare_new_pane`'s preset stepping, `next_split_preset`, `derive_preset`, `canonicalize_split_ids` go.
2. **Persistence.** `PersistedLayout` gains `tree: Option<PaneNode>` with serde on the node types. The writer emits `tree`, `slots`, `drawer`; `preset` and `sizes` are no longer written. The reader takes `tree` when present, else rebuilds the preset tree from `preset + slots + sizes` (`build_preset_tree` and the size application survive there, `PresetKind` with them). A row read through the legacy path is rewritten in the new shape on its next save.
3. **Size floor.** A pure `split_allowed(pane: Size<Pixels>, orientation, zoom) -> bool`: right needs width ≥ 2 × 240 × zoom, down needs height ≥ 2 × 160 × zoom. Record each pane's laid-out bounds once per frame (a map on the root filled from `render_pane`, or the terminal geometry the pane already has) and gate the items from it.
4. **Icon and menu.** One helper builds the two-item `PopoverMenu` for a pane id with the live shortcut pills, disabled items carrying the tooltip. Header: the `layout-picker-toggle` button becomes this menu's trigger with `columns-2.svg`, rendered only when the tab has one pane and it is a chat. Identity line: the same trigger beside `⋯` when grouped, acting on that pane. `split_pane_right` / `split_pane_down` call `split(focused_pane_id, …)`; then the `pick_preset` sequence: persist, reload tabs, attach, remember runner, mark viewed, focus. Delete `render_layout_picker`, `layout_picker_open` and its focus handle, escape and click-away handling, `pick_preset`, `split_pane`. `split_panes_tooltip` stays as the trigger tooltip.
5. **Docs.** `docs/arch` wherever it describes the preset picker or the persisted layout shape; `docs/tests/64-terminal-as-pane-option-smoke.md` lines that mention the picker. The spec is archived at landing, not by the crew.

## Rules of the road

- No backend change. No split left or up. Nothing in `⋯`. No settings. The fade, border, and glyph from PR 1 stay as they are.
- Stage by path, never `git add -A`. The tree carries nothing else at launch; if it does, ask.
- Do not launch the Runner app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-app`, `make clippy` (with `--features updater` too), `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR titled `feat(ui): split panes — layout tree, one split icon per pane, picker removed (570, PR 2)`, drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents.

## Tests

- `split` right and down on a root leaf and on a leaf inside a split: ids, sizes, focus, leaf order; then `close_pane` on the new leaf collapses back to the original tree.
- Serialize round-trip of a five-pane tree with mixed orientations and custom sizes; `slots` in the output equals the leaf session ids in order; `drawer` untouched.
- The legacy JSON of each of the six presets (write the old shape by hand) reads into the tree the old code built, sizes included.
- `split_allowed` at the floor on both axes and at zoom 1.5.
- Existing suites green: `cargo test -p runner-app`; tests that built layouts from presets rebuilt with `split`.

## Jason's smoke test (after landing)

1. Single-pane chat: the header split icon opens Split Right / Split Down with the shortcut pills; each gives a 50 / 50 split, the new empty pane focused; the header icon is gone and both identity lines carry the icon beside `⋯`.
2. Split from an unfocused pane's icon: that pane splits, not the focused one. `⌘D` / `⇧⌘D` split the focused pane. Four and five panes work; resize every gutter; `×` collapses only its own split.
3. Keep splitting until an item disables with the tooltip; widening the window re-enables it.
4. Quit and relaunch with a five-pane tab: tree, sizes, focus come back. Tabs saved by 0.8.7 in each preset open as the same shape.
5. A terminal-only tab: no split icon, `⌘D` does nothing. The sidebar row icons are unchanged.
