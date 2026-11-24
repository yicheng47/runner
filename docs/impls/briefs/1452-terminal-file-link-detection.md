# #1452 — Terminal file-link detection

## Starting point

- Issue: [#1452](https://github.com/yicheng47/runner/issues/1452), milestone 0.20. Draft PR: [#1456](https://github.com/yicheng47/runner/pull/1456).
- Branch: `feat/1452-terminal-file-link-detection`
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1452-terminal-file-link-detection`, based on `a084538b` and using its own `target/`. Never share `CARGO_TARGET_DIR`.
- The branch holds two spec commits (`f22131af`, `bb5e9ce6`) and this brief. Fold all three into the implementation commits, with the spec in the first one, so no docs-only commit lands on its own.
- Isolation: keep the root checkout on `main` and leave `.worktrees/feat-1444-code-block-header` alone; its neon trio mission is running.

## Problem and outcome

Paths printed by agent CLIs often fail to underline on hover or open on ⌘-click (Ctrl-click on Windows and Linux).

- Agent TUIs hard-wrap long paths, so the path never joins across rows.
- `file_link_regex()` only stops at ASCII punctuation, and treats `(` and `[` as ending a path.
- `@`, `a/` and `b/` prefixes, spaces and line ranges fail.
- Local `vscode://file/…` and other editor links open outside Runner.
- The Files view's root is not a lookup root.

The spec [`docs/features/1452-terminal-file-link-detection.md`](../../features/1452-terminal-file-link-detection.md) is authoritative. It covers wrapped paths, path tokens, editor-scheme and OSC 8 path links, the Files root as a lookup root, and a probe budget.

Where links open does not change. #1458 owns that, and #1459 owns shortened-path lookup.

## Read first

- The spec above, including its Prior art section. Orca's `src/shared/terminal/links/` is checked out read-only at `~/repos/ai/orca`; read it with `git show origin/main:<path>` and never modify that checkout.
- `crates/runner-terminal/src/terminal.rs`:
  - `TerminalMirror::link_at`
  - `terminal_link_at_with_hard_breaks`, `link_rows_joined`, `link_row_content`
  - `terminal_hyperlink_at`
  - `file_link_regex`, `url_regex`, `file_target_from_captures`
  - `candidate_paths`, `resolve_file_candidate`
  - `file_target_from_uri_with_platform`, `split_line_suffix`
- `crates/runner-terminal/src/terminal/tests.rs`: the `terminal_links_*` tests, `plain_text_and_osc8_file_uris_resolve_to_file_links`, and `file_link_regex_matches_windows_drive_and_unc_paths_with_suffixes`.
- `crates/runner-app/src/file_links.rs` (`FileLinkRoute`) and `crates/runner-app/src/terminal/element.rs`:
  - `refresh_hovered_link` and `hover_signature`
  - `mouse_down` and `mouse_up`
- `crates/runner-app/src/surfaces/files/panel_view/roots.rs` (`root_path`), and `link_files_view` in `surfaces/panes/header.rs` and `surfaces/mission_workspace/rail.rs`.
- `docs/guide/guides/chats-and-windows.md` and its zh-CN twin: the paragraph on ⌘-clicking a file path in a terminal.

## Deliverables

Follow the spec's Implementation Phases:

1. **Move link detection into `crates/runner-terminal/src/terminal/links.rs`.** Make it its own commit, with no behavior change and existing tests passing unchanged.
2. **Detection in `runner-terminal`** (spec sections 1, 2, 3 and 5):
   - the hard-wrap file step;
   - the allow-list path class and closer rules;
   - spaced-path growth;
   - `@`, `a/` and `b/` fallbacks;
   - the new location suffixes;
   - editor-scheme and OSC 8 bare-path targets;
   - `MAX_LINK_PATH_PROBES`;
   - `FileLinkContext` and `TerminalMirror::link_at_with`.
3. **Wiring in `runner-app`** (spec section 4): `FileLinkRoute::link_context`, the context passed through `element.rs`, and the extra root added to `hover_signature`.
4. **Guide:** both language versions of the ⌘-click paragraph, as the spec says.

Acceptance:
- Every automated case in the spec's Verification section has a test.
- All existing link tests pass without weakened assertions.
- URL hard-wrap joining behaves exactly as before.

Non-goals are as listed in the spec. Do not change `FilesView::open_terminal_link`, the routing in `FileLinkRoute::open`, or settings.

## Validation and handoffs

- **@coder:**
  - Run `cargo test -p runner-terminal`, `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check` and `git diff --check`.
  - Loop any new app test that waits on background work with `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'`, and state the count.
  - Hand `@reviewer` a local commit SHA with a summary, the changed files and the checks run. Do not push yet.
- **@reviewer:** Review that SHA against the spec, focusing on:
  - false joins between rows;
  - URL behavior left unchanged;
  - the probe budget, since hover runs on the UI thread;
  - Windows path handling in the new parsers;
  - rejecting malformed editor URIs;
  - highlight ranges.

  Name the selected live checks and the candidate SHA in the clean review.
- **@qa:**
  - Take `script/qa-lock acquire --timeout 540` before building or launching the macOS development app. The #1444 mission's qa may hold the lock, so wait cleanly.
  - Run TFL-01 to TFL-04 from the spec. Record each agent chat with `RUNNER_RECORD_INPUT_FIXTURE`, using an absolute scratch prefix.
  - Record pass or fail per check, with screenshots, in `docs/tests/runs/1452-terminal-file-link-detection-mac.md`.
  - If a wrapped shape fails, attach its recording to the verdict for `@coder`.
  - Archive only sessions qa created, clean up `/tmp`, and `script/qa-lock release`.

## Authorization

The approved crew is the `neon trio` (3 slots: `coder`, `reviewer`, `qa`).

After a clean review and qa's verdict, `@coder` may:
- shape the commits;
- push the existing branch with `git push --force-with-lease`;
- turn draft PR #1456 into the implementation PR: rewrite its title and description with `Fixes #1452`, and mark it ready for review;
- label it `risk-medium` and `frontend-only`, keeping `core-review` if the workflow adds it;
- keep `needs-live-test` until every live check passes;
- have `@reviewer` apply `reviewed` on the head SHA;
- drive CI green on all platforms.

Do not merge.
