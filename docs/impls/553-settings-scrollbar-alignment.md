# 553 — Settings content scrollbar hugs the window edge

Tracking issue: [#553](https://github.com/yicheng47/runner/issues/553). Bug, P2. Branch: **`fix/553-settings-scrollbar-alignment` already exists and is checked out** — it carries this brief; work on it, do not create another.

## What is wrong

In Settings the content column's scrollbar is painted at the window's right edge, from the very top of the window (behind the titlebar drag strip) to the bottom. The content it scrolls is a centered 760 px column with 40 px side padding, so on a 2000 px window the thumb sits 200 px or more from the cards. It reads as a stray bar on the window border. Every pane has it because it is the shared content column.

## What ships

The thumb aligns to the right edge of the centered content column (the card edge) and starts below the titlebar drag area, exactly as the list pages already do: the same thin `ScrollbarKind::App` overlay, no new widget, no change to how the content scrolls (it still scrolls under the drag strip). Zoom and rem size keep the alignment.

## Where the code is

- `crates/runner-app/src/surfaces/settings_page.rs:906-943` — the content column: a `relative` flex-1 wrapper holding (a) the titlebar drag area, absolute, `h(TITLEBAR_DRAG_HEIGHT * app_zoom)`; (b) `#settings-content-scroll`, `size_full overflow_y_scroll scrollbar_width(0)` with `px(40) pt(56) pb(64)` and the `mx_auto w_full max_w(760)` child; (c) `content_scrollbar` as a sibling of the whole column, so it spans the column.
- `crates/runner-app/src/ui/scrollbar.rs:255-262` — `Scrollbar` renders `absolute top_0 right_0 bottom_0 w(gutter)` inside whatever `relative` parent it is given; its track height is its own bounds (`:242`), so an inset parent gives a shorter track and the thumb math follows. `ScrollbarKind::App` gutter is 10 (`:26`). Do not change this file.
- `crates/runner-app/src/ui/list.rs:580-596` — the pattern that is right: `PaginatedListPage` puts the `relative` wrapper *inside* the padded column, scroll div and scrollbar as siblings in it, so the thumb overlays the content's right edge, not the page's.
- `crates/runner-app/src/surfaces/app_shell.rs:7` — `TITLEBAR_DRAG_HEIGHT = 28.` (already imported in `settings_page.rs:21`).
- `crates/runner-app/src/surfaces/settings_page.rs:2050-2115` — the existing `VisualTestContext` test: `window.resize`, `set_rem_size` for 16 and 20.8, `debug_bounds("SETTINGS_…")` selectors attached with `.debug_selector(|| "…".into())` at `:411-441`.
- Out of scope: the Skills detail takeover's document scrollbar (`settings/skills.rs:1040-1063`) lives inside a bordered card and is already aligned; the nav column's `nav_scrollbar` is fine.

## Fix shape

Keep the scroll container exactly as it is and give the scrollbar its own layer that mirrors the content column's geometry:

1. In `settings_page.rs` replace `.child(self.settings_page.content_scrollbar.clone())` at `:943` with an overlay: `div().absolute().top(px(TITLEBAR_DRAG_HEIGHT * zoom)).bottom_0().left_0().right_0().px(rems(40. / 16.))` containing `div().relative().mx_auto().w_full().max_w(rems(760. / 16.)).h_full().child(content_scrollbar)`. The side padding and max width must be the same values the scroll container uses — pull them into two consts next to `TITLEBAR_DRAG_HEIGHT`'s import (`SETTINGS_CONTENT_PADDING_X`, `SETTINGS_CONTENT_MAX_WIDTH`) and use them in both places so they cannot drift.
2. The overlay carries no `id`, cursor, or handlers, so it gets no hitbox and stays transparent to wheel and click; only the scrollbar inside has one. Do not add `overflow_hidden` or a background to it.
3. Add `debug_selector`s: `SETTINGS_CONTENT_COLUMN` on the `mx_auto` content wrapper inside the scroll container and `SETTINGS_CONTENT_SCROLLBAR_TRACK` on the overlay's inner `relative` div (the scrollbar entity's own root is not reachable from the page).
4. Nothing else moves: same `pt(56)`, content still scrolls under the drag strip, thumb colours and width unchanged.

## Rules of the road

- No change to `ui/scrollbar.rs`, `ui/list.rs`, or any pane.
- Keep the macOS and Windows platform chrome untouched; this is shared content.
- Do not launch the app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-app`, `make clippy` (also with `--features updater`), `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR, drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents.

## Tests

One `VisualTestContext` test in `settings_page.rs` beside the existing one, on a pane that scrolls (General or Missions is fine; make the window short enough that the content overflows, e.g. 1400×500). For each of window widths 1000 and 2000 and rem sizes 16 and 20.8:

- the track's right edge equals the content column's right edge, within 0.5 px;
- the track's left edge equals the column's right edge minus the 10 px gutter scaled by rem;
- the track's top equals `TITLEBAR_DRAG_HEIGHT * zoom` and its bottom equals the column wrapper's bottom;
- at 2000 px the column is 760 px × rem/16 wide and centered; at 1000 px it is the window's content width minus twice the padding.

The existing settings tests keep passing.

## Jason's smoke test (after landing)

1. Settings → General on a window 2000 px wide: the thumb sits on the right edge of the cards, not the window edge, and starts below the traffic-light strip.
2. Narrow the window to about 900 px: the thumb follows the card edge inward and stays 40 px from the window edge.
3. Zoom to 150 % (View → Zoom In): still aligned.
4. Wheel-scroll with the pointer over the cards' right edge and over the empty margin: both scroll; the thumb drags.

## Non-goals

Changing where the content scrolls to, the scrollbar look, the nav column, or the Skills detail card.
