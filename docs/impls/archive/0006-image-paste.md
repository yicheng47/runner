# Image paste into runner terminal

> Bugfix for #79. Restores NSPasteboard mid-paste so the embedded
> xterm's `Cmd+V` of a PNG produces the same agent attachment a host
> `Terminal.app` paste would — `[Image x]` placeholder with the real
> screenshot bytes attached.

## Why

After v0.1.3 a paste handler was wired onto xterm's hidden textarea
(`RunnerTerminal.tsx:225`, capture phase) to swallow image pastes and
inject `\x16` so claude-code / codex shell out to read the system
clipboard themselves. Field reports said it still wasn't working —
Cmd+V on a screenshot landed in the agent as an attachment described
as "a generic macOS PNG file icon, not actual content." The same
Cmd+V from the user's host `Terminal.app` running `claude` directly
works fine.

The difference is WKWebView. When WKWebView dispatches the JS `paste`
event, it materializes image clipboard items into `File` objects by
writing the image bytes to a temp file under the hood. As a side
effect it mutates NSPasteboard so `public.png` now resolves to the
temp file's *icon* rather than the original screenshot bytes.
`preventDefault()` doesn't help — the mutation happens before JS sees
the event. By the time our `\x16` reaches `claude` and `claude` reads
NSPasteboard, the OS clipboard has already been overwritten.

## Considered alternatives

1. **Type the path into the prompt**: read the bytes JS-side, persist
   to disk via Rust, type the absolute path with a trailing space.
   Verified the bytes survive end-to-end, but the agent then shows
   the raw path in the prompt instead of its native `[Image x]`
   placeholder. The user can submit and the agent reads the file
   from disk, but the UX is markedly worse than a host-terminal
   paste. Rejected.
2. **Re-implement `[Image x]` placeholder ourselves** (write a
   placeholder + send the file via some side channel). Requires
   guessing each CLI's internal protocol; brittle. Rejected.
3. **Restore NSPasteboard before the agent's clipboard read runs**
   (this doc). Native `[Image x]` flow keeps working, no per-CLI
   custom protocol. Selected.

## Fix

End-to-end pipeline:

1. `RunnerTerminal.tsx` `onPaste` (capture phase on xterm's hidden
   `<textarea>`):
   - Iterate `clipboardData.items`, find the first supported image
     file, get the `File` via `getAsFile()`, and preserve the image
     MIME type. The original fix filtered to PNG only; #234 extended
     the allowlist to PNG and JPEG, with extension fallback for copied
     files whose MIME is unavailable.
   - `preventDefault()` + `stopImmediatePropagation()` so xterm.js's
     bubble-phase text-paste handler doesn't also run.
   - `await file.arrayBuffer()` → `Uint8Array` (the original bytes
     WebKit captured when it built the JS event, before the
     downstream pbpaste sees the corrupted clipboard).
   - `await api.session.pasteImage(bytes, mimeType)` — Rust side
     restores NSPasteboard.
   - `await api.session.injectStdin(sid, "\x16")` — agent CLI now
     sees Ctrl-V, reads NSPasteboard, gets the real bytes, renders
     its `[Image x]` placeholder.

2. Rust command `session_paste_image(bytes, mimeType)`
   (`src-tauri/src/commands/session.rs`):
   - Writes `bytes` to a `tempfile::NamedTempFile` in `$TMPDIR` with
     prefix `runner-paste-` and a suffix chosen from the MIME type
     (`.png` or `.jpg`). `NamedTempFile` is deleted on `Drop`, so the
     file is gone before the command returns — pasted screenshots can
     be sensitive, shouldn't accumulate, and the OS reaper isn't
     load-bearing.
   - On macOS, runs:

     ```
     osascript -e 'set the clipboard to (read POSIX file "<path>" as «class PNGf»)'
     ```

     `«class PNGf»` is the four-char OSType code for PNG; JPEG uses
     `«class JPEG»`. The statement reads the file as image bytes and
     writes them to the matching NSPasteboard representation,
     overwriting whatever icon WebKit left there. The path is
     interpolated via Rust's `{:?}` debug format so paths with spaces
     or quotes survive (the escape syntax `\\` / `\"` is shared
     between Rust string literals and AppleScript string literals).
   - On non-macOS the command is a no-op — the embedded webview's
     paste behavior on Linux / Windows hasn't been audited and the
     runner doesn't ship there yet.
   - Returns `Result<()>`. An `osascript` failure propagates up
     through Tauri's invoke into the JS catch, which surfaces via
     `onErrorRef`.

3. Wired into the Tauri invoke handler in `lib.rs`.

Plain-text pastes are unchanged: when no clipboard item has a supported image type, the handler returns early without `preventDefault`, and xterm.js's bubble-phase paste handler runs normally.

## Format allowlist

The AppleScript writes the bytes verbatim into the selected pasteboard flavor, so the MIME must map to the correct OSType. The current allowlist is PNG (`«class PNGf»`) and JPEG (`«class JPEG»`, including `.jpg` / `.jpeg` copied-file fallbacks). GIF/WebP would need more OSType mappings or a transcode step before they can be allowed through.

## Considered: `arboard` / `tauri-plugin-clipboard-manager`

Both expose a "set image" API that bypasses `osascript`. Rejected for
v1:

- `arboard::Clipboard::set_image` takes pre-decoded RGBA, not PNG
  bytes — we'd need to add the `image` crate to decode first, then
  arboard re-encodes. Two unnecessary passes.
- `tauri-plugin-clipboard-manager` adds a plugin we'd otherwise not
  use. The single `osascript` call is the lightest path.

If we later need cross-platform support, arboard + the `image` crate
(or the plugin) becomes the right shape; the surface here stays the
same.

## What this replaces

- The earlier `\x16`-then-let-the-CLI-read-pbpaste path was correct
  in spirit but structurally broken across the webview boundary —
  the clipboard the CLI reads is no longer the clipboard the user
  copied.
- The intermediate "type the path into the prompt" pass got the
  bytes through but lost the native `[Image x]` rendering. Replaced
  by the clipboard-restore + `\x16` pipeline.
- The earlier `[#79 paste items]` diagnostic `console.log` is gone
  — we now control the full path end-to-end and don't need to
  probe.

## Out of scope

- **Drag-and-drop into the embedded terminal.** xterm.js's textarea
  fires `drop` events, but Tauri's webview drop handling needs its
  own wiring. Paste covers the immediate UX gap; drop is a
  follow-up.
- **Multi-image pastes.** Handler picks the *first* `image/png`
  item; multi-image clipboards are rare.
- **Non-PNG image formats.** See "PNG-only" above.
- **Linux / Windows clipboard restore.** Both ship with different
  clipboard models; we'd need separate platform branches and a
  non-`osascript` mechanism. Out of scope until the runner ships on
  those platforms.

## Verification

- Locally confirmed: after the osascript call, `osascript -e
  'clipboard info'` reports `«class PNGf»` populated at the exact
  byte count of the source PNG.
- User-confirmed end-to-end paste of a real screenshot now renders
  as `[Image x]` with the actual content in the runner chat.
- `pnpm tsc --noEmit`, `pnpm lint`, `cargo check`, `cargo clippy` —
  clean.
