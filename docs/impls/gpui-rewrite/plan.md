# GPUI rewrite — program plan

Single program document for the Rust-native UI rewrite on the `gpui-nightly` line: strategy, standing decisions, workstreams, and the roadmap. Merged 2026-08-18 from impl 0031 (approach plan) and impl 0046 (main-parity catchup); those numbers remain the citation keys used in the [program log](impl_log.md). Tracking issue: [runner#307](https://github.com/yicheng47/runner/issues/307).

## Status

Phases 1–3 (spike, core extraction, walking skeleton) are complete — records in §Roadmap and the appendix. Of the working milestones, M0–M2 and M3.1 have shipped; M3 (feature-parity slices) is in progress as serial codex-peer missions, task numbering pinned in the log's 2026-08-18 breakdown entry. Then M4 (UI parity), M5 (sweep + watermark), Phase 5 (app-shell services), Phase 6 (cutover).

## End state

One native Rust binary. No webview, no JS, no Tauri. The UI is rendered by GPUI with `alacritty_terminal` as both the terminal model and the renderer's buffer — one parser, one grid, no serialization boundary. This dissolves the JS/Rust parity tax, the WebGL glyph-atlas bug class, and the WKWebView GPU-context limits by construction (#307).

"Drop Tauri" is the end state, not step one. The Tauri app on `main` currently provides, beyond the webview: app packaging/codesigning/notarization, the updater, native dialogs (folder picker), multi-window management + position restore, logging, process restart, and URL opening. Each needs a native replacement (Phase 5) before cutover; those trees are already absent from the pioneer branch.

## What we keep vs. rewrite

Keep (~33k LOC Rust, extracted into UI-agnostic crates):

- `crates/runner-backend/src/session/` — PTY session manager (`portable-pty`).
- `crates/runner-backend/src/router/`, `event_bus/`, `repo/`, `db.rs` (SQLite), `mcp/`, `model.rs`, `error.rs`.
- `crates/runner-core/`, `cli/` — untouched.
- The SQLite schema and NDJSON event-log format — the native app reads the same data; you can switch between old and new app against the same state.

Rewrite (~28k LOC TS/React):

- 7 pages (`Runners`, `RunnerDetail`, `RunnerChat`, `Crews`, `CrewEditor`, `MissionWorkspace`, `SettingsPage`), ~30 components + `settings/` + `ui/`, the xterm.js terminal, the layout/pane store, sidebar (projects / folders / tabs), modals, command palette.
- Design system: Tailwind tokens → theme constants in Rust. The `.pen` files remain the visual source of truth; Tokyo Night + light themes carry over as data, not CSS.

Adapt (the seam, built in Phase 2): the ~85 `#[tauri::command]` handlers became plain async fns over an app-core crate (`runner_app::ops`); event fanout became a `tokio::sync::broadcast` channel with each frontend as a subscriber.

## Framework: GPUI, via gpui-ce

Decided by the Phase 1 spike (GO memo in the appendix): GPUI over egui (IME/text-editing maturity concerns for a chat-input-heavy app with Chinese input) and Slint (a markup DSL adds a layer for no benefit here). Zed's terminal *is* `alacritty_terminal` rendered by GPUI — the architecture #307 describes, with production reference code. Runner is macOS-only and stays so; GPUI's macOS-first posture is not a cost.

Dependency line (decided 2026-08-17, landed as M0): `gpui = { package = "gpui-ce", version = "0.3" }`. Zed's crates.io `gpui` froze at 0.2.2 (2025-10); `gpui-ce` is the maintained community fork, explicitly drop-in, actively synced with Zed upstream, recommended by Zed developers, and validated in production by pulse. Start on crates.io releases; pin a `gpui-ce/gpui-ce` git rev only if a reference pattern needs newer API. Exit hatch: drop-in reversibility to mainline `gpui` or a future Zed publish is a one-line change.

## Branch strategy

Revised 2026-07-19 by Jason's decision, superseding the original fold-into-main-behind-a-second-binary plan: `gpui-nightly` is a **Tauri-free pioneer branch**. It shares the backend with the Tauri app on `main` and carries the native UI; the two frontends never coexist in one tree.

- **Phase 1 (spike)** lived on `phase1-terminal-spike` off the nightly branch. Messy, throwaway-friendly, rebase allowed while private.
- **Phase 2 (core extraction)** is commit `461356e` on `refactor/0031-app-core`. PR #310 reads merged, but `main` was reset afterward and does not contain the extraction; the Tauri app on `main` still owns the unextracted backend. Nightly composition sourced `crates/runner-app` by merging the extraction branch directly.
- **Nightly composition** landed through `phase3-walking-skeleton` after review and human verification: merged `origin/main` plus the Phase 2 extraction, then deleted `src/` (React), `src-tauri/`, and the JS toolchain.

Revised again 2026-07-19 (repo split): the pioneer line briefly moved to its own **runner-gpui** repository. Revised again 2026-08-17 (single repo, current state): the line moved back into the `runner` repo as the long-lived `gpui-nightly` branch, pushed at `774aa35` with the standalone repo's full history; the separate GitHub repo is retired, and locally the branch lives in the `runner-gpui` worktree. Rationale: most of the tree differs between the lines anyway, and the shared part is ported by hand either way — one repo keeps issues, CI, and history together without cross-repo remote juggling.

- **Cross-branch flow**: backend changes on `main` are ported onto `gpui-nightly` by conscious cherry-pick, hand-relocated from `src-tauri/src/*` into `crates/runner-backend/*` paths. Nothing flows automatically in either direction. Schema- and protocol-affecting changes (`db.rs` migrations, the `runner-core` event log, `cli` message shapes) port promptly — both apps must keep reading the same `runner.db` and event logs; feature logic ports when a parity slice needs it.
- **Milestone work** happens on task branches off `gpui-nightly`, merged back only after human verification.
- **Cutover (Phase 6)** is a branch promotion: `gpui-nightly`'s tree replaces `main` in this repo.

Build: the pioneer line has Rust-only `fmt`, `clippy`, `test`, and `run` Make targets; CI is one macOS Rust-workspace job (pulse pattern) for pushes and PRs targeting `gpui-nightly`, including the GPUI Metal Toolchain component. Native release packaging is deferred to Phase 5.

## Decisions

2026-08-17 (Jason):

1. **`main` owns the design.** The whole product design follows `main`, including the node-tree sidebar (Feature 44). The GPUI line does not fork product decisions.
2. **Repo-and-below is identical.** `crates/runner-core`, `cli/`, `db.rs` + migrations, and `repo/` must match `main` line-for-line (modulo file paths: `src-tauri/src/*` ↔ `crates/runner-backend/src/*`). Anything GPUI-specific lives in the adapter layer (`ops/`) or above — never in repo/db. Migration numbers are allocated on `main` only; this branch never adds its own.
3. **Product UI never changes; only the tech stack changes.** The GPUI app renders the same surfaces, flows, and copy as `main`'s current React app. Divergence is a bug, not a design opportunity.
4. **Terminal architecture mimics Zed's `terminal` / `terminal_view` crate split** (studied from the local checkout at `~/repos/gui/zed`, tree of 2026-08-17), but on **upstream `alacritty_terminal`** from crates.io — not Zed's fork.
5. **Framework: `gpui-ce`** (see §Framework).
6. **Updater (Phase 5): Sparkle via the pulse pattern** (see §Roadmap, Phase 5).
7. **Crate renames after the M3 session-hardening slice** (timing revised 2026-08-18; originally at Phase 6 cutover): `runner-app` → `runner-backend` (the UI-agnostic core the two frontends shared), `runner-native` → `runner-app` (the binary; pulse convention: `-app` = the application). Landed 2026-08-18 as one mechanical commit right after M3.4 (Cargo package names, imports, Makefile, doc references), gated on `make verify` — so M4's UI code, the bulk of new code in this program, is written under the final crate layout. From then on the port mapping is `src-tauri/src/*` ↔ `crates/runner-backend/src/*`, and every mission goal states it explicitly with a warning that `runner-app` now names the binary — the name reuse is the wrong-crate hazard that kept this at cutover originally.

2026-08-18 (Jason):

8. **Pulse is the window-level UI reference** (see §UI reference below).
9. **UI parity is its own milestone (M4)**, after the M3 feature slices — the catchup plan's backend-first slices deliberately defer surface/style parity; M4 restores the full Phase 4 surface breadth as an explicitly planned stage.
10. **Nightly channel before cutover.** After the crate renames, the native app ships as a separate nightly build: own bundle id (`com.wycstudios.runner.nightly`), own Sparkle feed, own GitHub release tags never referenced from the Tauri updater's manifest — so no Tauri user can be updated into it, and the production bundle id + minisign keypair stay reserved for the cutover bridge. It shares the production `runner.db` (guaranteed by decision 2) under a run-one-app-at-a-time discipline — two live instances would fight over the MCP socket, session PIDs, and the startup orphan sweep. Gate: the first nightly ships when M4 (UI parity) is complete — the daily-drive bar is the full product UI including missions; before that the native app cannot run Jason's real workflow. Packaging/notarization/Sparkle infrastructure is prepared earlier in parallel (validated with throwaway internal builds) so the nightly can ship the day M4 closes. Rollout: Jason daily-drives it for ~a month, with bug fixes and remaining work landing as Sparkle nightly updates — this starts the Phase 6 daily-drive clock, proves the updater repeatedly, and surfaces real bugs early; colleagues opt in during the month at their choice (the mission feed exists by then). This pulls Phase 5's packaging/notarization/Sparkle work forward in nightly form.

## Ground rules

- **License guardrail:** Zed's `terminal` and `terminal_view` crates are GPL-3.0. They are an architectural reference only — mirror the shape, responsibilities, and data flow; copy no code. (`gpui`/`gpui-ce` are Apache-2.0 and fine as dependencies.)
- **Port unit is `main`'s commit/impl doc, not the diff blob.** Each ported change cites its `main` commit; `main`'s impl docs 0033–0045 are the port guides. Reading the impl doc before porting is the difference between a port and a guess.
- **Seamless upgrade invariant:** a `runner.db` written by any Tauri release must open in the GPUI app and vice versa. This holds automatically iff decision 2 holds — the migration chain is linear and must stay that way.
- No commits without human verification per landing step; task branches off `gpui-nightly` as usual.

## Workstream A — Backend catchup (repo-and-below)

### A1. Protocol crates: take `main` wholesale

`crates/runner-core` and `cli/` live at the same paths on both branches and this branch has zero commits on them since the fork — `git checkout origin/main -- crates/runner-core cli` is the entire port. Drift being adopted: `cli/src/msg.rs` payload cap (`c3ea601`), `roster.rs` cleanup, `runner-core` event-log changes, extended roundtrip tests.

### A2. DB layer: migrations 0014–0020 + backfills, verbatim

Copy `main`'s `src-tauri/src/db.rs` and `src-tauri/migrations/` over `crates/runner-backend/`'s, adjusting only the module path prefix. New content: `0014_nodes.sql` (nodes table + 1:1 row copy from folders/tabs), `0015_retire_folders.sql` (drops legacy tables), `0016_session_last_size`, `0017_session_resume_on_launch`, `0018_slot_model_override`, `0019_session_agent_options`, `0020_slot_effort_override`, plus the Rust backfill steps (`backfill_0014_nodes`, `backfill_0015_retire_folders`) and the peer-coding-crew seed (`d04ce3f`).

### A3. Repo layer: adopt `main`'s files, retire ours

Port `main`'s `repo/*.rs` verbatim: `node.rs` arrives; `tab.rs` and `folder.rs` are deleted (their tables no longer exist at `user_version ≥ 15`); `crew.rs`, `mission.rs`, `runner.rs`, `session.rs`, `slot.rs`, `mod.rs` take `main`'s versions (pagination, model/effort overrides, node-scoped queries).

### A4. Session / router / event-bus / mcp: commit walk

Port grouped by theme, each group citing `main`'s impl doc:

| Group | `main` commits | Impl docs |
|---|---|---|
| Session hardening: process reaping, resume seams, geometry, resize-storm coalescing, input latch | `48d2231`, `0dadea7`, `9f53047`, `02e7502`, `38af26c`, `e78718f`, `f926581`, `3f3bc04`, `c5b1ce4`, `cb32720` | 0035, 0038, 0039, 0041 |
| Runtimes: executable discovery, Qoder, TRAE, model catalog, model/effort overrides, codex trust preseed | `d23e065`, `40e4bd1`, `cc97ab5`, `d4c082f`, `3f58c29`, `f3a4071`, `1b7ee92` | 0033, 0034, 0036, 0043, 0045 |
| Router/inbox: nudge deferral + queueing, inbox reconciliation, blocked delivery, channel semantics, synthetic wake routing | `828c9fc`, `75a0bda`, `f7137a2`, `8e245de`, `b8add00`, `e13e991` | — |
| Mission feed: read-mostly feed, human channel composer, archive visibility path | `65ee46c`, `670a168`, `7157efc` | 0042, 0044 |
| Misc: pagination backend (`0cff744`), periodic update checks (`30bdbf6`), macOS wake bridge (`de53950`), stale unread cleanup (`328da9c`), paste file paths (`a8e7cf3`) | | 0040 |

Tauri-only material in these commits (webview emit sites, `commands/` wrappers, JS) is dropped at the `ops/` seam; the backend body ports verbatim.

## Workstream B — Node-tree sidebar (the UI consequence of A2/A3)

Feature 44 on `main` replaced `folders` + `tabs` + pin flags with one `nodes` tree (`folder` | `project` | `tab` | `mission`, `parent_id` + `position`, `pinned_position` for the pinned section). This branch's Phase 4 direct-chats sidebar was built on the retired model and must be rebased onto nodes, matching `main`'s current sidebar exactly: node tree with projects/folders, pinned section (`a172ca6`), tab attention watermarks carried on the node row, drag-reorder/rename/archive semantics as `main` ships them. This is UI work, but it is parity work — the target rendering is `main`'s sidebar, not a new design.

## Workstream C — Terminal split (`runner-terminal` / view side)

Mirror Zed's two-crate shape (reference: `~/repos/gui/zed/crates/terminal` and `crates/terminal_view`), adapted to Runner's ownership model:

| Zed | Responsibility | Runner target |
|---|---|---|
| `terminal/src/terminal.rs` | UI-agnostic model: `Term` behind `FairMutex`, VTE `Processor`, internal event queue, `last_content` renderable snapshot, selection/scroll state, search | `crates/runner-terminal/src/terminal.rs` |
| `terminal/src/alacritty.rs` + `alacritty/hyperlinks.rs` | isolation of `alacritty_terminal` API surface: grid iteration, selection types, event glue | `runner-terminal/src/alacritty.rs` (deferred until the view side needs it) |
| `terminal/src/mappings/{keys,mouse,colors}.rs` | keystroke→escape encoding, mouse reports, color conversion | `runner-terminal/src/mappings.rs` |
| `terminal/src/pty_info.rs` | foreground-process info | **not ported** — process identity is `SessionManager`'s domain |
| `terminal_view/src/terminal_element.rs` | GPUI `Element`: layout → batched text runs/rects/cursor, paint, IME `InputHandler` | stays in `runner-app/src/terminal_element.rs` (keeps the per-cell-origin alignment strategy) |
| `terminal_view/src/terminal_view.rs` | focus, key dispatch, blink, context menu | `runner-app` chat/pane views |
| `terminal_view/src/{terminal_panel,persistence,scrollbar,path_like_target}.rs` | workspace integration | `runner-app` pane layout / future slices |

**Deliberate deviations from Zed:**

- **The model crate does not own the PTY.** Zed's `Terminal` runs alacritty's `event_loop` + `tty` and owns the child process. In Runner, `SessionManager` (in `runner-app`, on `portable-pty`) owns spawn/resume/kill/resize because sessions outlive views and are mission-coordinated. `runner-terminal` stays a pure byte-consumer: `SessionManager` output events in, renderable content + encoded input bytes out. This keeps the fixture corpus a pure model-level replay harness.
- **Upstream `alacritty_terminal`, not Zed's fork.** Zed pins `zed-industries/alacritty`; we stay on crates.io. Upstream is the contract; if we hit something Zed's fork patches, we work around it inside `runner-terminal` rather than adopting a fork.
- The fixture corpus (`fixtures.rs`, `replay.rs`) lives in `runner-terminal` as its test suite — the corpus is a property of the model, not the renderer.

## UI reference: pulse (2026-08-18, Jason)

For M3+ native UI work, `~/repos/yicheng47/pulse` (our own pure gpui-ce app, in production) is the reference for window-level operations — hidden-native-titlebar setup (`TitlebarOptions`, traffic-light positioning in `pulse-app/src/main.rs`), titlebar drag areas and double-click zoom (`shell.rs`), app menus (`menu.rs`), and window-bounds handling. Its `components.rs`, `text_input.rs`, `theme.rs`, and settings surfaces are candidate reusable components/patterns. Check pulse before hand-rolling any window-level UI; it already anchors the Phase 5 Sparkle updater decision. Zed remains the reference for terminal/editor-grade patterns (architecture only — GPL guardrail above); pulse is the reference we can copy from directly.

## Roadmap

### Phases 1–3 — done

- **Phase 1 — terminal spike + framework decision** (2026-07-18/19): GO on GPUI; decision memo with the evidence table in the appendix. The terminal fixture corpus (real PTY byte logs replayed into grid snapshots) dates from here and gates every later terminal change.
- **Phase 2 — app-core extraction** (2026-07-19): `crates/runner-app` — command bodies as plain fns over `AppCore`, broadcast-channel event fanout. Lives on `refactor/0031-app-core` (`461356e`), consumed by this branch; see §Branch strategy for why it is absent from `main`.
- **Phase 3 — walking skeleton** (2026-07-19): `crates/runner-native` boots the app core against the same SQLite DB, lists direct chats, opens live terminals with an IME-capable composer. Composer IME validated here; terminal-pane IME deferred (now M4 scope, cutover blocker).

### Working milestones

Each milestone is a task branch off `gpui-nightly`, human-verified before merge. M numbering was introduced by impl 0046; M4/M5 were restructured 2026-08-18 (decision 9).

1. **M0 — framework swap** ✓ (2026-08-17): `gpui` → `gpui-ce 0.3`; fixture corpus green; no behavior change.
2. **M1 — repo-and-below parity** ✓ (2026-08-17; A1–A3 + minimal B): protocol crates wholesale; db + migrations + repo verbatim; `ops/` adapter updated; sidebar rewired onto nodes far enough to compile and function. Gate: migration test on a *copy* of a production `runner.db`, then the A/B switch test — Tauri `main` build and GPUI build alternately opening the same copy.
3. **M2 — terminal split** ✓ (2026-08-17; C): extract `runner-terminal`, consolidate input encoding into `mappings`, move the fixture corpus. No rendering change; corpus green.
4. **M3 — feature-parity slices** (in progress; A4 + B function, dogfood order): session hardening → runtimes/model-effort pickers → node sidebar polish + pinned section → mission feed (read-mostly + channel composer) → pagination/update checks/misc. Run as serial codex-peer missions; task numbering (M3.1, M3.2, …) pinned in the [program log](impl_log.md)'s 2026-08-18 breakdown entry. Each slice daily-driven before the next.
5. **M4 — UI parity** (new 2026-08-18): bring the native app to full product-UI parity with `main` — this restores the surface breadth of the original Phase 4 list that M3's backend-first slices deliberately defer. Entry task is a **surface inventory**: walk `main`'s `src/` (7 pages, ~30 components + `settings/` + `ui/`) and the `design/*.pen` files, classify every surface as present / unstyled / missing in `runner-app` (the binary crate), and pin the resulting task list in the program log (continuing the serial-mission numbering). Known scope: chat surface + composer styling; tab bar and layout picker; sidebar visual polish beyond M3's functional parity; mission workspace UI; runners/crews CRUD pages; settings; modals and dialogs; command palette; themes as data (Tokyo Night + light); window chrome per §UI reference (hidden titlebar, traffic lights, drag areas — pulse pattern); multi-window; and **terminal-pane IME** (marked-text composition and candidate-window positioning in the focused terminal, committed UTF-8 forwarded through `SessionManager`, raw handling preserved for control/navigation/function keys — a cutover blocker carried from Phase 3). Exit gate: side-by-side parity review against the Tauri app on every surface — same surface, same flow, same copy — and the native app is the daily driver for all workflows, not just chats.
6. **M5 — sweep + watermark** (was 0046's M4): diff `crates/runner-backend` against `main`'s `src-tauri/src` module-by-module; the diff should be adapter-shaped only. Record the synced `main` SHA in this doc as the watermark; subsequent `main` backend commits port promptly (schema/protocol) or per-slice (features).

### Phase 5 — App-shell services (replace what Tauri gave for free)

- Packaging: .app assembly, codesign, notarize, staple, DMG — scripted (`cargo-bundle` or hand-rolled; validated in Phase 1's criterion 5).
- Updater: replace `tauri-plugin-updater` with **Sparkle, via the pulse pattern** (decided 2026-08-17). Pulse is the in-house reference: `SPUStandardUpdaterController` through ~130 lines of `objc2` bindings behind an `updater` feature with a no-op dev fallback (`crates/pulse-app/src/updater.rs`), `Sparkle.framework` embedded by the bundle script (pinned version, SHA-256 checked), appcast served from GitHub Releases (`SUFeedURL` → `releases/latest/download/appcast.xml`, EdDSA key in `SUPublicEDKey`). Zed's hand-rolled `auto_update` (~2k lines) was considered and rejected — it earns its complexity from multi-platform + own server infra, neither of which applies here. One-time bridge at cutover: the last Tauri release updates users into the first native release through `tauri-plugin-updater`'s artifact format (same minisign keypair, same bundle id, higher version); Sparkle owns every update after that. This is real work; do not leave it for cutover week.
- Dialogs → `rfd`; opener → `open` crate; logging → `tracing` + file layer (keep the crash/panic hook); window position restore → reimplement `window_state.rs` on the native windowing layer.
- CI: nightly build + release artifacts for the native binary alongside the existing app.

### Phase 6 — Cutover

Criteria: 2+ weeks daily-driving the native app exclusively, M3–M5 done, updater proven by shipping at least one native-to-native update, fixture corpus green. Then: `gpui-nightly`'s tree replaces `main` (branch promotion), the native binary becomes `Runner.app`, major version bump. (The crate renames of decision 7 land earlier, after the M3 session-hardening slice.)

## Verification

- Fixture corpus replay green at every milestone (the terminal regression floor).
- `make verify` (check + workspace tests + clippy + fmt-check) per milestone.
- Migration + A/B switch test as in M1 — the seamless-upgrade proof, repeated whenever a migration ports.
- Parity review per M3/M4 slice against the running Tauri app side-by-side: same surface, same behavior, same copy.

## Risks

- **Parity treadmill** — the killer risk: `main` keeps moving while this line chases it (44 backend commits in the first month alone). Containment: the M5 watermark, the schema/protocol-ports-promptly rule, dogfood-ordered slices (pressure to finish is intrinsic), and a soft feature-freeze on new frontend surface on `main`. The real fix remains finishing the milestones.
- **Scale honesty**: this replaces 28k LOC of UI. Solo with agent crews, expect months of part-time sessions, not weeks. The milestone gates exist so the project survives motivation dips — every stage ends with something usable.
- **`gpui-ce` is community-run.** Mitigated by drop-in reversibility, its upstream-tracking discipline, and the M0 gate having kept the swap isolated.
- **GPUI docs are thin**; the vendored source, bundled examples, Zed's tree, and pulse are the actual references.
- **Packaging/notarization first-time cost**: codesign of a hand-rolled bundle validated in Phase 1; notarization deferred to Phase 5 by explicit decision.
- **Accessibility/native-behavior gaps** (VoiceOver, standard text-editing shortcuts) are real regressions vs. a webview; accepted for a personal-IDE product, noted for honesty.

## Non-goals

- Windows/Linux support (premature for a personal IDE; the GPUI choice reflects this).
- Any product/UX change relative to `main` — parity only; divergence is recorded as an issue, not shipped.
- Rewriting the backend, CLI, event-log format, or SQLite schema — frozen interfaces for this effort; migration numbers are allocated on `main` only.
- Adopting Zed's alacritty fork, or copying GPL code from Zed's terminal crates.
- Porting Tauri app-shell services (updater, packaging, dialogs) inside the parity milestones — that is Phase 5.

## Appendix — Phase 1 decision memo (2026-07-18; finalized 2026-07-19)

**Decision: GO on GPUI.** Human verification: criterion 3 (Pinyin IME in the native composer) confirmed explicitly; criteria 1/4 confirmed through live daily-style use with no tearing, lag, or reflow artifacts reported; criterion 2 machine-verified at the grid level with per-cell-origin rendering making fallback-advance skew structurally impossible. Criterion 5: .app assembly + Developer ID codesign proven; the notarize/staple run is **deferred to Phase 5 by explicit decision** — the failure-prone half (hand-rolled bundle of a bare cargo binary passing codesign with hardened runtime) is validated, notarization is bundle-agnostic, and the same credential flow already ships Runner's Tauri releases via CI.

Spike built on `phase1-terminal-spike` as `crates/native-spike/` (GPUI 0.2.2 + alacritty_terminal 0.26 + portable-pty, ~1.4k LOC). Verification state per exit criterion:

| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | Busy-output rendering | needs eyes | Live claude session renders; coalesced wakeups (PTY thread → channel → one notify/frame). Human: watch a chatty claude run for tearing/lag. |
| 2 | CJK/emoji/box-drawing width | machine-green, needs eyes | Replay tests assert WIDE_CHAR/spacer/zerowidth/leading-spacer cell semantics; renderer paints wide + non-ASCII glyphs at per-cell column origins so fallback advances can't skew the grid. Human: run `width-torture.sh` in the spike window. |
| 3 | IME in composer | needs eyes | Full `EntityInputHandler` (marked-range composition, candidate-window anchoring via `bounds_for_range`, Enter-guard during composition). Human: Pinyin typing test. |
| 4 | Resize reflow | machine-green, needs eyes | PTY + Term resize wired to element layout; tests pin alacritty's reflow semantics incl. cursor-line pinning and scrollback push (#308 class). Human: violent resize during live output. |
| 5 | .app + codesign + notarize | 2/3 done | `package.sh`: release build → .app assembly → Developer ID codesign validates. Notarize/staple scripted, needs credentials (NOTARY_PROFILE or APPLE_ID trio). |

Fixture corpus (spec 42 debt) exists and gates regressions: real claude-code interactive session, busy alt-screen `top`, glyph width-torture — recorded as raw PTY byte logs, replayed into the grid, snapshot-compared. Every later terminal change replays these.

Findings for the framework decision:

- GPUI worked as advertised: the terminal element (grid → quads + shaped runs) is ~350 lines; IME needed no platform code, just the input-handler protocol. API-churn risk is real (crates.io docs thin; the vendored source + bundled examples were the actual reference) but never blocking.
- One environment cost discovered: gpui's Metal shader build requires the Xcode 26 Metal Toolchain component (one-time ~3 GB download). Recorded in the spike README; CI runners need it.
- alacritty_terminal ships no default color palette; the embedder owns the 256-color table (done in `palette.rs`, Tokyo Night base 16). Its `term::test` module (public at runtime) gives `TermSize`/`mock_term` — useful beyond tests.
- No serialization boundary exists anywhere in the pipeline: reader thread locks the Term, advances the parser, UI paints from the same grid under the same lock. The #307 architecture holds.

Deviation from the original Phase 1 wire description ("spawn via the existing session manager"): the spike spawned through its own minimal portable-pty lifecycle, per the mission brief's explicit allowance. The spike therefore validated rendering/input/packaging risks only; the session-manager/app-core integration seam was validated in Phases 2–3, when the walking skeleton adopted `SessionManager` as the only spawn path.
