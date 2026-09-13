# GPUI as the terminal renderer uses it

Written against `gpui-ce` 0.3.3, the crates.io package Runner pins as `gpui`. Source: `~/.cargo/registry/src/*/gpui-ce-0.3.3/src/`. This note covers the slice of GPUI the terminal element touches; it is not a general GPUI tutorial.

GPUI is Zed's UI framework. The comparison that helps most from React: state lives in retained entities, but the element tree is not diffed. It is rebuilt from scratch every frame and thrown away. There is no virtual DOM. Layout is taffy, a Rust flexbox implementation. Painting produces a list of GPU primitives, and Metal draws them on macOS.

## The frame

Read `window.rs`, the `draw` function, then `draw_roots` right below. One frame is:

1. The root view's `render` builds the element tree.
2. `prepaint_as_root` runs `request_layout` on every element to register taffy nodes, computes the layout, then calls `prepaint` on every element with its final bounds. Hitboxes are inserted during this pass.
3. The window computes the mouse hit test against the new hitboxes.
4. `paint` runs on every element. Elements push primitives into the scene and register mouse listeners and input handlers.
5. The frame is finished: the text layout cache rolls over, the new frame swaps with the rendered one, focus-change listeners fire, and the window is flagged to present, which hands the scene to the platform window.

Frames are on demand, not on a timer. Something calls `notify` on an entity, the invalidator marks the window dirty, and the next display-link tick draws. If nothing is dirty, nothing draws. That is the whole reason Runner's terminal waker exists: the PTY thread cannot call `notify`, so it sends on a channel and a GPUI task does the notify.

Every paint-phase API asserts the phase in debug builds (`debug_assert_paint`, `debug_assert_prepaint`). Calling `paint_quad` from prepaint or `insert_hitbox` from paint panics in a dev build, which is the fastest way to learn the contract.

## The Element trait

`element.rs`. Three methods with typed state threaded forward: `request_layout` returns a `LayoutId` and some state, `prepaint` receives the bounds and returns more state, `paint` receives both. The module docs say most code should use `div` and the built-in elements, and to implement `Element` only when you need manual control, with a code editor as the example. A terminal is the same case: the grid is not flexbox. So Runner's `TerminalElement` asks taffy for one node that fills its parent, and does everything else by hand.

`request_layout` on the window takes a `Style` and child layout ids and registers a taffy node. `relative(1.)` for both dimensions is "fill the parent".

## The Scene

`scene.rs`. The scene is a struct of `Vec`s, one per primitive kind: `Shadow`, `Quad`, `Path`, `Underline`, `MonochromeSprite`, `PolychromeSprite`, `Surface`. A quad is a rectangle with a background, border, corner radii, and a content mask. A monochrome sprite is a glyph from the atlas tinted one colour. A polychrome sprite is an image or a colour emoji. A path is a tessellated shape. A surface is a platform video or camera layer.

Every `insert_primitive` records the primitive's bounds in a bounds tree to assign a draw order, so overlapping primitives keep painter's order even though they are stored by kind. `batches` then walks the order and yields maximal runs of the same kind. The Metal renderer in `platform/mac/metal_renderer.rs`, in `draw_primitives` under `draw`, does one instanced draw call per batch. Paths are the exception: they are drawn to an intermediate texture first and then composited, which is the end-encoding dance in that loop.

A terminal frame therefore compiles to roughly: one background quad, a batch of merged background quads, selection quads, one large batch of glyph sprites, a path batch for box drawing, and underlines. Runner's quad merging in prepaint is about keeping those batches short.

## Text

`text_system.rs` and `text_system/line.rs`. The steps:

- `resolve_font` turns a `Font` description into a `FontId` through the platform text system, CoreText on macOS.
- `em_advance` returns the width of the letter m at a size. For a monospace font that is the cell width. Runner uses it as its column unit.
- `shape_line` takes text, a size, and a slice of `TextRun`s. A `TextRun` is a byte length plus font, colour, underline, strikethrough, and background. It merges adjacent runs with identical decoration, then calls `layout_line`.
- `layout_line` asks the platform to shape the text into a `LineLayout`: ascent, descent, width, and runs of `ShapedGlyph`, each with a glyph id, an x position, and its byte index in the text.
- `ShapedLine::paint` computes the baseline from ascent and descent centred in the line height, then walks every glyph and calls `paint_glyph` on the window, opening and closing underline and strikethrough runs as the decoration changes.
- `paint_glyph` in `window.rs` picks a subpixel variant from the fractional origin, rasterizes the glyph through the platform if the atlas does not already have it, and pushes a `MonochromeSprite` pointing at the atlas tile. macOS gets four by four subpixel variants, which is why text looks crisp at fractional positions.

### The layout cache

