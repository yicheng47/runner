# 55 — Paste file paths into the terminal

Tracking: [#368](https://github.com/yicheng47/runner/issues/368)

## Motivation

Copying a file in GoLand (or Finder) and pressing ⌘V over a Runner terminal inserts nothing. Every native terminal — Terminal.app, iTerm2, Ghostty — inserts the file's POSIX path instead, which is how you hand an agent a file to look at without typing the path out.

The cause is that a file copy puts no text on the clipboard. It writes `public.file-url` / `NSFilenamesPboardType` flavors to NSPasteboard, and native terminals read those flavors explicitly and insert the path. Over the WKWebView boundary the paste arrives as a `File` in `DataTransfer` with an empty (or unusable) `text/plain`, so xterm.js's default paste has nothing to insert.

Runner already intercepts paste for exactly this class of problem. `RunnerTerminal.tsx:749` handles image paste — it walks `clipboardData.items` for `kind === "file"`, and for images ships the bytes to Rust to restore the NSPasteboard flavor (#79). A non-image file falls straight through: `inferPasteImageMime` returns null, the loop `continue`s, the handler returns **without** `preventDefault`, and xterm's default paste inserts the empty text. So the extension point exists and the gap is one branch wide.

The web layer cannot close it alone. `DataTransfer` deliberately withholds filesystem paths — `File.name` is only the basename, and WKWebView exposes no `file.path`. The path has to come from the native pasteboard, which is what the existing image path already reaches into.

## Scope

- ⌘V over a terminal pane, when the clipboard holds file references and no usable text, inserts the file's absolute path at the cursor, shell-quoted.
- Multiple copied files insert as multiple quoted paths separated by single spaces.
- Text pastes, image pastes, and every other clipboard shape keep their current behavior exactly.
- Out of scope: drag-and-drop of files onto a pane (same underlying need, but Tauri's drag-drop event supplies paths through an entirely different path — its own spec); pasting directory contents; translating paths to be relative to the session cwd; any change to the image-paste flow.

## Key Decisions

1. **Read the paths in Rust, not in the webview.** A new command returns the current pasteboard's file URLs as POSIX paths, read from `NSPasteboard.generalPasteboard`. `commands/session.rs:144-165` already carries the NSPasteboard plumbing and `objc2-app-kit` is already a dependency, so this is an addition to established code rather than new machinery. The frontend never sees a path it could have fabricated from `File.name`.
2. **Branch inside the existing paste handler.** Extend `onPaste` (`RunnerTerminal.tsx:749`): after the image scan fails, if any item is `kind === "file"`, call the new command. On a non-empty result, `preventDefault` and insert; on an empty result, fall through untouched so today's behavior is preserved for any clipboard shape the native read doesn't recognize.
3. **Prefer the native read over `text/uri-list`.** WKWebView sometimes exposes a `text/uri-list` flavor with `file://` URLs, which would avoid an IPC round-trip, but its presence is inconsistent across source applications. One authoritative path (the pasteboard) beats two code paths that disagree.
4. **Quote for the shell, always.** Wrap each path in single quotes with embedded single quotes escaped (`'` → `'\''`). Unquoted paths break on spaces, which are common in real project trees. Quoting unconditionally keeps the rule simple and matches what iTerm2 does.
5. **Insert as text, never submit.** Write the quoted string through the existing raw-stdin injection. Do **not** route through `inject_paste`, which appends Enter — a pasted path is the middle of a sentence the user is still composing. This also means the draft-gate state (spec 54) sees it as ordinary local input, which is correct: the user now has a pending draft.
6. **No cwd-relative rewriting.** Absolute paths always resolve regardless of where the agent has `cd`'d, and the agent can shorten them itself. Relativizing would need the session's live cwd, which Runner only knows at spawn time.

## Implementation Phases

### Phase 1 — native pasteboard read

- Add a command (`commands/session.rs`, beside the image-paste helpers at `:144-165`) returning `Vec<String>` of POSIX paths for the pasteboard's file URLs, empty when the flavor is absent.
- Unit-test the quoting helper: plain path, path with spaces, path with an embedded single quote, multiple paths.

### Phase 2 — frontend branch

- Extend `onPaste` (`RunnerTerminal.tsx:749-781`) per decision 2: file-kind items with no image mime → call the command → quote, join with spaces, `preventDefault` + `stopImmediatePropagation`, inject through `api.session.injectStdin`.
- Keep the existing early returns intact so a text paste never reaches the new branch.
- Component test: an image paste still takes the image path; a non-image file paste injects the quoted path; a text paste is untouched.

### Phase 3 — validation

- `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
- Manual pass over the Verification list.

## Verification

- [ ] Copy a `.go` file in GoLand, ⌘V over a terminal pane — the absolute path appears at the cursor, quoted, not submitted.
- [ ] Copy a file whose path contains a space — the pasted text is correctly quoted and the agent resolves it.
- [ ] Copy two files at once — both paths appear, space-separated.
- [ ] Copy a file in Finder — same result as GoLand.
- [ ] Copy an image — the existing `[Image #N]` attach flow is unchanged.
- [ ] Copy ordinary text — pastes exactly as before.
- [ ] Paste with the session stopped or the pane disabled — no injection, no error.
- [ ] The pasted path leaves the pane in a pending-draft state (spec 54), not a submitted one.

## Relevant Code

- `src/components/RunnerTerminal.tsx:749-783` — `onPaste`, the interception point; `:81-99` — `normalizePasteImageMime` / `inferPasteImageMime`, the mime filter that currently drops non-image files.
- `src-tauri/src/commands/session.rs:144-165` — existing NSPasteboard flavor plumbing for image paste.
- `src/lib/api.ts` — `session.pasteImage` / `session.injectStdin`, the call shapes to mirror.
- `docs/features/54-draft-aware-delivery-gate.md` — the draft model a pasted path feeds into (decision 5).
