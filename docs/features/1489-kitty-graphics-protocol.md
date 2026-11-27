# 1489 — Kitty graphics, pixel geometry and SGR-Pixels mouse in terminal panes

Tracking issue: [#1489](https://github.com/yicheng47/runner/issues/1489). No new UI chrome and no Pencil screen. The apps that need this, `terminal-browser` in the Launcher and the launch defaults, are [#1492](https://github.com/yicheng47/runner/issues/1492), which depends on Phase 1. The change touches `runner-terminal`, the daemon/PTY boundary (`session/pty_runtime.rs`), the session manager's output and resize paths and the client protocol, so its pull requests carry `core-review` and `risk-high`.

Code references are to `main` at `9d9c50b6` (2026-11-26); `main` has since changed only the app icon and where new `⌘T` tabs start, in no file this spec cites. The behavior reference is Ghostty v1.4.0 (MIT): `src/terminal/kitty/graphics_command.zig`, `graphics_exec.zig`, `graphics_image.zig`, `graphics_storage.zig`, `graphics_unicode.zig`, `graphics_animation.zig`, `graphics_pixel.zig` and `graphics_render.zig`, `src/terminal/Terminal.zig`, and `src/input/mouse_encode.zig` for SGR-Pixels. The protocol references are the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) and [xterm's control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) for `CSI 14 t`, `CSI 16 t` and mode 1016. Ghostty is read for behavior and test vectors; Runner's code is written separately.

## Motivation

Terminal tools now draw whole visual surfaces inside an ordinary terminal: headless browsers (`terminal-browser`, `KittyBrowser`), mirrored desktop windows (`krd`), device simulators (`rex-sim`, `scrcpyt`) and image viewers (`kitten icat`, `chafa`). They use the Kitty graphics protocol, the PTY's pixel size and SGR-Pixels mouse reports (mode 1016).

With these in Runner's terminal, such a tool runs as an ordinary `runnerd` session. Unlike the embedded web view retired in [#1260](https://github.com/yicheng47/runner/issues/1260), its pixels are painted by GPUI in the pane's own paint pass, so popovers, the command palette and the find bar draw over them, and they survive closing and reopening the app with the rest of the session.

## Current behavior

- **APC is dropped.** `vte` 0.15.0 routes `ESC _` into its `SosPmApcString` state and discards everything up to the terminator without calling the `Handler` (`vte-0.15.0/src/lib.rs:377`). A Kitty graphics command is never seen.
- **Output is published before it is parsed.** Each session's forwarder thread collects PTY reads into bursts of up to 1 MiB (`session/manager/output.rs:9`, `start_forwarder_thread` at `:130`). `ingest_output_for_launch` takes the session state lock, pushes the raw bytes to every subscriber as `TerminalFrame::Output` and then feeds the authoritative model (`output.rs:1334-1371`). Companion clients, the iPhone app and remote windows, subscribe the same way: a program's graphics command would reach them unchanged, host file paths included.
- **Pixel sizes are zero.** The PTY is opened and resized with `pixel_width: 0, pixel_height: 0` (`session/pty_runtime.rs:222-223`, `:534-535`), and `CSI 14 t` is answered with zero cell sizes (`terminal/model.rs:245-256`). `vte` does not dispatch `CSI 16 t` at all: its window-ops handler knows only 14, 18, 22 and 23 (`vte-0.15.0/src/ansi.rs:1740-1743`), so a program asking for the cell size gets no reply. alacritty formats `CSI 14 t` by multiplying `u16` fields (`vendor/alacritty_terminal/src/term/mod.rs:2289-2295`).
- **Resize carries only columns and rows.** `Transport::resize` (`runner-core/src/protocol/socket.rs:668`), the `RESIZE` frame (`daemon/server.rs:1136-1157`), `terminal_take_back` (`runner-core/src/protocol/api.rs:66`), `resize_controlled` (`output.rs:1049`, which compares only `(cols, rows)` at `:1072`), `TerminalFrame::Resized`, `TerminalSnapshot`, `RemoteTerminal` (`runner-core/src/remote/client.rs:25`), and the companion `AttachRequest`, `ViewportRequest`, `Snapshot` and `Resized` messages (`runner-core/src/companion/messages.rs:323`, `:352`, `:480`, `:503`).
- **Mode 1016 is unknown.** `alacritty_terminal` files 1016 under unknown modes: it ignores set and reset and answers DECRQM with "not recognised" (`term/mod.rs:2117`). The vendored crate may carry read-only accessors only (`vendor/alacritty_terminal/RUNNER_PATCH.md`), so the mode has to live outside it, as 2031 already does (`runner-terminal/src/terminal.rs:99-109`).
- **Mouse reports are per cell and only during a drag.** The encoders take a cell (`runner-terminal/src/mappings.rs:290-443`). The capture-phase move listener returns when no button is pressed or no drag is in progress (`runner-app/src/terminal/element.rs:963-1001`), and `TerminalInteraction::mouse_move` returns without a drag (`:448-458`), so motion without a button is never reported, even under mode 1003. A reported drag is deduplicated by cell (`DragKind::Reported`, `:82`).
- **Snapshots are terminal bytes.** `serialize_with_history` replays the grid as escape sequences, starting with `RIS` and ending with the unfinished bytes of an incomplete sequence (`runner-terminal/src/snapshot.rs:87-217`, `BoundaryScanner` at `:396`). A client rejects a snapshot or a reassembled output frame larger than `wire::MAX_FRAME`, 64 MiB, and drops its connection (`socket.rs:551`, `:576`).
- **Queues resync when full.** A local subscriber resyncs after 64 queued frames, however large they are (`session/manager/terminal.rs:15`, `:119-133`). The host's companion queue and the remote window's queue resync past 1 MiB (`runner-daemon/src/companion/mod.rs:173`, `remote/client/terminal.rs:20-35`). `push_frame` clones each frame into every subscriber's queue (`session/manager/terminal.rs:1198-1202`). The companion stream numbers its own messages (`companion/mod.rs:371`, `:405`), and local mirrors accept a gap in sequence numbers (`terminal.rs:921-925`).
- **A scroll tracker already wraps the terminal.** The mirror parses through `find::ScrollTracker`, a `Handler` that forwards every method to `Term` and counts the lines that move into history, synchronized updates included (`runner-terminal/src/terminal/find.rs:266-410`). Its fixture test proves it leaves the terminal as a plain parse would (`find.rs:908`). Other sites still feed `Term` directly: `TerminalModel::feed_output`, `flush_sync_update` (`model.rs:487`) and `sync_session_title` (`model.rs:572`), and `TerminalMirror::restore` (`terminal.rs:769`, `:773`).
- **alacritty moves rows into history whenever a scroll region starts at the top**, even a partial one, and for DL at row 0: `Grid::scroll_up` rotates the whole buffer when `region.start == 0` and swaps the rows below the region back (`vendor/alacritty_terminal/src/grid/mod.rs:271-295`; `delete_lines` at `term/mod.rs:1537-1546`).

## Scope

**Phase 1** (unblocks #1492, `terminal-browser`, `krd`, `rex-sim` and `kitten icat`; macOS and Linux, Windows per [Decisions](#decisions) 11):

- Kitty graphics in local terminal panes: every control key; actions `a=t`, `a=T`, `a=p`, `a=d` and `a=q`; formats `f=24`, `f=32` and `f=100`; `o=z`; media `t=d`, `t=f`, `t=t` and `t=s`; chunked transfers; placements with source rectangles, cell offsets, cell sizes, z-index, relative placements and `C=1`; every delete selector; per-screen storage with Kitty's limits; and Ghostty's replies and error messages.
- Pixel geometry: real pixel sizes in the PTY's window size, `CSI 14 t` answered with them and `CSI 16 t` answered at all.
- Mode 1016 (SGR-Pixels), with DECRQM, and motion reports without a pressed button under mode 1003.
- Graphics in attach snapshots, so closing and reopening the app keeps them.
- Companion clients: graphics removed from what they receive, and the session's cell size added to the companion messages.

**Phase 2**:

- Unicode placeholders (`U=1`), so graphics survive multiplexers and flow with text.
- Animation: `a=f`, `a=a` and `a=c`, playing only while a pane shows the session.
- Graphics for companion clients that ask for them, within a budget.

Out of scope:

- The Launcher, **Settings → Launcher**, `TERMINAL_BROWSER_TERMINAL_SOCKET` and macOS `⌘` routing ([#1492](https://github.com/yicheng47/runner/issues/1492)).
- Sixel and iTerm2 (`OSC 1337`) images.
- Unwrapping tmux's DCS passthrough. Unicode placeholders are the multiplexer path.
- Bundling `terminal-browser`, a browser engine or any other visual tool.

## Decisions

1. **runnerd interprets graphics; subscribers get a normalized stream.** The model in runnerd executes every graphics command and answers it. In what it publishes, every other byte is unchanged and each graphics command is rewritten into a self-contained form: a transmission appears once, complete, as direct data naming its image by `i=`; queries, failed commands and image numbers never appear; deletions the model makes on its own — for evictions, and for an existing image deleted by a chunked start (`m=1`) or a failed replacement — appear as deletes, while a succeeding single-command replacement relies on the mirror replacing `i=` directly when it applies the transmission. A mirror therefore never reads a file, allocates an image ID or decides an eviction, so it can neither race the model nor disagree with it.
2. **Local files are read once, before the session lock.** The session's forwarder thread checks each `t=f`, `t=t` or `t=s` command's header and then reads, and where the medium says so deletes, the file or shared-memory object before `ingest_output_for_launch` takes the state lock. A slow read delays only that session's output, as it would in Kitty. Decoding and validating an image needs the screen it belongs to, so it happens in the model when the transmission completes, under the lock, as Ghostty does on its IO thread; replies must stay in order with every other reply, so it cannot be deferred.
3. **A burst with graphics is parsed before it is published.** Only the model can name an image, so a burst that contains a graphics command goes to the model first and its normalized bytes are published after. Every other burst keeps today's order, so plain output keeps its concurrency.
4. **One cell size per session.** The pane that owns the PTY size reports its cell size in device pixels with its columns and rows. runnerd sets the PTY's pixel size from it, answers `CSI 14 t` and `CSI 16 t` with it, and sends it to every mirror in `Resized` frames and snapshots. Placement spans are computed from that size in runnerd and in every mirror alike, so their grids and cursors stay identical; a viewer whose own cells differ scales each image to the same span.
5. **Graphics keep their place inside a synchronized update.** A command needs the cursor that the bytes before it leave. When one arrives while `vte` holds a synchronized update, the stream applies the held bytes, executes the command and reopens the update. Mirrors do not wake their viewers while an update is open, so the rest of the frame still appears at once, and the update still ends 150 ms after it first opened.
6. **Kitty's memory limits, plus a daemon-wide one that never crosses sessions.** Each screen keeps Ghostty's 320 MB limit. runnerd also caps the bytes it stores across all sessions, but a session only ever evicts its own images, so no session takes another's lock. runnerd keeps images in their transmitted encoding; a mirror decodes only for a visible pane.
7. **Snapshots carry graphics as terminal bytes.** Each image is transmitted again at the start of its screen, and each placement is emitted with `C=1` when its row is replayed, so a placement in scrollback scrolls up into it with its row. The snapshot keeps its wire shape and its 64 MiB limit; graphics get what the text leaves of it, at most 48 MiB.
8. **Companion clients get no graphics in Phase 1.** Companion messages are at most 1 MiB and their queues resync when full. Phase 1 removes graphics commands from what companion clients receive and leaves the rest of the stream unchanged; Phase 2 sends graphics to clients that ask, within a budget. Phase 1 does add the session's cell size to the companion messages, so a client can report SGR-Pixels positions in the session's pixel space.
9. **SGR-Pixels coordinates start at zero.** Kitty and Ghostty report the top-left pixel as `0;0`; xterm reports `1;1` (`button.c`, `EmitMousePosition`). The tools #1492 targets are written against Kitty and Ghostty.
10. **Local files follow Ghostty's checks.** Anything a program prints, including a file an agent displays, can ask runnerd to read a local file. Runner applies Ghostty's checks unchanged and adds none: the result is an image on this computer's screen, and Phase 1 sends no graphics to other devices.
11. **Windows: parsing and painting, no PTY pixels.** ConPTY's `ResizePseudoConsole` takes only columns and rows (`portable-pty-0.9.0/src/win/psuedocon.rs:41`), so Windows programs learn pixel sizes only from `CSI 14 t` and `CSI 16 t`. Whether ConPTY passes APC sequences through is a live check; Windows support is claimed only once it passes. `t=s` is Unix-only, and `t=f` and `t=t` on Windows follow Ghostty's `kitty/windows.zig` path rules.

## Design

### Module

`crates/runner-terminal/src/kitty/`:

- `command.rs`: one command's control keys and payload, every key in the protocol (Ghostty `graphics_command.zig`).
- `scanner.rs`: splits a byte stream into text and complete graphics commands.
- `image.rs`: chunk accumulation, base64, `o=z` inflation, PNG decoding and the size and dimension checks (`graphics_image.zig`). No file access.
- `media.rs`: `t=f`, `t=t` and `t=s`, used only by runnerd's forwarders.
- `storage.rs`: one screen's images and placements, byte accounting, eviction and delete selectors (`graphics_storage.zig`).
- `exec.rs`: applies a command to storage and the grid and returns its reply and its normalized form (`graphics_exec.zig`).
- `snapshot.rs`: writes a screen's graphics into a snapshot.
- Phase 2: `unicode.rs` (`graphics_unicode.zig`) and `animation.rs` (`graphics_animation.zig`).

`runner-terminal` already has `base64` 0.22; it gains `flate2` and `png`. It stays free of GPUI.

### The stream handler

Every place that feeds `Term` goes through one `Handler` around it, generalized from `find::ScrollTracker`: `TerminalModel::feed_output`, `flush_sync_update` and `sync_session_title`, and `TerminalMirror::apply_output_chunk`, `flush_held_output` and `restore`. It forwards every method, keeps find's line count, and adds what follows.

Each placement at the cursor keeps a viewport line, negative in history, and a column. The handler updates them per call, from the operation and the scroll region (`term.snapshot_state().scroll_region`), never from find's count alone:

- **Lines entering history.** Around each scrolling call the handler measures, by row identity as `ScrollTracker` does, how many rows alacritty moved into history. Placements anchored in history move up by that many; once a placement's last row is above the oldest retained line, it is deleted with any relative children it orphans.
- **A full-screen scroll up** (SU, IND, or a linefeed or wrap at the bottom with no region set) moves every screen placement up with its row, into history.
- **A scroll inside a region** (a region is set when it is not the whole screen) follows Kitty (Ghostty `ImageStorage.scrollMarginsBegin`): SU, IND, SD and RI move placements lying wholly inside the region with the content and clip them at its margins, deleting them once wholly clipped; placements straddling a margin, outside the region or in history do not move. When the region starts at row 0, alacritty still moves its top rows into history; the placements inside the region are clipped all the same, and only history placements move.
- **A full-screen scroll down** (SD, RI at the top) moves screen placements down, clipped at the bottom.
- **IL and DL never move placements**, as in Kitty; DL at row 0 can still push rows into history, which moves history placements only.
- **ED 2** (`clear_screen(All)`) follows Ghostty `ImageStorage.clearScreen`: it deletes every placement that intersects the visible screen, including one anchored in history that reaches into it, then the relative placements that orphans, then every image left without a placement, including images that were never placed. Rows alacritty moves into history on the clear keep their placements. ED 3 (`clear_screen(Saved)`) deletes the placements in history. ED 0 and ED 1 leave graphics alone.
- **RIS** (`reset_state`) empties both screens' storage, including a chunked load in progress, and clears mode 1016. DECSTR touches neither, as in Ghostty `softReset`.
- **Mode 1049**, the only alternate-screen mode alacritty implements: commands act on the storage of the screen active at their position. Entering the alternate screen erases it as an ED 2 does, as Ghostty does on entry (`Terminal.zig:5046`), so its placements and every image left without one are freed.
- **Placements relative to another placement, and virtual ones,** never move with scrolls; they follow their parent.

Two changes do not pass through the handler, so they apply the same rules directly:

- **Scrollback limit.** `TerminalModel::configure` and `TerminalMirror::configure` (`model.rs:641`, `terminal.rs:871`) can shorten history through `set_options`; afterwards they delete placements above the oldest retained line, and the children that orphans.
- **Resize.** `Term::resize` resizes both screens, lines first and then columns (`term/mod.rs:685-735`, `grid/resize.rs:14-98`). Before it runs, the handler computes each screen's line shift from that screen's own history and cursor, as alacritty will apply it: growing by `n` lines moves content down by `min(history length, n)` (`grow_lines`, `resize.rs:43-68`), and shrinking to `n` lines moves it up by `max(0, cursor line + 1 − n)` (`shrink_lines`, `resize.rs:76-98`). These amounts hold on the alternate screen, which has no history, and on a primary screen whose history is full, where the history length does not change. Placements move by that shift. A column change then leaves them where they are, as Kitty's `grman_resize` does, because alacritty reflows rows without exposing the mapping. Placements that end up outside the retained lines or past the last column are deleted, with their orphans.

The handler also keeps mode 1016: `set_private_mode(Unknown(1016))` sets it, its reset clears it, and setting 1005, 1006 or 1015 clears it, since the latest format wins. `report_private_mode(Unknown(1016))` answers `CSI ? 1016 ; 1 $ y` or `; 2 $ y` at its position instead of alacritty's "not recognised". `vte` already splits a combined list such as `CSI ? 1003 ; 1016 h` into one call per mode (`ansi.rs:1603-1611`). Resetting 1016 leaves whatever alacritty's own bits select, SGR when 1006 is still set, where Ghostty falls back to X10. Runner keeps alacritty's handling of the modes it already knows, a deliberate choice that leaves today's 1005 and 1006 behavior as it is.

`text_area_size_pixels` (`CSI 14 t`) is answered by the handler itself, computing in `u32`, because alacritty's `u16` multiplication overflows past 65,535 pixels. The model also answers `CSI 16 t`, which `vte` never dispatches, with `CSI 6 ; <cell height> ; <cell width> t` at its position, using the needle scan that already finds the 2031 query (`scan_scheme_sequences`, `terminal.rs:144`).

### Scanning graphics commands

`kitty::Scanner` keeps state across chunks and splits each chunk into text and complete graphics commands, `ESC _ G <control> [; <payload>] ESC \`. Any other APC passes to `vte` as today. In runnerd the scanner belongs to the session's forwarder, which hands the model the text and commands it found; a mirror runs its own over the frames it receives. Text runnerd injects itself, such as the cwd notice on resume (`session/manager/resume.rs:765`), carries no graphics and goes to the model as text. A session resumed on a new process gets a new forwarder and so a fresh scanner, and `sync_session_title`, which already drops the old process's unfinished parser and scanner state (`model.rs:557-605`), also drops both screens' chunked loads, so nothing the old process left half-sent reaches the new one. Its edge cases follow `vte`:

- CAN or SUB cancels a command; an ESC that does not begin `ESC \` cancels it and starts a new sequence that goes to `vte`.
- In `ESC ESC _ G`, the first ESC goes to `vte` as text and the second starts the command (`vte-0.15.0/src/lib.rs:387`).
- A command split across reads is held until its terminator.
- One command may be at most 48 MiB, below Ghostty's 65 MiB (`apc.zig`, `defaultMaxBytes`). A longer one is discarded through its terminator without a reply; CAN, SUB and a stray ESC end the discard as they end a command.

Graphics bytes never reach `vte`, so they cannot fill its 2 MiB synchronized-update buffer (`vte-0.15.0/src/ansi.rs:39`), and they never become a snapshot's unfinished bytes. The model's other scanners, for 2031, `CSI 16 t`, OSC 7 and program status (`model.rs:348-411`), run on each text segment in turn, between the graphics commands, so every reply leaves in stream order.

### Synchronized updates

`vte` buffers everything between `CSI ? 2026 h` and `CSI ? 2026 l`, up to 2 MiB or 150 ms (`ansi.rs:36-48`), and applies it at once. When a graphics command arrives while it holds such an update, the stream calls `stop_sync` through the handler to apply the held bytes, calls `parser.boundary.end_sync()`, executes the command and feeds `CSI ? 2026 h` to reopen the update, so the boundary tracking starts the reopened update afresh (`snapshot.rs:405-415`).

Reopening restarts `vte`'s 150 ms timeout (`ansi.rs:411`). The parser therefore remembers when the update first opened, and the model's and the mirror's flush timers end it 150 ms after that, whatever `vte` reports, so a stream of graphics commands inside an unclosed update cannot postpone the flush.

While an update is open, the mirror wakes no viewer: neither after a frame (`terminal.rs:570-609`) nor through `EventProxy`'s waker when the held bytes raise an event such as a title change (`terminal.rs:70-75`). The update's end, or the timeout, wakes them. A repaint forced by something else, such as a cursor blink, can still show the part applied so far.

### Executing commands

`exec.rs` follows Ghostty `graphics_exec.zig`:

- `i` and `I` together are `EINVAL` for every action. Replies are `ESC _ G i=<id>[,I=<number>][,p=<placement>];<message> ESC \`. `q=1` suppresses `OK` and `q=2` everything; a continuation chunk inherits the first chunk's `q` unless it sets its own. An image with neither `i` nor `I` gets an ID and no reply. Error messages are Ghostty's (`encodeError`), for example `ENOENT: image not found`, `ENODATA: insufficient data` and `EINVAL: dimensions too large`.
- `a=q` loads the image completely, replies and stores nothing.
- A transmission naming an existing `i=` deletes that image and its placements when it begins, before it completes or fails (`graphics_exec.zig:1040-1047`).
- A command with a medium other than `t=d` abandons a chunked load in progress and starts a new transmission.
- Each screen has its own storage and its own chunked load.
- Relative placements (`P`, `Q`, `H`, `V`) resolve their parent chain up to 8 links, with `ENOPARENT`, `ECYCLE` and `ETOODEEP`; deleting a parent deletes its orphaned children.
- A placement's span is `c` × `r` cells when both are given. Otherwise it is the placement's pixel size plus its cell offset, divided by the session's cell size and rounded up, where the pixel size is the source rectangle's, or follows its aspect ratio when only `c` or only `r` is given (Ghostty `Placement.pixelSize`, `gridSize`). `X` and `Y` are clamped to one pixel less than the cell size.
- With `C=0`, a placement at the cursor moves the cursor down `rows − 1` lines, plus one when `x + cols` reaches the right edge, with IND, so it scrolls at the bottom margin; the count is bounded by the distance to the region's bottom plus one screen. The cursor then goes to column `x + cols`, or 0 when it wrapped. Relative placements never move the cursor.
- Delete selectors `a`, `i`, `n`, `c`, `p`, `q`, `x`, `y`, `z` and `r`, lowercase deleting placements and uppercase also freeing images left without one. `f` and `F` are parsed and act once Phase 2 adds frames.
- Limits: width and height at most 10,000 (Kitty and Ghostty); at most 32 MiB of transmitted data per image after base64 and before inflation, so one transmission fits one output frame (below); and decoded data at most the per-screen limit, enforced while decoding. For `f=24` and `f=32`, inflation stops past `s × v × bytes per pixel`. For `f=100`, whose size is known only from its header, inflation stops at 32 MiB, the most a PNG may be, and the PNG is refused from its header if `width × height × 4` exceeds the per-screen limit, before its pixels are allocated. Ghostty allows 400 MiB of data; Runner's limit is lower because a transmission crosses the client socket as one frame.

### Local media

Every forwarder, `start_forwarder_thread` (`output.rs:130`, its loop at `:166-232`) and the forwarder of an unlisted terminal (`session/manager/spawn.rs:62`), passes each burst through the session's scanner before it calls `ingest_output_for_launch`. For a command with `t=f`, `t=t` or `t=s`, it first runs the checks that need no state, those Ghostty makes before it opens anything: `i` with `I`, `a=q` without `i`, an unknown `f`, a medium the platform lacks. A command that fails them goes on to the model, which replies with the error, and no file is touched. Otherwise the path is the base64 payload, and the rules are Ghostty's `LoadingImage.readFile` and `readSharedMemory`:

- **`t=f`:** open first, then take the canonical path from the open handle. Refuse `/proc/`, `/sys/`, and `/dev/` other than `/dev/shm/`, and anything that is not a regular file. Seek to `O`, read exactly `S` bytes or to the end, at most 32 MiB.
- **`t=t`:** as `t=f`, and the canonical path must lie in `/tmp`, `/dev/shm`, `/private/tmp` on macOS, runnerd's temporary directory or the `TMPDIR` the session was spawned with, each canonicalized, and contain `tty-graphics-protocol`. The file is deleted once it has been opened and checked, whether or not the image then loads.
- **`t=s`** (Unix): `shm_open` read-only with the name as sent, `shm_unlink` it, size it with `fstat`, map it and copy `S` bytes or the whole object.
- The forwarder hands the model the bytes it read, or the error. A failure becomes Ghostty's error, which the model replies with at the command's position. A query reads, and for `t=t` and `t=s` deletes, exactly as a transmission does.

### The normalized stream

For a burst that contains a graphics command, under the state lock, runnerd feeds the model and publishes the bytes the model returns. That stream holds:

- every text byte, unchanged and in order;
- a transmission (`a=t`, `a=T`) once its last chunk has arrived, as one command: the action, `i=<id>`, `f`, `s`, `v`, `o`, for `a=T` its placement keys, `q=2`, and the transmitted data base64-encoded again from the decoded bytes, since a program may pad its chunks (Ghostty's own vectors do, `graphics_exec.zig:1349`);
- `a=p` with its image named by `i=`, and `a=d` as sent with `q=2`, except that the number selectors `n` and `N` become `i` and `I`;
- `a=d,d=I,i=<id>,q=2` where the model deletes an image on its own: when a chunked transmission (`m=1`) naming that image's `i=` begins, when a single-command transmission naming that image's `i=` fails after deleting the old image, when a single-command replacement both triggers evictions and interacts with relative placements (so mirror orphan cleanup runs in the same order as runnerd), and for each image it evicts, before the command that caused it. Otherwise a succeeding single-command `a=t` or `a=T` reusing an existing `i=` omits this pre-delete because the mirror's own replacement deletes the old image and its placements atomically when it applies the transmission, preventing a split `> 1 MiB` replacement from painting a blank frame between transport frames.

Queries, failed commands and continuation chunks never appear. The normalized bytes are published as consecutive frames, each with its own sequence number: a frame is closed before a text segment or command that would take it past 1 MiB, so a transmission larger than that travels alone, below 44 MiB with the 32 MiB transmission limit, well under `wire::MAX_FRAME`. A burst whose normalized form is empty still advances the session's sequence number, which the model records, and publishes nothing; mirrors accept the gap. The model records the last sequence number of the frames it published, so a snapshot taken after them matches `output_seq`. A local subscriber's queue also resyncs once more than 128 MiB of frames are queued; today only the 64-frame count does.

Replies leave as they do today: the model's event worker writes them through `TerminalHost::write_reply`, in order with every other reply (`model.rs:171-305`). Mirrors discard replies, as they already do.

### Storage and memory

- Each screen has Ghostty's 320,000,000-byte limit, counted as Ghostty counts it: RGB at 3 bytes per pixel and RGBA at 4 (`graphics_image.zig`, `graphics_storage.zig:411`). An image over the limit on its own is `ENOMEM: out of memory`. Eviction follows Ghostty `evictImageExcept`: images without a placement go before placed ones, oldest first.
- runnerd keeps each image in its transmitted encoding (PNG, zlib or raw), decoding it once, on completion, to validate it and learn its size.
- runnerd counts the bytes it stores across all sessions, reserving a transmission's bytes in that count atomically, so two sessions cannot pass the limit together. A transmission that would take the total past 1 GiB fails with `ENOMEM` at once when its own screen does not hold enough to cover the excess; otherwise it evicts from its own screen, in the same order, until it fits. A session never evicts another session's images, so no session ever takes another's state lock, and never from its inactive screen, where the stream's deletes could not reach it. A session's images are freed when it ends.
- A mirror evicts nothing on its own: it applies the deletes in the stream, with a safety cap of twice the per-screen limit. It also stores the transmitted encoding, and it decodes only for a session that a pane shows: on its frame worker after releasing the term lock and before it wakes the viewers, or, when a pane starts showing a session whose images are not decoded, on a background task, the image appearing once it is ready. A mirror whose last viewer lease has dropped keeps no decoded pixels and no textures; since the app attaches a mirror to every live session, this keeps hidden sessions cheap.

### Pixel geometry

- `TerminalElement::prepaint` already measures the cell: `em_advance` for the width and `row_height()` for the height (`element.rs:1127-1132`). The cell size it reports is each of those times `window.scale_factor()`, rounded. `TerminalGridSize` gains the two values, and `size_push_verdict` pushes when any of the four changes (`runner-app/src/terminal/resize.rs:2`, `:30`), so a font size change, a line height change or moving the owning window to a display with another scale pushes.
- Every path that sets a session's size carries the cell size: `TerminalMirror::resize` and `Transport::resize` with the `RESIZE` frame, `terminal_take_back` for **Take back** (`session/manager/terminal.rs:1416`), and for remote terminals `RemoteTerminal`, `remote_claim_control` (`runner-daemon/src/remote/mod.rs:144`) and the remote client's viewport and take-control requests. The app and runnerd come from one build, which the hello's executable hash enforces (`wire::Hello`), so these frames change in place.
- `resize_controlled` compares the cell size too. `PtyRuntime` opens and resizes the PTY with `pixel_width = cols × cell width` and `pixel_height = rows × cell height`, each clamped to `u16::MAX`. On Windows, a change of cell size alone updates runnerd and the mirrors but does not resize ConPTY, which repaints on any resize (`output.rs:1081-1083`).
- A cell size from any client is clamped to 1–1,024 pixels per side. A cell size of zero, from a caller that has none such as a test, the CLI or an older companion client, keeps the session's current one. runnerd remembers the last cell size any desktop pane reported and opens new PTYs with it.
- The model keeps `(cols, rows, cell width, cell height)` and answers `CSI 14 t` (`4;<rows × cell height>;<cols × cell width>t`) and `CSI 16 t` from it, zero until a size is known.
- `TerminalFrame::Resized`, the `RESIZED` frame and the snapshot header carry the cell size to mirrors. Both places that emit `Resized` keep the session's current cell size: `resize_controlled` (`output.rs:1095`) and `push_controller` (`session/manager/terminal.rs:1296-1310`). `last_requested_size` holds the cell size with the columns and rows, so `install_handle_state`, which applies the size requested while a fork or resume was in flight to the runtime and the model (`session/manager/attach.rs:155`, `:222`, `:246`), applies it too.

### SGR-Pixels and motion

- With 1016 on and any mouse mode, the encoders emit `CSI < <button> ; <x> ; <y> M` (`m` for a release), zero-based in the session's pixels: `x = floor((pointer x − pane content left) ÷ viewer cell width × session cell width)`, `y` likewise with heights, each clamped to `0 ..= cols × session cell width − 1` (rows and height for `y`), as Kitty clamps to its window. When the session's cell size is zero, the viewer's own device-pixel cell size stands in. Wheel reports use the same position. Without 1016 the encoders are unchanged.
- Under 1016, a drag reports every pixel move, so `DragKind::Reported` keeps the last pixel position instead of the cell, as Kitty and Ghostty do.
- Under mode 1003, moving over an interactive pane with no button pressed reports motion (button code 35 plus modifiers). The capture-phase listener sends it only while the pane's hitbox is hovered (`move_hitbox.is_hovered(window)`, as `refresh_hovered_link` checks), so moving over a popover, the command palette, the find bar or another pane reports nothing. It drops a report equal to the last one, by cell or by pixel under 1016, and forgets the last position when the pointer leaves. Browsers and simulators need it for hover. Shift still bypasses reporting.
- A snapshot ends its mode list with `CSI ? 1016 h` when the mode is on, after 1005 and 1006 (`snapshot.rs:131-147`).

### Painting

runner-app keeps one texture cache per mirror, keyed by image ID and generation, alive while any `TerminalView` lease on the mirror is held, so hiding one of two panes that show a session keeps its textures. A decoded image becomes a BGRA `Arc<RenderImage>`, halved until each side is at most 8,192 pixels, as the Files viewer does (`surfaces/files/image.rs:36`, `:155-168`); Metal's atlas takes 16,384 but WGPU's limit is the device's own. Textures are released as the Files viewer releases them (`release_render_images`, `surfaces/files/image.rs:837-863`): at once with `cx.drop_image(image, Some(window))` when the release happens in a window, otherwise with `cx.drop_image(image, None)` and one deferred pass. That happens when an image is replaced, deleted or evicted, when the last lease drops, and when the mirror is dropped; none of it waits for a prepaint, which a hidden pane or a closed window never runs.

`TerminalElement::paint` keeps its order and adds three bands, sorted by `(z, image ID, placement ID)` inside each:

1. pane background;
2. images with `z < −1,073,741,824`;
3. cell background quads, find highlights and the selection;
4. images with `−1,073,741,824 ≤ z < 0`;
5. procedural glyphs and text;
6. images with `z ≥ 0`;
7. cursor and IME marked text, so neither is ever hidden.

Sizes in the session's pixels map to the viewer's logical pixels by `sx = viewer cell width ÷ session cell width` and `sy = viewer line height ÷ session cell height`, or `1 ÷ scale_factor` when the session's cell size is zero. A placement's destination has its origin at `(column × viewer cell width + X × sx, row × viewer line height + Y × sy)` and the size of Ghostty's `Placement.pixelSize` times `(sx, sy)`: the source rectangle's own size, scaled by aspect ratio when only `c` or `r` is given, or `c × cell width − X` by `r × cell height − Y` when both are (`graphics_storage.zig:2098-2172`, `graphics_render.zig`). Each image is drawn with `paint_image(bounds, image_bounds, …)` (`gpui-pre-0.3.14/src/window.rs:5547`): `bounds` is that destination, and `image_bounds` is the whole image scaled so that the source rectangle fills it, so GPUI crops without a copy. Everything is clipped to the element's bounds with `with_content_mask`, the rectangle its text and backgrounds already stay in, which keeps images under popovers, the command palette and the find bar.

### Snapshots

`serialize_with_history` gains the active screen's graphics, and the primary's when the alternate screen is active:

- Right after a screen's grid begins, each image is transmitted as `a=t,t=d,q=2` with its ID, format keys and stored data.
- While the grid is replayed, after the last cell of a row, each placement anchored at the cursor on that row is emitted between `ESC 7` and `ESC 8`: `CSI <column> G`, then `a=p,C=1,q=2` with its IDs and keys. The pending-wrap state survives the save and restore, so wrapped rows still replay exactly.
- Relative placements are emitted right after their parent, parents before children, wherever they sit on screen. A relative placement whose parent has no placement ID resolves on replay to the parent image's preferred placement (Ghostty `PlacementId.preferredOver`); when that image has several placements without IDs, the replayed parent can differ, a rare case in which one viewer differs from runnerd.
- An inactive alternate screen's graphics are left out, as its cells already are.
- The budget counts serialized bytes, base64 and escape sequences included: graphics may take at most `min(48 MiB, wire::MAX_FRAME − the text's serialized bytes − 1 MiB)`. Past that, images are left out least recently used first, those with a visible placement last, together with their placements. A viewer then lacks an image runnerd still has; runnerd's replies stay correct.
- The snapshot header carries the session's cell size, and the mirror applies it before parsing the snapshot. The snapshot is taken under the session state lock, as now; its base64 work is bounded by the budget.

`TerminalMirror::restore` already builds a new `Term` and `ParserState` and parses the snapshot (`terminal.rs:733-843`); it does so through the handler with a fresh graphics state.

### Companion clients in Phase 1

- `push_frame` gives each companion subscriber its frame with graphics commands removed, computed once per frame, and nothing when that leaves the frame empty, so graphics are never cloned into their queues and a 30 fps image stream sends no empty messages. Normalized frames hold only whole commands, so this needs no state between frames, and the companion stream numbers its own messages. Mode 1016 still reaches clients.
- Every companion snapshot is taken without graphics: `attach_terminal_locked` (`session/manager/terminal.rs:1020-1028`), which both `attach_companion_terminal` and a companion re-attach use, gains a flag for it, and so does the final snapshot in `finish_companion_terminals` (`:242`).
- `Snapshot` and `Resized` gain `cellWidth` and `cellHeight`: the session's cell size in device pixels, 0 when unknown. `AttachRequest` and `ViewportRequest`, which carries both `viewport` and `takeControl`, gain optional `cellWidth` and `cellHeight`: a client that controls the size with them sets the session's cell size, and one that sends none leaves it as it was. The fields are additive with defaults, so `companion::VERSION` stays 1; the golden fixtures in `runner-core/src/companion/fixtures` gain them, and an old client's message without them still decodes. The host's companion stream sends the cell size in its first `Resized` too (`Stream.initial_size`, `companion/mod.rs:227`, `:296`, `:341`).
- A client that reports SGR-Pixels positions maps its pointer into cells of `cellWidth` × `cellHeight`, so a program sees one pixel space whichever client is pointing. The iPhone app adopts this separately.
- The remote window's client (`runner-daemon/src/remote/client/terminal.rs`) passes the fields through to `TerminalFrame::Resized` and `TerminalSnapshot`, keeps them in its snapshot-assembly and resize checks (`:368-397`), and sends its owning pane's cell size in its attach, viewport and take-control requests.

### Phase 2

- **Unicode placeholders.** A cell holding U+10EEEE draws a slice of a virtual placement (`U=1`). The image ID comes from the cell's foreground colour, the placement ID from its underline colour, and the row, the column and the image ID's high byte from its first three diacritics, which alacritty keeps as the cell's zero-width characters (`CellExtra.zerowidth`), using Kitty's diacritic table (`graphics_unicode.zig`). The renderer skips the placeholder glyph and draws the image slice; placeholders replay in snapshots as ordinary text.
- **Animation.** `a=f`, `a=a` and `a=c` with Ghostty's composition and loop semantics (`graphics_animation.zig`). runnerd stores frames and playback state for snapshots and replies but runs no timers. A mirror advances frames only while a `TerminalView` lease is held and its window is visible; otherwise its timer stops. Frames count against the same limits.
- **Companion graphics.** A client lists `kittyGraphics` in a new `features` field of the companion `Hello`, additive like the fields above. The host then forwards graphics commands to it, gives its snapshots a smaller graphics budget, and, when the client's credit runs short, drops a transmission that a newer one for the same image replaces before it is sent. Phase 2's companion pull request fixes those numbers in this spec before it starts. Phase 2 changes nothing in Phase 1's model.

## Implementation Phases

1. **1a, `runner-terminal`:** the `kitty` module, the stream handler and its scroll, clear, reset, screen, configure and resize rules, mode 1016 and its DECRQM answer, `CSI 14 t` and `CSI 16 t`, the encoders' pixel and motion paths, and the snapshot's graphics. Unit tests against Ghostty's test vectors.
2. **1b, `runner-core` and `runner-daemon`:** the cell size on every size path and in the companion messages; PTY pixel sizes; local media on the forwarders; parsing before publishing for bursts with graphics, frame splitting and the queue's byte limit; the daemon-wide count; the companion filter and graphics-free companion snapshots.
3. **1c, `runner-app`:** pushing the cell size, the texture cache and its releases, the paint bands, pixel mouse reports and motion without a button; then profiling a 30 fps stream. If the profile shows `push_frame` copying large frames once per local subscriber, `TerminalFrame::Output` carries shared bytes.

Phase 1 is one pull request, its commits following 1a to 1c, and this spec lands with it. Phase 2 is two later pull requests: placeholders and animation on the desktop, then companion graphics, which also needs the iPhone app.

#818's spec is on `main` (`b6f7ab1a`) and its implementation is in progress on `fix/818-terminal-round-trips`; it changes the same frame worker and output path. Whichever lands second rebases onto the other. #818's removal of the per-frame metadata request also shortens the frame worker's work, which graphics frames add to.

## Guide pages

- `docs/guide/concepts/sessions.md` and `docs/guide/zh-CN/concepts/sessions.md`: a section beside **Open terminal links** saying that terminals show images from programs that use the Kitty graphics protocol, such as `kitten icat`, and report pixel mouse positions to programs that ask.
- `docs/guide/guides/remote-access.md` and its zh-CN page: images in a terminal show only on the computer that runs it, not in a remote window or on the iPhone.
- Not guide pages, but updated in the same pull request: `docs/tech/terminal-rendering.md` (the scanner, the normalized stream, parsing before publishing for graphics, the stream handler, the paint bands), `docs/tech/alacritty-terminal.md` (`vte` drops APC and never dispatches `CSI 16 t`; 1016 is an unknown mode; regions at the top rotate into history), `docs/tech/remote-control.md` (graphics removed for companion clients; the cell size fields) and `docs/arch/arch.md` §5.5 (snapshots carry graphics).

## Verification

### Automated

- **Scanner:** a stream holding text, a chunked transmission and other APCs gives the same result split at every byte offset; CAN, SUB, a stray ESC and `ESC ESC _ G` behave as in `vte`; a command over 48 MiB is discarded and CAN, SUB or ESC end the discard; another APC still reaches `vte`; replies from text before, between and after graphics commands leave in order.
- **Commands and storage**, from Ghostty's vectors: `f=24`, `f=32` and `f=100` with and without `o=z`; short and long data; chunking with padded chunks and `q` inheritance; `i` with `I`; implicit IDs; queries storing nothing; retransmitting an existing `i=` and then aborting or failing it deletes the old image in runnerd and in a mirror; placements with source rectangles, `X`/`Y`, `c`/`r` and z; relative chains with each error; every delete selector in both cases; eviction order and the per-screen limit with `f=24` and `f=32`; an `o=z` or PNG payload that would inflate past its limit is refused without allocating it; each error message.
- **Cursor and scrolling:** `C=0` and `C=1`; wrapping at the right edge; scrolling at the bottom margin; the bounded move. Full-screen scrolls carry placements into history; SU, SD, IND and RI move placements inside a region, at row 0 and below it, and IL and DL do not, including DL at row 0; full-screen SD and RI; margins clip and delete. ED 2 frees unplaced images; ED 3, RIS and entering 1049. Lowering scrollback through `configure` trims placements; rows-only, columns-only and combined resizes follow their rules on both screens, including shrinking rows on the alternate screen and on a primary screen with full history. A session resumed while the old process had sent half a command or half a chunked load starts clean. The fixture corpus still replays identically through the new handler.
- **Synchronized updates:** an image between two runs of text inside one update lands where the first run left the cursor, and the second run follows it; the snapshot's unfinished bytes match after a reopen; a mirror wakes no viewer during the update, from frames or from events; graphics commands repeated inside an unclosed update do not delay the flush past 150 ms from its opening.
- **Mode 1016:** set and reset, alone and in `CSI ? 1003 ; 1016 h`; a later `CSI ? 1006 h` and RIS clear it; set, query, reset, query in one chunk answer `1` then `2`; a snapshot round trip keeps it.
- **Geometry:** the PTY receives `cols × cell width` and `rows × cell height`, clamped; `CSI 14 t` and `CSI 16 t` answer them, zero before a size is known, with no overflow at large sizes; a change of scale alone pushes a resize; **Take back**, a remote attach and a remote take-control carry the cell size; `push_controller` and `install_handle_state` keep it; a zero or out-of-range cell size from a client is ignored or clamped.
- **Encoders:** SGR-Pixels press, release, drag, wheel and motion without a button, zero-based and clamped, and with a session cell size of zero; SGR and X10 unchanged without 1016; motion without a button only under 1003, only over the hovered pane, and never twice for the same position.
- **Local media:** `t=f` refuses `/proc`, `/sys`, `/dev` outside `/dev/shm`, a directory, a FIFO and a symlink into `/proc`; `t=t` refuses a file outside the temporary directories and a wrong name, and deletes the file after opening it even when loading fails; a `t=t` or `t=s` command that fails its header checks leaves the file in place; `t=s` round-trips and unlinks; `S` and `O`; the 32 MiB limit; each error message.
- **Single reader and single reply** (`runner-daemon`, real daemon socket): with zero, one and two mirrors, `a=q` and `a=T,t=t` give exactly one reply on the PTY, the file is read and deleted once, and both mirrors show the image. Mirrors never receive a query, a file medium, an image number or a continuation chunk; a failed replacement, chunked replacement start, or eviction reaches them as a delete, while a succeeding single-command replacement replaces `i=` directly on the mirror (unless eviction ordering with relative placements requires the pre-delete first); a burst of many sub-1 MiB `t=f` transmissions totalling over 64 MiB is published in frames of at most 1 MiB, a large one alone; a burst with an empty normalized form publishes nothing and a later snapshot's sequence number still matches; 128 MiB queued resyncs.
- **Daemon-wide limit:** two sessions streaming images past 1 GiB evict only their own active-screen images, never block each other and never pass the limit together; a session whose screen cannot cover the excess fails with `ENOMEM` and keeps the images it had.
- **Snapshots:** images and placements on the screen, in scrollback and on both screens survive `TerminalMirror::restore` after the temporary file is gone; relative placements replay after their parents; a snapshot whose text and graphics would pass 64 MiB stays under it, leaving out least recently used images first; the unfinished bytes never hold graphics; companion attach, re-attach and final snapshots hold none.
- **Companion:** a 30 MiB transmission reaches a companion client as nothing, with the surrounding text intact, no empty message and no resync; the cell size fields round-trip in the golden fixtures; a message without them decodes.
- **App:** replacing one image 200 times, deleting it, evicting it, dropping the last lease and dropping the mirror each release the old textures with `drop_image`, without another prepaint; hiding one of two panes that show the session keeps them; a render test checks the bands against backgrounds, text and the cursor, and a placement's destination at 1× and 2× scale.
- **Gates:** `cargo test -p runner-terminal -p runner-core -p runner-daemon -p runner-app`, `make clippy`, `cargo fmt --all -- --check` and `git diff --check`. New tests with daemon threads or timers wait on state, never sleep, and pass `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'`.

### Live checks

On the macOS development app by `qa` holding `script/qa-lock`, on Linux where a desktop session is available, and on Windows for its ConPTY check. Programs are a bounded Python 3 emitter and a raw-TTY receiver written to an owned scratch folder under `/tmp`; `kitten icat` and `chafa` are used where installed. Remote checks use the [same-Mac two-host relay fixture](../tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos).

| Check | Why |
| --- | --- |
| TERM-RENDER-01 | Painting gains three image bands around backgrounds and text; ordinary coloured TUIs and tab switching must not change. |
| TERM-RENDER-02 | The stream handler now wraps `Term` and changes mouse handling; a pager and an editor must restore the shell's screen and modes. |
| TERM-RESIZE-01 | Resizes carry the cell size to the PTY and runnerd; divider drags and window resizes must keep text and geometry in step. |
| TERM-RESIZE-02 | Only the owning pane sets the size, now including the cell size; a second window must not cause resize storms. |
| TERM-SCROLL-01 | The model now parses through the scroll-tracking handler; scrollback, selection and the return to the live prompt must stay exact. |
| TERM-FIND-04 | `find::ScrollTracker` is generalized into the stream handler; a selected match must stay put while output streams. |
| TERM-QUERY-01 | Graphics replies, `CSI 14 t`, `CSI 16 t` and DECRQM 1016 come from the model only; one reply with zero, one and two viewers. |
| TERM-HISTORY-02 | Snapshots carry graphics and 1016; keeping, closing and reopening must keep text history intact. |
| TERM-SET-02 | Line height changes the cell height in pixels; rows and the reported pixel size must follow it. |
| TERM-PERF-02 | Every chunk now passes the scanner and the handler; idle and spinner CPU must not rise. |
| REMOTE-TERM-02 | Resizes on a remote terminal carry the new companion fields. |
| REMOTE-TERM-03 | **Take back** and **Take control** now carry the cell size. |
| Static images | Emit RGB, RGBA, PNG, `o=z`, `t=d`, `t=t` and `t=f` images in an owned shell, once on a Retina display and once on a non-Retina one, including one with `c` and `X`/`Y` set and one at its native size. Images are sharp at their native pixels and keep their aspect ratio, `z < 0` lies under text and `z ≥ 0` over it, they stay inside the pane, and the command palette and find bar draw over them; `clear` and `reset` remove them. |
| Scrollback and snapshots | Place an image, print until it is in scrollback, scroll to it and back, then Keep, quit and reopen the app: it is still on its row, after the temporary file is gone. Lower the scrollback limit until the image's row is dropped: the image goes with it. |
| Pixel geometry and 1016 | A raw-TTY receiver reads the window size and `CSI 14 t` and `CSI 16 t`, enables `CSI ? 1003 ; 1016 h`, and logs a diagonal drag, a hover, moving onto the command palette and a wheel turn. Sizes are non-zero and match the pane, pixel positions start at 0 and grow smoothly within cells, hover is reported over the pane only, and the shell is clean after exit. Then move the window to a display with another scale: the receiver sees the new size. |
| Streaming and memory | A 30 fps loop replaces a full-pane image 300 times in a split beside a live shell, with the session also open in a second window. Typing in the shell stays responsive, the mirrors do not resync repeatedly, closing one window keeps the image in the other, and the app's and daemon's memory return to their earlier level once the image is deleted. Where `terminal-browser` is installed, also click and scroll a page in it. |
| Remote window | On the relay fixture, show a host shell that displays an image in a remote window: the remote terminal shows the text around it with no stray bytes and no repeated resync, and the host's own pane shows the image. |
| Windows ConPTY (win) | Emit a `t=d` image and query `CSI 14 t` and `CSI 16 t` in an owned PowerShell. Record whether the image appears and the replies arrive; Windows support is claimed only if they do. |

Selected regression cases: `TERM-RENDER-01`, `TERM-RENDER-02`, `TERM-RESIZE-01`, `TERM-RESIZE-02`, `TERM-SCROLL-01`, `TERM-FIND-04`, `TERM-QUERY-01`, `TERM-HISTORY-02`, `TERM-SET-02`, `TERM-PERF-02`, `REMOTE-TERM-02`, `REMOTE-TERM-03`. Phase 2's pull requests add their own: placeholders inside `tmux`, an animated image pausing when its tab is hidden, and companion graphics on the relay fixture.
