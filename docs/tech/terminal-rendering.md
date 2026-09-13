# Terminal Rendering in Runner

How a byte written by an agent CLI becomes pixels in a pane. This note follows the code as it stands at v0.8.7; function names are stable pointers, line numbers are not, so the text names functions and files only.

The note assumes you know what a PTY is, how an escape sequence is shaped, and why a terminal is a grid of cells. If any of that is new, start with the [background reading](./README.md#background-reading).

## The mental model

A terminal is a fixed grid of cells, not a text box. The child process writes a byte stream. Most bytes are printable characters that land in the cell under the cursor. Some are escape sequences that move the cursor, set colors, clear regions, or switch screens. An emulator parses the stream and keeps the grid current. A renderer paints the grid. Runner writes no emulator: it embeds alacritty's and writes only the renderer and the glue between the two. [`alacritty-terminal.md`](./alacritty-terminal.md) covers the emulator; [`gpui-rendering.md`](./gpui-rendering.md) covers the framework the renderer is written against.

Three jobs, split across two crates:

- Emulation: `alacritty_terminal`, wrapped by `crates/runner-terminal`.
- Painting: the custom GPUI element in `crates/runner-app/src/terminal/`.
- Input: the encoders in `crates/runner-terminal/src/mappings.rs`, IME routing in `crates/runner-app/src/terminal_ime.rs`, and the mouse handling at the top of the element file.

```
child ──► PTY master ──► reader thread ──► SessionManager ──► TerminalBridge ──► TerminalSession
 (tty)                   (blocking read)   (stamps seq)       (SessionEvents)     └─ Term (alacritty)
                                                                                   └─ waker ──► AppStore ──► cx.notify()
                                                                                                                  │
                                                             pane render ──► TerminalElement ──► prepaint/paint ◄─┘
                                                                                     │
                                                              keys / IME / mouse ────┴──► mappings ──► PTY writer
```

## The pipeline, file by file

1. **PTY read.** `crates/runner-backend/src/session/pty_runtime.rs`, the reader loop. One blocking OS thread per session calls `read` into a buffer and forwards each chunk as a stream message. The same loop feeds the idle detector, with a grace window after a resize so a SIGWINCH repaint does not count as activity.
2. **Manager forwarder.** `crates/runner-backend/src/session/manager/output.rs`, `ingest_output_chunk` and `record_output`. Stamps a per-session monotonic `seq` and calls the installed `SessionEvents` observer with an `OutputEvent` carrying session id, mission id, seq, and bytes.
3. **TerminalBridge.** `crates/runner-terminal/src/terminal.rs`, the `SessionEvents` impl on `TerminalBridge`. The bridge is that observer. It keeps one `TerminalSession` per live session id in a map. `spawned` creates or replaces one with the spawn's cols and rows; `exit` and `archived` drop one; `output` looks one up and feeds it. If output arrives before the spawn event, the bridge creates the session on the spot from the persisted last size, defaulting to 80×24. `set_palette` fans a theme change out to every live session.
4. **feed_output.** `TerminalSession::feed_output` in the same file. This is where bytes become grid. It locks alacritty's VTE `Processor` and the `Term`, then calls `advance`. That one call is the entire emulator. Everything around it is bookkeeping: skip a chunk whose seq is not newer than the last; record to the fixture recorder if `RUNNER_RECORD_INPUT_FIXTURE` is set; scan for the DEC 2031 colour-scheme sequences alacritty ignores and feed the parser in pieces so each takes effect at its place in the stream; note the seq of the latest alt-screen or bracketed-paste enable as `tui_ready_seq`; record `first_paint_seq` once any visible cell is non-whitespace; drop the selection when mouse mode turns on; hand the grid to the input-state detector; and finally wake GPUI, but only if `viewers > 0`.
5. **Waker.** `crates/runner-app/src/app_store.rs`, `AppStore::new`. The waker is a closure that sends a unit on an unbounded channel. A GPUI task drains it, coalesces every wake that lands within 4 ms of the first, bumps `revisions.terminal_wake`, and calls `cx.notify()`. Output never pushes pixels. It mutates a `Term` and rings a bell.
6. **Paint.** On the next frame the pane's render function builds a fresh `TerminalElement` and GPUI drives it through the `Element` trait. Construction sites: `crates/runner-app/src/surfaces/panes.rs` for chat panes and the drawer, `crates/runner-app/src/surfaces/mission_workspace.rs` for mission slots.

## The TerminalSession

`TerminalSession` is one live terminal. Its fields are the whole state of a pane's screen:

- `term`: the alacritty `Term` behind a `FairMutex`. The PTY feed thread and the GPUI paint thread both take it; the fair variant stops a chatty agent from starving the renderer. Cheap reads such as `mode()` and hit testing use `lock_unfair`.
- `parser`: the VTE `Processor`, which carries partial-sequence state between chunks.
- `size`: the last pushed cols and rows, the value `prepaint` compares against.
- `palette`: the active `TerminalPalette` plus its expanded 256-entry base table.
- `scheme`: DEC 2031 subscription state and the byte tail carried across chunk boundaries.
- `viewers`: an atomic count of panes currently showing this terminal.
- `user_input`: `Inline` for direct chats, `Queued` for mission slots. Queued input goes through a per-session thread so a delivery waiting on the draft gate can never park GPUI's render thread; a failure surfaces as a `session/input-error` app event.
- `input_tracker` and `fixture_recorder`: the composer-state detector and the optional fixture capture.

`attach_with_input_mode` builds all of this and spawns a per-session event thread. That thread receives alacritty's `Event`s through the `EventProxy` listener: `PtyWrite` answers to queries the TUI sent, `ColorRequest` resolves a palette index through `query_color_for`, `TextAreaSizeRequest` reports the current size, and `Title` updates the stored title. It also rewrites the one DECRQM answer alacritty gets wrong for mode 2031, reporting set or reset from the session's own subscription flag.

`view()` returns a `TerminalView` guard that increments `viewers` and decrements on drop. A hidden terminal keeps ingesting but never wakes GPUI. The chat surface holds a lease for every session on the active tab and drops the rest in `ensure_owned_active_tab_attached`; the mission workspace holds one per attached slot.

## The Element trait, three phases per frame

GPUI runs every element through three phases each frame. `TerminalElement` implements all three in `crates/runner-app/src/terminal/element.rs`:

- `request_layout` asks taffy for one node that fills its parent and nothing more. The grid is not flexbox; everything else is done by hand.
- `prepaint` does all the work. It computes cell geometry, locks the `Term`, walks every visible cell, and fills a `GridPrepaint` struct: background quads, selection quads, procedural cells, shaped text lines, the cursor, and any IME marked text.
- `paint` is deliberately dumb. Full background, background quads, selection quads, procedural glyphs, text lines, cursor on top. Then it registers the IME input handler, picks the mouse cursor style, and registers the mouse listeners.

### prepaint, step by step

1. **Geometry.** Cell width is the em advance of the configured mono font at the configured size, with a fallback of six tenths of the size if the font cannot be measured. Line height is the size times `LINE_HEIGHT_FACTOR` (1.4), rounded. A hitbox covering the bounds is inserted so mouse listeners can test hover.
2. **Resize.** `terminal_grid_size` floors the bounds into cols and rows, minimum two each, or `None` when the element has no size yet. `size_push_verdict` in `crates/runner-app/src/terminal_resize.rs` compares that against the session's last pushed size: unchanged is a no-op, a non-owner pane never pushes, and an unplaced pane returns an empty prepaint so nothing is painted from a zero-size layout. A real push calls `TerminalSession::resize`, which fires the PTY ioctl through `session_resize` and reflows the `Term` in the same call, every frame of a drag. Nothing clears the grid; the TUI's SIGWINCH repaint plus alacritty's reflow is the whole story.
3. **Hover.** The hovered link is refreshed from the mouse position. Hitbox ids are fresh every frame and the hit test runs after prepaint, so `is_hovered` is always false here; the code tests the bounds directly, as GPUI's own tooltips do. If a link has been hovered for 500 ms a tooltip is registered.
4. **The cell walk.** Under the `Term` lock, `renderable_content` yields the visible cells, the display offset, the cursor, the selection, the colour overrides, and the mode. For each cell:
   - Map the grid point to a viewport row and column with `point_to_viewport`; skip anything off screen.
   - If the cell is selected, emit a selection quad, merged into the previous one when adjacent.
   - Skip wide-char spacer cells; the glyph cell before them is painted two columns wide.
   - Resolve foreground and background through `palette::resolve_for`, swap them for `INVERSE`, dim the foreground for `DIM`.
   - Emit a background quad only when the colour differs from the pane background, merged with the previous quad when the colour matches and the rectangles touch. This keeps the GPU batch short.
   - Skip hidden cells and plain spaces.
   - Try `ProceduralCell::new` first. Box drawing, block elements, and braille from U+2500 up are painted from geometry, never from a font, unless the cell carries an underline or strikeout. A block cursor on such a cell gets a second procedural cell in cursor colours.
   - Otherwise build a `TextRun`: bold and italic from the flags, the accent colour when a link is armed by its modifier, an underline for links and the five underline flags, a strikethrough for `STRIKEOUT`. Zero-width combining characters are appended to the cell's text.
   - A block cursor on a text cell gets its own shaped line in the cursor accent colour so the glyph stays readable over the cursor quad.
   - Extend the open span only when both the span so far and this cell are plain narrow ASCII. Anything else starts a new span at its own column. See the alignment decision below.
5. **Cursor.** The cursor point is mapped to the viewport; if visible, its bounds are recorded, one or two columns wide, and its shape is kept unless it is `Hidden`.
6. **Shaping.** Every span is shaped with `shape_line` into a `ShapedLine`, positioned at its column origin.
7. **Marked text.** If the IME has a composition in progress, it is shaped in the pane foreground colour with an underline and will paint over the cursor cell.

### paint

Order matters because there is no depth buffer, only painter's order. Background first, then the merged background quads, then selection, then procedural glyphs, then text, then either the marked text over the cursor cell or the cursor itself. A focused block cursor paints a filled quad and then the cell's glyph clipped to the cursor bounds; an unfocused one paints an outline. Underline and beam cursors are thin quads scaled by app zoom.

After painting, `handle_input` registers the `TerminalInput` entity as the IME handler with the cursor cell as its bounds, `set_cursor_style` picks pointing hand over an armed link, crosshair with Alt held, arrow when the TUI owns the mouse, and I-beam otherwise, and `register_mouse_listeners` installs this frame's listeners.

## Procedural glyphs

`crates/runner-app/src/terminal/glyphs.rs` and the generated `glyph_data.rs`. Box drawing and block elements drawn from fonts leave gaps and misaligned joins at some sizes. The glyph table is ported from xterm.js's webgl addon and describes each of its codepoints as one or more parts: octant rectangles on an 8×8 sub-grid, a pattern with a flat alpha, an SVG-style path filled or stroked, a dynamic path with per-cell substitutions, or a negative path cut out of a filled cell. Braille is eight dots computed directly from the codepoint bits.

`snapped_cell_bounds` rounds every cell edge to device pixels so adjacent cells' lines meet. Axis-aligned strokes are painted as quads; anything else goes through GPUI's `PathBuilder`. Pattern coverage is preserved as alpha instead of a device-pixel dither so the shade survives app zoom without moiré.

## Four decisions worth understanding

1. **Column alignment.** GPUI's text shaper positions glyphs by the font's real advances and does not know about cells. Shape a whole row as one string and a fallback font's advance skews every column after it. So a span only grows while both it and the new cell are plain ASCII, where byte length equals column count and the monospace advance is exact. Wide chars, CJK, emoji, and box drawing each get their own span painted at their own column origin. GPUI's per-frame layout cache makes the per-cell shaping cheap: a column of identical glyphs is one shape call and many cache hits.
2. **Procedural glyphs.** Painting borders from geometry is what makes TUI frames look clean at any zoom, on any font. It also removes the dependence on the Nerd Font having every box-drawing codepoint.
3. **Wake gating.** A hidden terminal keeps ingesting but never wakes GPUI; only a pane holding a `TerminalView` lease does. Combined with the 4 ms coalescing in the app store, this is what stops a chatty agent from forcing hundreds of frames a second across every hidden tab.
4. **Resize ownership.** Only the pane that owns the terminal's size pushes it, so two panes showing the same session cannot fight over the PTY size. The push is immediate and synchronous, every frame of a drag. A 175 ms settle thread in the session manager persists the final size once per storm.

## Input, the other direction

- `crates/runner-terminal/src/mappings.rs` holds the pure encoders. Terminal mode goes in, bytes come out: `encode_key` for keys under the various keypad and kitty modes, `encode_scroll` for wheel events when the TUI wants them, `encode_mouse_press`, `encode_mouse_release`, and `encode_mouse_motion` for mouse reporting, and `encode_paste` for bracketed paste. `classify_key` labels a key as content, edit, submit, cancel, or navigate for the input-state detector.
- `crates/runner-app/src/terminal_ime.rs` routes each key. Platform-modified keys are app shortcuts, keys during a composition go to the IME, control, alt, function, and named special keys go raw to the PTY, and everything else goes to the IME so Pinyin composition works. `TerminalComposition` holds the marked text with UTF-16 selection math for the platform.
- `TerminalInteraction` at the top of the element file handles the mouse. On mouse down: a modifier-click on a link starts a link drag; if the TUI has enabled mouse mode and Shift is not held, the press is encoded and sent; otherwise left starts a selection (simple, semantic on double click, lines on triple, block with Alt, extended with Shift) and right starts a semantic selection on a non-whitespace cell. On move: link drags note movement, reported drags send motion, local drags update the selection and start autoscroll at the edges. On up: an unmoved link click opens the URL or file, a reported release is encoded, and a quick unmoved Alt-click moves the cursor to that cell. A 50 ms task drives autoscroll while a drag sits outside the bounds.
- The pane wraps the element in a div with the `Terminal` key context, focus tracking, `on_key_down`, copy and paste actions, and a scroll-wheel handler only when the route allows scrolling. The same `scrollable` flag, combined with the layout's `is_resize_owner` check in the multi-pane path, decides whether the pane is the resize owner. Overlay states fade the terminal while a session starts, resumes, or has ended.

## Reading order

1. `TerminalSession::feed_output`, about 70 lines.
2. The cell loop inside `TerminalElement::prepaint`.
3. `TerminalElement::paint`.
4. `glyphs.rs` from `paint_part` down. Skip `glyph_data.rs`; it is a generated table.
5. `TerminalInteraction::mouse_down` for the mouse model.
6. The pane's render function in `panes.rs` to see how the element is hosted.
