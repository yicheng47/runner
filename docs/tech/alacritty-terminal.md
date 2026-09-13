# alacritty_terminal: the emulator without a window

Written against `alacritty_terminal` 0.26.0 and its parser crate `vte` 0.15.0. Source: `~/.cargo/registry/src/*/alacritty_terminal-0.26.0/src/` and `~/.cargo/registry/src/*/vte-0.15.0/src/`, or a clone of `github.com/alacritty/alacritty` at tag `alacritty_terminal_v0.26.0`, which is byte-identical.

Alacritty the app is a terminal emulator. The crate is that app with the window, fonts, and PTY stripped out. What remains is a pure state machine: bytes in, a grid of cells out. Zed embeds the same crate. Runner uses none of its `tty` or `event_loop` modules, only the model, and feeds it from its own PTY runtime.

It is three layers, bottom up.

## Layer 1: the byte parser

`vte/src/lib.rs` implements Paul Williams' DEC ANSI parser state machine. It knows nothing about screens. It reads a byte stream and classifies it into a handful of actions: print a character, execute a control code such as linefeed, dispatch a CSI sequence such as cursor-move with its numeric parameters, dispatch an ESC sequence, or dispatch an OSC string such as set-title. It also reassembles UTF-8 across chunk boundaries, which is why the embedder can feed arbitrary PTY read sizes without worrying about a split multibyte character.

## Layer 2: the semantic handler

`vte/src/ansi.rs` wraps the parser in a `Processor` and defines the `Handler` trait. This trait is the vocabulary of a terminal. It has about seventy methods, all with empty default bodies: input a char, goto line and column, linefeed, carriage return, clear screen, clear line, insert and delete lines, set and unset mode, report mode, set color, reset color, set title, set cursor style, report device status, and so on. The `Processor`'s job is to turn a raw CSI dispatch such as `ESC [ 5 ; 10 H` into one call to `goto` with those two numbers. The one public entry point is `advance`, which takes a handler and a byte slice. That is the call Runner makes in `feed_output`.

The `Processor` also implements the synchronized-update extension: between `CSI ? 2026 h` and `CSI ? 2026 l` it buffers bytes and applies them at once, with a timeout, so a TUI can repaint without tearing.

## Layer 3: the Term

`alacritty_terminal/src/term/mod.rs`, the struct `Term<T>`, which implements `Handler`. This is the emulator proper. Its fields tell you everything it tracks:

- `grid` and `inactive_grid`: the primary and alternate screens. A TUI such as Claude Code switches into the alternate grid with `CSI ? 1049 h`. When it exits, the primary grid comes back untouched with the shell history intact.
- `mode`: the `TermMode` bitfield, below.
- `scroll_region`: the row range that scrolls on linefeed, settable by DECSTBM.
- `colors`: only the colours a program has overridden with OSC 4 and friends. There is no default palette in the crate.
- `cursor_style`, `title` and its stack, `tabs`, the charset index, the keyboard mode stack.
- `damage`: per-line dirty tracking for renderers that want it. Runner walks the full viewport every frame instead.
- `event_proxy`: the generic `T`, an `EventListener`, for the things a `Term` cannot do itself.

The best single function to read is `Term::input`. It shows the whole cell model in under eighty lines: it asks the character's display width; handles zero-width combining marks by attaching them to the previous cell; wraps the line if the previous write hit the last column; in insert mode shifts the row right; writes the char through the cursor's template cell; and for a double-width character writes the glyph with the `WIDE_CHAR` flag and then a blank spacer cell with `WIDE_CHAR_SPACER`, or a `LEADING_WIDE_CHAR_SPACER` when the glyph does not fit before the wrap. Those spacers are why a renderer skips spacer cells and draws the glyph across two columns.

`Term::linefeed` is the other one to read. If the next line is the end of the scroll region it scrolls up by one; otherwise it just moves the cursor down.

## The Grid and its ring buffer

`grid/mod.rs` and `grid/storage.rs`. A `Grid<T>` is a `Storage<T>` of `Row<T>` plus a cursor, a saved cursor, a column count, a visible line count, a `display_offset`, and a maximum scrollback.

`Storage` is a ring buffer: a `Vec` of rows and a `zero` index. Scrolling the screen up by one line does not move any memory. It is a modular add on `zero`, in `rotate`. Indexing by `Line` goes through `compute_index`, which applies the offset. Read `Grid::scroll_up`: when the region starts at the top, the scrollback limit grows, the buffer rotates, and the rows that rotated in are reset from the cursor's template. The old top row is now history. When the region does not start at the top, rows are swapped within the region and nothing enters history.

Coordinates follow from this. `Line` is a signed integer. Zero is the top of the visible screen, positive goes down, negative goes up into scrollback. `Column` is unsigned. `display_offset` is how many lines the user has scrolled back. The two helpers `point_to_viewport` and `viewport_to_point` in `term/mod.rs` are a single addition each; Runner calls them in the paint loop and in mouse hit testing.

`display_iter` walks from `-display_offset` for `screen_lines` rows, so a renderer gets exactly the visible cells in row order and never touches scrollback it is not showing.

## The Cell

`term/cell.rs`. A `Cell` is a `char`, a foreground `Color`, a background `Color`, a sixteen-bit `Flags` field, and an `Option<Arc<CellExtra>>` for rare extras. The extras are zero-width chars, an underline colour, and an OSC 8 hyperlink. Keeping those behind an `Option` keeps the common cell small, which matters when a grid with ten thousand lines of history is reflowed.

