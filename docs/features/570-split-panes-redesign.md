# Split panes redesign: divider only, unfocused fade, and a split menu

Tracking issue: [#570](https://github.com/yicheng47/runner/issues/570). Status: spec, 2026-09-12; Pencil-first, code waits for the frame sign-off. Priority P2.

## Motivation

Two things about a split tab read heavy or limiting today. The focused pane is marked with a 1 px accent border on all four sides; next to the 26 px identity line and the divider the accent box competes with the terminal content, and in a three-pane split the eye lands on the frame before the text. And splitting goes through a preset picker: six shapes, at most three panes, and `⌘D` / `⇧⌘D` step the whole tab to the next preset rather than splitting the pane you are in (`next_split_preset` bails with "this tab already has three panes").

Ghostty answers the first: no focus border at all, one hairline divider, and the *unfocused* splits dimmed by painting the background over them at partial opacity (`unfocused-split-opacity` 0.7, `unfocused-split-fill` the background). Zed answers the second: one split icon on each pane that opens a menu of directions, each acting on that pane, no preset list and no ceiling. Jason asked for both on 2026-09-12, folded them into one issue, and cut the menu to the two directions Runner already binds: right and down.

## Behavior

### Focus is brightness, not a frame

- **The focused pane has no border.** The accent border goes, and so does the transparent placeholder border on the others.
- **Unfocused panes are dimmed to 70 %.** The pane body below the identity line is drawn at 70 % opacity: the grid, its selection and cursor, and whatever sits over the grid, the ended card or the empty stub. GPUI element opacity, the same mechanism disabled controls already use, so nothing is overlaid and hit-testing, hover, the cursor style, and scrolling in an unfocused pane are unchanged. Ghostty dims the whole split the same way.
- **The divider is the only chrome between panes**: the 1 px line inside the 5 px resize gutter, as today.
- **The identity line does not dim.** It carries focus in its own colors already, and its status dot has to stay readable when a working agent sits in a dimmed pane.
- **The identity line's glyph says chat or terminal; focus changes only its color.** A chat pane shows `message-square`, a terminal pane `square-terminal`, the same pair the sidebar rows and the command palette already use; today the identity line and the header title draw a chat as the bare `terminal` prompt, which reads as a different state rather than a different kind. Focused and unfocused panes keep the same glyph and differ in tint, as now.
- **Clicking a dimmed pane focuses it**, as today, and a dimmed Resume or New chat button still works on the first click. Both theme modes: opacity fades toward whatever the pane is painted on.

### Splitting is one icon per pane, two directions

- **One split icon on every pane**, opening a two-item menu: **Split Right** `⌘D` and **Split Down** `⇧⌘D`. The two keymap entries and their defaults are unchanged; the shortcuts act on the focused pane, the icon on its own pane. No left or up: a pane splits to its right or below, which is Ghostty's pair, and the new pane is where the eye already is.
- **The icon lives with the pane it splits.** A single-pane tab has no identity line, so its header keeps today's split icon and that icon opens the menu. Once the tab is split, each identity line carries the icon next to `⋯`, always visible like `⋯` and `×`, and the header icon goes; one affordance per pane, Zed's placement.
- **Each item splits that pane**, not the tab: its leaf becomes a split with a new empty pane on the chosen side at 50 / 50. The new pane takes focus and shows the empty-pane stub (New chat / New terminal), as a preset split does today.
- **`⋯` is unchanged.** Stop, Rename…, Archive chat, nothing more.
- **The glyph is `columns-2`** on both surfaces, the icon the sidebar already uses for a split tab. The header button's `square-split-horizontal` changes to match; the two glyphs said the same thing in two drawings.
- **No count limit; a size floor instead.** An item is disabled, tooltip "Too small to split", when either resulting pane would be narrower than 240 px or shorter than 160 px at 1× zoom, scaled with the app zoom. The gate reads the pane's last laid-out bounds.
- **The layout picker is gone**, with `next_split_preset`, `pick_preset`, and the preset list. A three-column tab is two Split Right clicks.
- **The layout persists as a tree.** Today's `preset + slots + sizes` shape cannot describe a tree the picker did not draw, so the layout is stored as nested splits (orientation, sizes, id) and leaves (session id). Rows in the old shape read through a legacy path that rebuilds the same tree the preset built, and are rewritten on their next save. Leaf ids are `p<n>` with the next unused number in the tab, split ids `s<n>`; the `<preset>:outer` / `:inner` scheme goes. `PresetKind` survives only inside the legacy reader.
- **A split holds chats only.** Since the terminal drawer ([469](./archive/469-terminal-drawer.md)) a shell lives in the drawer under the tab or in a terminal-only single-pane tab; the empty pane offers New chat alone, so no split mixes a chat with a terminal. Nothing here changes that.
- **Close and resize are unchanged.** `×` / `⌘W` collapse the split as they do today, and the gutter drag resizes by split id.

### What does not change

- Single-pane tabs: no split, nothing to dim; the header icon is their split icon.
- The mission workspace, the terminal drawers, and secondary windows.
- The identity line.
- Keyboard focus movement between panes.

## Non-goals

- A setting for the fade amount or the size floor.
- Drag to rearrange, swap, or maximize a pane.
- Split left or up. Split items in `⋯`.
- Presets as a feature; there is no one-click three-column shape any more.

## Design

`design/runner.pen`: one frame, `Spec — Split panes redesign (570) · v1`, built from the two- and three-pane surfaces of spec 64. It shows today's two-pane split with the accent border, the same split after with the border gone and the unfocused pane dimmed, the three-pane split with the middle pane focused, and the two-item split menu open under a single-pane header's icon next to a split pane's identity line carrying its own icon.

## Implementation Phases

1. **Design.** The frame above. Stop for sign-off.
2. **Fade and border.** The pane body container in `render_pane` (`panes.rs:1855` onward, the `body` element that stacks the terminal and its overlay) gets `.opacity(UNFOCUSED_PANE_OPACITY)` when `grouped && !focused`, one constant at `0.7`; the identity line sits outside it. GPUI applies element opacity to every quad and glyph painted underneath, the terminal element included. Remove the border at `panes.rs:2345`. `pane_identity_icon` (`panes.rs:2424`) and the header title glyph (`panes.rs:454`) return `message-square.svg` for chats. Lands as its own PR.
3. **Layout tree.** `PaneLayout::split(pane_id, side) -> pane id` in `pane_layout.rs`; tree persistence with the legacy reader; the new id scheme; `apply_preset`, `next_split_preset`, `derive_preset`, and `canonicalize_split_ids` removed with the picker (`panes.rs:1338` `render_layout_picker`, `chat.rs:1660` `pick_preset`, `:1717` `split_pane`). Anything that derived a glyph or label from the preset reads the tree instead.
4. **Icon and menu.** The header icon (`panes.rs:400`) opens a two-item `UiMenu` in the `⋯` popover's shape and renders only for single-pane tabs; the identity line gains the same icon beside `⋯` when grouped, opening the same menu for its pane; both use `columns-2.svg` (the header swaps off `square-split-horizontal.svg`); `⌘D` / `⇧⌘D` call the new split on the focused pane; the size gate and its tooltip; `split_panes_tooltip` (`panes.rs:2411`) is unchanged. Lands with phase 3 as the second PR.
5. **Docs.** `docs/arch` wherever it describes presets; the 64 smoke test's chrome section.

## Verification

- Two-pane split with two live chats: both identity lines show the chat glyph, a terminal pane the boxed terminal glyph, matching their sidebar rows; no border on either pane; the unfocused grid is dimmed, the focused one is at full brightness; clicking the dimmed pane swaps the fade at once; both identity lines look as before, and the unfocused pane's status dot stays at full strength while its agent works.
- Three-pane split: the two unfocused panes are dimmed. Single-pane tab: nothing dims. Runner Light: a light haze.
- Single-pane tab: the header icon opens Split Right / Split Down with the live shortcut pills; each splits the pane on its side and focuses the new empty pane; after the split the header icon is gone and each identity line has its own; `⌘D` and `⇧⌘D` split the focused pane; rebinding one changes the pill.
- The icon on an unfocused pane's identity line splits that pane, not the focused one. `⋯` still has three items.
- Split a pane until an item disables; the tooltip says why; widening the window re-enables it.
- Quit and relaunch with a five-pane tab: the tree, sizes, and focus come back. A tab saved by 0.8.7 in each of the six presets opens as the same shape.
- `×` on a pane in a nested split collapses only that split; resize drag works on every gutter.
- Unit tests: split on each side for a root leaf and a nested leaf; serialize round-trip; the six legacy presets decode to the trees they built; the size gate; the opacity constant is pinned and a grouped unfocused pane's body carries it while a focused or single pane's does not. `cargo test -p runner-app` green.
