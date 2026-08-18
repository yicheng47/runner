# GPUI rewrite — program log

Progress record for the whole gpui-rewrite program ([README](README.md)), from the Phase 1 spike onward — the 0031-era log is folded in here. Newest entries at the bottom; keep entries short: what happened, what's next, blockers.

## Current state (update with each entry)

- **Branch**: `gpui-nightly` (M0+M1 merged via PR #407, M2 via PR #408, both 2026-08-17; M3.1 via PR #409 and M3.2 via the `feat/0046-m3-resume-seams` PR, both 2026-08-18); M3.3 is implemented on `feat/0046-m3-resize-storms` pending review and human verification.
- **Done**: 0046 M0 (gpui-ce swap), M1 (repo-and-below at `main` parity, node tree adopted, protocol crates wholesale; human-verified 2026-08-17), M2 (terminal split + shell modularization), M3.1 (session reaping), M3.2 (resume seams + geometry); M3.3 implementation and gates are complete.
- **Next**: clean working-tree review and human verification for M3.3, then M3.4 (input latch + native quit stamping); later M3 slices continue in the 2026-08-18 breakdown order, followed by M4 (UI parity) → M5 (sweep + watermark) per the merged [program plan](plan.md). Parity references are `main`'s React frontend (`src/`) and the `design/*.pen` files via the pencil MCP.
- **Parity watermark**: `origin/main` fully ported for repo-and-below as of `1b7ee92` (v0.5.2 line, 2026-08-17); feature-logic lag is M3 scope.

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

## 2026-08-17 — Single repo restored, spec 0046, docs regrouped

- Jason reverted the repo split: `gpui-nightly` is back as a long-lived branch on `yicheng47/runner`, pushed at `774aa35` (no force needed — the branch didn't exist on the remote). The standalone `runner-gpui` GitHub repo is retired; locally the line lives in the `runner-gpui` worktree. CI re-targeted from `main` to `gpui-nightly` (`dde41e7`).
- Impl 0046 (main-parity catchup) authored and committed (`14e9680`): decisions (main owns design + schema, repo-and-below verbatim, product UI never diverges, gpui-ce, Zed-style terminal split on upstream alacritty), workstreams A–D, milestones M0–M4. 0031's Phase 4 slice list superseded.
- Docs regrouped: gpui-rewrite impls now live under `docs/impls/gpui-rewrite/` (0031, 0046, this log, README).

## 2026-08-17 — M0: gpui-ce swap (same session)

- `gpui 0.2` (crates.io, stale since 2025-10) → `gpui = { package = "gpui-ce", version = "0.3" }`. True drop-in: zero source changes; the whole diff is Cargo.toml + lockfile.
- Gates green: fixture corpus (10/10 incl. CJK/emoji/box-drawing/reflow), workspace tests, clippy, binary links.
- `make run-native` renamed to `make run` (Makefile, AGENTS.md, READMEs).

## 2026-08-17 — M1: repo-and-below catchup (same session)

- First `make run` against the dev DB hit `sqlite: no such table: tabs` — the dev DB (shared with the Tauri dev build by design) was at main's migration v20, where 0015 dropped `tabs`/`folders`. Exactly the incompatibility 0046 predicted.
- Ported wholesale from `origin/main` (merge-base `d9483ca`): migrations 0014–0020 + Rust backfills, `db.rs` (LKG login-shell env in `_app_state`, peer-coding seed + `examples/peer-coding/`), `model.rs`, `repo/` (node.rs in, tab.rs/folder.rs out), `shell_path.rs`, `crates/runner-core`, `cli/`. Migrations 0001–0013 verified byte-identical across branches — the chain never forked, so seamless open-either-way holds.
- Adapter layer: `ops/node.rs` ported from main's `commands/node.rs` (bodies + tests verbatim, mechanical Tauri→`AppCore` translation). Node-maintenance hunks applied to mission (start/archive/pin/reset/unarchive/set-project), project (create node; delete archives children through the tree), session (archive removes from node; pin pins the tab node; set-project reconciles placement). Resume path adopts the caller→persisted-last-size→default geometry resolution (migration 0016).
- UI minimally rewired: `pane_layout` + sidebar read tab nodes via `node_list`, write via `node_tab_upsert`. Full node sidebar (projects/folders/pinned) deferred to M3.
- Gates: 498 workspace tests green (incl. main's ported node/db suites), clippy `-D warnings` clean.
- Dev DB sharing restored: the parked main-schema dir was copied back live (v20, nodes present); backup deleted after. Both dev builds share `com.wycstudios.runner-dev` — run one app at a time.
- Deliberately not ported (M3 scope): session hardening (reaping, resume seams, resize-storm coalescing), runtimes (discovery, Qoder, TRAE, model catalog, model/effort override plumbing beyond schema), router/inbox, mission feed, pagination, update checks. `main`'s impl docs 0033–0045 are the port guides.

## 2026-08-17 — Phase 5 updater decided: Sparkle via the pulse pattern (same session)

- Studied pulse (`~/repos/yicheng47/pulse`, pure gpui-ce at a pinned git rev): `SPUStandardUpdaterController` via ~130 lines of `objc2` bindings behind an `updater` feature with a no-op dev fallback; `Sparkle.framework` embedded by `script/bundle-mac` (pinned + SHA-256 checked); appcast on GitHub Releases (`SUFeedURL`, `SUPublicEDKey`).
- Zed's hand-rolled `auto_update` (~2k lines: dmg mount/replace/relaunch, own release API, per-OS helpers) reviewed and rejected — its complexity pays for multi-platform + own-server needs Runner doesn't have.
- Cutover bridge stands: last Tauri release updates users into the first native release via `tauri-plugin-updater`'s artifact format (same minisign keypair + bundle id); Sparkle owns native→native after. Recorded in 0031 Phase 5.

## 2026-08-17 — M2: terminal split + main.rs modularization

- New `crates/runner-terminal` — the `terminal` half of the Zed-style split: `terminal.rs` (`TerminalSession` + `TerminalBridge`), `palette.rs`, `fixtures.rs`, `replay.rs`, and a new `mappings.rs` consolidating keystroke/paste encoding as pure functions (mode state in, bytes out). The fixture corpus (data + `fixture_replay.rs` + `osc_color_query.rs` tests) moved with the model; the replay-race unit test now builds its own minimal `AppCore` instead of borrowing runner-native's bootstrap. No gpui dependency — the crate stays UI-agnostic; the PTY remains `SessionManager`'s (the documented deviation from Zed). The `alacritty.rs` API-isolation module from the 0046 mapping table is deferred until the view side needs it.
- `runner-native/src/main.rs` (1,405 lines) split for M3's parallel lanes: `chat.rs` (session attach/lifecycle, tab activation, key/scroll/paste routing, chat start/resume, split resize), `sidebar.rs` (tab list rendering + labels), `panes.rs` (active-tab surface, layout picker, pane tree). `main.rs` is down to 291 lines of app shell (structs, boot, render root).
- Gates: `make verify` green (check + 498-scale test suite + clippy `-D warnings` + fmt), corpus green in its new crate, binary links.

## 2026-08-18 — M3.1: session process reaping (A4 session-hardening slice begins)

- Ported `main`'s `48d2231` (verify and reap killed processes, #342) and `0dadea7` (reap mission siblings after slot exit, #346) via `feat/0046-m3-session-reaping`: PTY stop verifies death and reaps with a SIGKILL process-group fallback; the startup orphan sweep identity-checks recorded processes; manager kill sweeps aggregate failures and log DB reconciliation errors; unexpected mission-slot exit or spawn failure cancels and reaps live siblings, with the `was_killed` guard keeping intentional kills from recursing. `pty_runtime.rs` SHA-256-matches main's post-commit source; manager tests ported line-identical.
- Reap calls land in `mission_start_impl_with_size` and `mission_reset_impl`'s spawn-failure branch; `ops/node.rs` already carried `48d2231`'s kill aggregation since M1, so no diff there.
- Recorded divergence: `runner-native/src/bootstrap.rs` makes `cleanup_orphan_processes_on_startup` fatal where main's Tauri builder logs-and-continues — deliberate, matching `boot_core`'s existing fatal stale-row convention; the only error paths are DB failures that would abort one line earlier anyway. Also noted: `stop()` may now block ~500ms per stuck session (callers are quit/close handlers and MCP ops).
- Gates: `make verify` green; 428 `runner-app` tests including the ten ported reaping tests; codex-peer review clean with independent SHA and hunk-level verification.

## 2026-08-18 — M3 task breakdown (mission numbering)

M3's slices (0046 §Sequencing) run as serial codex-peer missions, one task at a time; mission titles use this numbering:

- **M3.1** — session reaping: `48d2231`, `0dadea7`. Merged via PR #409 (2026-08-18).
- **M3.2** — resume seams + geometry: `9f53047` (#349), `02e7502`, `cb32720`, `3f3bc04`. Port guides: main's impl 0032 (archived), 0035, 0038. M1 already ported `repo/session.rs`, migrations 0016/0017, and the resume geometry resolution — diff against `origin/main` before applying hunks.
- **M3.3** — resize-storm coalescing: `38af26c` (#373), `e78718f`. Port guide: 0039. Large frontend half needs GPUI translation in `runner-native`.
- **M3.4** — input latch + native quit stamping: `f926581` (impl 0041), `c5b1ce4`.
- **M3.5+** — later slices decomposed when reached, in 0046 dogfood order: runtimes/model-effort pickers (0033/0034/0036/0043/0045), node sidebar polish + pinned section (Workstream B), router/inbox group, mission feed (0042/0044), pagination/update checks/misc.

## 2026-08-18 — M3.2: resume seams + terminal geometry

- Ported `9f53047` and `02e7502` into `runner-app`: resume now emits an in-band clean seam for keep-ring runtimes or RIS reset for purge runtimes before the child forks; terminal tracking covers bracketed paste plus mouse modes; every spawn resolves and persists a concrete PTY size; stopped-pane measurements persist without requiring a live handle; unknown-session resize still errors without creating manager state. Main's source manager tests were ported, and a `runner-terminal` regression proves alacritty applies both seam policies to the mounted native grid.
- Native frontend mapping: active stopped panes already stay mounted and call `TerminalSession::resize` from GPUI prepaint, so the backend liveness split is the stopped-geometry persistence trigger; manual resume already supplies the attached pane's current size. Main's invisible `PersistentSurfaces` behavior has no native counterpart yet because this line has no route-retained chat/mission surface, so there is no hidden cross-surface layer to resize.
- `cb32720`'s repo state machine and tests were already present line-for-line from M1. Its command-side claim finalization remains deferred because the native app does not yet expose launch auto-resume or quit-time resume stamping; there is no claim consumer to finalize in this slice.
- Ported `3f3bc04`'s structured `SessionUpdatedEvent` emission with `mission_id`. Its mission resume-all refresh/retry helper remains deferred because `runner-native` has no mission workspace surface; direct-chat resume already refreshes synchronously and through the session event bridge.
- Gates: `make verify` green; 432 `runner-app` tests plus the native seam/reset regression pass.

## 2026-08-18 — M3.3: resize-storm coalescing + owner-pane size gating

- Ported `38af26c` and follow-up `e78718f`: width changes for full-repaint runtimes now settle after 175 ms of quiescence, collapse a storm to one ioctl/purge, skip the purge and force a rows-nudge repaint when the storm round-trips, persist requested geometry immediately, and abandon stale work across kill/resume/respawn. A real width settle purges to one in-band `ESC[2J ESC[H` event emitted under the session-state lock immediately ahead of repaint bytes; failed ioctls leave the ring and cols gate untouched. M3.2's resume seams, concrete spawn sizing, and stopped-geometry persistence remain intact.
- Native frontend mapping: `TerminalElement` derives the grid from real GPUI prepaint bounds and refuses zero/unplaced geometry; `PaneLayout` selects exactly one resize owner for a session in a grouped pane tree; non-owners neither refit the shared `alacritty` grid nor push PTY geometry. Pure Rust tests translate main's pane-geometry and size-verdict coverage, and the manager test hunks cover storm collapse, round trips, failed settle, kill/respawn linearization, automatic settle, and live in-band clear emission.
- Deferred with reason: React's `PersistentSurfaces`, hidden pool, `visibility:hidden` mounts, transitional `resizeDisabled` wrapper, and `frontend_log` placement/suppression telemetry have no native mount counterpart. `runner-native` renders only the active tab tree; retained `TerminalSession`s are models, not hidden surfaces, and keep parsing at the last owner-consistent grid until active prepaint refits and pushes. New-chat terminals mount only after spawn, while resume retains the already placed owner grid; backend `resuming` linearization covers stale settles. The explicit grouped-pane owner gate remains as protection against duplicate persisted slots and for future surface expansion.
- Gates: `make verify` green (workspace check, 439 `runner-app` tests, all native/helper tests, clippy `-D warnings`, fmt-check); the 10-test terminal fixture corpus stays green. The first sandboxed run's Unix-socket bind test hit the expected `EPERM`; the identical permitted run passed.