The flags are what a renderer reads: `INVERSE`, `BOLD`, `ITALIC`, `DIM`, `HIDDEN`, `STRIKEOUT`, five underline kinds gathered as `ALL_UNDERLINES`, `WRAPLINE` marking a soft wrap, and the two wide-char markers.

`Color`, defined in vte's `ansi.rs`, is an enum with three shapes. `Spec` is a literal 24-bit RGB. `Indexed` is one of 256. `Named` is one of the 16 ANSI slots plus foreground, background, cursor, and their dim and bright variants. Resolving a `Color` to real RGB is the embedder's job: check the `Term`'s overrides, then fall back to a palette. That is why `crates/runner-terminal/src/palette.rs` exists, and why the Rosé Pine and Catppuccin tables live in Runner, not in alacritty. `term/color.rs` defines the `Colors` override table with 269 slots; the index layout comes from `NamedColor` in vte's `ansi.rs`: 0 to 255 are the standard palette, then foreground, background, cursor, the eight dim colours, bright foreground, and dim foreground, which is why Runner's `resolve_index_for` special-cases indices 256 through 267 and lets 268 fall through to dim foreground.

## Mode flags

`TermMode` is the bitfield the TUI toggles through DECSET and DECRST sequences. The ones a renderer and input encoder care about:

- `SHOW_CURSOR`: whether to paint a cursor at all.
- `ALT_SCREEN`: which grid is active. The alternate screen has no scrollback and never reflows.
- `MOUSE_REPORT_CLICK`, `MOUSE_MOTION`, `MOUSE_DRAG`, gathered as `MOUSE_MODE`: the TUI wants mouse events encoded and sent instead of handled locally. `SGR_MOUSE` and `UTF8_MOUSE` pick the encoding.
- `BRACKETED_PASTE`: wrap pastes in `ESC [ 200 ~` and `ESC [ 201 ~` so the program can tell paste from typing.
- `APP_CURSOR` and `APP_KEYPAD`: change how arrow and keypad keys are encoded.
- `LINE_WRAP`: auto-wrap at the last column.
- The kitty keyboard protocol bits, `DISAMBIGUATE_ESC_CODES` and friends, which change key encoding when a program opts in.

## The read side

`renderable_content` is the only API a renderer needs. It returns a `RenderableContent` with a `display_iter` over exactly the visible cells, the `display_offset`, the cursor as a `RenderableCursor` with point and shape, the selection as a `SelectionRange`, the colour overrides, and the mode. The cursor already has the wide-char adjustment applied, sits at the vi cursor in vi mode, and reads as `Hidden` when `SHOW_CURSOR` is off. `SelectionRange::contains_cell` handles the corner cases: it does not invert a block cursor at the selection boundary, and it treats a wide char's trailing spacer as selected with its glyph.

## Selection

`selection.rs`. A `Selection` has a type (simple, block, semantic, lines), two anchors with a `Side` each, and is resolved against the `Term` into a `SelectionRange` at read time. Semantic selection expands to word boundaries using the configured `semantic_escape_chars`; Runner sets those to xterm's word separators. `Term::selection_to_string` extracts the selected text following soft wraps and skipping trailing whitespace.

## Resize and reflow

`grid/resize.rs`. Growing or shrinking lines is cheap: rows are pulled from or pushed to history so the cursor stays at the bottom. Changing columns with reflow on walks every row, joins rows flagged `WRAPLINE` back into logical lines, and re-splits them at the new width. `Term::resize` reflows the primary grid and not the alternate, because a TUI owns every cell and repaints on SIGWINCH anyway. It also drops the selection when the column count changes and rotates it when only the line count changes.

## Talking back

Some sequences require a reply to the child: a colour query, a text-area size query, a device status report, or a clipboard read. A `Term` cannot write to a PTY, so it calls `send_event` on the generic parameter `T` with an `Event`. The variants in `event.rs` that matter:

- `PtyWrite(String)`: bytes to send to the child.
- `ColorRequest(index, formatter)`: the embedder resolves the index and formats the answer with the closure.
- `TextAreaSizeRequest(formatter)`: same shape, for the size.
- `Title(String)` and `ResetTitle`.
- `Wakeup`: new content is available. Runner does not use this; it wakes from `feed_output` directly.

Runner's `EventProxy` pushes these onto a channel and a per-session thread writes them to the PTY. That thread is also where Runner intercepts the DECRQM answer for mode 2031, which alacritty reports as unrecognised because it files the mode under unknown modes.

## FairMutex

`sync.rs`. Two parking_lot mutexes: one for the data, one as a turnstile. `lock` takes the turnstile then the data, so a thread that has been waiting gets the next turn before the current holder can re-lock. `lock_unfair` skips the turnstile. Runner's PTY feed thread and the GPUI paint thread both contend for the `Term`; the fair variant keeps a chatty agent from starving the renderer, and the unfair one serves cheap reads such as mode queries and hit tests.

## What the crate does not do

No fonts, no pixels, no PTY, no default palette, no input encoding. Everything in that list is the embedder's, which is why Runner's `runner-terminal` crate has `palette.rs` and `mappings.rs` next to `terminal.rs`.

## Reading order

1. `term/mod.rs`: the `Term` struct, then `input`, `linefeed`, `resize`, `renderable_content`.
2. `grid/storage.rs`: the whole file, it is short.
3. `grid/mod.rs`: `scroll_up`, `display_iter`, `scroll_display`.
4. `term/cell.rs`: the whole file.
5. `vte/src/ansi.rs`: the `Handler` trait, then skim `Performer` to see how CSI parameters become calls.
6. `event.rs` and `sync.rs`.
