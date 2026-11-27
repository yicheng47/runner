# #1508 — Keep terminal engine types inside runner-terminal

Jason approved the neon duo (Antigravity `gemini-4-neon` coder, Codex `gpt-6.2-astra` reviewer) on 2026-11-27. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/refactor-1508-terminal-boundary` on the branch `refactor/1508-terminal-boundary` (base `origin/main` `47b59c77`, its own `target/`). Never touch the root checkout or sibling worktrees, in particular `feat/1508-libghostty-vt`, where a libghostty-vt trial is running.

## Problem and outcome

#1508 plans to replace `alacritty_terminal` with Ghostty's `libghostty-vt`. Today alacritty's types cross `runner-terminal`'s boundary in two ways:

- **Twelve files outside it import `alacritty_terminal` directly**, through the `alacritty_terminal = "0.26"` dependency in `runner-app` and `runner-daemon`. They are `runner-app`'s `app_settings.rs`, `terminal/element.rs`, `terminal/find.rs`, `terminal/input.rs`, `surfaces/settings/page/appearance.rs`, `surfaces/settings/page/tests.rs`, `surfaces/remote_client/tests.rs` and `surfaces/files/syntax.rs`, and `runner-daemon`'s `shell_integration.rs`, `session/state/tests.rs`, `session/manager/tests/archive.rs` and `session/manager/terminal/tests.rs`. `TerminalElement`'s prepaint walks `term.renderable_content().display_iter` itself. Separately, `runner-daemon`'s `runtimes/codex/terminal.rs` runs its own `vte::Parser` for Codex title events, through the daemon's `vte = "0.15"` dependency.
- **`runner-terminal`'s own public API is written in alacritty's types.** `InputTracker::observe_output`, `track_delivery_input`, `track_delivery_submit` and `unsent_notice` take `&Term<T>`. `TerminalMirror::mode` returns `TermMode`. Selection takes alacritty's `Point`, `Side` and `SelectionType`. `find::newest_matches` and `snapshot::serialize` take `&Term`, `palette` uses alacritty's `Rgb`, `Color` and `Colors`, and `configure` takes alacritty's `CursorShape`. Its tests in `crates/runner-terminal/tests/` import alacritty too.

Outcome: no crate other than `runner-terminal` names an `alacritty_terminal` or `vte` type. `runner-terminal`'s public API and its tests use only Runner-owned types, and Runner behaves exactly as it does now. After this, replacing the engine touches only `runner-terminal` and the build.

## Read first

- `docs/features/1508-libghostty-vt.md` on `feat/1508-libghostty-vt` (`git show origin/feat/1508-libghostty-vt:docs/features/1508-libghostty-vt.md`): Decision 3 and **Public `runner-terminal` boundary**. The types named there are the target shapes. Design them to map onto both engines: Ghostty `vt-1.0.2`'s `terminal.h`, `render.h`, `screen.h`, `grid_ref.h`, `selection.h`, `search.h`, `color.h` and `modes.h`, fetched with `gh api 'repos/ghostty-org/ghostty/contents/include/ghostty/vt/<file>?ref=vt-1.0.2' --jq .content | base64 -d`. Make them Runner's own shapes, not a copy of either engine.
- `runner-terminal`: `lib.rs`, `terminal.rs` (the `TerminalMirror` public methods), `terminal/find.rs`, `terminal/links.rs`, `input_state.rs`, `delivery_probe.rs`, `palette.rs`, `snapshot.rs`, `replay.rs` and `mappings.rs`, and `tests/`.
- The twelve files above.
- `AGENTS.md`, Test Scope: the definition of a mechanical refactor.

## Deliverables

1. **Runner-owned types in `runner-terminal`:** colors (an `Rgb` and the color and named-color shapes `palette`, the app and Files syntax highlighting use), `CursorShape`, mode flags covering every mode a caller tests, grid coordinates (a point with line and column, and a side), selection kind, a find match, and the cell flags callers read.
2. **A read-only screen view** for the code that reads the grid: `TerminalElement`'s prepaint, `InputTracker`, `InputProbe`, find and links. It exposes the visible rows of cells (text including combining characters, wide and spacer flags, style flags, foreground, background and underline colors, and hyperlink), the cursor, the modes, the display offset and the history size. `runner-terminal` implements it over alacritty's `Term` internally. Shape it as #1508's `ViewportSnapshot` will carry it, so Phase 1 changes only where the data comes from.
3. **Borrow, don't copy, on the paint path.** Prepaint reads cells every frame. The view yields borrowed Runner-typed cells under the lock prepaint already takes, adding no allocation or copy per cell or per frame.
4. **The public API and callers.** `runner-terminal`'s public API takes and returns only these types, converting inside `runner-terminal`. The twelve files and `runner-terminal`'s tests use only that API. Tests that build a raw parser or `Term` (`Processor`, `TermSize`) get a `runner-terminal` helper instead.
5. **The compiler enforces the boundary.** The Codex title parser moves into `runner-terminal` behind a Runner API, unchanged in behavior and with its tests. Then `alacritty_terminal` is removed from `runner-app`'s and `runner-daemon`'s `Cargo.toml` and `vte` from `runner-daemon`'s, dev-dependencies included, so a future import fails to build. `cargo tree -i alacritty_terminal` and `cargo tree -i vte`, for both crates, show them only through `runner-terminal`.
6. **Docs.** Update the pointers in `docs/tech/alacritty-terminal.md` and `docs/tech/terminal-rendering.md` that name moved types.

Not in scope: any change of behavior, painting, GPUI entity or view structure, notification, timing or threading; `libghostty-vt`; the `ViewportSnapshot` publishing and actors of #1508's Phase 1; and unrelated renames. If a caller cannot move without a behavior change, stop and report to the operator instead of changing behavior.

## Validation and handoffs

- **@coder:** `cargo test -p runner-terminal -p runner-core -p runner-daemon -p runner-app`, with every fixture snapshot and golden unchanged (never re-bless or weaken an assertion); `make clippy`; `cargo fmt --all -- --check`; `git diff --check`; and the `cargo tree` checks. Run heavy suites one at a time with other worktrees. Hand `@reviewer` local commits with a summary, the checks run and the list of types and API that moved.
- **@reviewer:** checks that behavior is unchanged: the same cells, colors, modes, selection, find matches, links, input states and delivery outcomes, with no new allocation on the paint path and no change to locking. Checks that nothing outside `runner-terminal` names an engine type, and that the new types would map onto `libghostty-vt` as well as alacritty. The work is meant to be a mechanical refactor under AGENTS.md with no live checks. If the diff goes beyond that, the clean review names the live checks needed and the operator adds `qa`.

## Authorization

Approved crew: the neon duo. The crew may commit, push `refactor/1508-terminal-boundary` and open a pull request against `main` titled `refactor(terminal): keep engine types inside runner-terminal (#1508)` with `Refs #1508`, labeled as AGENTS.md requires, and drive CI green. Fold this brief into the commits. No co-author trailers. Do not merge: Jason merges it.
