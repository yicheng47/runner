# 0031 impl log

Progress record for the Rust-native UI rewrite ([plan](plan.md), issue [#307](https://github.com/yicheng47/runner/issues/307)). Newest entries at the bottom. Keep entries short: what happened, what's next, blockers.

## 2026-07-18 — Phase 1 kickoff

- Plan doc committed on `rust-ui-nightly` (`163c048`), then moved into this dedicated dir (`docs/impls/0031-rust-native-ui-rewrite/`).
- Branch strategy per Jason: Phase 1 work happens on `phase1-terminal-spike` (off `rust-ui-nightly`); merge back into nightly only after the spike works.
- Scaffolded `crates/native-spike/` as a new workspace member. Dependency versions resolved: `gpui 0.2.2`, `alacritty_terminal 0.26.0`, `portable-pty 0.9` (matches `src-tauri`).
- Next: fixture recorder + replay harness (no GUI dependency), then the GPUI window wiring PTY → `Term` → grid element, then composer/IME, then the throwaway .app packaging pass.

## 2026-07-18 — Phase 1 spike built (same session)

- Environment: gpui's shader build needed the Xcode 26 Metal Toolchain (`xcodebuild -downloadComponent MetalToolchain`) — installed once; noted as a build prerequisite.
- `crates/native-spike/` complete: `terminal.rs` (portable-pty ⇄ `alacritty_terminal::Term`, event pump answers PtyWrite/ColorRequest/TextAreaSize queries), `terminal_element.rs` (custom GPUI element: bg quads + shaped runs + cursor; ASCII spans shaped together, wide/non-ASCII glyphs shaped per cell at their own column origin so fallback-font advances can't skew the grid), `composer.rs` (full `EntityInputHandler` IME field adapted from gpui's Apache-2.0 input example), `main.rs` (window, focus, keybindings, scroll, paste).
- gpui is optional behind the default-on `gui` feature; harness + recorder build without Metal via `--no-default-features`.
- Fixture corpus started (spec 42 debt): `record-fixture` bin logs real PTY bytes to NDJSON; replay tests snapshot the grid. Corpus: real interactive `claude-session` (boot → prompt → streamed reply → /exit palette), `top-busy`, `width-torture`. 10/10 tests green, including wide-char spacers, combining marks, leading-spacer wrap, reflow round-trip (asserts alacritty's real cursor-pinned scrollback-push semantics), alt-screen restore, scrollback clamp.
- Packaging criterion: `package.sh` builds release, assembles `Runner Native Spike.app`, codesigns with the local Developer ID (validates). Notarization prepared (`--notarize`, NOTARY_PROFILE or APPLE_ID trio) — needs Jason's credentials.
- Licensing note: Zed's `terminal`/`terminal_view` crates (GPL) were used as architectural reference only; all code here is original or adapted from the Apache-2.0 `gpui` crate itself (its `examples/input.rs`).
- Pending: human-eyes criteria (busy-output smoothness, IME typing, resize feel, notarize+staple) — checklist in `crates/native-spike/README.md`; reviewer pass on the working-tree diff.

## 2026-07-18 — Review round 1 (same session)

- Reviewer found 6 issues; all addressed:
  1. (High) `TerminalSession` Arc cycle — event thread held the session while blocking on the event receiver whose senders the session (transitively) owned, so Drop never ran and the PTY child leaked. Event thread now holds only writer/size/title Arcs; Drop kills **and reaps** the child. Regression: `tests/session_lifecycle.rs` proves dropping the last session Arc terminates the child.
  2. OSC 10/11/12 color queries clamped named slots (256/257/258) into the 0-255 palette — every probe got grayscale slot 255. Added `palette::resolve_index` handling alacritty's named color-table slots; `tests/osc_color_query.rs` asserts the emitted OSC 10/11/12/4 replies and the DA `PtyWrite` path.
  3. Renderer span extension let an ASCII cell join a span started by a narrow non-ASCII glyph (box drawing/braille), re-creating the fallback-advance skew. Spans now track `ascii_only`; only pure-ASCII spans extend.
  4. IME marked-text selection offsets were converted against the whole content; NSTextInputClient sends them relative to the marked string. Extracted `text_util::marked_selection` (convert against `new_text`, then anchor) with multibyte-prefix tests.
  5. Composer navigation/backspace used scalar boundaries, tearing ZWJ emoji/skin tones/flags/combining marks. Now grapheme-cluster boundaries via `unicode-segmentation` (`text_util`), tested against exactly the width-torture classes.
  6. Spec gap: plan said "spawn via the existing session manager"; the mission brief explicitly allowed direct portable-pty for the spike. Recorded as an explicit deviation in the decision memo and narrowed its claims — integration seam validation is Phase 2/3, not spike-validated.
- Post-fix: fmt/clippy clean, 23 native-spike tests green (10 fixture + 6 OSC + 6 text_util + 1 lifecycle), full workspace battery (fmt --check, clippy --workspace, test --workspace) re-run green.

## 2026-07-18 — Review round 2 (same session)

- Re-review passed findings 1/3/4/5 and accepted the documented session-manager deviation. One must-fix remained from finding 2: color-query replies ignored runtime OSC overrides (a program that sets OSC 10 then queries it got the static default, disagreeing with the renderer which reads `content.colors`).
- Fix: new `terminal::query_color` prefers `Term::colors()[index]` then falls back to `palette::resolve_index`; the event pump reaches the Term through a `Weak` (upgrade per query) so the drop-cycle fix from round 1 stays intact. The OSC tests now resolve replies through `query_color` against the live Term — including set-then-query (OSC 10, OSC 4;1) and OSC 104 reset-then-query coverage.
- Final state: 26 native-spike tests green, workspace battery green (fmt clean, clippy zero warnings, 473 tests passing).
- Round 3: reviewer's sandbox denies `ps`, which made the lifecycle regression environment-dependent (it treated any `ps` failure as process-dead). Rewritten hermetically: hold `Weak<TerminalSession>`, drop the last Arc, assert `upgrade()` is None — the exact signal the original event-thread Arc cycle trips — with child teardown implied by Drop's synchronous kill+wait. 6/6 isolated runs green.
- Review clean after round 3; spike committed on `phase1-terminal-spike` (`6d6ee2e` code, `43d0b28` docs).

## 2026-07-19 — Phase 1 verdict: GO

- Jason smoke-tested the spike: live claude session renders and drives correctly ("works mostly"), and after clarifying that the IME target is the composer bar (the terminal pane intentionally has no IME in the spike), **Pinyin input in the native composer confirmed working** — the hard requirement.
- Jason added Makefile targets for the spike and `default-run = "native-spike"` (two bins made bare `cargo run -p native-spike` ambiguous).
- Known cosmetic artifact triaged: block-glyph striping in the claude logo (glyphs don't fill the 1.4× line box); fix options noted (tighter line height now, cell-box glyph stretching in Phase 3).
- Notarization (criterion 5's second half) deferred to Phase 5 by Jason's decision — codesign of the hand-rolled bundle is proven, notarytool flow already ships Tauri releases, so the residual risk is negligible.
- Decision memo in [plan.md](plan.md) finalized: **go on GPUI**. Next: merge `phase1-terminal-spike` → `rust-ui-nightly` on Jason's word, then Phase 2 (app-core extraction on `main`).

## 2026-07-19 — Phase 2 shipped; branch strategy revised

- Phase 2 (app-core extraction) implemented on `refactor/0031-app-core` off `main`: new workspace crate `crates/runner-app` holds the UI-agnostic core — db/repo/model, session manager, event bus, router, MCP server, and the ~85 command bodies as plain fns in `runner_app::ops` over a new `AppCore`. Tauri command wrappers are one-line delegations (invoke contract byte-identical); event fanout swapped to a `tokio::sync::broadcast` channel with the Tauri layer as a single forwarding subscriber (one `app.emit` left in the whole crate). Event-name/payload inventory verified identical pre/post. No schema, stored-format, or frontend changes.
- Review clean in one round (no must-fix; reviewer confirmed command-registration parity, zero tauri deps in runner-app via cargo tree, and accepted the two documented tradeoffs: Weak session-manager upgrade in the emitter, bounded 8192 broadcast queue). `make ci` green (441 tests); PR [#310](https://github.com/yicheng47/runner/pull/310) open against `main`, CI green. Reviewer's maintenance note applied: release skill now bumps `crates/runner-app/Cargo.toml` too.
- **Branch-strategy change (Jason's decision)**: `rust-ui-nightly` becomes a Tauri-free pioneer branch — after `main` merges in post-#310, delete `src/`, `src-tauri/`, and the JS toolchain from nightly; nightly = shared backend crates + native UI only. Cutover becomes merging nightly into `main`. Modify/delete merge conflicts on the deleted trees are the accepted cost. Details in [plan.md](plan.md) §Branch strategy.
- Next: Jason merges #310 → merge `main` into nightly → Tauri-deletion commit on nightly (incl. Makefile/CI adjustments, TBD in plan) → Phase 3 walking skeleton as `crates/runner-native/`.

## 2026-07-19 — Branch renamed to `gpui-nightly`; #310 re-homed off `main`

- `rust-ui-nightly` renamed to `gpui-nightly` (local + remote, old ref deleted, same SHA `8584caa`) — the branch is a GPUI bet specifically, so the name should say so. plan.md references updated; historical log entries above keep the old name.
- Jason reset `main` after merging #310, so the PR reads MERGED on GitHub but its tree is not on `main`; the extraction lives on `refactor/0031-app-core` (`461356e`). Nightly composition therefore sources `crates/runner-app` by merging that branch (plus current `main`) rather than getting it via `main`. Whether the extraction ever re-lands on `main` is open — not needed for the nightly line.

## 2026-07-19 — Nightly composition

- Created `phase3-walking-skeleton` from `gpui-nightly`, merged current `origin/main` (frontend fixes #309/#311 plus v0.3.17), then merged the Phase 2 extraction from `refactor/0031-app-core` (`461356e`). Hand-check confirmed #309/#311 have no backend half to preserve; their `src/` changes are pure deletions on the Tauri-free line.
- Deleted the React/Tauri trees and JS toolchain. `crates/runner-app` is the surviving backend and was aligned to v0.3.17; the workspace lockfile drops the Tauri dependency graph.
- Build decision: the Makefile exposes only `fmt`, `clippy`, `test`, and `run-native`; CI is one `macos-26` Rust-workspace job for pushes/PRs targeting `gpui-nightly`. The obsolete Tauri release workflow is removed, with native packaging intentionally deferred to Phase 5.

## 2026-07-19 — Phase 3 walking skeleton

- Replaced `crates/native-spike/` with `crates/runner-native/`, preserving its GPUI terminal element, palette, IME-capable composer, and terminal fixture corpus. The binary boots `runner_app::AppCore` against the same release/debug SQLite database and logs directories as Tauri, lists existing direct chats, and opens or resumes a selected chat in a live terminal with a working composer.
- Repaired the Phase 1 deviation at the integration seam: `runner-app`'s canonical `SessionManager` now owns spawn/resume, environment composition, input, resize, and lifecycle. `runner-native` only attaches an `alacritty_terminal::Term` to the manager snapshot plus `AppCore` broadcasts; it has no direct `portable-pty` lifecycle. A `/bin/cat` integration test proves direct-session output reaches the rendered grid through that path.
- Removed the superseded spike and its throwaway recorder, lifecycle test, and packaging script; updated the workspace default binary and `make run-native` to `runner-native`. Native validation covers 27 tests, including all recorded PTY fixture replays; `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` are green.
- Next: Jason smoke-tests with `make run-native`, clicks a direct chat, verifies terminal input/output, scrolling, resize, and Pinyin composition in the composer. The branch remains local and unmerged until that confirmation.

## 2026-07-19 — Phase 3 review round 1

- Reviewer found three branch-composition issues: the authoritative plan still described Phase 2 as present on `main` and left Makefile/CI undecided; a Tauri sidecar-staging script and release skill survived their deleted targets; CI did not install GPUI's Metal Toolchain prerequisite.
- Fixed all three: the plan now records the extraction's actual branch and the completed pioneer-line decisions, stale Tauri automation is removed, and CI installs the Metal Toolchain component before building. The bug skill's adjacent version lookup now reads `crates/runner-app/Cargo.toml`.
- Post-fix validation is green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `git diff --check`.

## 2026-07-19 — Human smoke-test follow-ups

- The walking skeleton rendered and drove existing direct chats successfully; Jason confirmed the final switch/reattach fix and authorized merge back to `gpui-nightly`.
- `make run-native` incorrectly used a release build and therefore opened the production database during testing. The working-tree fix switches the development command to the debug profile and its `com.wycstudios.runner-dev` database; reviewer found the delta clean.
- Chinese IME works in the composer but not the focused terminal because the terminal handles raw key events without a GPUI input handler. Jason declared terminal IME a pre-release requirement; it is now explicit in the Phase 4 direct-chat slice and remains a Phase 6 cutover blocker.
- Switching between running chats replayed cursor-addressed TUI output at a hard-coded `100×30` before resizing, corrupting the grid when the live PTY used the actual pane width. The fix inherits the outgoing rendered size for resume and reattach; reviewer found it clean and Jason confirmed it by retest. Known parity limitation: after resizing the window while a shell-runtime chat is inactive, its old-width snapshot can remain mis-wrapped because shells do not repaint on `SIGWINCH`; Claude Code and Codex repaint after the genuine width change.

## 2026-07-19 — Repo split: `gpui-nightly` → `runner-gpui`

- The nightly line moved to its own repository, [yicheng47/runner-gpui](https://github.com/yicheng47/runner-gpui), seeded by pushing `gpui-nightly` (`7da6fea`, post-Phase-3 merge) as `main` — full history preserved. The name deliberately says the framework; "runner-native" was considered and rejected as implying multi-platform, and this line is macOS-only by plan.
- Rationale: after `main` was reset off #310, the shared-crates merge premise broke (main's backend at `src-tauri/src/*`, this line's at `crates/runner-app/*`), so every branch sync would have leaned on rename detection across the extraction plus modify/delete conflicts — monorepo costs without monorepo guarantees. The split makes the fork honest, and cutover-on-success becomes a repo rebrand (archive `runner`, rename this) instead of an in-place tree-swap merge.
- The `runner` repo is wired as the `upstream` remote here; backend fixes on the Tauri line are ported by conscious cherry-pick, not automatic merges. `gpui-nightly` and the `runner-wt1` worktree are retired; `runner`'s `main` remains the shipping 0.3.x Tauri app in maintenance.
- Docs cleanup: the open Tauri-line specs (features 05/19/21/24/37, impl 0019 landing page) moved to their `archive/` dirs unshipped — frozen during the parity-only rewrite, noted as such in the features README. `product/vision.md` stays as-is; `arch/arch.md` keeps its model/protocol sections but its stack section still says Tauri 2 + React — update pending, tracked as Phase 4-adjacent doc debt.
