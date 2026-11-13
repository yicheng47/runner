# Fix Ctrl+clicking Windows `file://` links and drive-letter paths (#1300)

## Starting point

- Issue: `yicheng47/runner#1300`
- Branch: `fix/1300-windows-file-links`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1300-windows-file-links`
- Base: `origin/main` at `c43f8fe6`
- Parallel isolation: sibling worktrees `.worktrees/feat-1295-antigravity-osc7501` and `.worktrees/fix-1306-remote-no-roles-flash` are active in parallel. Stay strictly inside this worktree with its own Cargo `target/` directory; do not touch root `main` or sibling worktrees.

## Problem and outcome

On Windows, Ctrl+clicking `file:///C:/.../report.html` (plain text or OSC 8 hyperlinks) or Windows drive-letter paths (`C:\...\report.html`, `C:/.../report.html`) in a terminal fails because `crates/runner-terminal/src/terminal.rs` lacks Windows path and `file://` support:

1. `file_link_regex()` only allows `(?: ~/ | \.\.?/ | / )?` prefixes, excludes `:` and `\`, and only joins `/` segments, so `C:\...` and `C:/...` (as well as backslash-separated Windows paths) never match.
2. `resolve_file_candidate()` only recognizes `~/` and leading `/`, not Windows drive-letter/UNC paths (`normalize_path` also assumes native `Component` parsing on non-Windows tests unless factored cleanly).
3. `url_regex()` only matches `https?://`, ignoring plain-text `file://` URIs outside OSC 8 hyperlinks.
4. `file_target_from_uri()` extracts `&rest[rest.find('/')?..]` without stripping the leading `/` before a Windows drive letter (`/C:/...` -> `C:/...`, unlike `parse_cwd_report()` which already uses `strip_drive_slash()`).

Expected behavior:

- Plain-text and OSC 8 `file://` URIs resolve cleanly via `file_target_from_uri()` (strip Windows drive slash on `file:///C:/...` on Windows, handle case-insensitive `file://` scheme, query/fragment/line suffixes, and check whether plain-text `file://` matching should validate local host / path safely).
- `file_link_regex()` and `resolve_file_candidate()` recognize Windows drive-letter paths (`C:\...` and `C:/...`, with optional `:LINE[:COL]`, `(LINE,COL)`, `#LLINE` suffixes — without breaking `:LINE` parsing vs `C:`). Verify `runner-app/src/file_links.rs` (`DefaultApp` -> `rundll32.exe url.dll,FileProtocolHandler`) handles the resolved `FileLinkTarget` cleanly on Windows.
- **Explicit non-goal:** Do **not** alter `FilesView`'s existing rule of previewing files inside the project Files root (`crates/runner-app/src/surfaces/files/links.rs`); keep that routing unchanged.

## Read first

- `crates/runner-terminal/src/terminal.rs` (`parse_cwd_report`, `strip_drive_slash`, `terminal_hyperlink_at`, `terminal_link_at_with_hard_breaks`, `file_link_regex`, `file_target_from_captures`, `resolve_file_candidate`, `file_target_from_uri`, `url_regex`)
- `crates/runner-terminal/src/terminal/tests.rs`
- `crates/runner-app/src/file_links.rs` (`FileLinkRoute`, `open_file_link`, `open_argv`, `editor_argv`, Windows tests)
- `crates/runner-app/src/terminal/element.rs` (terminal link hit/open handling)
- `crates/runner-app/src/surfaces/files/links.rs` (`open_terminal_link`, `relative_in`)

## Deliverables and boundaries

- Fix link detection, candidate resolution, and `file://` URI path parsing in `crates/runner-terminal/src/terminal.rs` (and any Windows path handling gap in `runner-app` if found).
- Extract or structure Windows `file:///C:/...` / drive-path normalization helpers so both platform-agnostic unit tests on macOS/Linux and `#[cfg(windows)]` behavior compiled/tested in Windows CI verify the logic thoroughly (including regressions for `file:///C:/.../report.html`, `file://localhost/C:/...`, `C:\...\report.html`, `C:/.../report.html`, `:LINE:COL` suffixes, and non-Windows regression safety).

## Validation and handoffs

- **Automated**: tests in `runner-terminal` (`terminal::tests`) and `runner-app`, plus `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and `cargo clippy --locked --workspace --all-targets --features updater --profile ci -- -D warnings`. Both macOS and Windows CI (`Rust / Windows`) must pass.
- **Handoffs**: `coder` implements on branch `fix/1300-windows-file-links` and hands off an unpushed local commit SHA to `reviewer`. `reviewer` reviews the snapshot and names the required live checks for `qa`.
- **Live QA (`qa`)**:
  - Acquire `script/qa-lock acquire --timeout 540` before launching/wrapping dev builds.
  - On macOS, verify the cross-platform `file://` plain-text and OSC 8 link behavior in the running dev app (e.g., clicking a `file:///tmp/.../report.html` link opens it via `DefaultApp` without breaking relative/`https://` links).
  - Record results in `docs/tests/runs/1300-windows-file-links-mac.md` (new per-issue QA log convention — do not append to `YYYY-MM-DD-mac.md`). Keep existing user sessions untouched and release `qa-lock` once QA's app exits.
  - **Windows live gap:** Native Windows live checks (`C:\...` drive-letter paths and `file:///C:/...` via `rundll32.exe`) cannot execute on this macOS host. Disclose that platform gap explicitly in the QA record and PR body, and **keep `needs-live-test` on the PR** per `AGENTS.md`.

## Authorization

Jason approved the `agy trio` (`coder`, `reviewer`, `qa`) to fix `#1300`, commit, push `fix/1300-windows-file-links`, open a PR against `main` (`Closes #1300`) with `reviewed` from `@reviewer` on the final head SHA, appropriate risk/scope labels, and `needs-live-test` retained for the untested Windows native check, and drive CI green across all platforms (especially `Rust / Windows`). Fold this brief into the implementation commit per `AGENTS.md` before final push. Stop at the open PR; do not merge or clean up the worktree.
