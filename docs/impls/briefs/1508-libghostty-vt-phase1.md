# #1508 Phase 1 — runner-terminal on libghostty-vt, graphics off

Jason approved the neon trio (Antigravity `gemini-4-neon` coder, Codex `gpt-6.2-astra` reviewer, Codex `gpt-6.2-sol` qa) on 2026-11-27, after the trial passed. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1508-libghostty-vt` on the branch `feat/1508-libghostty-vt`, rebased onto `origin/main` `82ad98de` (its own `target/`). Its draft PR is [#1509](https://github.com/yicheng47/runner/pull/1509). Never touch the root checkout or sibling worktrees (`fix/818-terminal-round-trips`, `feat/1458-file-open-setting`).

## Problem and outcome

Runner's terminal engine is a vendored `alacritty_terminal` 0.26 with `vte` 0.15, surrounded by byte scanners for protocols it lacks. Outcome: `runner-terminal` runs Ghostty's `libghostty-vt` `vt-1.0.2` in runnerd's model and the app's mirrors, the scanners and trackers are gone, and `vendor/alacritty_terminal` and `vte` are deleted, with behavior unchanged except where the spec's Decisions 11 and 12 say. Kitty graphics stay off (storage limit 0); #1489 turns them on later.

The spec is authoritative: [`docs/features/1508-libghostty-vt.md`](../../features/1508-libghostty-vt.md), Phase 1, Decisions 1–6, 11 and 12, Design and Verification. [`docs/tech/libghostty-vt-migration.md`](../../tech/libghostty-vt-migration.md) explains why and how.

## Starting point

- `crates/ghostty-vt-sys` from the trial: vendored `vt-1.0.2`, Zig 0.17.0 from `ZIG` (`~/.local/share/zig/0.17.0/zig` on this Mac), the Zig dependencies via `make ghostty-deps`, `bindgen` declarations, and `tests/fixture_parity.rs`. The trial's report is in the spec's Implementation Phases.
- The boundary refactor (#1514) is on `main`. Nothing outside `runner-terminal` names an engine type. `types.rs` and `screen.rs` hold Runner's own types (`GridPoint`, `CellFlags`, `TerminalModeFlags`, `SelectionKind`, `FindMatch` and others) and the borrowed `ScreenView`. Phase 1 reimplements them over `libghostty-vt` and keeps their shapes, so the app and the daemon change as little as possible.

## Read first

- The spec and the migration note.
- `runner-terminal`: `types.rs`, `screen.rs`, `terminal.rs`, `terminal/model.rs`, `snapshot.rs`, `program_status.rs`, `input_state.rs`, `delivery_probe.rs`, `terminal/find.rs`, `terminal/links.rs`, `mappings.rs`, `replay.rs` and `tests/`.
- Ghostty's headers in `vendor/libghostty-vt/include/ghostty/vt/`, mainly `terminal.h`, `render.h`, `screen.h`, `grid_ref.h`, `selection.h`, `search.h`, `key.h`, `mouse/`, `snapshot.h`, `formatter.h` and `sys.h`. Ghostty's source at `vt-1.0.2` is in `/Volumes/samsung/repos/terminal/ghostty`.
- herdr's code, for reference only: `/Volumes/samsung/repos/terminal/herdr-app` (`src/vt.rs`), `herdr-core` (`crates/ghostty-vt`) and `herdr`. Read it for how they use the API; write Runner's own code (Apache-2.0, and Runner's design differs). Copy nothing.
- `docs/features/archive/861-program-status.md`, the `OSC 7501` rules.

## Deliverables

Follow the spec's 1a, 1b and 1c as commits that each build and pass their tests. Hand each to `@reviewer` as it is ready.

1. **1a:** `ghostty-vt-sys` in `nightly.yml` and `release.yml` for every target, including macOS x86_64. A safe wrapper inside `runner-terminal`, with the PNG and log callbacks installed. Measure the release app's size change.
2. **1b:**
   - `TerminalModel` and `TerminalMirror` as single-owner threads with no `unsafe impl Send` (Decision 4), and graphics storage 0.
   - The callback options replace `CwdReports`, `SchemeState`, the `program_status` byte parser, `kitty_flags_reply`, `ScrollTracker`, `TrackedProcessor` and `BoundaryScanner`. `OSC 7501` must match #861's rules; record the result in Open question 2.
   - Local snapshots use `ghostty_snapshot_encode` and the decoder. Companion snapshots use the `Formatter`, with `companion::VERSION` unchanged.
   - `input_state`, `delivery_probe`, `find`, `links`, `mappings` and `replay` run on the new engine.
   - Benchmark 25 open sessions for Open question 3.
3. **1c:** `TerminalElement` paints from the view with no per-frame copies. Selection and find run on `selection.h` and `search.h`. `vendor/alacritty_terminal`, `alacritty_terminal` and `vte` leave the workspace (`cargo tree -i` shows neither).
4. **Docs:** `docs/tech/libghostty-vt.md` replaces `docs/tech/alacritty-terminal.md`, and the other pages the spec lists are updated. The migration note stays until #1489 lands, with its status updated.

Every existing test and saved screen passes unchanged. The only exceptions are `resize_reflows_wrapped_lines`, whose expectation follows Decision 11, and any change Decision 12 requires. Never re-bless anything else or weaken an assertion; a difference elsewhere goes to the operator with the evidence. If the design does not work as written, stop and report instead of changing it.

## Validation and handoffs

- **@coder:** the spec's Automated gates: `cargo test -p runner-terminal -p runner-core -p runner-daemon -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`, `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'` for each new test that waits on threads or timers, and the `cargo tree` checks. Run heavy suites one at a time with other worktrees.
- **@reviewer:** reviews each handed-off commit against the spec and Ghostty's headers. Look hardest at: unchanged results in `input_state_replay` and `delivery_probe_replay`, and agent status from `OSC 7501`; reply order and a single reply source; thread ownership and shutdown; snapshot round trips including unfinished sequences; companion compatibility; paint-path cost; and no leftover engine types outside `runner-terminal`. The clean review names the live checks and the candidate commit, adds `reviewed` and records the commit in a PR comment.
- **@qa:** the spec's live checks on the macOS development app, holding `script/qa-lock`, with remote checks on the same-Mac two-host relay fixture. Record pass, fail or blocked per check, with screenshots, in `docs/tests/runs/1508-libghostty-vt-mac.md`. Failures go back to `@coder`. Linux and Windows coverage comes from CI and from the soak in the spec's Verification.

## Authorization

Approved crew: the neon trio. The crew may commit and push `feat/1508-libghostty-vt`, with `git push --force-with-lease` when rewriting its own commits, and use PR #1509. Retitle it `feat(terminal): run runner-terminal on libghostty-vt (#1508 phase 1)`, keep `Refs #1508`, label it `core-review`, `risk-high` and `needs-live-test`, mark it ready once review is clean, and drive CI green on every platform. Fold this brief, the trial brief and the spec into focused commits. No co-author trailers. After clean review and live checks, tell the operator: the operator opens the Preview Nightly list change and tells the crew when to rebase onto it. Do not merge: the soak and Jason decide.
