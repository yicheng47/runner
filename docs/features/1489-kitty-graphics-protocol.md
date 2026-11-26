# 1489 — Kitty Graphics Protocol, pixel geometry and SGR-Pixels mouse in terminal panes

Tracking issue: [#1489](https://github.com/yicheng47/runner/issues/1489). No new UI chrome or Pencil screen of its own; visual applications that launch into these panes (`terminal-browser`, Launcher polish and launch defaults) are tracked separately in [#1492](https://github.com/yicheng47/runner/issues/1492). Because this work touches `runner-terminal`, `runner-daemon` (PTY output, resize and query replies) and `runner-core` (client wire protocol and attach snapshots), pull requests carry `core-review`.

Code references are to `main` at `9d9c50b6` (2026-11-26). Ghostty's MIT-licensed `src/terminal/kitty/` (`graphics_command`, `graphics_image`, `graphics_storage`, `graphics_exec`, `graphics_unicode`, full protocol parity as of Ghostty 1.4.0) is the behavioral and test-vector reference.

## Motivation

Modern terminal applications render rich visual surfaces directly inside ordinary terminal panes — headless browsers (`terminal-browser`, `KittyBrowser`), mirrored desktop windows (`krd`), mobile simulators (`rex-sim`, `scrcpyt`) and inline image viewers (`kitten icat`, `chafa`) — using the Kitty Graphics Protocol, real pixel geometry reporting and SGR-Pixels mouse coordinates (`DECSET 1016`).

Supporting the full Kitty Graphics Protocol natively in Runner's terminal pipeline lets visual tools run as ordinary `runnerd`-owned PTY sessions. Unlike an in-process `wry` child WebView (retired in [#1260](https://github.com/yicheng47/runner/issues/1260)), terminal graphics paint inside GPUI's normal z-order behind popovers, tooltips and the command palette, clip cleanly to rounded pane islands, work identically on Linux Wayland, survive app restarts via daemon snapshots and can stream to remote desktop clients and `runner-ios`.

## Scope

In scope across two implementation phases sharing one unified Ghostty-aligned protocol model:

- **Phase 1 — Core Kitty graphics, pixel geometry, SGR-Pixels mouse, GPUI rendering and local snapshots (unblocks [#1492](https://github.com/yicheng47/runner/issues/1492), `terminal-browser`, `krd`, `rex-sim` and `kitten icat`):**
  - APC byte-stream pre-scanner before `alacritty_terminal`'s `vte` processor so `ESC _ G <control>;<payload> ESC \` (including chunked `m=1`/`m=0` transfers and synchronized updates `CSI ? 2026 h/l`) executes in exact cursor order without dropping sequences.
  - Full Kitty graphics command parser covering all action and control keys defined by the spec (`a=q/t/T/p/d/f/a/c`, `i`, `I`, `p`, `f`, `t`, `s`, `v`, `o`, `m`, `x`, `y`, `w`, `h`, `X`, `Y`, `c`, `r`, `C`, `z`, `q`, `U`, `P`, `Q`, `H`, `V`, `d`, `S`, `O`), wired into a Ghostty-style `ImageStorage` state machine with primary/alternate screen separation and bounded per-terminal memory with LRU eviction.
  - Image decoding and transmission mediums: `f=24` (RGB), `f=32` (RGBA), `f=100` (PNG), `o=z` (zlib), and `t=d` (direct base64), `t=f` (local file), `t=t` (local temp file read-and-unlink) and `t=s` (POSIX shared memory read-and-unlink where supported), with daemon-only local ingestion so mirrors and remote clients never touch host files or race on unlink.
  - Classic and relative placements (`a=t`, `a=T`, `a=p`, `P`/`Q`/`H`/`V` parent offsets, `x`/`y`/`w`/`h` source crop, `X`/`Y` sub-cell offset, `c`/`r` cell scaling, `C=1` no-cursor-move, `z`-index layering), scrollback anchoring/trimming and all deletion selectors (`a`/`A`, `i`/`I`, `n`/`N`, `c`/`C`, `p`/`P`, `q`/`Q`, `x`/`X`, `y`/`Y`, `z`/`Z`, `r`/`R`, `f`/`F`), plus clear-screen (`ED 2`/`3`) and reset (`RIS`/`DECSTR`) cleanup.
  - Authoritative query and response handling (`a=q` and `OK`/error replies honoring `q=1`/`q=2`) emitted strictly from `TerminalModel` in `runnerd`, never from `TerminalMirror`.
  - Real pixel geometry on PTY open and resize (`portable_pty::PtySize` `pixel_width`/`pixel_height` for `TIOCGWINSZ`, plus non-zero `cell_width`/`cell_height` in `Event::TextAreaSizeRequest` answering `CSI 14 t` and `CSI 16 t`).
  - SGR-Pixels mouse reporting (`DECSET 1016` set/reset, `DECRQM` query, snapshot persistence and pixel-relative `CSI < btn ; px ; py M/m` encoding for press, release, drag/motion and wheel).
  - GPUI rendering in `TerminalElement` (`crates/runner-app/src/terminal/element.rs`) with `z < 0` behind text/cells and `z >= 0` above text, clipped to the pane's content mask, using BGRA `RenderImage` uploads and explicit `cx.drop_image` cleanup on replacement, deletion, eviction and mirror teardown.
  - Local attach snapshots (`TerminalSnapshot`, [#42](https://github.com/yicheng47/runner/issues/42)) carrying active visible/retained image state so closing and reopening the desktop app preserves live graphics without re-reading unlinked temp files.
- **Phase 2 — Full Ghostty 1.4 parity and remote streaming:**
  - Unicode placeholders (`U=1`, `U+10EEEE` plus diacritic row/column/image-ID encoding read from `alacritty_terminal`'s `Cell` and `CellExtra.zerowidth`), enabling graphics inside multiplexers and inline text flow.
  - Animation commands (`a=f` transmit frame, `a=c` compose frame, `a=a` playback control with gaps, loops and composition modes) with visibility-gated GPUI frame scheduling so hidden tabs/windows run no timers.
  - Remote desktop and `runner-ios` graphics channel delivery with compression, size caps and frame-drop backpressure for high-fps streaming tools.

Out of scope for `#1489`:

- Launcher UI, `Settings → Launcher`, `TERMINAL_BROWSER_TERMINAL_SOCKET` and macOS `⌘` browser shortcut routing (tracked in [#1492](https://github.com/yicheng47/runner/issues/1492)).
- Sixel or iTerm2 (`OSC 1337`) image protocols.
- Bundling `terminal-browser`, Chromium, Electron or any external visual tool with Runner.

## Design

### Reference architecture (Ghostty `src/terminal/kitty/`)

Organize the terminal graphics subsystem in `crates/runner-terminal/src/kitty/` mirroring Ghostty's clean separation of concerns:

- `command.rs` (reference: `graphics_command.zig`): zero-allocation-friendly parser for `k=v` control pairs and base64 payload framing, supporting every protocol key and action from phase 1 so phase 2 adds execution rather than re-parsing.
- `image.rs` (reference: `graphics_image.zig`): chunked transfer accumulator (`LoadingImage`), transmission-medium loader (`t=d/f/t/s`), zlib decompression (`o=z`), RGB/RGBA/PNG decoding into normalized RGBA (or BGRA-ready) pixel buffers and strict dimension/byte bounds.
- `storage.rs` (reference: `graphics_storage.zig`): per-screen `ImageStorage` holding images by `image_id` and `image_number`, placements keyed by `(image_id, placement_id)`, relative parent→child links, scroll-anchored row coordinates, LRU byte accounting and deletion filters.
- `exec.rs` (reference: `graphics_exec.zig`): pure state transitions from parsed `Command` + current cursor/screen dimensions → storage mutations + optional Kitty reply string (`OK` / `EINVAL` / `ENOENT` / `ENODATA` / `ENOSPC`).
- `unicode.rs` (phase 2; reference: `graphics_unicode.zig`): `U+10EEEE` placeholder and Kitty diacritic table decoding over visible `alacritty_terminal` cells.

### APC pre-scanner and stream ordering

`alacritty_terminal`'s `vte` 0.15.0 parser ignores APC (`ESC _ ... ESC \`) sequences without calling any `Handler` method. `TerminalModel::feed_output` (`crates/runner-terminal/src/terminal/model.rs`) already runs byte-stream pre-scanners (`scan_scheme_sequences` and `program_status.scan`) that split chunks at exact sequence offsets and call `parse(&mut parser, &mut term, &bytes[fed..end])` before handling each intercepted event.

Extend that pre-scanner pattern (shared by both `TerminalModel` and `TerminalMirror` via `TrackedProcessor` / a shared terminal stream splitter in `crates/runner-terminal`) with a stateful cross-chunk scanner that detects:

- **Kitty APC packets:** `ESC _ G <control>[;<payload>] ESC \` (and cancels cleanly on `CAN` `0x18`, `SUB` `0x1A` or unexpected `ESC` without leaving the scanner wedged). Bytes preceding each APC packet are fed to `vte` first so `term.grid().cursor.point` and `TermMode::ALT_SCREEN` reflect the exact cursor position and active screen when the Kitty command executes.
- **DEC private mode 1016 (`SGR-Pixels`):** `CSI ? 1016 h`, `CSI ? 1016 l` and `CSI ? 1016 $ p` (`DECRQM`), plus resets on `RIS` (`\x1bc`) and `DECSTR` (`\x1b[!p`), since `alacritty_terminal` 0.26.0 does not define a `TermMode` bit for 1016.
- **Synchronized updates (`CSI ? 2026 h/l`):** while `vte` buffers a synchronized frame, Kitty graphics mutations either queue in lockstep with the sync buffer or commit atomically at `CSI ? 2026 l` (and on sync timeout flush) so streaming apps like `terminal-browser` do not tear between text and image updates.

### Single-reader daemon ingestion for `t=f`, `t=t` and `t=s`

In Runner, `TerminalModel` runs inside `runnerd` while each attached app window maintains its own `TerminalMirror` fed from `TerminalFrame::Output`. If both the daemon and the mirror executed raw `t=t` (read-and-unlink temp file) or `t=s` (read-and-unlink POSIX shared memory) sequences, they would race and fail with `ENOENT`, and remote mirrors would attempt to read host paths that do not exist locally.

Therefore `runnerd` is the **sole reader and unlinker** for local file and shared-memory mediums:

1. During output ingestion in `runnerd` (before publishing `TerminalFrame::Output` to subscribers), the daemon's graphics ingestor resolves any `t=f`, `t=t` or `t=s` Kitty APC sequence once on the host:
   - **`t=t`:** enforce Kitty's safety checks — path must be a regular file (never symlink-following into sensitive directories, no devices/FIFOs), located under system temp dirs (`/tmp`, `/var/tmp`, `std::env::temp_dir()`) and containing `tty-graphics-protocol` in its path component (matching Ghostty/Kitty), bounded by `MAX_IMAGE_FILE_BYTES`, then read and unlink.
   - **`t=f`:** regular file only (never `/dev/*`, `/proc/*`, FIFOs or directories), bounded by `MAX_IMAGE_FILE_BYTES`, read without unlinking.
   - **`t=s` (Unix):** open POSIX shared memory object (`shm_open`), verify bounded size, map/read and `shm_unlink`.
   - Reject local-path mediums (`t=f`, `t=t`, `t=s`) outright for sessions whose origin is untrusted/remote if ever applicable; local host PTYs execute them inside `runnerd`.
2. After reading the payload bytes in `runnerd`, normalize the outbound subscriber representation so mirrors receive inline bytes (`t=d`, or decoded/compressed image frames over the attach/output stream) without ever touching the filesystem or POSIX shared memory.
3. Queries (`a=q`) and responses (`OK` / error codes) are emitted exclusively by `TerminalModel` through `TerminalHost` (`crates/runner-terminal/src/terminal/model.rs`). `TerminalMirror` discards all Kitty replies, preserving the single-reply invariant (`TERM-QUERY-01`).

### Memory limits and placement lifecycle

- **Per-image limits:** reject images exceeding `10,000 × 10,000` pixels or `64 MiB` decoded RGBA bytes (matching Ghostty's guardrails and Runner's `wire::MAX_FRAME` ceiling), replying with `EINVAL` unless silenced by `q=2`.
- **Per-terminal budget:** default `320 MiB` decoded image storage per terminal screen (configurable constant in `runner-terminal`, matching Ghostty's `320 MiB` default), with LRU eviction of unreferenced or oldest images when exceeded and immediate eviction when a session ends.
- **Screen and scrollback lifecycle:** primary and alternate screens own separate placement tables (clearing alt-screen on `CSI ? 1049 l` drops alt-screen placements without wiping primary images). Primary-screen placements anchor to absolute scrollback line indices (tracking `Grid` line rotation as history grows) and drop automatically when their last visible row scrolls past `max_scrollback` or is erased by `ED 2`/`3`, `RIS` or `DECSTR`.
- **Cursor movement:** when `C=0` (default) on `a=T` or `a=p`, advance the cursor by the placement's cell span (`cols`, `rows`) through `alacritty_terminal` (including scrolling if the placement reaches the bottom margin) just as Ghostty's `graphics_exec.zig` does; when `C=1`, leave the cursor unchanged.

### Pixel geometry (`TIOCGWINSZ`, `CSI 14 t` and `CSI 16 t`)

Today `TerminalFrame::Resized`, `wire::RESIZE`, `TerminalMirror::resize`, `SessionManager::resize_terminal`, `TerminalModel` and `PtyRuntime` only pass `(cols, rows)` and hardcode `pixel_width: 0`, `pixel_height: 0` (`crates/runner-daemon/src/session/pty_runtime.rs:222-223`, `:534-535`) and `cell_width: 0`, `cell_height: 0` (`crates/runner-terminal/src/terminal/model.rs:251-252`).

Extend the resize contract end to end:

- Carry `cell_width: u16` and `cell_height: u16` (in physical device pixels — i.e., logical cell size multiplied by `window.scale_factor()`, rounded, matching Ghostty/Kitty on Retina displays so high-DPI tools render sharp pixels; document logical-vs-device pixel ratio clearly for `DECSET 1016` consistency) alongside `cols` and `rows` from `TerminalElement::prepaint` → `TerminalMirror::resize` → `wire::RESIZE` → `SessionManager::resize_terminal` → `PtyRuntime::resize` and `TerminalModel`.
- Set `PtySize { rows, cols, pixel_width: cols.saturating_mul(cell_width), pixel_height: rows.saturating_mul(cell_height) }` on `openpty` (when initial metrics are known) and on every `master.resize(...)`.
- Store `(cols, rows, cell_width, cell_height)` in `TerminalModel` and populate `WindowSize { num_lines: rows, num_cols: cols, cell_width, cell_height }` in `Event::TextAreaSizeRequest` (`crates/runner-terminal/src/terminal/model.rs:245-256`) so `CSI 14 t` (`4;height;width t`) and `CSI 16 t` (`6;cell_h;cell_w t`) return exact non-zero pixel dimensions.
- Keep `wire::RESIZE`, `wire::RESIZED` and `TerminalFrame::Resized` wire-compatible within the build (app and `runnerd` already match `Hello::exe_sha256`), and treat zero pixel fields from CLI/test callers as "preserve previous cell pixel size".

### SGR-Pixels mouse (`DECSET 1016`)

- Track `sgr_pixels_mouse: bool` in `TerminalModel` and `TerminalMirror` alongside `TermMode`, persisted via `\x1b[?1016h`/`\x1b[?1016l` in `crates/runner-terminal/src/snapshot.rs` and answered on `DECRQM` (`\x1b[?1016;1$y` or `\x1b[?1016;2$y`).
- Extend `crates/runner-terminal/src/mappings.rs` (`encode_mouse_press`, `encode_mouse_release`, `encode_mouse_motion`, `encode_scroll`) with an optional pixel-coordinate encoding path: when `sgr_pixels_mouse` is active alongside any `TermMode::MOUSE_MODE` bit, emit `CSI < btn ; px ; py M` (or `m` on release) where `px` and `py` are clamped pane-relative coordinates in the same pixel coordinate space reported by `CSI 14 t` / `TIOCGWINSZ` (never 1-based cell indices), and fall back to standard `SGR_MOUSE` (`1006`) or X10 encoding otherwise.
- In `crates/runner-app/src/terminal/element.rs`, update `TerminalInteraction` drag deduplication (`DragKind::Reported`) to track pixel coordinates instead of cell coordinates while `1016` is enabled so sub-cell drags (drawing, sliders, text selection inside `terminal-browser` or `rex-sim`) send smooth motion events rather than coalescing per cell.

### GPUI rendering and atlas lifecycle

Follow the texture lifecycle pattern established in `crates/runner-app/src/surfaces/files/image.rs`:

- Convert decoded Kitty RGBA image buffers to GPUI's BGRA `Arc<RenderImage>` lazily on first paint (or background upload) and cache them per `TerminalMirror` keyed by `(image_id, image_generation)` so streaming frames that reuse an `image_id` replace the previous texture cleanly.
- Track a per-mirror retired-image queue (`Vec<Arc<RenderImage>>`) populated whenever an image is overwritten, deleted, evicted by LRU or cleared on reset/alt-screen exit. On the next `TerminalElement::prepaint` (with `&mut Window`) — and on mirror teardown via `cx.drop_image(img, None)` — drain and call `cx.drop_image(img, Some(window))` so 30–40 fps streaming sessions never leak GPUI's polychrome sprite atlas.
- In `TerminalElement::prepaint` and `paint`, split visible placements into three painter's-order bands matching the Kitty spec:
  1. `z < i32::MIN / 2` (below cell background quads — painted immediately after pane background fill, before non-default cell background quads).
  2. `i32::MIN / 2 <= z < 0` (above cell background quads, below text/procedural glyphs and cursor).
  3. `z >= 0` (above text and selection, clipped to the pane's `ContentMask`, below any app popovers/command palette/find bar).
- Paint placements sorted by `(z, image_id, placement_id)` using source crop (`x, y, w, h`) — pre-cropped or sub-image sliced when a source rectangle is specified — scaled to the target cell rectangle (`c, r` or natural pixel dimensions divided by cell pixel size, shifted by `X, Y` sub-cell offsets).
- Benchmark GPUI `paint_image` + `cx.drop_image` at 30 fps full-pane updates (e.g. `terminal-browser` scrolling) during phase 1; only introduce a dedicated GPU texture/surface bypass if profiling reveals sprite-atlas fragmentation or frame-time regression.

### Attach snapshots (`#42`) and remote streaming

- **Phase 1 (local snapshots):** extend `TerminalSnapshot` (`crates/runner-core/src/protocol/terminal.rs` and `crates/runner-core/src/protocol/wire.rs`) with an optional serialized graphics snapshot payload (chunked via `SNAPSHOT_DATA` or a dedicated graphics snapshot chunk frame within `wire::MAX_FRAME`) carrying active images (PNG- or zstd/zlib-compressed, capped by a snapshot budget so attach stays fast) and active primary/alt placements plus `sgr_pixels_mouse` state. When a local client attaches or receives `Resync`, `TerminalMirror::attach` restores both the VT grid and the live image/placement tables atomically at `snapshot.seq`.
- **Phase 2 (remote and `runner-ios`):** gate remote/companion graphics transmission behind capability negotiation (`Hello` / companion capabilities), compress full/dirty frames, and coalesce or drop superseded frames for the same `(image_id, placement_id)` under relay backpressure so a 30 fps browser stream never stalls text typing or triggers `Resync` loops.

## Implementation Phases

1. **Phase 1a — `runner-terminal` Kitty core (`kitty/` module + APC scanner + `DECSET 1016`):** port Ghostty's `command`, `image`, `storage` and `exec` semantics in pure Rust with unit tests covering Ghostty's protocol test vectors (chunking, `f=24/32/100`, `o=z`, `t=d/f/t/s`, classic + relative placements, z-index, cropping, `C=1`, every delete selector, LRU eviction, primary/alt separation, `a=q` / `q=1` / `q=2`, and `DECSET 1016` + `mappings.rs` pixel mouse encoding).
2. **Phase 1b — Daemon ingestion, pixel geometry and local snapshots (`runner-core` + `runner-daemon`):** wire `cell_width`/`cell_height` through resize frames to `PtySize` and `TextAreaSizeRequest` (`CSI 14 t` / `16 t`), enforce single-reader daemon ingestion for `t=f/t/s` before subscriber fan-out, wire authoritative Kitty replies via `TerminalHost`, and persist graphics + `1016` mode in `TerminalSnapshot`.
3. **Phase 1c — GPUI rendering and atlas cleanup (`runner-app`):** render z-ordered Kitty placements in `TerminalElement::prepaint`/`paint` with source cropping and sub-cell offsets, drain retired `Arc<RenderImage>` textures via `cx.drop_image`, wire pixel coordinates from `TerminalInteraction` for `DECSET 1016`, and profile 30 fps streaming with `terminal-browser` / synthetic frame bursts.
4. **Phase 2 — Full Ghostty parity and remote throttling:** implement `U=1` Unicode placeholders (`unicode.rs` over `alacritty_terminal` `CellExtra.zerowidth`), `a=f/a/c` animation frames and visibility-gated playback timers, and remote/companion backpressure coalescing for desktop remote windows and `runner-ios`.

Phase 1 (1a–1c) is the shippable milestone that unblocks [#1492](https://github.com/yicheng47/runner/issues/1492); Phase 2 follows in a dedicated pull request without changing the Phase 1 data model.

## Guide pages

Phase 1 adds no new user-facing settings, shortcuts or commands; `docs/tech/terminal-rendering.md` and `docs/arch/arch.md` should be updated to describe the APC graphics pipeline, daemon-only local file ingestion, pixel geometry reporting and `DECSET 1016` mouse encoding. User-facing guide updates for launching `terminal-browser` belong to [#1492](https://github.com/yicheng47/runner/issues/1492).

## Verification

### Automated

- **Parser and storage (`runner-terminal`):** unit tests for chunked `m=1`/`m=0` reassembly across arbitrary byte splits, `f=24/32/100`, `o=z`, `t=d`, `t=f`, `t=t` (including path-safety rejections and unlink verification), `t=s` (Unix), classic and relative placements, source-rect cropping, `C=0` vs. `C=1` cursor advancement, z-ordering, scrollback anchoring/trimming, alt-screen isolation, every delete selector (`a/i/n/c/p/q/x/y/z/r/f` lowercase and uppercase), LRU eviction under memory cap, and `a=q` / `q=1` / `q=2` reply formatting.
- **Pixel geometry and `DECSET 1016` (`runner-terminal` + `runner-daemon`):** verify `PtySize` receives non-zero `pixel_width`/`pixel_height` on resize, `CSI 14 t` and `CSI 16 t` return exact pane and cell pixel sizes, `DECRQM` for `1016` reports set/reset accurately, and `mappings.rs` emits pixel-relative `CSI < btn ; px ; py M/m` for press, release, drag and scroll without breaking `SGR_MOUSE` (`1006`).
- **Single-reply and single-reader invariants (`runner-daemon`):** test with zero, one and two attached mirrors that `a=q` and `a=T` produce exactly one reply on the PTY, and that a `t=t` temp-file transmission is read and unlinked once by `runnerd` while both mirrors still receive the decoded image.
- **Attach snapshots (`runner-terminal` + `runner-core`):** round-trip active images, placements and `1016` state across `TerminalSnapshot` and `Resync`, including after the original `t=t` temp file has been unlinked.
- **Atlas lifecycle (`runner-app`):** test that replacing an image ID 200 times, deleting placements and dropping a `TerminalMirror` enqueue and release old `Arc<RenderImage>` handles through `cx.drop_image`.
- **Phase 2:** unit tests for `U+10EEEE` + diacritic decoding (including wide/cropped cells) and animation frame composition/looping plus visibility-gated timer pause.
- **Gates:** `cargo test -p runner-terminal -p runner-core -p runner-daemon -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`, plus a 20-run `cargo nextest run --stress-count 20` loop for every new async/daemon/timer test.

### Live checks

On the macOS development app (and Windows/Linux where tools are available), by `qa` holding `script/qa-lock`, using an owned scratch directory under `/tmp`:

| Check | Why |
| --- | --- |
| `TERM-RENDER-01` | `TerminalElement::prepaint`/`paint` gains three image z-order bands around backgrounds and text; ordinary colored TUIs and tab switching must stay intact. |
| `TERM-RESIZE-01` | Resize frames now carry cell pixel dimensions to `PtySize` and `TerminalModel`; continuous divider drags and window resizes must keep text and PTY geometry synchronized. |
| `TERM-SCROLL-01` | Primary-screen image placements anchor to scrollback lines; local scrollback, selection and returning to live prompt must remain exact. |
| `TERM-QUERY-01` | The pre-scanner now intercepts Kitty APC queries and `DECRQM 1016`; single-reply authority with zero, one and two viewers must remain unbroken. |
| `TERM-HISTORY-02` | `TerminalSnapshot` gains graphics and `1016` state; Keep/close/reopen must preserve both text history and live images. |
| `TERM-RESIZE-02` | Switching tabs and viewing one session in two windows must render identical image geometry without duplicate resize storms. |
| `TERM-PERF-02` | Pre-scanning every output chunk and painting GPUI textures must not regress idle or streaming terminal CPU. |
| Static image display (`kitten icat` or Python Kitty APC emitter) | Emit RGB, RGBA, PNG (`o=z`), `t=d` and `t=t` images in an owned shell: verify sharp rendering at Retina scale, crisp `z < 0` behind text and `z >= 0` over text, clipping inside rounded pane bounds, command palette/find bar drawing above the image, scrollback movement, `clear`/`reset` deletion and preservation across app Keep/reopen. |
| Pixel geometry & `DECSET 1016` probe | Run a bounded raw-TTY receiver in an owned shell querying `TIOCGWINSZ`, `CSI 14 t` and `CSI 16 t`, enabling `CSI ? 1016 h` + `CSI ? 1003 h`, clicking and dragging diagonally across a pane: verify non-zero pixel dimensions matching the pane, sub-cell monotonic pixel coordinates on drag, and clean restoration on exit. |
| High-fps streaming & memory stability (`terminal-browser` or 30 fps frame loop) | Stream 300+ full-pane image replacements at ~30 fps in a split pane beside a live shell (or launch `terminal-browser` directly in an owned shell on macOS): verify smooth interactive clicking/scrolling, responsive typing in the adjacent pane, and flat RSS/VRAM after stopping and clearing the image (no GPUI sprite-atlas leak). |
| Phase 2 (when Phase 2 lands): `U=1` placeholders & animations | Render a `U=1` placeholder image inside `tmux`/inline text and an animated GIF via `kitten icat`: verify animation plays while visible, pauses CPU work when switched to another tab, and resumes cleanly when returning. |

Selected regression cases: `TERM-RENDER-01`, `TERM-RESIZE-01`, `TERM-SCROLL-01`, `TERM-QUERY-01`, `TERM-HISTORY-02`, `TERM-RESIZE-02`, `TERM-PERF-02`.
