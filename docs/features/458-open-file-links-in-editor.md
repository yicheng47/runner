# Open file links from the terminal in your editor

Tracking issue: [#458](https://github.com/yicheng47/runner/issues/458). Status: planned. Priority P2.

## Motivation

Agents talk in file paths. Claude Code and Codex cite `crates/runner-app/src/surfaces/panes.rs:1311` in every summary, diff header, and review note; `cargo` and `clippy` print `--> src/foo.rs:12:5`; `git status` prints bare relative paths. Today none of that is clickable. The terminal links only OSC 8 hyperlinks and `http(s)://` URLs (`crates/runner-terminal/src/terminal.rs:702`, `:781`), and every link opens through `cx.open_url` (`crates/runner-app/src/terminal/element.rs:348`) — so a file reference means select, copy, switch to the editor, paste into "go to file". Zed, Ghostty, and VS Code's terminal all make paths ⌘-clickable, and going from Runner's terminal to the code is the single most common hop in a review loop.

Two things:

1. **File paths are links.** A path printed by the TUI — absolute, `~`-relative, or relative to the session's working directory, with an optional `:line` or `:line:col` — gets the same ⌘-hover underline and pointer as a URL today.
2. **They open in the editor the user picked.** Settings → General gains **Open file links in**: Zed, VS Code, Cursor, the system default app, or a custom command. ⌘-click opens the file at the cited line.

## Behavior

### What becomes a link

- **Path grammar.** A run of non-whitespace starting with `/`, `~/`, `./`, `../`, or a bare relative segment, and containing at least one `/` or a file extension (`.rs`, `.md`, `Makefile`-style names without an extension link only when they exist on disk — see below). Optional trailing `:LINE`, `:LINE:COL`, `(LINE,COL)`, or `#LLINE`. Trailing `.`, `,`, `:`, `)`, `]`, `'`, and `"` are stripped the way the URL regex already strips them.
- **Must exist.** A candidate is resolved against the session's working directory and becomes a link only if the resolved path is a regular file on disk. This is the false-positive guard: `a/b`, `and/or`, `src/lib.rs` in a project that has no `src/`, and version strings like `1.2.3` never underline. Directories are not links (v1 has no "open folder" action).
- **Working directory.** The session row's `cwd` (`crates/runner-backend/src/repo/session.rs:51` — set for chats and mission slots alike), else the owning project's directory, else `settings.default_working_dir`. With none of those, only absolute and `~` paths link. The terminal session resolves this once through its `AppCore` handle and caches it; a shell pane's `cd` is not tracked (OSC 7 is a follow-up).
- **OSC 8 `file://` links** map to the same target: the URI's path, plus a line from a `:LINE` suffix or `#LLINE` fragment if present. Tools that emit them (some `ls` and `eza` builds, `rg --hyperlink-format`) get editor-open instead of Finder.
- **Where.** Every terminal element: chat panes, mission slot panes, and shell panes (spec 64). One detector, one element.
- **Hover and click** are unchanged from URLs: link modifier held (`⌘` or `⌃`, `element.rs:445`) underlines the whole path-plus-line span and shows the pointer; ⌘-click without a drag opens it. Plain click still goes to the TUI. Hover already re-runs `link_at` only when the grid, scroll offset, or pointer cell changes (`element.rs:137`), so the added `stat` per candidate is not on a hot path.

### Which editor

- **Settings → General → Open file links in**, a picker beside Default working directory. Options: **Zed**, **VS Code**, **Cursor**, **Default app**, **Custom command**. Default is **Default app**.
- Zed, VS Code, and Cursor run their CLIs — `zed PATH:LINE:COL`, `code --goto PATH:LINE:COL`, `cursor --goto PATH:LINE:COL` — resolved through the user's login-shell `PATH` (the `shell_path` module already does this for agent spawns; a GUI app's own `PATH` does not see `/opt/homebrew/bin` or `~/.local/bin`). An option whose CLI is not on that `PATH` shows an inline hint under the picker — "`zed` not found — Zed → Install CLI" / "`code` not found — VS Code → Shell Command: Install" — and is still selectable, because the user may be about to install it.
- **Default app** runs `open PATH`. The line is lost; the hint says so.
- **Custom command** reveals a text field with a template: `{path}`, `{line}`, `{column}` are substituted (shell-quoted) and the result runs through `/bin/sh -lc`. `{line}` and `{column}` substitute as `1` when the link had none. Example placeholder: `idea --line {line} {path}`. This is the escape hatch for JetBrains, Emacs, Neovim-in-a-terminal, and anything else, instead of a growing list.
- **Failure.** If the spawn fails or exits non-zero, Runner falls back to `open PATH` and logs a warning with the command and exit status. No dialog: a failed editor launch should not steal focus from the terminal.

