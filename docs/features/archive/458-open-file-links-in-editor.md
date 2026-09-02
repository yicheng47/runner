# Open file links from the terminal in your editor

Tracking issue: [#458](https://github.com/yicheng47/runner/issues/458). Status: shipped 2026-09-02 in [#467](https://github.com/yicheng47/runner/pull/467). Priority P2.

## Motivation

Agents talk in file paths. Claude Code and Codex cite `crates/runner-app/src/surfaces/panes.rs:1311` in every summary, diff header, and review note; `cargo` and `clippy` print `--> src/foo.rs:12:5`; `git status` prints bare relative paths. Today none of that is clickable. The terminal links only OSC 8 hyperlinks and `http(s)://` URLs (`crates/runner-terminal/src/terminal.rs:702`, `:781`), and every link opens through `cx.open_url` (`crates/runner-app/src/terminal/element.rs:348`) — so a file reference means select, copy, switch to the editor, paste into "go to file". Zed, Ghostty, and VS Code's terminal all make paths ⌘-clickable, and going from Runner's terminal to the code is the single most common hop in a review loop.

Two things:

1. **File paths are links.** A path printed by the TUI — absolute, `~`-relative, or relative to the session's working directory, with an optional `:line` or `:line:col` — gets the same ⌘-hover underline and pointer as a URL today.
2. **They open in the editor the user picked.** Settings → General gains **Open file links in**: Zed, VS Code, Cursor, or the system default app. ⌘-click opens the file at the cited line.

## Behavior

### What becomes a link

- **Path grammar.** A run of non-whitespace starting with `/`, `~/`, `./`, `../`, or a bare relative segment, and containing at least one `/` or a file extension (`.rs`, `.md`, `Makefile`-style names without an extension link only when they exist on disk — see below). Optional trailing `:LINE`, `:LINE:COL`, `(LINE,COL)`, or `#LLINE`. Trailing `.`, `,`, `:`, `)`, `]`, `'`, and `"` are stripped the way the URL regex already strips them.
- **Must exist.** A candidate is resolved against the session's working directory and becomes a link only if the resolved path is a regular file on disk. This is the false-positive guard: `a/b`, `and/or`, `src/lib.rs` in a project that has no `src/`, and version strings like `1.2.3` never underline. Directories are not links (v1 has no "open folder" action).
- **Working directory.** The session row's `cwd` (`crates/runner-backend/src/repo/session.rs:51` — set for chats and mission slots alike), else the owning project's directory, else `settings.default_working_dir`. With none of those, only absolute and `~` paths link. The terminal session resolves this once through its `AppCore` handle and caches it; a shell pane's `cd` is not tracked (OSC 7 is a follow-up).
- **OSC 8 `file://` links** map to the same target: the URI's path, plus a line from a `:LINE` suffix or `#LLINE` fragment if present. Tools that emit them (some `ls` and `eza` builds, `rg --hyperlink-format`) get editor-open instead of Finder.
- **Where.** Every terminal element: chat panes, mission slot panes, and shell panes (spec 64). One detector, one element.
- **Hover, tooltip, and click** are shared with URLs, and both kinds gain discoverability they lacked. Resting the pointer on a link with no modifier underlines the whole span in the text colour and, after 500 ms, shows a tooltip that teaches the gesture and names the destination: "⌘-click to open in Zed", "⌘-click to open in default app", "⌘-click to open in browser". With the link modifier held (`⌘` or `⌃`) the underline switches to the accent, the pointer becomes a hand, and the tooltip flips to the action and target: "Open in Zed · panes.rs:1311", "Open in default app · line 1311 is lost", "Open in default app · code not on PATH", "Open in browser". ⌘-click without a drag opens it; plain click still goes to the TUI. `link_at` re-runs only when the grid, scroll offset, or pointer cell changes, so the added `stat` per candidate is not on a hot path.

### Which editor

- **Settings → General → Open file links in**, a picker beside Default working directory. Options: **Zed**, **VS Code**, **Cursor**, **Default app**. Default is **Default app**.
- Zed, VS Code, and Cursor run their CLIs — `zed PATH:LINE:COL`, `code --goto PATH:LINE:COL`, `cursor --goto PATH:LINE:COL` — resolved through the user's login-shell `PATH` (the `shell_path` module already does this for agent spawns; a GUI app's own `PATH` does not see `/opt/homebrew/bin` or `~/.local/bin`). An option whose CLI is not on that `PATH` shows an inline hint under the picker — "`zed` not found — Zed → Install CLI" / "`code` not found — VS Code → Shell Command: Install" — and is still selectable, because the user may be about to install it.
- **Default app** runs `open PATH`. The line is lost; the hint says so.
- **Failure.** If the spawn fails or exits non-zero, Runner falls back to `open PATH` and logs a warning with the command and exit status. No dialog: a failed editor launch should not steal focus from the terminal.

## Non-goals

- File paths in the mission feed's markdown (`surfaces/mission_markdown.rs:799` — it only links `http`). Natural follow-up; the target model from phase 1 is reusable.
- Opening directories, revealing in Finder, or a "copy path" action on links.
- Tracking `cd` in shell panes (OSC 7) or the agent's own `cwd` changes mid-session.
- Per-project editor choice, or auto-detecting the "current" editor from the front application.
- Windows and Linux command shapes.
- A custom command template. Rejected in design review (2026-09-02): a settings field that runs arbitrary shell text is not a surface Runner wants. Another editor is a new entry in the fixed list, not a template.

