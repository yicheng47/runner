# 1489 — Kitty graphics, pixel geometry and SGR-Pixels mouse in terminal panes

Tracking issue: [#1489](https://github.com/yicheng47/runner/issues/1489). No new UI chrome and no Pencil screen. This work builds on [#1508](https://github.com/yicheng47/runner/issues/1508)'s Phase 1, which moves `runner-terminal` from `alacritty_terminal` to Ghostty's `libghostty-vt` with Kitty graphics switched off. This spec is #1508's Phase 2 in detail, and #1508's own Phase 2 section defers to it. The apps that need it, `terminal-browser` in the Launcher and the launch defaults, are [#1492](https://github.com/yicheng47/runner/issues/1492). The change touches `runner-terminal`, the daemon/PTY boundary (`session/pty_runtime.rs`), the session manager's output and resize paths and the client protocol, so its pull request carries `core-review` and `risk-high`.

An earlier design built all of this on `alacritty_terminal` by re-implementing Ghostty's graphics engine in Rust. Its implementation is kept on `backup/1489-alacritty-kitty` (`7b09eae1`) and will not land. Its behavioral decisions carry over here: one reply authority, one cell size per session, zero-based SGR-Pixels, three z bands, companion stripping and the live checks. Its engine-independent code is reused (see [Reused from the Alacritty implementation](#reused-from-the-alacritty-implementation)); its engine code is not.

References: Ghostty `vt-1.0.2` (`include/ghostty/vt/*.h`, MIT), above all `kitty_graphics.h`, `terminal.h`, `render.h`, `snapshot.h`, `mouse/encoder.h` and `sys.h`; the #1508 spec at `8a17a520`; and, for code outside `runner-terminal`, `main` at `47b59c77`, where none of the files cited below has changed since `9d9c50b6`. The [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) and [xterm's control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) define the behavior.

## Motivation

Terminal tools now draw whole visual surfaces inside an ordinary terminal: headless browsers (`terminal-browser`, `KittyBrowser`), mirrored desktop windows (`krd`), device simulators (`rex-sim`, `scrcpyt`) and image viewers (`kitten icat`, `chafa`). They use the Kitty graphics protocol, the PTY's pixel size and SGR-Pixels mouse reports (mode 1016).

With these in Runner's terminal, such a tool runs as an ordinary `runnerd` session. Unlike the embedded web view retired in [#1260](https://github.com/yicheng47/runner/issues/1260), its pixels are painted by GPUI in the pane's own paint pass, so popovers, the command palette and the find bar draw over them, and they survive closing and reopening the app with the rest of the session.

## What libghostty-vt does, and what Runner adds

`libghostty-vt` implements the protocol itself:

- **Commands:** every control key and action, including `a=q`, `a=t`, `a=T`, `a=p`, `a=d`, the animation actions `a=f`, `a=a` and `a=c`, chunked transfers, formats `f=24`, `f=32` and `f=100`, and `o=z`. PNG decoding is a callback the embedder installs (`sys.h`).
- **Media:** `t=d`, and `t=f`, `t=t` and `t=s` when the embedder allows each one (`GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_MEDIUM_FILE`, `_TEMP_FILE` and `_SHARED_MEM`), with Ghostty's path checks.
- **Storage:** images and placements per screen, a storage limit (`GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT`, 10 MB by default) and eviction. Stored pixels are always decoded and uncompressed: PNG becomes RGBA, and zlib payloads are inflated (`GHOSTTY_KITTY_GRAPHICS_IMAGE_DATA_PIXELS`). Each image has a generation that changes when its pixels or its current animation frame change.
- **Geometry:** placements follow scrolls, clears, resets, screen switches and resizes. The embedder reads them through a placement iterator with viewport positions, pixel sizes, source rectangles and render info, and reads Unicode placeholder runs from the render state (`render.h`).
- **Replies:** through the `WRITE_PTY` callback, in order with every other reply.
- **Pixel geometry and mouse:** `ghostty_terminal_resize(cols, rows, cell_width_px, cell_height_px)` sets the cell size used by size reports and images, the size callback answers `CSI 14 t` and `CSI 16 t`, mode 1016 and its DECRQM answer are built in, and the mouse encoder has an SGR-Pixels format.
- **Synchronized updates:** mode 2026 is a render hold (`GHOSTTY_TERMINAL_OPT_RENDER_HOLD`). Parsing continues while it is held, so a graphics command inside an update lands where the bytes before it leave the cursor, with nothing to flush or reopen.
- **Snapshots:** the binary snapshot carries images, animation frames and placements (`snapshot.h`). The VT `Formatter` writes none.

Runner adds everything around one engine instance: a single reader and replier among several viewers, a relay that hands mirrors the pixels runnerd decoded, the cell size on every size path, memory across sessions, hidden sessions, snapshot transport, painting in GPUI, hover reports, and the companion protocol.

## Current behavior

After #1508's Phase 1:

- **Graphics are off.** runnerd's `TerminalModel` and the app's `TerminalMirror` run `libghostty-vt` with a storage limit of 0, so graphics commands are parsed and dropped (#1508 Decision 6).
- **Output is published before it is parsed.** `ingest_output_for_launch` takes the session state lock, pushes the raw bytes to every subscriber as `TerminalFrame::Output` and then feeds the model (`session/manager/output.rs:1334-1371`). Companion clients, the iPhone app and remote windows, subscribe the same way.
- **Pixel sizes are zero.** The PTY is opened and resized with `pixel_width: 0, pixel_height: 0` (`session/pty_runtime.rs:222-223`, `:534-535`).
- **Resize carries only columns and rows:** `Transport::resize` (`runner-core/src/protocol/socket.rs:668`), the `RESIZE` frame, `terminal_take_back` (`runner-core/src/protocol/api.rs:66`), `resize_controlled` (`output.rs:1049`, comparing only `(cols, rows)` at `:1072`), `TerminalFrame::Resized`, `TerminalSnapshot`, `RemoteTerminal` (`runner-core/src/remote/client.rs:25`), and the companion `AttachRequest`, `ViewportRequest`, `Snapshot` and `Resized` messages (`runner-core/src/companion/messages.rs:323`, `:352`, `:480`, `:503`).
- **Motion without a button is never reported**, even under mode 1003. The capture-phase move listener returns when no button is pressed or no drag is in progress (`runner-app/src/terminal/element.rs:963-1001`), and so does `TerminalInteraction::mouse_move` (`:448-458`).
- **Queues resync when full.** A local subscriber resyncs after 64 queued frames (`session/manager/terminal.rs:15`, `:119-133`); the companion and remote-window queues resync past 1 MiB (`runner-daemon/src/companion/mod.rs:173`, `remote/client/terminal.rs:20-35`). `push_frame` clones each frame into every subscriber's queue (`session/manager/terminal.rs:1198-1202`).
- **A local snapshot is one binary frame**, under `wire::MAX_FRAME` (64 MiB), which a client enforces by dropping its connection (`socket.rs:551`, `:576`).

## Scope

**Phase 1** (this issue's first pull request, after #1508's Phase 1; macOS and Linux, Windows per [Decisions](#decisions) 11):

- Kitty graphics in local terminal panes: every command `libghostty-vt` implements, including Unicode placeholders and animation, with Ghostty's replies and error messages.
- Pixel geometry: real pixel sizes in the PTY's window size, and `CSI 14 t` and `CSI 16 t` answered with them.
- Mode 1016 (SGR-Pixels) and motion reports without a pressed button under mode 1003.
- Graphics in attach snapshots, so closing and reopening the app keeps them.
- Companion clients: graphics removed from what they receive, and the session's cell size added to the companion messages.

**Phase 2:** graphics for companion clients that ask for them, within a budget, together with the iPhone app (#1508 Phase 3).

Out of scope: the Launcher, **Settings → Launcher**, `TERMINAL_BROWSER_TERMINAL_SOCKET` and macOS `⌘` routing (#1492); Sixel and iTerm2 (`OSC 1337`) images; unwrapping tmux's DCS passthrough, since Unicode placeholders are the multiplexer path; bundling `terminal-browser` or any other visual tool.

## Decisions

1. **runnerd is the only reader and replier.** Sessions run with no viewer at all, after **Keep**, so only runnerd can answer a query or read a file then. `t=t` and `t=s` are deleted by whoever reads them first, and a snapshot for reattach needs the images. So runnerd's model allows `t=f`, `t=t` and `t=s`, executes every command and writes every reply; mirrors never read a program's files and their replies are discarded, as today. herdr-core does the opposite, relaying bytes unchanged and letting a viewer answer (`herdrdev/herdr-core`, `src/relay/graphics.rs`), which cannot cover a session nobody is viewing.
2. **Mirrors get a relayed stream.** Text bytes pass through unchanged. Each graphics command is replaced by what a mirror needs to reach runnerd's state: a completed transmission or animation frame (`a=f`) as one command naming its image by `i=`, with its stored pixels handed over as [Pixel transport](#pixel-transport) describes; `a=p`, `a=d`, `a=a` and `a=c` with `q=2` and image numbers resolved to IDs; and nothing for queries, failed commands and chunks before the last. Both sides run the same engine, so a mirror replaces an image by ID exactly as runnerd does, and no delete is sent for it. runnerd sends `a=d,d=I,i=<id>,q=2` only for an image it lost in a way the mirror cannot see: an eviction, and a transmission that began, deleting the old image, and then failed. A mirror's storage limit is twice runnerd's, so it never evicts on its own.
3. **A burst with graphics is parsed before it is published.** Only the model knows what a command did, so a burst that contains `ESC _ G` goes to the model first and its relayed bytes are published after. Every other burst keeps today's order and cost.
4. **One cell size per session**, in device pixels, from the pane that owns the PTY size, on every size path. runnerd passes it to `ghostty_terminal_resize` and the PTY, and sends it to every mirror in `Resized` frames and snapshots, so placement spans match everywhere; a viewer whose own cells differ scales each image to the same span.
5. **SGR-Pixels coordinates start at zero**, as in Kitty and `libghostty-vt`'s encoder; xterm reports `1;1`.
6. **Memory.** Each screen's limit is 320,000,000 bytes, Ghostty's own, set explicitly because the library's default is 10 MB. A limit across sessions is [Open question](#open-questions) 2.
7. **Snapshots carry graphics as binary snapshots, streamed.** With images, a binary snapshot can outgrow `wire::MAX_FRAME`, so a local snapshot is sent as a sequence of chunks. `snapshot.h` encodes through a writer callback and decodes from a reader callback, so neither side holds the whole snapshot as one frame.
8. **Hidden sessions get no graphics.** A local subscriber with no viewer receives no graphics commands for that session. When a pane starts showing it again after its graphics changed, the mirror resyncs from a snapshot (#1508 Decision 8).
9. **Companion clients get no graphics in Phase 1.** They receive the text stream with graphics commands removed and companion snapshots from the `Formatter`, which writes none. They do get the session's cell size, so a client can report SGR-Pixels positions in the session's pixel space.
10. **Local files follow Ghostty's checks.** Anything a program prints, including a file an agent displays, can ask runnerd to read a local file. Runner adds no checks of its own: the result is an image on this computer's screen, and Phase 1 sends no graphics to other devices.
11. **Windows: parsing and painting, no PTY pixels.** ConPTY's `ResizePseudoConsole` takes only columns and rows (`portable-pty-0.9.0/src/win/psuedocon.rs:41`), so Windows programs learn pixel sizes only from `CSI 14 t` and `CSI 16 t`. Whether ConPTY passes APC sequences through is a live check, and Windows support is claimed only once it passes. `t=s` is Unix-only.
12. **The engine-independent half of the Alacritty implementation is reused**, not rewritten.

## Design

### The graphics relay

- runnerd's forwarder scans each burst for `ESC _ G` with the scanner from the Alacritty implementation (`kitty/scanner.rs`), which splits a byte stream into text and complete graphics commands across burst boundaries and whose split-at-every-offset tests come with it. A burst with no graphics command takes today's path.
- For a burst with graphics, under the state lock, runnerd feeds each text segment and each command to the model in order. After each command it compares that screen's images (ID and generation) with the ones before it, which `kitty_graphics.h` exposes, and appends the command's relayed form to the output: a transmission whose image appeared or changed generation, with its placement keys for `a=T`; the forwarded small commands; and a delete for each image that vanished other than one a completed transmission with the same ID replaced.
- An image number (`I=`, `N=`) in `a=p` or `a=d` is resolved to the ID runnerd's model resolved it to. A mirror never receives a number, so it never allocates an ID.
- Relayed frames stay at most 1 MiB, as today's bursts are; a transmission's handoff is a short command, so no frame grows with an image. A burst whose relayed form is empty still advances the session's sequence number, and mirrors accept the gap.
- Animation: runnerd stores frames and playback state for snapshots and replies, but never ticks. A mirror calls `ghostty_kitty_graphics_animation_tick` only while a pane shows the session, scheduling its next wake from the time it returns.
- Resume and rebind: `sync_session_title` resets the scanner with the parser state it already resets, so a dead process's unfinished `ESC _ G` cannot swallow the new process's output.

### Pixel transport

A full-pane Retina image is about 7.9 MB; 30 replacements per second is about 240 MB/s. The relayed command must hand over the stored pixels without base64 and without one socket copy per subscriber. Two ways fit `libghostty-vt`'s media, and Phase 1 measures both before choosing ([Open question](#open-questions) 1):

- **Shared memory (`t=s`), Unix:** runnerd writes the stored pixels once per local subscriber into a POSIX shared-memory object and relays `a=t` or `a=T` with `t=s`, the stored format (`f=24` for RGB, `f=32` for RGBA, and any other stored format converted to RGBA, which the mirror's doubled limit absorbs), `s`, `v`, `i` and `q=2`. The mirror's engine reads and unlinks the object, so every subscriber needs its own object; memory only, no disk.
- **A private file (`t=f`), all platforms:** runnerd writes the pixels once to a `0600` file in a `0700` directory it owns, keyed by session, image ID and generation, and every subscriber reads the same file (#1508 Decision 7). Files are removed once superseded and sent to every local subscriber, after a grace period, and on session exit and runnerd startup. On macOS the temporary directory is on the SSD, so a 30 fps stream writes about 240 MB/s to disk.

Either way, a mirror that cannot read a handoff asks for a resync rather than diverging, and mirrors allow only the medium the relay uses, never `t=t`.

### Pixel geometry

- `TerminalElement::prepaint` measures the cell: `em_advance` for the width and `row_height()` for the height (`element.rs:1127-1132`). The cell size it reports is each of those times `window.scale_factor()`, rounded. `TerminalGridSize` gains the two values, and `size_push_verdict` pushes when any of the four changes (`runner-app/src/terminal/resize.rs`), so a font size change, a line height change or moving the owning window to a display with another scale pushes.
- Every path that sets a session's size carries the cell size: `TerminalMirror::resize` and `Transport::resize` with the `RESIZE` frame, `terminal_take_back` for **Take back**, and for remote terminals `RemoteTerminal`, `remote_claim_control` and the remote client's viewport and take-control requests. The app and runnerd come from one build, which the hello's executable hash enforces, so these frames change in place.
- `resize_controlled` compares the cell size too. `PtyRuntime` opens and resizes the PTY with `pixel_width = cols × cell width` and `pixel_height = rows × cell height`, each clamped to `u16::MAX`. On Windows, a change of cell size alone updates runnerd and the mirrors but does not resize ConPTY, which repaints on any resize (`output.rs:1081-1083`).
- A cell size from any client is clamped to 1–1,024 pixels per side. A cell size of zero, from a caller that has none such as a test, the CLI or an older companion client, keeps the session's current one. runnerd remembers the last cell size any desktop pane reported and opens new PTYs with it.
- Both places that emit `Resized` keep the session's current cell size: `resize_controlled` and `push_controller`. `last_requested_size` holds the cell size with the columns and rows, so `install_handle_state` applies it too (`session/manager/attach.rs`).

### SGR-Pixels and motion

- With 1016 on, the app's mouse encoder uses the SGR-Pixels format in the session's pixel space: `x = floor((pointer x − pane content left) ÷ viewer cell width × session cell width)`, `y` likewise with heights, each clamped to the pane's pixel area, which is what `libghostty-vt`'s encoder clamps to when given the session's screen and cell sizes. When the session's cell size is zero, the viewer's own device-pixel cell size stands in. Under 1016 a drag reports every pixel move, so `DragKind::Reported` keeps the last pixel position instead of the cell.
- Under mode 1003, moving over an interactive pane with no button pressed reports motion. The capture-phase listener sends it only while the pane's hitbox is hovered (`move_hitbox.is_hovered(window)`, as `refresh_hovered_link` checks), so moving over a popover, the command palette, the find bar or another pane reports nothing. It drops a report equal to the last one and forgets the last position when the pointer leaves. Shift still bypasses reporting.

### Painting

- The mirror publishes visible placements with its `ViewportSnapshot` (#1508), sorted into three bands by `(z, image ID, placement ID)`: images with `z < −1,073,741,824` under the cell backgrounds, find highlights and the selection; images with `−1,073,741,824 ≤ z < 0` between those and the text; and images with `z ≥ 0` over the text and under the cursor and IME marked text, so neither is ever hidden. Unicode placeholder runs from the render state are painted in the same bands.
- Sizes in the session's pixels map to the viewer's logical pixels by `sx = viewer cell width ÷ session cell width` and `sy = viewer line height ÷ session cell height`, or `1 ÷ scale_factor` when the session's cell size is zero. A placement's destination comes from its render info, scaled by `(sx, sy)`. Each image is drawn with `paint_image(bounds, image_bounds, …)` so GPUI crops the source rectangle without a copy, clipped to the element's bounds with `with_content_mask`, which keeps images under popovers, the command palette and the find bar.
- Textures come from the texture cache in the Alacritty implementation's `runner-app/src/terminal/graphics.rs`, keyed by image ID and generation and alive while any `TerminalView` lease on the mirror is held. It already solves what QA found: a replaced texture is retired with `cx.drop_image` only after its replacement has been painted, and never while a render hold is set, so a replacement never shows an empty frame. Images are halved until each side is at most 8,192 pixels, as the Files viewer does.
- On macOS, a placement whose image changes more than five times a second paints through `window.paint_surface` with a small pool of `CVPixelBuffer`s instead of allocating a new atlas texture per frame (#1508 Decision 9). Other platforms use the texture path.
- One case may still show the pane without an image for a frame: replacing an image whose only visible placement is anchored to another image that runnerd evicts in the same command. Jason accepted it on 2026-11-27.

### Snapshots

- A local snapshot is a binary snapshot (#1508 Decision 5) and carries both screens' images, frames and placements. runnerd encodes it under the session state lock into a writer that cuts it into chunks of at most 1 MiB, sent as one snapshot stream. The mirror decodes from a reader fed by those chunks, with a storage limit twice runnerd's. The stream's total is bounded by the two screens' limits plus the text, not by `wire::MAX_FRAME`.
- The snapshot header carries the session's cell size, which the mirror applies before decoding.
- Companion snapshots come from the `Formatter`, which writes no graphics, so they need no change beyond the cell size fields.

### Companion clients in Phase 1

- `push_frame` gives each companion subscriber its frame with graphics commands removed, computed once per frame, and nothing when that leaves the frame empty, so graphics are never cloned into their queues and a 30 fps stream sends no empty messages. Relayed frames hold only whole commands, so this needs no state between frames, and the companion stream numbers its own messages. Mode 1016 still reaches clients.
- `Snapshot` and `Resized` gain `cellWidth` and `cellHeight`: the session's cell size in device pixels, 0 when unknown. `AttachRequest` and `ViewportRequest` gain optional `cellWidth` and `cellHeight`: a client that controls the size with them sets the session's cell size, and one that sends none leaves it as it was. The fields are additive with defaults, so `companion::VERSION` stays 1; the golden fixtures in `runner-core/src/companion/fixtures` gain them, and an old client's message without them still decodes. The host's companion stream sends the cell size in its first `Resized` too.
- A client that reports SGR-Pixels positions maps its pointer into cells of `cellWidth` × `cellHeight`, so a program sees one pixel space whichever client is pointing. The iPhone app adopts this separately.
- The remote window's client (`runner-daemon/src/remote/client/terminal.rs`) passes the fields through to `TerminalFrame::Resized` and `TerminalSnapshot`, keeps them in its snapshot-assembly and resize checks, and sends its owning pane's cell size in its attach, viewport and take-control requests.

### Phase 2

A client lists `kittyGraphics` in a new `features` field of the companion `Hello`, additive like the fields above. The host then forwards relayed graphics to it, with pixels inline under a per-client budget, and coalesces frames under backpressure. The iPhone app draws them.

## Reused from the Alacritty implementation

From `backup/1489-alacritty-kitty`, ported onto #1508's types:

- `runner-terminal`: `kitty/scanner.rs` and its tests, for the relay's framing.
- `runner-core` and `runner-daemon`: the cell size on every size path (`Transport::resize`, `RESIZE`, `terminal_take_back`, `resize_controlled`, `push_controller`, `last_requested_size`, the remote client), PTY pixel sizes, the companion fields, filter and golden fixtures, the relayed frame splitting, and their tests, including `session/manager/tests/terminal_size.rs`.
- `runner-app`: the texture cache and its release lifecycle (`terminal/graphics.rs`), the paint bands, the cell-size push in `terminal/resize.rs`, and the pixel and hover paths in `terminal/element.rs`.

Not reused: `kitty/command.rs`, `storage.rs`, `image.rs`, `exec.rs`, `media.rs` and `snapshot.rs`, `terminal/stream.rs` and the model's synchronized-update changes. `libghostty-vt` does all of that.

## Implementation phases

1. **1a, `runner-core` and `runner-daemon`:** the cell size on every size path and in the companion messages; PTY pixel sizes; graphics on in the model with its media and limits; the relay, its pixel transport and the deletes; parsing before publishing for bursts with graphics, frame splitting and the queue's byte limit; hidden sessions; streamed snapshots; the companion filter.
2. **1b, `runner-app`:** pushing the cell size; graphics on in mirrors; the texture cache, the paint bands, Unicode placeholders and the streaming surface; animation ticks; SGR-Pixels reports and motion without a button; then profiling a 30 fps stream.

Phase 1 is one pull request on this branch, rebased onto `main` once #1508's Phase 1 has merged, and this spec lands with it. Phase 2 is a later pull request together with the iPhone app.

## Open questions

1. **Pixel transport:** shared memory per subscriber or one private file per image generation. Measure a 30 fps full-pane stream with each: CPU in runnerd and the app, memory, bytes written to disk and blank frames. Windows has only the file path.
2. **A limit across sessions:** `libghostty-vt` limits each terminal, not the process, so 25 sessions could hold 16 GB of images in runnerd alone. Either runnerd lowers a session's limit before each graphics command so the sum stays under 1 GiB, which needs checking against what lowering `GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT` does to the inactive screen and whether a lowered limit fails a transmission as Ghostty's `ENOMEM`, or Phase 1 ships per-screen limits only.
3. **Snapshot cost:** encoding two screens of images under the session state lock delays that session's output. Measure attach with 300 MB of images, and decide whether images need a smaller limit or an encode outside the lock.
4. **Windows ConPTY:** whether it passes `ESC _ G … ESC \` through intact, and whether `t=f` accepts runnerd's directory under `%TEMP%`.

## Guide pages

- `docs/guide/concepts/sessions.md` and `docs/guide/zh-CN/concepts/sessions.md`: a section beside **Open terminal links** saying that terminals show images from programs that use the Kitty graphics protocol, such as `kitten icat`, and report pixel mouse positions to programs that ask.
- `docs/guide/guides/remote-access.md` and its zh-CN page: images in a terminal show only on the computer that runs it, not in a remote window or on the iPhone.
- Not guide pages, but updated in the same pull request: `docs/tech/libghostty-vt.md` (from #1508: graphics on, the relay and its transport, hidden sessions, streamed snapshots), `docs/tech/terminal-rendering.md` (the paint bands, textures and the streaming surface), `docs/tech/remote-control.md` (graphics removed for companion clients; the cell size fields) and `docs/arch/arch.md` §5.5 (snapshots carry graphics).

## Verification

### Automated

- **Relay:** for each command kind, a mirror fed the relayed stream ends with runnerd's images, placements and generations, through `kitty_graphics.h`; replacing an image by ID sends no delete; an eviction and a failed transmission after a delete send one; numbers become IDs; queries, failed commands and chunks before the last relay nothing; a burst without graphics is published byte for byte before the model sees it; the scanner's split-at-every-offset tests pass.
- **Pixel transport:** a handoff is readable by two mirrors; a superseded one is removed after the grace period, on session exit and on startup; a mirror that cannot read a handoff resyncs; mirrors refuse `t=t` and, for the file transport, `t=s`.
- **Single reader and single reply** (`runner-daemon`, real daemon socket): with zero, one and two mirrors, `a=q` and `a=T,t=t` give exactly one reply on the PTY, the file is read and deleted once, and both mirrors show the image.
- **Geometry:** the PTY receives `cols × cell width` and `rows × cell height`, clamped; `CSI 14 t` and `CSI 16 t` answer them, zero before a size is known; a change of scale alone pushes a resize; **Take back**, a remote attach and a remote take-control carry the cell size; `push_controller` and `install_handle_state` keep it; a zero or out-of-range cell size from a client is ignored or clamped.
- **Mouse:** SGR-Pixels press, release, drag, wheel and motion without a button, zero-based and clamped, and with a session cell size of zero; SGR and X10 unchanged without 1016; motion without a button only under 1003, only over the hovered pane, and never twice for the same position.
- **Snapshots:** images, placements and animation state on both screens survive a streamed snapshot after the temporary file is gone; a snapshot over 64 MiB attaches; a companion snapshot carries no graphics.
- **Hidden sessions:** a subscriber with no viewer receives no graphics, and showing the pane again resyncs to the latest image.
- **Companion:** a 30 MiB transmission reaches a companion client as nothing, with the surrounding text intact, no empty message and no resync; the cell size fields round-trip in the golden fixtures; a message without them decodes.
- **App:** replacing one image 200 times, deleting it, evicting it, dropping the last lease and dropping the mirror each release the old textures without another prepaint; a replacement paints no empty frame, including inside a render hold.
- **Gates:** `cargo test -p runner-terminal -p runner-core -p runner-daemon -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`, `python3 script/check-guide-translations.py`, and `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'` on every new test that waits on daemon threads or timers.

### Live checks

On the macOS development app by `qa` holding `script/qa-lock`, on Linux where a desktop session is available, and on Windows for its ConPTY check. Programs are a bounded Python 3 emitter and a raw-TTY receiver written to an owned scratch folder under `/tmp`; `kitten icat` and `chafa` are used where installed. Remote checks use the [same-Mac two-host relay fixture](../tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos).

| Check | Why |
| --- | --- |
| TERM-RENDER-01 | Painting gains three image bands around backgrounds and text; ordinary coloured TUIs and tab switching must not change. |
| TERM-RENDER-02 | Mouse handling changes; a pager and an editor must restore the shell's screen and modes. |
| TERM-RESIZE-01 | Resizes carry the cell size to the PTY and runnerd; divider drags and window resizes must keep text and geometry in step. |
| TERM-RESIZE-02 | Only the owning pane sets the size, now including the cell size; a second window must not cause resize storms. |
| TERM-QUERY-01 | Graphics replies, `CSI 14 t`, `CSI 16 t` and DECRQM 1016 come from the model only; one reply with zero, one and two viewers. |
| TERM-HISTORY-02 | Snapshots now stream and carry graphics; keeping, closing and reopening must keep text history intact. |
| TERM-SET-02 | Line height changes the cell height in pixels; rows and the reported pixel size must follow it. |
| TERM-PERF-02 | Bursts with graphics are parsed before publishing; idle and spinner CPU must not rise. Compare with a build of `main`. |
| REMOTE-TERM-02 | Resizes on a remote terminal carry the new companion fields. |
| REMOTE-TERM-03 | **Take back** and **Take control** now carry the cell size. |
| Static images | Emit RGB, RGBA, PNG, `o=z`, `t=d`, `t=t` and `t=f` images in an owned shell, once on a Retina display and once on a non-Retina one, including one with `c` and `X`/`Y` set and one at its native size. Images are sharp at their native pixels and keep their aspect ratio, `z < 0` lies under text and `z ≥ 0` over it, they stay inside the pane, and the command palette and find bar draw over them; `clear` and `reset` remove them. |
| Placeholders and animation | A Unicode-placeholder image inside `tmux` follows its cells as the pane scrolls; an animated image plays while shown and stops when its tab is hidden. |
| Scrollback and snapshots | Place an image, print until it is in scrollback, scroll to it and back, then Keep, quit and reopen the app: it is still on its row, after the temporary file is gone. Lower the scrollback limit until the image's row is dropped: the image goes with it. |
| Pixel geometry and 1016 | A raw-TTY receiver reads the window size and `CSI 14 t` and `CSI 16 t`, enables `CSI ? 1003 ; 1016 h`, and logs a diagonal drag, a hover, moving onto the command palette and a wheel turn. Sizes are non-zero and match the pane, pixel positions start at 0 and grow smoothly within cells, hover is reported over the pane only, and the shell is clean after exit. Then move the window to a display with another scale: the receiver sees the new size. |
| Streaming and memory | A 30 fps loop replaces a full-pane image 300 times in a split beside a live shell, with the session also open in a second window. No capture shows the pane without an image, typing in the shell stays responsive, the mirrors do not resync repeatedly, a hidden tab receives no graphics, closing one window keeps the image in the other, and the app's and daemon's memory return to their earlier level once the image is deleted. Where `terminal-browser` is installed, also click and scroll a page in it. |
| Remote window | On the relay fixture, show a host shell that displays an image in a remote window: the remote terminal shows the text around it with no stray bytes and no repeated resync, and the host's own pane shows the image. |
| Windows ConPTY (win) | Emit a `t=d` image and query `CSI 14 t` and `CSI 16 t` in an owned PowerShell. Record whether the image appears and the replies arrive; Windows support is claimed only if they do. |

Selected regression cases: `TERM-RENDER-01`, `TERM-RENDER-02`, `TERM-RESIZE-01`, `TERM-RESIZE-02`, `TERM-QUERY-01`, `TERM-HISTORY-02`, `TERM-SET-02`, `TERM-PERF-02`, `REMOTE-TERM-02`, `REMOTE-TERM-03`. `TERM-SCROLL-01` and `TERM-FIND-04` belong to #1508's Phase 1, which moves scrolling and find onto the new engine.
