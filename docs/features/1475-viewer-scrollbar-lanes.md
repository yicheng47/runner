# 1475 — Scrollbars in the file viewer

Tracking issue: [#1475](https://github.com/yicheng47/runner/issues/1475). Corrects the code view from [#634](https://github.com/yicheng47/runner/issues/634) ([spec](./archive/634-file-preview.md)) and the diff from [#1141](https://github.com/yicheng47/runner/issues/1141) ([spec](./archive/1141-git-changes.md)), which share one viewer since [#1421](https://github.com/yicheng47/runner/issues/1421).

Design: no board. Jason chose the 10 px lane and the bar from renders of the real views (2026-11-25). A square block bar as in VS Code and Zed was rendered too and dropped, because it does not fit Runner's look.

## Motivation

The viewer's scrollbars lie over the code: the vertical thumb covers the last visible columns, and the horizontal thumb covers the last visible line. Neither bar should overlap the code (Jason, 2026-11-25). The text area runs to the view's full width and height, and the two bars are drawn over that area. Menus and terminals already keep a lane for their bar.

The rendered Markdown page has no scrollbar at all, so nothing shows how long a page is or where you are in it. And a bar that widens under the pointer is not needed in the viewer (Jason, 2026-11-25).

## Scope

### The bar

Code, Diff and the rendered Markdown page use one bar: the app's rounded thumb, 4 px wide and 3 px from the edge, which does not widen under the pointer or while dragged. It shows and hides as it does now: while the pointer is over the file, while scrolling and while dragged.

### Code and Diff

Code (`crates/runner-app/src/surfaces/files/code.rs`, Markdown source included) and Diff, unified and split (`crates/runner-app/src/surfaces/changes/diff.rs`).

- **Lanes.** A lane is reserved at the right edge when the file scrolls vertically and at the bottom when it scrolls sideways. The text area is what is left. An axis that fits reserves nothing.
- **Hidden bars.** A lane depends only on whether its axis scrolls, so it stays while its bar is hidden and the text does not move when the bar appears.
- **What ends at the lanes.** Text, line numbers, signs, hunk headers, selection, find highlights and the **Mark viewed** footer. In split, only the right side gives up width to the vertical lane; the divider and the columns stay where they are.
- **Diff row colours.** Added, removed, filler, hunk and cursor-row backgrounds run under the vertical lane to the right edge, so no blank stripe shows. They end above the bottom lane.
- **Scrolling.** Scroll limits, page up and down, jumping to a line, match or hunk, and dragging a selection past an edge use the text area, so the last line and column can be brought fully into view beside the bars.
- **Pointer.** Selecting text and the text cursor apply over the text area. The wheel and trackpad scroll over the lanes too.

### Markdown page

The rendered page (`crates/runner-app/src/ui/markdown/view.rs`) gains the vertical bar. It sits in the page's 20 px margin, so it needs no lane.

Not in this change: the bars of other lists and menus, which still widen; the terminal; and an unwrapped code block's own bar in rendered Markdown, which already keeps its width.

## Sizes

The lane is **10 px**, the width of the bar's strip and of the lane menus keep. With a 4 px thumb that keeps its width, code stays 3 px from it. Sizes are at 100% app zoom and scale with it.

## Design

- **Overflow.** Each lane narrows the other axis, so the two are settled together, in this order, with no dependence on the previous frame:
  1. The vertical lane is needed when the content is taller than the view.
  2. The horizontal lane is needed when the content is wider than the text area left after step 1.
  3. If the horizontal lane is needed and the vertical one was not, the vertical lane is needed when the content is taller than the view less the bottom lane.

  In split, each side has its own text and width and the two scroll together, so "wider" is the side that reaches furthest past its own column; a line that fits its side never makes the other scroll. Lines never wrap, so a narrower area does not change the content's height. "Taller" and "wider" use the scrollbar's own one-pixel threshold, so a lane is never reserved for a bar that does not show.
- **Shared helper.** The lane's width is `app_scrollbar_gutter`. The overflow rule, `ScrollbarLanes::measure`, lives beside it in `crates/runner-app/src/ui/scrollbar.rs`, and both views call it.
- **Fixed width.** `Scrollbar::fixed_width` keeps the app thumb's resting width. Other lists do not use it.
- **Metrics.** Each view's `Metrics` gains the text area's right and bottom edges, set from the rule during prepaint. `text_width`, `scroll_viewport_width`, `shape_viewport_width`, `right_text_width`, `side_right_abs`, `clamp_scroll`, `apply_reveal`, `measure_scrollbars`, the page keys, `current_new_line` and the paint masks use them in place of `bounds`. Row backgrounds keep the full width.
- **Bars.** The vertical bar keeps the view's full height and the corner. The horizontal bar ends at the vertical lane when there is one, and reaches the right edge when there is not.
- **Footer.** The **Mark viewed** footer is clipped to the text area, and its right edge follows the vertical lane instead of the fixed `app_scrollbar_gutter`.
- **Markdown bar.** `Scrollbar::list` reads and moves a GPUI `ListState`, whose rows are measured as they are drawn. Rows not yet drawn get the page's line height as an estimate, given again when the list drops it, after a width change or a reload, and when the text size changes, which the list would otherwise keep the old estimate over. The place in the page stays. The list freezes its height during a drag so the thumb stays under the pointer, and dragging to the end of the track goes to the list's end, since an estimated height would stop short of it.
- **Missed release.** A bar that is unmounted or loses its thumb during a drag misses the release, so the next pointer move with no button held ends the drag. This covers every scrollbar; the Markdown bar also unfreezes its list.

## Implementation steps

1. **Lanes.** The overflow rule, then Code and Diff with the footer.
2. **Bars.** The fixed width, and the Markdown page's bar.
3. **Guide and tests.** The guide pages below and the cases in Verification.

## Guide pages

- `docs/guide/guides/chats-and-windows.md` and its zh-CN page: the viewer's scrollbars and the rendered page's bar.

## Verification

### Automated

- The overflow rule: neither axis, each axis alone, both, and the two cases where one axis scrolls only because of the other's lane; a sub-pixel overflow reserves nothing.
- Code and Diff, unified and split, for neither, each and both axes: the text area ends at the lanes whether the bars are hidden or shown, no thumb reaches it, and the two bars do not meet. In split, the divider does not move.
- Scrolled to the end, the last line, the longest line's last column and the **Mark viewed** footer are inside the text area.
- Lanes scale with the app zoom; the wheel scrolls over a lane, a press on one selects nothing, and a drag into one scrolls.
- The fixed-width thumb keeps its size under the pointer and while dragged, zoomed and not.
- A release missed while the bar is unmounted or has no thumb ends the drag; a line that fits only the left of a split takes no bottom lane; a text size change reaches the rows not yet drawn and keeps the place.
- The Markdown bar covers rows not yet drawn, keeps its width under the pointer, stays off the text, drags to the last row, and appears when a short page grows.
- Gates: `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check` and `git diff --check`.

### Live checks

On the macOS development app, in dark and light, recorded in `docs/tests/runs/1475-viewer-scrollbar-lanes-mac.md`:

1. **Code**: open a file longer and wider than the viewer, then one that overflows only vertically, one only sideways, and one that fits. At rest and hovering, no bar covers the code on either axis and the text does not move; a file that fits has no lane. The thumbs keep their 4 px under the pointer and while dragged. Scroll to both ends, drag both thumbs, drag a selection past the right and bottom edges, and find a match at the end of a long line. Reason: the text area and every scroll limit changed.
2. **Diff**: repeat in unified and split on a changed file with long lines, and reach **Mark viewed** at the end. Row colours reach the right edge, the split divider stays centred, and the footer clears the horizontal bar. Reason: the diff has its own geometry.
3. **Markdown page**: open a long rendered page. The bar shows on hover and while scrolling, keeps its 4 px, covers no text, and dragging it to both ends reaches the first and last lines. Edit the file on disk so it grows, resize the panel, and check the bar again. Reason: the bar is new and its length is estimated.
4. **Zoom**: repeat check 1's first file and check 3's page at 150% app zoom, and with the viewer's text at 8 px and 32 px. Reason: the lane and the bars follow the app zoom and not the viewer's text size.

Selected regression cases: `WS-FILES-03`'s find and drag-selection steps, `WS-CHANGES-02`'s selection and zoom steps `WS-CHANGES-06`'s split step and `WS-FILES-06`, because they cover the changed selection, find and split geometry and the Markdown page's reload. `WS-SCROLL-01`'s lists share the changed scrollbar, so its hover, drag and hide steps on the sidebar and an open select are included.
