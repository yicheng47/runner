# 55 — Paste file paths into the terminal

Tracking: [#368](https://github.com/yicheng47/runner/issues/368)

History: first landed in PR [#369](https://github.com/yicheng47/runner/pull/369) and reverted when main was rebuilt around the #372 spawn-width fix. This spec re-lands the feature with one correction: the original decision 3 let a file reference beat image bytes, which changed the paste behavior of Finder-copied image *files*. The redo inverts that — the image flow is untouchable, and the path branch runs only where today's handler does nothing.

## Motivation

Copying a file in GoLand (or Finder) and pressing ⌘V over a Runner terminal inserts nothing. Every native terminal — Terminal.app, iTerm2, Ghostty — inserts the file's POSIX path instead, which is how you hand an agent a file to look at without typing the path out.

The cause is that a file copy puts no text on the clipboard. It writes `public.file-url` / `NSFilenamesPboardType` flavors to NSPasteboard, and native terminals read those flavors explicitly and insert the path. Over the WKWebView boundary the paste arrives as a `File` in `DataTransfer` with an empty (or unusable) `text/plain`, so xterm.js's default paste has nothing to insert.

Runner already intercepts paste for exactly this class of problem. `onPaste` in `RunnerTerminal.tsx` handles image paste — it walks `clipboardData.items` for `kind === "file"`, and for images ships the bytes to Rust to restore the NSPasteboard flavor (#79). A non-image file falls straight through: `inferPasteImageMime` returns null, the loop `continue`s, the handler returns **without** `preventDefault`, and xterm's default paste inserts the empty text. So the extension point exists and the gap is one branch wide.

The web layer cannot close it alone. `DataTransfer` deliberately withholds filesystem paths — `File.name` is only the basename, and WKWebView exposes no `file.path`. The path has to come from the native pasteboard, which is what the existing image path already reaches into.

## Scope

- ⌘V over a terminal pane, when the clipboard holds file references and neither an image the current flow would attach nor usable text, inserts the file's absolute path at the cursor, shell-quoted when needed.
- Multiple copied files insert as multiple quoted paths separated by single spaces.
- Text pastes and image pastes — **including Finder-copied image files** — keep their current behavior exactly.
- Out of scope: drag-and-drop of files onto a pane (same underlying need, but Tauri's drag-drop event supplies paths through an entirely different path — its own spec); pasting directory contents; translating paths to be relative to the session cwd; any change to the image-paste flow.

## Key Decisions

1. **The image flow is untouchable; the path branch runs only in today's dead zone.** (Inverts the reverted spec's decision 3.) The handler keeps main's exact ordering: the image scan runs first, and anything it catches — pasted screenshot bytes, browser image copies, *and* Finder-copied `shot.png`, which `inferPasteImageMime`'s extension fallback classifies as an image — takes the #79 attach flow unchanged. Then non-empty `text/plain` falls through to xterm unchanged. Only when both yield nothing — the case where today's handler inserts nothing at all — does the path branch consult the native pasteboard. Consequence accepted: an image *file* pastes as an attachment, not a path; users who want an image's path can get it from Finder's Copy-as-Pathname. The re-land constraint from the revert is exactly this: #368 must not change any behavior the image paste (#79 / #234) has today.

2. **Read the paths in Rust, not in the webview.** A command returns the current pasteboard's file URLs as POSIX paths. The frontend never sees a path it could have fabricated from `File.name`. This is a direct `objc2` NSPasteboard binding, not another `osascript` shell-out like the image write: the read happens on every candidate paste, and a process spawn per paste isn't worth the symmetry. `objc2-app-kit` is declared `default-features = false`, so the `NSPasteboard` feature must be added in `src-tauri/Cargo.toml`.

3. **Decide synchronously, act asynchronously.** `preventDefault()` after an `await` does nothing — the browser has already run the default action by the time an IPC round-trip resolves. So the handler cannot "call the command, then decide". It reads the event synchronously (image scan, text check), commits (`preventDefault` + `stopImmediatePropagation`) once both come up empty, and only then goes async to the pasteboard command. An empty result then swallows a paste that had nothing to insert anyway — a no-op.

4. **Prefer the native read over `text/uri-list`.** WKWebView sometimes exposes a `text/uri-list` flavor with `file://` URLs, which would avoid an IPC round-trip, but its presence is inconsistent across source applications. One authoritative path (the pasteboard) beats two code paths that disagree.

5. **Quote only when the path needs it.** iTerm2 quotes unconditionally because its panes are shell prompts. Runner's panes are usually *agent* prompts, where `'/Users/jason/foo.go'` is noise and defeats `@`-style file completion. Quote when the path contains whitespace or shell metacharacters, using single quotes with embedded quotes escaped (`'` → `'\''`); otherwise insert it bare.

6. **Insert as text, never submit.** Write through the existing raw-stdin injection (`api.session.injectStdin`). Do **not** route through `inject_paste`, which appends Enter — a pasted path is the middle of a sentence the user is still composing. The draft-gate state (spec 54) sees it as ordinary local input, which is correct: the user now has a pending draft.

7. **No cwd-relative rewriting.** Absolute paths always resolve regardless of where the agent has `cd`'d, and the agent can shorten them itself. Relativizing would need the session's live cwd, which Runner only knows at spawn time.

## Verification

- [ ] Copy a `.go` file in GoLand, ⌘V over a terminal pane — the absolute path appears at the cursor, unquoted, not submitted.
- [ ] Copy a file whose path contains a space — the pasted text is quoted and the agent resolves it.
- [ ] Copy two files at once — both paths appear, space-separated.
- [ ] Copy a file in Finder — same result as GoLand.
- [ ] Take a screenshot to the clipboard (⌘⇧4), paste — the existing `[Image #N]` attach flow is unchanged (#79).
- [ ] Copy an image *file* in Finder, paste — it **attaches as an image**, exactly as today (decision 1; this inverts the reverted spec).
- [ ] Copy ordinary text — pastes exactly as before, with no IPC round-trip.
- [ ] Paste with an empty clipboard — nothing happens, no error.
- [ ] Paste with the session stopped or the pane disabled — no injection, no error.
- [ ] The pasted path leaves the pane in a pending-draft state (spec 54), not a submitted one.

## Relevant Code

- `src/components/RunnerTerminal.tsx` — `onPaste` (~`:808`), the interception point whose image-first ordering decision 1 preserves; `inferPasteImageMime` (~`:97`), whose filename fallback is what keeps Finder-copied image files on the attach flow.
- `src-tauri/src/commands/session.rs` — `session_paste_image` and its MIME→OSType table, the plumbing neighborhood for the new read command.
- `src-tauri/Cargo.toml` — `objc2-app-kit` with `default-features = false`; `NSPasteboard` must be added to its feature list.
- `src/lib/api.ts` — `session.injectStdin`; `session.pasteImage`, the call shape to mirror.
- `docs/features/54-draft-aware-delivery-gate.md` — the draft model a pasted path feeds into (decision 6).
- Reverted implementation for reference: PR #369, commit `61cafd2` (local branch `fix/367-368-spawn-width-and-paste-paths`) — `src/lib/terminalPaste.ts`, `src/lib/terminalPaste.test.ts`, and `session_clipboard_file_paths` are all reusable; only the orchestration's precedence order changes.
