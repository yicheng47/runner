# Paste file paths: re-land #368 without touching the image flow

## Status

Implemented (#368 closed 2026-07-29, `a8e7cf3`; native port in `crates/runner-app/src/terminal_paste.rs`). Tracking issue [#368](https://github.com/yicheng47/runner/issues/368), spec [55](../features/55-paste-file-paths.md). This is a **redo**: the feature first landed in PR [#369](https://github.com/yicheng47/runner/pull/369) (commit `61cafd2`) and was reverted when main was rebuilt around the #372 spawn-width fix. The reverted code survives on the local branch `fix/367-368-spawn-width-and-paste-paths` and is the reference implementation — most of it re-lands verbatim; the one behavioral correction is decision 1.

## Problem

Copying a file (Finder, GoLand) puts only `public.file-url` flavors on NSPasteboard — no text — so ⌘V over a Runner terminal inserts nothing. Native terminals insert the POSIX path. The webview cannot recover the path (`DataTransfer` withholds it by design), so it must be read from NSPasteboard in Rust.

The reverted implementation solved this but changed an existing behavior along the way: its precedence rule ("a file reference beats image bytes") made a Finder-copied `shot.png` paste its *path*, where today it attaches as an image via `inferPasteImageMime`'s extension fallback. The re-land constraint is explicit: **the current image paste behavior (#79 / #234) must not change in any case.**

## Key Decisions

1. **Image scan first, text second, path branch only in the dead zone.** (Inverts the reverted feature-55 decision 3.) `onPaste` keeps main's exact ordering: the synchronous image scan runs first and anything it catches — screenshot bytes, browser image copies, and Finder-copied image *files* — takes the existing attach flow untouched. Non-empty `text/plain` still falls through to xterm's default paste. Only when both come up empty (today: the handler returns and nothing is inserted) does the new branch commit the event and consult the pasteboard. Structurally this guarantees the constraint: every clipboard shape that does something today is dispatched before the new code runs.

2. **Reuse the reverted orchestration module, reordered.** `src/lib/terminalPaste.ts` from `61cafd2` re-lands as the testable orchestration (`handleTerminalPaste` + effects interface), with its internal order changed from text→file→image to image→text→file per decision 1. Its helpers re-land unchanged: `clipboardHasUsableText`, `shellQuotePath` (quote only when whitespace/metacharacters require it, `'` → `'\''`), `formatPastedPaths` (space-joined), and the `normalizePasteImageMime` / `inferPasteImageMime` pair moves out of `RunnerTerminal.tsx` with it. `RunnerTerminal.tsx`'s `onPaste` shrinks to reading refs and delegating.

3. **Reuse the reverted Rust command as-is.** `session_clipboard_file_paths` (`commands/session.rs` on the reference branch): direct `objc2` NSPasteboard binding reading `NSPasteboardTypeFileURL` per item, decoding through `NSURL` (never hand-parsing the percent-encoded URL), filtering non-file URLs, returning `Vec<String>` of POSIX paths; empty when the flavor is absent; `#[cfg(not(target_os = "macos"))]` returns empty. Requires adding `NSPasteboard` (and the `NSPasteboardItem`/`NSURL` support it compiles against) to the `objc2-app-kit` feature list in `src-tauri/Cargo.toml` and registering the command in `lib.rs`. Its doc comment must be updated: the flavor no longer decides path-vs-attach (that was old decision 3); it only feeds the dead-zone branch.

4. **Commit synchronously, act asynchronously.** `preventDefault` after an `await` is a no-op, so the handler reads the event synchronously (image scan + text check), commits with `preventDefault` + `stopImmediatePropagation`, then goes async to the pasteboard command. An empty result swallows a paste that had nothing to insert anyway.

5. **Insert via `injectStdin`, never `inject_paste`.** A pasted path is mid-sentence; `inject_paste` appends Enter. The draft gate (spec 54) sees ordinary local input.

6. **Frontend API surface**: one `api.ts` addition, `session.clipboardFilePaths(): Promise<string[]>`, mirroring the `session.pasteImage` call shape from the reference branch.

## Goals

- Copying a non-image file and pasting into a pane inserts its absolute path (quoted only when needed); multiple files insert space-separated.
- Every clipboard shape that does something today — text, screenshot bytes, browser image copy, Finder-copied image file — behaves byte-for-byte as before, with no added IPC on those paths.
- The orchestration is unit-tested without mounting xterm, including regression tests pinning the image-file-still-attaches behavior.

## Non-Goals

- Drag-and-drop of files onto a pane (different Tauri mechanism, its own spec).
- Directory contents, cwd-relative rewriting, `text/uri-list` parsing.
- Any change to the image-paste flow, the #367/#372 spawn-width machinery, or `inject_paste` semantics.

## Implementation Notes

- `src-tauri/Cargo.toml` — add `NSPasteboard` feature(s) to `objc2-app-kit`; `objc2-foundation` may need `NSURL` if not already enabled.
- `src-tauri/src/commands/session.rs` — `session_clipboard_file_paths` (from `61cafd2`, doc comment corrected per decision 3).
- `src-tauri/src/lib.rs` — register the command.
- `src/lib/api.ts` — `session.clipboardFilePaths`.
- `src/lib/terminalPaste.ts` — orchestration from `61cafd2`, reordered per decision 1; mime helpers move here from `RunnerTerminal.tsx`.
- `src/lib/terminalPaste.test.ts` — re-land the reverted suite, updated for the new precedence; add the two pinning tests: image *bytes* attach, image *file* attaches (not a path), even with file-urls on the pasteboard.
- `src/components/RunnerTerminal.tsx` — `onPaste` delegates to `handleTerminalPaste` with effects wired to `api.session.*` and `onErrorRef`; keep the `!sid || disabledRef.current` early return ahead of everything.

Reference diff: `git show 61cafd2` (or diff the local branch `fix/367-368-spawn-width-and-paste-paths`) — take only the paste-path files; the spawn-width/db changes there are superseded by #372 and must not be re-landed.

## Validation

- Frontend (vitest, `terminalPaste.test.ts`): text paste → null return, no IPC; image bytes → attach flow + Ctrl-V; **image file with file-urls present → attach flow, never a path** (the redo's pin); non-image file → committed, quoted path injected; multiple files → space-separated; empty pasteboard → committed no-op; quoting table (bare, spaces, embedded `'`, metacharacters).
- Rust: quoting lives frontend-side, so backend needs no new logic tests beyond compile; `cargo test --workspace` for regressions.
- Checks: `cargo fmt --check`, `cargo clippy`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
- Manual: the spec's Verification list, run by Jason in the app — most importantly ⌘⇧4 screenshot paste and Finder image-file paste behaving exactly as v0.4.7.
