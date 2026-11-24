# 1452 — Improve terminal file-link detection across agent CLIs

Tracking issue: [#1452](https://github.com/yicheng47/runner/issues/1452)

## Motivation

Runner's Files side-panel preview and configured external editor depend on terminal link detection (`terminal_link_at` in `crates/runner-terminal/src/terminal.rs`) to recognize file paths on hover and ⌘-click (Ctrl-click on Windows/Linux). Even with Codex's built-in TUI file viewer off by default ([#1451](https://github.com/yicheng47/runner/issues/1451)), paths printed by Codex, Claude Code, Antigravity CLI, pi, OpenCode, Grok Build and Copilot CLI still often fail to link.

Four root causes, each checked against the current code:

1. **Hard-wrapped paths never join across rows.** Agent TUIs wrap long lines with explicit newlines or cursor moves, so the terminal's `WRAPLINE` flag is unset. `link_rows_joined` joins such rows only when they carry an OSC 8 hyperlink or an underline, and when no URL matches the joined text, `terminal_link_at_with_hard_breaks` retries on the soft-wrapped line alone. `file_link_regex()` therefore never sees a path that spans a hard break. In a narrow split pane almost any long path wraps and becomes unclickable, as #1451's live QA saw with Codex printing `(/private/tmp/runner-` and `1451-qa.UAUZ0TI8/codex-chat/qa-link-1451.md)` on two rows.
2. **Common path shapes fail to match or resolve.** `file_link_regex()` lists the characters that end a path, and that list holds only ASCII punctuation: `查看 src/lib.rs，测试` becomes the single token `src/lib.rs，测试`. Parentheses and brackets always end a path, which breaks `app/users/[id]/page.tsx` and `app/(marketing)/layout.tsx`. `@path` mentions and `git diff`'s `a/` and `b/` prefixes never resolve, a path with a space links only its first word, and line ranges (`:10-25`, `#L10C3-L14C9`) lose their location.
3. **Editor-scheme links skip Runner.** `terminal_hyperlink_at` turns only `file://` into `LinkTarget::File`. OSC 8 links such as `vscode://file/…`, which Codex emits through `file_opener`, become `LinkTarget::Url` and open outside Runner. In plain text, `url_regex()` does not match these schemes at all. An OSC 8 target that is a bare path (`src/lib.rs:12`) also becomes a `LinkTarget::Url` that opens nothing.
4. **The Files root is not a lookup root.** Agent sessions never report a shell `live_cwd` through OSC 7, so a relative path resolves only against the spawn cwd. A path relative to a different folder or linked worktree chosen in the Files panel does not link.

## Decisions (Jason, 2026-11-24)

- **Split.** This feature ships causes 1–4 above. Resolving shortened citations (`launch.rs:42`, `agents/launch.rs`) against the Files index is moved to [#1459](https://github.com/yicheng47/runner/issues/1459), which waits on what this feature leaves unlinked.
- **Editor-scheme links are file links.** Local `…://file/…` links route like any other file link, with no extra setting. Where a file link opens (the Files preview or an editor, including files outside the Files root) is the subject of [#1458](https://github.com/yicheng47/runner/issues/1458), so this feature does not change routing.

## Prior art

[Orca](https://github.com/stablyai/orca) (read at `5384689307`, 2026-11-23) has a mature terminal file-link detector, itself ported from VS Code's terminal link detectors. This spec borrows four of its rules:

- the hard-wrap continuation rule (`src/shared/terminal/links/wrapped-terminal-link-ranges.ts` and `hard-wrapped-terminal-path-fragments.ts`);
- the allow-list of path characters, with an unmatched closer ending the path (`terminal-links.ts` and `terminal-file-link-detection-ranges.ts`);
- growing a path across spaces (`terminal-spaced-path-candidates.ts`);
- letting the file-existence check decide between candidates.

Orca has no editor-scheme links, no `@`, `a/` or `b/` stripping, and no line ranges or `#L` suffixes in plain text. It resolves relative paths against one cwd only.

## Scope

Detection stays synchronous inside `TerminalMirror::link_at` on the UI thread, as today. Every rule below proposes candidates that link only when the existing existence check (`resolve_file_candidate`, an `is_file()` probe) finds the file. Section 5 caps how many probes one lookup may run.

### 1. Hard-wrapped paths (`runner-terminal`)

- **URLs unchanged.** The OSC 8 lookup, the URL hard-wrap join (`link_rows_joined`) and the URL match stay as they are, along with their tests: `terminal_links_keep_unrelated_hard_lines_separate`, `terminal_links_keep_underlined_file_rows_separate`, `terminal_links_do_not_join_an_underlined_scrollback_block`, and the OSC 8 table-cell tests. The new-URL check in `link_rows_joined` also recognizes the editor schemes from section 3.
- **New last step, for file paths only.** It runs when the soft-wrapped line under the pointer has no file link there, or has one that ends at that row's last non-blank column (`src/lib.rs` whose `:42` is on the next row). If the step finds nothing, the single-line result stands.
- **Continuation rule (Orca's).** Leading and trailing whitespace is trimmed from every row first.
  - **Starting row:** its trailing run of path characters must look like a path start: it contains a separator, or it is a root prefix (`/`, `~/`, `./`, `../`, `C:`).
  - **Following rows:** a row continues the path only when its whole content is path characters with no spaces. Path characters here are `[A-Za-z0-9._~@%+=:,/\\-]` plus Unicode letters, numbers and marks.
  - **The first mixed row ends the join.** A row that mixes path characters with other text adds its leading run of path characters and nothing more: `1451-qa.UAUZ0TI8/codex-chat/qa-link-1451.md)` adds everything up to the `)`.
  - **URLs never continue a path.** A row that starts with a URL scheme (`http://`, `https://`, `file://` or an editor scheme) never continues a file path.
  - **No width or style test.** Unlike `link_rows_joined` for URLs, the rule checks neither the right margin nor text styles.
- **Bounded.** A join spans at most `MAX_WRAP_JOIN_ROWS = 8` rows, counting rows above the pointer, since the path may start above it.
- **Longest existing candidate wins.** Candidates are tried longest first, and the first one that exists becomes the link. Two paths on consecutive rows, `src/a.rs` and `src/b.rs`, join as `src/a.rssrc/b.rs`, which does not exist, so each row keeps its own link.
- **Highlight.** The link covers only the matched cells on each row, never indentation, padding or frame glyphs.
- **No frame-glyph stripping.** Box-drawing borders (`│`, `┃`, `▎`) end a path under section 2 and are not stripped. Orca does not strip them either, and none of the wraps observed so far sit inside a border. If a live check finds a bordered wrap that stays unlinked, its recording goes back to the coder.

### 2. Path tokens, prefixes and location suffixes (`runner-terminal`)

- **Allow-list of path characters.** In `file_link_regex()`, the class of excluded characters becomes an allow-list: Unicode letters, numbers and marks, plus `. _ - ~ / \ % + @ ( ) [ ]`. A `:` belongs to a path only in a drive prefix or a location suffix. Everything else ends a path:
  - CJK and typographic punctuation (`，` `。` `、` `：` `；` `（` `）` `「` `」` `【` `】` `—` `–` `…`);
  - box-drawing glyphs and quotes;
  - the ASCII delimiters the regex already excludes.

  CJK names stay whole (`docs/计划.md`), and decomposed accents in macOS filenames stay inside the path because marks are allowed.
- **Parentheses and brackets inside a path.**
  - `app/(marketing)/layout.tsx`, `app/users/[id]/page.tsx`, `src/routes/[...slug]/+page.svelte` and `[[optional]]` segments match.
  - Parentheses and brackets may appear only after a path's first separator, so a path never starts inside `Read(` or `[label](`. An unmatched closing `)` or `]` ends the path. As a result `(qa-link-1451.md)`, `Read(crates/runner-app/src/lib.rs)`, `Read(app/(marketing)/layout.tsx)` and `[label](docs/guide.md)` link only the path.
  - A trailing `(LINE)`, `(LINE,COL)` or `(LINE, COL)` is always read as a location suffix, never as part of the path.
- **Paths with spaces (Orca's growth rule).** This runs only when the path as matched does not exist. It then grows across the single-spaced words that follow, and each reading counts only if the file exists:
  - A rooted path (starting with `/`, `~/`, `./`, `../` or a drive) grows one word at a time, up to 10 words.
  - A relative path grows only up to the first word that looks like a filename (`plan.md`), so `src/a.rs src/b.rs` stays two links.
  - Growth stops at a quote, a bracket, a word that starts another rooted path, or a `scheme://` word.

  So `/Users/me/My Project/readme.md`, `~/Library/Application Support/Runner/state.json` and `"docs/my plan.md"` link, and the quotes are not highlighted.
- **Prefix fallbacks in `resolve_file_candidate`.** The candidate as written is always tried first, so real paths like `a/foo.rs` and `@types/node/index.d.ts` resolve verbatim. Only when it does not exist, a leading `@` is stripped (`@crates/runner-app/src/lib.rs`), then a `git diff` prefix `a/` or `b/` (`a\` or `b\` on Windows). The highlight leaves out the stripped prefix.
- **Location suffixes**, in plain text and in `file://` and editor-scheme URIs:
  - Kept: `:LINE`, `:LINE:COL`, `(LINE,COL)`, `(LINE, COL)` and `#LLINE`.
  - Added, opening at the start line and column while the highlight covers the whole suffix: `:LINE-ENDLINE` and `:LINE–ENDLINE` (en dash), `:LINE:COL-ENDLINE:ENDCOL`, `:LINE:COL-ENDCOL`, `#LLINECCOL` (`#L12C3`), `#LLINE-LENDLINE` and `#LLINECCOL-LENDLINECENDCOL` (`#L10C3-L14C9`).
  - Added: `(LINE)` right after the path (`src/lib.rs(42)`), opening at that line.

### 3. Editor-scheme links and OSC 8 paths (`runner-terminal`)

- **Schemes.** `vscode://file/`, `vscode-insiders://file/`, `cursor://file/`, `windsurf://file/` and `zed://file/` are recognized case-insensitively, both in OSC 8 hyperlinks (`terminal_hyperlink_at`) and in plain text (`url_regex()` gains these schemes).
- **Parsing.** Here `file` is a literal authority, not a host, so the `file://` host check (`is_local_host_for`) does not run. After the `<scheme>://file` prefix, the rest is percent-decoded and read as an absolute local path. It follows the same rules as `file://`:
  - reject NUL bytes and the UNC and `//` forms;
  - normalize Windows drive letters;
  - read `:LINE[:COL]` and the `#L` forms from section 2.

  The result is `LinkTarget::File`.
- **Other editor links stay links.** Editor URIs without the `file` authority (`vscode://vscode-remote/…`, `cursor://settings/…`) stay `LinkTarget::Url` and open externally as today.
- **Malformed links.** A malformed `…://file/…` OSC 8 target returns `Osc8Lookup::Rejected`, as a malformed `file://` does, and never falls back to the visible label.
- **OSC 8 bare paths.** An OSC 8 target with no scheme (`src/lib.rs:12`, `/tmp/report.md`) is read as a path. It is resolved against the lookup roots like plain text and links only if the file exists.
- **Routing.** Once parsed, these are ordinary file links. ⌘/Ctrl-click routes them through `FileLinkRoute` like any other (#1458 owns where file links open).

### 4. Files root as an extra lookup root (`runner-terminal` + `runner-app`)

- **Lookup order.** When the terminal's `FileLinkRoute` has a Files view with a resolved root (`FilesView::root_path`), relative paths also try that root, after the live and spawn cwds and without duplicates.
- **Availability.** The root is available only while the side panel and Files are on and the session is local (`link_files_view` in `surfaces/panes/header.rs` and `surfaces/mission_workspace/rail.rs`). Otherwise lookup uses the two cwds, as today.
- **API.**
  - `runner-terminal` adds `FileLinkContext { extra_root: Option<PathBuf> }` and `TerminalMirror::link_at_with(point, &FileLinkContext)`. `link_at(point)` stays as a wrapper that passes the default context, for existing callers and tests.
  - `FileLinkRoute::link_context(cx)` reads the root the Files view already holds. It is synchronous and never fetches.
  - The callers of `refresh_hovered_link`, `mouse_down` and `mouse_up` in `crates/runner-app/src/terminal/element.rs` compute the context and pass it in.
- **Hover refresh.** `hover_signature` includes the extra root, so a resting hover re-resolves when the Files root resolves or changes.

### 5. Probe budget (`runner-terminal`)

`link_at` runs on the UI thread on every pointer move and every output change. One lookup stops after `MAX_LINK_PATH_PROBES = 32` existence checks, where each candidate tried against each lookup root counts as one. Candidates are tried in this order:

1. the single-line match;
2. hard-wrap joins, longest first;
3. spaced readings;
4. prefix-stripped forms.

Candidates past the cap are not tried.

### Non-goals

- Shortened citations resolved against the Files index ([#1459](https://github.com/yicheng47/runner/issues/1459)).
- Where file links open, and files outside the Files root ([#1458](https://github.com/yicheng47/runner/issues/1458)).
- Resolving paths for remote-host sessions, which keep `file_links: None`.
- Relative paths with spaces beyond their first filename word.
- New settings, or changes to the ⌘/Ctrl-click gesture.

## Implementation Phases

1. **Move link detection out of `terminal.rs` (no behavior change).** Move it into `crates/runner-terminal/src/terminal/links.rs`, alongside `find.rs` and `model.rs`; `terminal.rs` is 2,390 lines. This lands as its own commit, with existing tests passing unchanged.
2. **`runner-terminal` detection,** with its unit tests:
   - the allow-list path class, parentheses and closer handling, and location suffixes;
   - `@`, `a/` and `b/` fallbacks;
   - spaced-path growth;
   - editor-scheme and OSC 8 bare-path targets;
   - the hard-wrap file step;
   - the probe budget;
   - `FileLinkContext` and `link_at_with`.
3. **`runner-app` wiring:** `FileLinkRoute::link_context`, threading the context through `element.rs`, and `hover_signature`.
4. **Guide.** In `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md`, extend the paragraph on ⌘-clicking a file path in a terminal. It should say which shapes link: wrapped paths, `@` and `git diff` prefixes, line ranges and `#L` anchors, editor links such as `vscode://file/…`, and paths relative to the folder the Files view shows.

## Verification

### Automated

- `cargo test -p runner-terminal`, with new tests that build hard-wrapped rows directly (explicit `\r\n` and cursor positioning, leaving `WRAPLINE` unset, as agent TUIs do):
  - **Wrapped paths:**
    - the #1451 shape `(/private/tmp/runner-` + `1451-qa…/qa-link-1451.md)`;
    - a path over 2–8 rows;
    - a `:42` suffix on the next row;
    - a wrapped CJK path;
    - a path starting on the row above the pointer.
  - **Rows that must not join:**
    - `src/a.rs` and `src/b.rs` on consecutive rows;
    - a path row followed by a URL row;
    - a 3,000-row scrollback block;
    - every existing URL join test, unchanged.
  - **Path shapes and prefixes:**
    - `查看 src/lib.rs，测试`, `src/lib.rs—see` and `docs/计划.md`;
    - `app/users/[id]/page.tsx`, `app/(marketing)/layout.tsx`, `src/routes/[...slug]/+page.svelte`;
    - `(qa-link-1451.md)`, `Read(src/lib.rs)`, `[label](docs/guide.md)`, `src/lib.rs(12, 4)`, `src/lib.rs(42)`;
    - `@path`, `a/path` and `b/path`, while real `a/foo.rs` and `@types/foo/index.d.ts` resolve verbatim.
  - **Spaced paths:** `/tmp/My Project/readme.md`, `"docs/my plan.md"`, `src/a.rs src/b.rs`.
  - **Location suffixes:** `:10-25`, `:10–25`, `:10:5-20`, `#L10C3`, `#L10-L20`, `#L10C3-L14C9`.
  - **Editor and OSC 8 links:**
    - the five editor schemes in OSC 8 and plain text, on POSIX and Windows path helpers;
    - `vscode://vscode-remote/…` stays `LinkTarget::Url`;
    - a malformed `vscode://file/` link is rejected;
    - an OSC 8 bare relative path resolves.
  - **Budget and extra root:**
    - a lookup stops at `MAX_LINK_PATH_PROBES`;
    - `link_at_with` resolves a path that exists only under `extra_root`.
- `cargo test -p runner-app`, with tests in `file_links.rs` and `surfaces/files/tests.rs`:
  - a relative path that resolves only under a Files root different from the spawn cwd links and opens in the preview;
  - an editor-scheme OSC 8 link opens in the preview.

  New app tests that wait on the Files root's resolution use `h.wait(…)`, and run under `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'` before hand-off.
- `make clippy`, `cargo fmt --all -- --check`, `git diff --check`.

### Live checks (macOS development app)

These are selected because the change alters which terminal text underlines on hover and opens on ⌘-click, in real agent TUIs and plain terminals. Setup is a scratch git repository under `/tmp` with these files:

- `crates/demo/src/deep_module/unique_file.rs`
- `app/users/[id]/page.tsx`
- `app/(marketing)/layout.tsx`
- `docs/my plan.md`
- `docs/guide.md`
- `src/a.rs` and `src/b.rs`

QA runs each agent chat with `RUNNER_RECORD_INPUT_FIXTURE` set, so a failing shape comes back to the coder as a recording.

1. **TFL-01 — Wrapped paths in agent TUIs.** Open a Codex chat and a Claude Code chat in the scratch repository, each in a pane about 50 columns wide. Ask each agent to print the absolute path of `unique_file.rs` followed by `:3`. Hovering either row underlines the whole path across both rows, without indentation or padding, and ⌘-click opens the file at line 3.
2. **TFL-02 — Path shapes in a terminal tab.** In a terminal tab in the repository, print each of these with `printf`:
   - `@crates/demo/src/deep_module/unique_file.rs:3-5`
   - `b/crates/demo/src/deep_module/unique_file.rs`
   - `查看 crates/demo/src/deep_module/unique_file.rs，测试`
   - `app/users/[id]/page.tsx#L2C1`
   - `Read(app/(marketing)/layout.tsx)`
   - the absolute path of `docs/my plan.md` with `:2`
   - an OSC 8 `vscode://file/<absolute path of unique_file.rs>:3`
   - a plain-text `cursor://file/<same path>:3`
   - an OSC 8 link whose target is `docs/guide.md`

   Each one underlines only its path and suffix, without the prefix, quotes, parentheses or CJK punctuation, and ⌘-click opens the right file at the right line.
3. **TFL-03 — Adjacent paths stay separate.** In a narrow terminal tab, print `src/a.rs` and `src/b.rs` on consecutive rows. Hovering and ⌘-clicking each row opens only its own file.
4. **TFL-04 — Files root as a lookup root.** In a terminal tab started at the repository root, choose `crates/demo` as the Files folder (`⋯` → **Choose folder…**). Print `src/deep_module/unique_file.rs:3`. It links, and ⌘-click opens it in the preview at line 3.

Record results with screenshots in `docs/tests/runs/1452-terminal-file-link-detection-mac.md`.