## Non-goals

- File paths in the mission feed's markdown (`surfaces/mission_markdown.rs:799` — it only links `http`). Natural follow-up; the target model from phase 1 is reusable.
- Opening directories, revealing in Finder, or a "copy path" action on links.
- Tracking `cd` in shell panes (OSC 7) or the agent's own `cwd` changes mid-session.
- Per-project editor choice, or auto-detecting the "current" editor from the front application.
- Windows and Linux command shapes.

## Decisions

- **Existence check, not a tighter regex.** Path grammar is ambiguous in prose; the filesystem is not. A `stat` against the session cwd on ⌘-hover is microseconds and turns "looks like a path" into "is a file", which is the only rule that keeps `and/or` unlinked while linking `Makefile`.
- **Line information is part of the link span.** The underline covers `panes.rs:1311`, not just `panes.rs`, so the user sees that the line is going to be honoured.
- **CLIs, not `open -a`.** `open -a Zed file` loses the line and column; every editor on the list ships a CLI that takes them. The cost — the CLI may not be installed — is handled by the hint, not by a fallback that silently drops the line.
- **Custom command over a longer list.** One template field covers every editor Runner will never enumerate. Shell-quoting the substitutions and running through `sh -lc` gives the user's real `PATH` and aliases.
- **Default is Default app**, not Zed. Runner should not assume an editor; the first ⌘-click works everywhere and the Settings row is one click away.

## Design (to do in `design/runner.pen` before the brief)

- Settings → General: the **Open file links in** row with its picker, in the two states that differ — CLI found, and CLI missing with the inline install hint — plus the **Custom command** state with the template field and its placeholder.
- The ⌘-hover state of a file link in a chat pane (`src/foo.rs:12:5` underlined through the column), confirming it reads like the existing URL hover and needs no new treatment.

## Implementation Phases

1. **Detection (`runner-terminal`).** `TerminalLink` gains a target: `Url(String)` or `File { path: PathBuf, line: Option<u32>, column: Option<u32> }`. `terminal_link_at` (`terminal.rs:702`) tries OSC 8 first as today (mapping `file://` to `File`), then the URL regex, then the path regex with the existence check against the session's resolved cwd. `TerminalSession` resolves and caches the cwd from its `AppCore` (session row → project → `default_working_dir`). Tests in the existing `terminal_links_*` style (`terminal.rs:985`) with a `tempdir` that has real files.
2. **Settings and launcher (`runner-app`).** `AppSettings` gains `file_link_editor: FileLinkEditor` (an enum modelled on `TerminalCursorStyle`, `app_settings.rs:103` — Zed / VsCode / Cursor / DefaultApp / Custom, default `DefaultApp`) and `custom_editor_command: String`; the struct-level `#[serde(default)]` means existing `ui-settings.json` files load unchanged. A `file_links` module builds the argv for a target, resolves the binary through `shell_path`, and spawns it on `cx.background_spawn` the way `reveal_mission_cwd` does (`surfaces/mission_workspace.rs:4778`), with the `open` fallback and warning. General pane row in `surfaces/settings_page.rs` beside Default working directory (`:171`), with the CLI-found check and the Custom field.
3. **Click routing.** The mouse-up `DragKind::Link` arm (`element.rs:340`) matches on the target: `Url` → `cx.open_url`, `File` → the launcher with the current settings. Hover, underline, and pointer are untouched.

## Verification

- `cargo test -p runner-terminal`: `src/lib.rs:12:5`, `./README.md`, `~/x.txt`, `/abs/path.rs`, `path.rs(12,5)`, and `path.rs#L12` link with the right line and column when the file exists; the same tokens do not link when it doesn't; `a/b`, `and/or`, and `1.2.3` never link; trailing `,` `.` `)` are excluded from the span; the span covers the `:line:col` suffix; an OSC 8 `file:///tmp/x.rs:7` yields `File` with line 7; with no resolvable cwd only absolute and `~` paths link.
- `cargo test -p runner-app`: argv for each editor kind at a target with and without a line; the custom template substitutes and quotes a path with spaces; `AppSettings` deserialises a pre-feature `ui-settings.json` with the default editor.
- `make clippy` and `make fmt`.
- Manual smoke (Jason): ⌘-hover a claude-code diff header in a chat pane underlines the path through `:line`; ⌘-click opens Zed at that line with Zed selected; switch to Default app and the file opens in the default app; pick VS Code on a machine without `code` and the hint appears; a `git status` bare path in a shell pane links, a version string in agent prose does not.
