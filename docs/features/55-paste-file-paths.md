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

1. **Read the paths in Rust, not in the webview.** A new command returns the current pasteboard's file URLs as POSIX paths. The frontend never sees a path it could have fabricated from `File.name`.

   This is **new machinery**, contrary to an earlier draft of this spec. `commands/session.rs:144-165` is a MIME→OSType lookup table, not NSPasteboard plumbing: the image write shells out to `osascript -e "set the clipboard to (read POSIX file … as «class PNGf»)"` (`:225`), and the repo contains no NSPasteboard binding anywhere. `objc2-app-kit` is a dependency but is declared `default-features = false` with `["std", "NSButton", "NSControl", "NSResponder", "NSView", "NSWindow", "NSWorkspace"]`, and objc2 gates every class behind its own feature — so `NSPasteboard` does not compile today. The work is one Cargo feature plus a first direct binding. Take the binding rather than matching the `osascript` precedent: reads happen on every candidate paste, and a process spawn per paste is not worth the symmetry.

2. **Decide synchronously, act asynchronously.** `preventDefault()` after an `await` does nothing — the browser has already run the default action by the time an IPC round-trip resolves. So the handler cannot "call the command, then decide", which an earlier draft of this spec asked for. It must commit before it knows the answer, exactly as the image branch does (`RunnerTerminal.tsx:767`: decide from `clipboardData`, *then* go async).

   Gate on **absence of usable text**, not on `kind === "file"`. If `clipboardData.getData("text/plain")` is non-empty, return immediately and let xterm paste it — that is every ordinary paste, unchanged and with no IPC. Otherwise `preventDefault` eagerly and consult the pasteboard; an empty result then swallows a paste that had nothing to insert anyway, which is a no-op.

   This also removes the spec's riskiest assumption. Whether WKWebView exposes an arbitrary non-image file as `kind === "file"` is unverified; if it does not, a `kind`-based trigger never fires and the feature silently does nothing. Text-absence does not depend on that behavior.

3. **A file reference beats image bytes.** `inferPasteImageMime` falls back to the filename extension (`:100-103`) and WKWebView sets `DataTransferItem.type` from the file's UTI, so copying `shot.png` in Finder currently takes the image branch and attaches it — the path branch would never run. Native terminals paste the path.

   Resolve by clipboard *flavor*, not MIME sniff: if the pasteboard carries `public.file-url`, the user copied a reference and gets a path; otherwise they copied content and get today's attach flow. This preserves [#79](https://github.com/yicheng47/runner/issues/79) exactly — screenshots and copies from Preview or a browser put bytes on the pasteboard with no file-url — and changes only the Finder/GoLand image-*file* case, which is the case the user is asking about. Requires reading the file-url flavor before the image scan.

4. **Prefer the native read over `text/uri-list`.** WKWebView sometimes exposes a `text/uri-list` flavor with `file://` URLs, which would avoid an IPC round-trip, but its presence is inconsistent across source applications. One authoritative path (the pasteboard) beats two code paths that disagree.

5. **Quote only when the path needs it.** iTerm2 quotes unconditionally because its panes are shell prompts. Runner's panes are usually *agent* prompts, where `'/Users/jason/foo.go'` is noise and defeats `@`-style file completion. Quote when the path contains whitespace or shell metacharacters, using single quotes with embedded quotes escaped (`'` → `'\''`); otherwise insert it bare.

6. **Insert as text, never submit.** Write through the existing raw-stdin injection (`api.session.injectStdin`). Do **not** route through `inject_paste`, which appends Enter — a pasted path is the middle of a sentence the user is still composing. This also means the draft-gate state (spec 54) sees it as ordinary local input, which is correct: the user now has a pending draft.

7. **No cwd-relative rewriting.** Absolute paths always resolve regardless of where the agent has `cd`'d, and the agent can shorten them itself. Relativizing would need the session's live cwd, which Runner only knows at spawn time.

## Implementation Phases

### Phase 1 — native pasteboard read

- Add the `NSPasteboard` feature to `objc2-app-kit` in `src-tauri/Cargo.toml` (decision 1).
- Add a command (`commands/session.rs`, beside the image-paste helpers at `:144-165`) returning `Vec<String>` of POSIX paths for the pasteboard's file URLs, empty when the flavor is absent. Non-macOS returns empty.
- Unit-test the quoting helper: plain path (unquoted), path with spaces, path with an embedded single quote, path with shell metacharacters, multiple paths.

### Phase 2 — frontend branch

- Extend `onPaste` (`RunnerTerminal.tsx:749-781`) per decisions 2–3. Order matters:
  1. Non-empty `text/plain` → return immediately, xterm handles it.
  2. Otherwise `preventDefault` + `stopImmediatePropagation`, then call the pasteboard command.
  3. Non-empty result → quote as needed, join with single spaces, inject through `api.session.injectStdin`.
  4. Empty result → fall back to the existing image scan; if that also finds nothing, do nothing.
- Keep the `!sid || disabledRef.current` early return ahead of all of it.
- Component tests: text paste untouched and no IPC; image *bytes* paste still attaches; image *file* paste injects a path; non-image file paste injects a quoted path; empty pasteboard is a silent no-op.

### Phase 3 — validation

- `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
- Manual pass over the Verification list.

## Verification

- [ ] Copy a `.go` file in GoLand, ⌘V over a terminal pane — the absolute path appears at the cursor, unquoted, not submitted.
- [ ] Copy a file whose path contains a space — the pasted text is quoted and the agent resolves it.
- [ ] Copy two files at once — both paths appear, space-separated.
- [ ] Copy a file in Finder — same result as GoLand.
- [ ] Take a screenshot to the clipboard (⌘⇧4), paste — the existing `[Image #N]` attach flow is unchanged (#79).
- [ ] Copy an image *file* in Finder, paste — its **path** appears, not an attachment (decision 3, deliberate change).
- [ ] Copy ordinary text — pastes exactly as before, with no IPC round-trip.
- [ ] Paste with an empty clipboard — nothing happens, no error.
- [ ] Paste with the session stopped or the pane disabled — no injection, no error.
- [ ] The pasted path leaves the pane in a pending-draft state (spec 54), not a submitted one.

## Relevant Code

- `src/components/RunnerTerminal.tsx:749-781` — `onPaste`, the interception point; `:767` — the synchronous commit point the new branch must mirror; `:81-103` — `normalizePasteImageMime` / `inferPasteImageMime`, whose filename fallback creates the collision in decision 3.
- `src-tauri/src/commands/session.rs:143-160` — `paste_image_format`, a MIME→OSType table (**not** NSPasteboard plumbing); `:225` — the `osascript` shell-out that actually writes the pasteboard.
- `src-tauri/Cargo.toml:67` — `objc2-app-kit` with `default-features = false`; `NSPasteboard` must be added to its feature list.
- `src/lib/api.ts:396-397` — `session.injectStdin`; `:412` — `session.pasteImage`, the call shape to mirror.
- `docs/features/54-draft-aware-delivery-gate.md` — the draft model a pasted path feeds into (decision 6).
