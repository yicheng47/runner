# Terminal Rendering in Runner

How a byte written by an agent CLI becomes pixels in a pane. This note follows `main` as checked on 2026-10-22, with daemon-owned terminal models and app-side mirrors; function names are stable pointers, line numbers are not, so the text names functions and files only.

The note assumes you know what a PTY is, how an escape sequence is shaped, and why a terminal is a grid of cells. If any of that is new, start with the [background reading](./README.md#background-reading).

## The mental model

A terminal is a fixed grid of cells, not a text box. The child process writes a byte stream. Most bytes are printable characters that land in the cell under the cursor. Some are escape sequences that move the cursor, set colors, clear regions, or switch screens. An emulator parses the stream and keeps the grid current. A renderer paints the grid. Runner writes no emulator: it embeds alacritty's and writes only the renderer and the glue between the two. [`alacritty-terminal.md`](./alacritty-terminal.md) covers the emulator; [`gpui-rendering.md`](./gpui-rendering.md) covers the framework the renderer is written against.

Three jobs span the daemon, terminal library and app:

- Process IO and terminal ownership: `crates/runner-daemon/src/session/`.
- Emulation and snapshots: `alacritty_terminal`, wrapped by `crates/runner-terminal`.
- Painting and interaction: `crates/runner-app/src/terminal/`, with pure encoders in `runner-terminal/src/mappings.rs` and IME routing in `runner-app/src/terminal/ime.rs`.

```text
child → PTY reader → daemon SessionManager forwarder
                         ├─ numbered bytes → client queue → app TerminalMirror → viewport → GPUI
                         └─ authoritative TerminalModel → draft observations / terminal replies

keys / IME / mouse → app encoders → client input frame → daemon ordered input worker → PTY
```

The daemon keeps parsing when the app is closed. App attachment restores a snapshot and subscribes to the bytes after its sequence number; reconnecting does not start another child. The [process model](../arch/process-model.md) explains workers, ownership and lifecycle in detail.

## The pipeline, file by file

1. **PTY read.** [`session/pty_runtime.rs`](../../crates/runner-daemon/src/session/pty_runtime.rs) runs a blocking reader per session. Its output channel feeds the manager forwarder; terminal activity also feeds the runtime's idle detector.
2. **Manager forwarder.** [`session/manager/output.rs`](../../crates/runner-daemon/src/session/manager/output.rs), `ingest_output_chunk`, increments the session sequence, publishes a `TerminalFrame::Output` to bounded client queues and feeds the authoritative model under the same state lock. Publishing bytes first allows the app and daemon parsers to run concurrently. Input observations are published after the state lock is released.
3. **Authoritative parsing.** [`terminal/model.rs`](../../crates/runner-terminal/src/terminal/model.rs), `TerminalModel::feed_output`, locks the parser and `Term`, then advances the VTE processor. It also handles synchronized-update deadlines, shell OSC 7 cwd reports, DEC 2031 colour-scheme sequences, fixture recording and input observations. Its event worker answers terminal queries through `TerminalHost`; the app never answers them a second time.
4. **App mirror.** [`terminal.rs`](../../crates/runner-terminal/src/terminal.rs), `TerminalMirror::attach`, restores the snapshot, including unfinished parser input, and starts a frame reader. `feed_output` rejects duplicate sequence numbers and advances a separate VTE processor and `Term`. Overflow requests a new snapshot rather than blocking the agent. `TerminalBridge` owns the mirrors and routes session events and palette changes.
5. **Wakes.** [`app_store.rs`](../../crates/runner-app/src/app_store.rs) receives session IDs from the bridge, coalesces them over 4 ms and emits `TerminalUpdates` for the changed terminals. [`terminal/viewport.rs`](../../crates/runner-app/src/terminal/viewport.rs) subscribes by session ID and notifies its own entity. Title or cwd changes also notify the app store; ordinary terminal output does not require rerendering the whole shell.
6. **Paint.** The pane creates a `TerminalElement` for its mirror. Construction sites include [`surfaces/panes/`](../../crates/runner-app/src/surfaces/panes/) for chats and drawers and [`surfaces/mission_workspace/`](../../crates/runner-app/src/surfaces/mission_workspace/) for mission terminals. GPUI drives its layout, prepaint and paint phases.

## Model and mirror

`TerminalModel` in the daemon owns the authoritative grid, parser, sequence, palette, input tracker, cwd reports and optional fixture recorder. It reports title and input changes through `TerminalHost` and answers terminal queries even when no app is attached. A snapshot and the subsequent subscription are captured atomically at the manager's sequence boundary.

`TerminalMirror` in the app owns another grid and parser, its subscriber ID, local selection and display offset, metadata, palette and viewer count. Its `FairMutex<Term<EventProxy>>` lets the frame reader and painter share the grid without a chatty stream starving the renderer. Cheap mode and hit-test reads use `lock_unfair`.

The mirror discards query replies. User keys, text, paste and composition observations go back through `DaemonClient`; both chat and mission input use the daemon's ordered input path. The daemon's delivery gate and writer serialization remain authoritative.

`view()` returns a `TerminalView` lease that increments the viewer count and decrements it on drop. Hidden attached mirrors keep parsing, but only visible viewers wake the UI. The authoritative model's parsing and status work do not depend on those leases.

## The Element trait, three phases per frame

GPUI runs every element through three phases each frame. `TerminalElement` implements all three in `crates/runner-app/src/terminal/element.rs`:

- `request_layout` asks taffy for one node that fills its parent and nothing more. The grid is not flexbox; everything else is done by hand.
- `prepaint` does all the work. It computes cell geometry, locks the `Term`, walks every visible cell, and fills a `GridPrepaint` struct: background quads, selection quads, procedural cells, shaped text lines, the cursor, and any IME marked text.
- `paint` is deliberately dumb. Full background, background quads, selection quads, procedural glyphs, text lines, cursor on top. Then it registers the IME input handler, picks the mouse cursor style, and registers the mouse listeners.

### prepaint, step by step

1. **Geometry.** Cell width is the em advance of the configured mono font at the configured size, with a fallback of six tenths of the size if the font cannot be measured. Line height is the size times `LINE_HEIGHT_FACTOR` (1.4), rounded. A hitbox covering the bounds is inserted so mouse listeners can test hover.
2. **Resize.** `terminal_grid_size` floors the bounds into cols and rows, minimum two each, or `None` when the element has no size yet. `size_push_verdict` in `crates/runner-app/src/terminal/resize.rs` compares that against the session's last pushed size: unchanged is a no-op, a non-owner pane never pushes, and an unplaced pane returns an empty prepaint so nothing is painted from a zero-size layout. A real push calls `TerminalMirror::resize`, which reflows the local grid immediately and sends a one-way resize frame. The daemon applies the PTY/model size and informs the other subscribers. Nothing clears the grid; the TUI's SIGWINCH repaint plus alacritty's reflow is the whole story.
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
3. **Wake gating.** A hidden terminal keeps ingesting but never wakes GPUI; only a pane holding a `TerminalView` lease does. Together with 4 ms coalescing and session-scoped viewport notifications, this bounds painting to visible terminals without stopping daemon parsing.
4. **Resize ownership.** Only the pane that owns the terminal's size pushes it, so two panes showing the same session cannot fight over the PTY size. Local reflow is immediate; the IPC resize is one-way. The session manager applies the PTY size and persists settled geometry after a resize storm. The applied-size mirror follow-up in [#986](https://github.com/yicheng47/runner/pull/986) is on `nightly-remote`, outside this `main` walkthrough.

## Input, the other direction

- `crates/runner-terminal/src/mappings.rs` holds the pure encoders. Terminal mode goes in, bytes come out: `encode_key` for keys under the various keypad and kitty modes, `encode_scroll` for wheel events when the TUI wants them, `encode_mouse_press`, `encode_mouse_release`, and `encode_mouse_motion` for mouse reporting, and `encode_paste` for bracketed paste. `classify_key` labels a key as content, edit, submit, cancel, or navigate for the input-state detector.
- `crates/runner-app/src/terminal/ime.rs` routes each key. Platform-modified keys are app shortcuts, keys during a composition go to the IME, control, alt, function, and named special keys go raw to the PTY, and everything else goes to the IME so Pinyin composition works. `TerminalComposition` holds the marked text with UTF-16 selection math for the platform.
- `TerminalInteraction` at the top of the element file handles the mouse. On mouse down: a modifier-click on a link starts a link drag; if the TUI has enabled mouse mode and Shift is not held, the press is encoded and sent; otherwise left starts a selection (simple, semantic on double click, lines on triple, block with Alt, extended with Shift) and right starts a semantic selection on a non-whitespace cell. On move: link drags note movement, reported drags send motion, local drags update the selection and start autoscroll at the edges. On up: an unmoved link click opens the URL or file, a reported release is encoded, and a quick unmoved Alt-click moves the cursor to that cell. A 50 ms task drives autoscroll while a drag sits outside the bounds.
- The pane wraps the element in a div with the `Terminal` key context, focus tracking, `on_key_down`, copy and paste actions, and a scroll-wheel handler only when the route allows scrolling. The same `scrollable` flag, combined with the layout's `is_resize_owner` check in the multi-pane path, decides whether the pane is the resize owner. Overlay states fade the terminal while a session starts, resumes, or has ended.

## Reading order

1. `TerminalModel::feed_output` in `terminal/model.rs`, then `TerminalMirror::attach`, `start_frames` and `feed_output` in `terminal.rs`.
2. The cell loop inside `TerminalElement::prepaint`.
3. `TerminalElement::paint`.
4. `glyphs.rs` from `paint_part` down. Skip `glyph_data.rs`; it is a generated table.
5. `TerminalInteraction::mouse_down` for the mouse model.
6. The pane's render function in `surfaces/panes/pane.rs` to see how the element is hosted.