## Decisions

- **Existence check, not a tighter regex.** Path grammar is ambiguous in prose; the filesystem is not. A `stat` against the session cwd on ⌘-hover is microseconds and turns "looks like a path" into "is a file", which is the only rule that keeps `and/or` unlinked while linking `Makefile`.
- **Line information is part of the link span.** The underline covers `panes.rs:1311`, not just `panes.rs`, so the user sees that the line is going to be honoured.
- **CLIs, not `open -a`.** `open -a Zed file` loses the line and column; every editor on the list ships a CLI that takes them. The cost — the CLI may not be installed — is handled by the hint, not by a fallback that silently drops the line.
- **Default is Default app**, not Zed. Runner should not assume an editor; the first ⌘-click works everywhere and the Settings row is one click away.
- **Plain hover teaches, ⌘ acts.** A modifier-only affordance is undiscoverable: nobody learns ⌘-click by holding ⌘ first. Underlining on plain hover plus a delayed tooltip is the convention VS Code's terminal uses, and applying it to URLs too fixes the same gap they had. The tooltip is also where the missing-line and missing-CLI caveats surface at the moment of use, not only in Settings.
- **Two cwd tiers, not three.** The session row's `cwd` already has `default_working_dir` baked in at spawn (`resolve_direct_start` / `session_start_shell`), so the detector resolves session row → project directory and stops; the settings fallback the first draft listed would never fire. Resolved lazily on the first ⌘-hover and cached per terminal, so sessions nobody hovers never touch the database.

## Design (`design/runner.pen`, done 2026-09-02)

- Settings → General (`Settings — General` frame): the **Open file links in** row sits under Default working directory in the Defaults card, showing the CLI-found state.
- `Spec — Open file links in editor (458)`: the row in its three states — Default app, CLI found, CLI missing (install hint in the warning colour as the row subtitle) — plus two chat-pane mocks: plain hover (text-colour underline, gesture tooltip, one file link and one URL) and ⌘-hover (accent underline, action tooltip).

## Implementation Phases

1. **Detection (`runner-terminal`).** `TerminalLink` gains a target: `Url(String)` or `File { path: PathBuf, line: Option<u32>, column: Option<u32> }`. `terminal_link_at` (`terminal.rs:702`) tries OSC 8 first as today (mapping `file://` to `File`), then the URL regex, then the path regex with the existence check against the session's resolved cwd. `TerminalSession` resolves and caches the cwd from its `AppCore` (session row → project → `default_working_dir`). Tests in the existing `terminal_links_*` style (`terminal.rs:985`) with a `tempdir` that has real files.
2. **Settings and launcher (`runner-app`).** `AppSettings` gains `file_link_editor: FileLinkEditor` (an enum modelled on `TerminalCursorStyle`, `app_settings.rs:103` — Zed / VsCode / Cursor / DefaultApp, default `DefaultApp`); the struct-level `#[serde(default)]` means existing `ui-settings.json` files load unchanged. A `file_links` module builds the argv for a target, resolves the binary through `shell_path`, and spawns it on `cx.background_spawn` the way `reveal_mission_cwd` does (`surfaces/mission_workspace.rs:4778`), with the `open` fallback and warning. General pane row in `surfaces/settings_page.rs` beside Default working directory (`:171`), with the CLI-found check.
3. **Click routing.** The mouse-up `DragKind::Link` arm (`element.rs:340`) matches on the target: `Url` → `cx.open_url`, `File` → the launcher with the current settings. Hover, underline, and pointer are untouched.

## Verification

- `cargo test -p runner-terminal`: `src/lib.rs:12:5`, `./README.md`, `~/x.txt`, `/abs/path.rs`, `path.rs(12,5)`, and `path.rs#L12` link with the right line and column when the file exists; the same tokens do not link when it doesn't; `a/b`, `and/or`, and `1.2.3` never link; trailing `,` `.` `)` are excluded from the span; the span covers the `:line:col` suffix; an OSC 8 `file:///tmp/x.rs:7` yields `File` with line 7; with no resolvable cwd only absolute and `~` paths link.
- `cargo test -p runner-app`: argv for each editor kind at a target with and without a line; `AppSettings` deserialises a pre-feature `ui-settings.json` with the default editor.
- `make clippy` and `make fmt`.
- `cargo test -p runner-app`: tooltip copy for URL and file targets with and without the modifier, across found, missing, and default-app editors.
- Manual smoke (Jason): rest the pointer on a claude-code diff header in a chat pane: it underlines and after half a second the tooltip reads "⌘-click to open in Zed"; hold ⌘ and the underline turns accent, the pointer becomes a hand, and the tooltip names the file and line; the same on a URL says browser; ⌘-click opens Zed at that line with Zed selected; switch to Default app and the file opens in the default app; pick VS Code on a machine without `code` and the hint appears; a `git status` bare path in a shell pane links, a version string in agent prose does not.
