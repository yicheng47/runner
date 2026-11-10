# #1248 items 6 and 7 — dead code and module placement

Jason approved this mission for the codex duo on 2026-11-10. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/chore-1248-dead-code-module-placement` on `chore/1248-dead-code-module-placement`, based on `origin/main` at `7f7cde843575ae1d712546f6c10a10a5b38024d7`. The root checkout stays on `main`. Use scratch paths prefixed `/tmp/1248-m1-` so this mission doesn't collide with another.

## Problem

[#1248](https://github.com/yicheng47/runner/issues/1248) is the post-0.19 cleanup audit. This mission takes its two small, mechanical items: item 6 (dead code) and item 7 (module placement). Read those two sections of the issue first; they name every file and function. The later items (launch parameters, Markdown, Claude accounts, the `NativeRoot` and Files splits) are separate missions and out of scope here.

## Deliverables

Item 6: delete these `pub` functions, which nothing in the workspace references. Confirm each has no caller first (`rg -w <name> crates`), and delete whatever becomes unused as a result, such as an import or a private helper that only the deleted function called.

- `runner-daemon/src/ops/session.rs`: `session_resize`, `session_last_size`, `session_set_live_title`, `session_start_direct_impl`
- `runner-daemon/src/repo/slot.rs`: `count_for_crew`
- `runner-daemon/src/repo/mission.rs`: `unarchived_ids_for_crew`
- `runner-app/src/ui/agent_status.rs`: `header_status_indicator`
- `runner-app/src/theme.rs`: `text_display_xl`

Leave `session_start_direct_impl_with_speed` and the other `_with_speed` twins alone: item 4's mission reshapes them.

Item 7, all in `runner-app/src`:

- Move `surfaces/mission_composer.rs` and `surfaces/mission_feed.rs` under `surfaces/mission_workspace/`, `surfaces/sidebar_logic.rs` under `surfaces/sidebar/`, and `surfaces/project_field.rs` under `surfaces/start_chat/`. Pick module names that don't clash with the existing `mission_workspace/feed.rs` and `composer.rs`; if two files cover the same thing, merging them is fine, but don't rewrite logic. Leave `surfaces/mission_markdown.rs` where it is (item 2 owns it).
- Move the six crate-root `terminal_*.rs` files (`terminal_clipboard`, `terminal_fonts`, `terminal_ime`, `terminal_paste`, `terminal_resize`, `terminal_redraw_tests`) under `terminal/`. `terminal_ime`, `terminal_paste` and `terminal_resize` are `pub mod` in `lib.rs`: check who uses them from outside the module tree (the bin, integration tests, other crates) and keep their public paths working, with re-exports if needed.
- Merge the two home-to-`~` helpers into one: `surfaces/files/picker.rs::display_path` and the abbreviation inside `ui/field/working_dir.rs::compact_path_candidates`. Keep each caller's current output, including Windows separators, and keep their tests passing.

Use `git mv` so history follows the files. Moves change only paths and `use`/`mod` lines; `include_str!` paths, `super::` paths and any test names that a Windows self-exec probe matches by name can break on a move, so grep for them before cutting. No behavior change, no unrelated cleanup, no renames beyond what the moves need.

## Validation

Name the changed behavior (none) and why each check covers the change. Run `make verify` in this worktree with its own `target/`, logging to a file and checking the exit code instead of piping through `tail`. The `runner-app` tests and workspace Clippy cover the moves; the daemon tests cover the deletions. No live checks: nothing a person sees or operates changes. CI must be green on macOS, Windows and Linux.

## Authorization

The codex duo (coder and reviewer) may commit, push this branch and open one PR against `main` that closes neither #1248 nor any other issue (reference it as "Part of #1248"). Label it `no-behavior-change` and `risk-low`. The coder can split deletions and moves into two commits if that reads better.
