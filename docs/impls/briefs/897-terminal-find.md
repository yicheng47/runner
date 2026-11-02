# #897 — Find in terminal

Jason approved this `claude duo` mission on 2026-11-02 and will add a qa slot later. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-897-terminal-find` on `feat/897-terminal-find`, based on `origin/main` at `0b4eae1f83d71d0a0a7bbcd7393f79f215bab7f4`. The root checkout stays on `main`; this worktree has its own `target/`.

## Problem and outcome

Runner's terminal has no search; finding an earlier build error means scrolling for it. Add ⌘F find to every terminal view: a floating find bar with match case and regex toggles, a match count, highlights on every visible match, stepping, and Esc to close.

The spec is [docs/features/897-terminal-find.md](../../features/897-terminal-find.md); every decision in it is signed off. The design is `runner/desktop/specs/897-terminal-find.pen` in `~/repos/yeelinks/yeelinks_design` (commit `ac804f7`): `cmp/TerminalFindBar`, the find bar states board, and the five screens. The Pencil frames win on visual detail and the spec wins on behaviour. Read `.pen` files only through the Pencil MCP tools, never as text; if they are unavailable, the spec carries the colours and states.

## Read first

- `vendor/alacritty_terminal/src/term/search.rs`: `RegexSearch`, `RegexIter`, `regex_search_right`. `RegexSearch::new` has smart case built in; the spec says how the toggles override it in the pattern.
- `crates/runner-terminal/src/terminal.rs`: `TerminalMirror`, whose `term` is the app's copy of the session, with `scroll_state` and `scroll_to_display_offset`.
- `crates/runner-app/src/terminal/element.rs`: `TerminalElement`, `TerminalInteraction`, and the `display_iter` loop in prepaint that builds selection quads; match highlights follow the same pattern.
- The three terminal views: `surfaces/mission_workspace/terminal_pane.rs` (chat panes), `surfaces/mission_workspace/view.rs` (mission slots) and `surfaces/panes/drawer.rs` (drawer terminals). The agent update dialog's terminal is out of scope.
- `crates/runner-app/src/keymap.rs` (`KeymapEntry`, `windows_default`), `crates/runner-app/i18n/{en,zh-hans}.toml`, `crates/runner-app/src/ui/field/` for the text field, and `theme.rs` for the highlight tokens.
- `docs/tech/alacritty-terminal.md` for grid coordinates, `display_offset` and the ring buffer.

## Deliverables

- Everything under Behaviour in the spec, in chat panes, mission slots and drawer terminals, matching the design's states: just opened, matches, match case on, regex on, no results, invalid regex, the alternate-screen hint, and hover.
- The search runs in the app against the mirror `Term`. Nothing changes in runnerd, the CLI, the client protocol or storage.
- Live output: while the bar is open, recompute matches as output arrives (throttled, not per byte), keep the current match on the same text and hold the viewport still once a match is selected. Alacritty already raises `display_offset` for a scrolled-back viewport; at the bottom, the content's movement has to be counted. History growth gives it until the scrollback limit is full. After that nothing public does, and Jason approved one read-only accessor in the vendored crate for it, such as the grid storage's rotation index. Note that `Storage::initialize` rezeros while history grows, and that resize and clear rebuild the grid. Keep the accessor read-only with no logic change, list it in `vendor/alacritty_terminal/RUNNER_PATCH.md`, and repeat the vendor audit that file describes (`diff -ru` against the published 0.26.0 source shows only the documented additions). If you find an exact approach that needs no accessor, prefer it and say so.
- The keymap entry with its Windows/Linux defaults (Ctrl+Shift+F; F3 and ⇧F3 for the in-bar ⌘G and ⇧⌘G), the Settings → Keyboard shortcuts row, English and Simplified Chinese strings, and the guide pages the spec names.
- The spec's line under Active in `docs/features/README.md` stays.

Non-goals: everything under Not in this issue in the spec, any PTY resize when find opens, changes to how selection, links or IME behave, and refactors outside what find needs.

## Validation

Before running checks, list the changed behaviours and the reason for each selected check (AGENTS.md, Test Scope). In-process tests cover the cases under Verification in the spec. Time a whole-scrollback search on the long Claude transcript fixture in `crates/runner-terminal/fixtures/` and report the number; move the search off the UI thread only if that number calls for it. Run the `runner-terminal` and `runner-app` tests, workspace Clippy with `--profile ci -D warnings`, `cargo fmt --all --check` and `git diff --check`, all with locked dependencies.

`coder` implements and runs the checks; `reviewer` reviews the working-tree diff through Runner when `coder` asks. Neither slot launches the app. The clean review names the live checks the diff needs. A qa slot Jason adds later runs them on the same candidate, so the PR lists them as not yet run.

## Authorization and handoff

Jason authorized implementation, validation, commits on this branch, pushing it, and opening one PR against `main` labelled `frontend-only` and `risk-medium`, then driving the macOS, Windows and Linux CI green. Open the PR only after the review is clean. Shape the history as AGENTS.md, Crew Missions describes: fold this brief and the spec into the implementation commits, fold review and CI fixes into the commits they correct, and push rewrites with `git push --force-with-lease`. Do not add tool-specific co-author trailers. Rebase only when AGENTS.md requires it; never merge `main` into the branch.

Stop at an open PR with green CI and report the PR URL, final SHA, a short summary, check results and the live checks still to run. Do not merge, release, cut a nightly, archive the mission, delete branches or remove worktrees. Do not edit the `.pen` files. When waiting for a teammate, finish the turn and rely on Runner's pushed inbox notification.