`text_system/line_layout.rs`. `LineLayoutCache` keeps two frames of layouts keyed by text, font size, font runs, and forced width. A lookup checks the current frame, then moves a hit from the previous frame forward, and only shapes on a miss. `finish_frame` swaps the two and clears the old one. So a line that was on screen last frame costs a hash lookup this frame. Runner shapes every non-ASCII cell as its own one-character line, and this cache is what makes that cheap: a column of box-drawing characters is one shape call and hundreds of hits.

This also explains Runner's alignment rule. `layout_line` positions glyphs by the font's real advances. When a character falls back to a different font, its advance differs, and every glyph after it in the same shaped line drifts off the cell grid. Runner avoids the drift by never putting a fallback glyph and an ASCII glyph in the same line. Each non-ASCII cell is positioned at its column origin by Runner, not by the shaper. `layout_line` has a `force_width` option that snaps glyph x to multiples of a width when the shaper's position is more than a pixel off, but Runner does not use it.

## Paths

`path_builder.rs` wraps lyon. You build a path with SVG-style commands, and `fill` or `stroke` tessellates it on the CPU into triangles. `paint_path` on the window scales it by the device scale factor, attaches the current content mask and opacity, and inserts a `Path` primitive. Runner's `glyphs.rs` uses this for the box-drawing shapes that are not plain rectangles. Plain rectangles go through `paint_quad`, which is cheaper and batches with everything else.

## Content masks

`with_content_mask` pushes a clip rect intersected with the current one for the duration of a closure. Every primitive carries the mask at insert time, and the shader discards pixels outside it. Runner clips each procedural glyph and the cursor glyph to its cell this way.

## Hitboxes and mouse

`insert_hitbox` runs in prepaint and hands back a fresh id every frame, pushed onto the next frame's hitbox list. After prepaint the window runs a hit test at the mouse position and stores the hovered ids; `is_hovered` is a membership check against that list. This is the contract behind the comment in Runner's prepaint that `is_hovered` is always false there: the hit test has not run yet for the ids just inserted.

`on_mouse_event` runs in paint and registers a boxed listener for this frame only. Dispatch has a `Capture` phase from root to leaf, then `Bubble` from leaf to root. Runner uses `Bubble` for mouse down so a listener nearer the leaf can claim the click first, and `Capture` for move and up so an in-progress drag keeps receiving events even when the pointer leaves the element. `on_modifiers_changed` is the same shape and lets the link underline follow the modifier key without the mouse moving.

`set_cursor_style` takes the hitbox so the style applies only while that hitbox is hovered. `set_tooltip` registers a tooltip view with a visibility check the window re-runs each frame.

## IME input

`handle_input` runs in paint and, if the focus handle is focused, registers an input handler with the platform window. On macOS that becomes the `NSTextInputClient` the system IME talks to. The trait in `input.rs` is `EntityInputHandler`: text for range, selected range, marked range, unmark, replace text in range, replace and mark text in range, bounds for range, character index for point. `ElementInputHandler` adapts an entity implementing that trait plus the element's bounds.

Runner's `TerminalInput` implements it. Pinyin composition works because the platform calls `replace_and_mark_text_in_range` with the in-progress candidate, Runner stores it as marked text, the element paints it at the cursor cell with an underline, and `replace_text_in_range` on commit sends the final text to the PTY. `bounds_for_range` is what positions the candidate window under the cursor cell.

## Entities

`Entity<T>` in `app/entity_map.rs` is a retained, reference-counted handle to state that outlives frames. `update` takes a closure with the state and a `Context`, and `notify` on that `Context` marks the owning view dirty. `read` borrows it. `TerminalInteraction` and `TerminalInput` are entities so drag state and IME composition survive while the element itself is rebuilt each frame. `cx.spawn` on a context starts a task that can `update` a weak handle later, which is how the autoscroll timer and the link tooltip delay are driven.

`Window::refresh` marks the whole window dirty without naming a view; Runner uses it from the tooltip delay task.

## Where the two libraries meet

The `Term` is the source of truth. Once per frame, under the fair lock, the element asks for `renderable_content` and walks every visible cell into GPU primitives. Nothing is diffed and alacritty's damage tracking is not used. The work is bounded by columns times rows, shaping is cached, and quads merge, so a full walk is cheaper than tracking what changed.

## Reading order

1. `element.rs`: the first 120 lines, module docs and the `Element` trait.
2. `window.rs`: `draw`, `draw_roots`, `paint_quad`, `paint_glyph`, `insert_hitbox`, `on_mouse_event`, `handle_input`, `with_content_mask`.
3. `scene.rs`: `Scene`, `insert_primitive`, `batches`.
4. `text_system.rs`: `shape_line`, `TextRun`, `em_advance`; then `text_system/line.rs` `paint_line` and `text_system/line_layout.rs` `layout_line` and `finish_frame`.
5. `input.rs`: `EntityInputHandler`.
6. `platform/mac/metal_renderer.rs`: the batch loop in `draw_primitives`, to see the primitives become draw calls.
