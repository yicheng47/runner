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

- 7 pages (`Runners`, `RunnerDetail`, `RunnerChat`, `Crews`, `CrewEditor`, `MissionWorkspace`, `SettingsPage`), ~30 components + `settings/` + `ui/`, the xterm.js terminal, the layout/pane store, sidebar (projects / tabs / missions), modals, command palette.
- Design system: Tailwind tokens → theme constants in Rust, read from `main`'s shipped `src/index.css` and `src/lib/settings.ts`. `main`'s four shipped app variants (Runner/Carbon, Catppuccin Mocha, Codex Light, Catppuccin Latte) and three terminal palettes carry over as data, not CSS.

Adapt (the seam, built in Phase 2): the ~85 `#[tauri::command]` handlers became plain async fns over the UI-agnostic core (`runner_backend::ops` after decision 7's crate rename); event fanout became a `tokio::sync::broadcast` channel with each frontend as a subscriber.

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

1. **`main`'s shipped React code is the source of truth; `design/runner.pen` is a reference only** (tightened 2026-08-18, Jason; supersedes the earlier "the `.pen` files are the visual source of truth where a matching frame exists" framing). The whole product design follows `main`, including the node-tree sidebar (Feature 44), and `src/` wins on *every* axis — structure, flow, copy, and visual treatment (layout, spacing, sizing, colors, tokens, states). Pencil stays useful for design intent, naming, and as a sanity check on what a surface is meant to be, but it never overrides shipped code and a Pencil-only detail is never grounds to add or change a surface. Design evolution therefore flows design → `main` → port, never design → this line directly. The GPUI line does not fork product decisions.
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

2026-08-20 (Jason):

11. **`main` is feature-frozen; one line of work.** No new features land on `main` and nothing is ported back to it — Jason will not carry two versions. The rewrite finishes (M4 → M5), `gpui-nightly` is promoted (Phase 6), and work beyond parity (M6) starts on the one remaining line. Decision 2's "migration numbers are allocated on `main` only" ends here: new migrations are allocated on `gpui-nightly` and deferred to cutover unless the M1 A/B gate is re-run; the M5 watermark is the last point at which `runner-backend` and `src-tauri/src` are expected to be adapter-shaped diffs of each other. The "soft feature-freeze on new frontend surface on `main`" under §Risks is now a hard freeze on all of `main`.

2026-08-21 (Jason):

12. **Build stamps, release channels, and the cutover bridge.** `CFBundleVersion` is always a build stamp (`YYYYMMDD.HHMM`, UTC) on both channels, permanently; `CFBundleShortVersionString` carries the marketing semver (`0.6.0-nightly.<stamp>` on nightly, `0.6.0` on production); the crate version stays `0.6.0-nightly` untouched until the one hand bump to `0.6.0` at cutover. Nightlies publish to one rolling `nightly` GitHub **prerelease** with its own feed URL — never the `releases/latest` alias, which `main`'s Tauri updater polls — and, since the evening of 2026-08-21, are cut **on demand only** (`workflow_dispatch`; a push runs CI alone), so a build is a deliberate act and never lands mid-mission by accident. One Sparkle EdDSA keypair serves both feeds. Cutover is a single release, `v0.6.0`, carrying both the Sparkle artifacts and the Tauri-format bridge trio; the signed tarball lives on `v0.6.0` only, and every later 0.6.x production release re-uploads just the 1 KB `latest.json` still pointing at `v0.6.0`'s tarball and signature (Jason, 2026-08-22) — so a dormant 0.5.x install takes two hops, Tauri → 0.6.0 → Sparkle to latest, at no per-release cost beyond copying one file. **Hard cutoff at `0.7.0`**: no `latest.json` from then on. Mechanics and the GA runbook: §Release channels and the cutover bridge (under Phase 5). Refines decision 10: "own GitHub release tags never referenced from the Tauri updater's manifest" is not the isolation — the manifest URL is a dynamic alias; the prerelease flag and the feed URL are.
13. **M5 is closed; parity is recorded, not re-swept.** `origin/main` has not moved since `276a3a4` (2026-08-20, the merge of #424) — under decision 11 it never will — so the backend watermark and `main`'s HEAD are the same commit. Parity was established layer by layer and verified as it landed: repo-and-below byte-identical at M1 (`1b7ee92`), every backend group ported with `main`'s own test suites through M3.8 (router/event-bus files byte-identical apart from the seams), UI surfaces compared against the shipped React app mission by mission and in Jason's daily drives. A further module-by-module sweep would re-verify the same commit; Jason's judgment (2026-08-21) is that the two apps behave the same. Final watermark: **`276a3a4`**. The deviation register in `impl_log.md` is the authoritative list of where the lines intentionally differ.

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

Feature 44 on `main` unified the old sidebar records in one `nodes` tree; follow-up migration `0015_retire_folders.sql` and commit `031eee5` retired its folder containers. The current tree is `project` | `tab` | `mission` with `parent_id` + `position` and `pinned_position` for the pinned section. Projects are the only containers; tab and mission leaves may sit at root or under a project. This branch's Phase 4 direct-chats sidebar was built on the retired model and must be rebased onto nodes, matching `main`'s current sidebar exactly: project tree, pinned section (`a172ca6`), tab attention watermarks carried on the node row, drag-reorder/rename/archive semantics as `main` ships them. This is UI work, but it is parity work — the target rendering is `main`'s sidebar, not a new design.

## Workstream C — Terminal split (`runner-terminal` / view side)

Mirror Zed's two-crate shape (reference: `~/repos/gui/zed/crates/terminal` and `crates/terminal_view`), adapted to Runner's ownership model:

| Zed | Responsibility | Runner target |
|---|---|---|
| `terminal/src/terminal.rs` | UI-agnostic model: `Term` behind `FairMutex`, VTE `Processor`, internal event queue, `last_content` renderable snapshot, selection/scroll state, search | `crates/runner-terminal/src/terminal.rs` |
| `terminal/src/alacritty.rs` + `alacritty/hyperlinks.rs` | isolation of `alacritty_terminal` API surface: grid iteration, selection types, event glue | `runner-terminal/src/alacritty.rs` (deferred until the view side needs it) |
| `terminal/src/mappings/{keys,mouse,colors}.rs` | keystroke→escape encoding, mouse reports, color conversion | `runner-terminal/src/mappings.rs` |
| `terminal/src/pty_info.rs` | foreground-process info | **not ported** — process identity is `SessionManager`'s domain |
| `terminal_view/src/terminal_element.rs` | GPUI `Element`: layout → batched text runs/rects/cursor, paint, IME `InputHandler` | stays in `runner-app/src/terminal/element.rs` (keeps the per-cell-origin alignment strategy) |
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
4. **M3 — backend parity slices** (in progress; re-scoped 2026-08-18, Jason: M3 is backend-only for a clean M3/M4 cut — the UI halves moved to M4): session hardening ✓ → runtimes ✓ → start-chat modal + pane controls ✓ (M4 pull-forward; the one UI task closing the runtime slice) → router/inbox ✓ → mission-feed backend (0042/0044 backend halves) ✓ → misc backend (macOS wake bridge `de53950`, stale unread cleanup `328da9c`, paste-file-paths op `a8e7cf3`; pagination landed inside M4.4, and the periodic update checks `30bdbf6` are deferred out of M3 to the decision-10 nightly work — porting the Tauri-updater check loop would be discarded when Sparkle replaces it; re-scoped 2026-08-20, Jason). Serial codex-peer missions; task numbering (M3.1, M3.2, …) pinned in the [program log](impl_log.md)'s 2026-08-18 breakdown entry. Backend groups are verified by their ported test suites; their observable daily-driven verification arrives with the M4 surfaces — an accepted deviation from the per-slice dogfood rule.
5. **M4 — UI rebuild to parity** (new 2026-08-18; inventory and stages refined 2026-08-18): the native UI today is the Phase 3 walking-skeleton look end to end — layout, spacing, colors, chrome, and copy all deviate from the Tauri app, and most product surfaces do not exist at all. M4 is therefore a **rebuild of the UI to `main`'s design**, not a polish pass: nothing currently rendered is presumed to survive as-is. The entry audit is [m4-surface-inventory.md](m4-surface-inventory.md): 75 parity surfaces, 3 functional-but-unstyled, 22 partial, 50 missing. The evidence swaps the old M4.4/M4.5 scopes so lower-complexity entity pages mature the shared list/form/editor primitives and Crew Editor launch path before the mission workspace consumes them. Stages, in order, each a serial mission run daily-driven:
   - **M4.1 — App shell** (first, everything renders inside it):
     1. Replace the visible “Runner Native” system titlebar with `main`'s owned chrome: hidden titlebar, correctly positioned traffic lights, leading sidebar toggle, drag regions, double-click zoom, fullscreen treatment, and the current app menu/key actions, using pulse's `main.rs`, `shell.rs`, and `menu.rs` patterns.
     2. Introduce the shared theme roles from `runner.pen` and `main`'s `index.css`; port Auto/Light/Dark resolution, Runner/Carbon, Catppuccin Mocha, Codex Light, Catppuccin Latte, app fonts, app zoom, base typography/spacing, focus rings, and global scrollbars. Remove Tokyo Night as native product chrome rather than preserving a fifth divergent theme.
     3. Build `main`'s route-capable frame and header zones, including the resizable/collapsible sidebar slot, hover preview, persistent page/surface layer, settings takeover layer, global toast host, and loading/error boundary.
     4. Reuse the existing Runner icon and mark in the shell; leave Dock/bundle resource wiring to the decision-10 nightly packaging task in Phase 5.
   - **M4.2 — Shared widget set**:
     1. Establish shared Button, icon-button, card, pill/badge, status, focus, disabled, destructive, and loading treatments from `main` and Pencil's reusable components.
     2. Generalize the Start Chat IME field into Field/Label/Input/Textarea/Error without regressing marked text, candidate placement, selection, clipboard, or Enter-during-composition behavior.
     3. Port SearchInput, StyledSelect, RuntimeSelect, ModelField, WorkingDirField, Toggle, settings Stepper, and their keyboard/validation/swatch states over the shared runtime catalog.
     4. Port Modal, Drawer, ConfirmDialog, PopoverMenu, Tooltip, CopyValueButton, RunnerAvatar/presence, and SessionControl.
     5. Port PaginatedListPage primitives, EmptyStateCard, `cmp/Pager` windowing, app/terminal scrollbars, and reusable settings headers/cards/rows.
   - **M4.3 — Direct chat and sidebar**:
     1. Replace the flat direct-tab list with `main`'s exact node sidebar: Workspace entries, search/brand trigger, pinned section, project containers, root chats/missions, create menu, Settings/update footer, inline rename, context menus, drag/reparent/reorder, archive, and project-scoped starts. The only node types are project, tab, and mission.
     2. Port single-tab rows, multi-pane accordion groups, active pane treatment, working/unread attention priority, collapsed-container rollups, and viewed-watermark behavior.
     3. Rebuild Runner Chat's topbar, metadata/status, warning/error states, pin/rename/archive/project and stop/resume group actions, and resizable runner/runtime side panel.
     4. Keep the tested six-layout/focus/resize model, but rebuild pane headers, gutters, empty panes, layout picker, per-pane menus, and Starting/Resuming/Stopped/Archiving overlays to exact `main` rendering and copy.
     5. Close Start Chat's remaining parity gaps: shared widgets, project scope and cwd precedence, persisted default agent/default cwd, empty-agent Settings route, and exact copy.
     6. Close terminal-view parity: current terminal theme/font/size/cursor/scrollback settings, Cmd-click URLs, image and copied-file-path paste, wake/refit behavior, focus/visibility rules, and retained surfaces.
   - **M4.4 — Runners, crews, and project entry points** (moved ahead of missions to mature shared list/form/editor primitives before the most compositional workspace):
     1. Consume the M3 paginated runner/crew operations and land the shared list-page loading/error/empty/no-match/search/count/pager contract.
     2. Port Runners list cards, create modal, edit/delete actions, Runner Detail, memberships/activity/immutable details, and Chat now.
     3. Port Crews list cards and create/delete flows, then Crew Editor's inline fields, team conventions, default goal, and Start Mission entry.
     4. Port Add/Edit Slot, runtime/model/effort layering, Set lead, Edit runner, Remove, and slot drag ordering.
     5. Port Start Project and all project sidebar context actions. Do not create a Projects page or folder UI; neither exists on `main`.
   - **M4.5 — Mission surfaces** (after the remaining M3 router/inbox and mission-feed backend):
     1. Port mission load/attach/error/warning state and the workspace topbar, pin/rename/reset/archive/stop/resume controls, feed/slot tab strip, last-terminal behavior, and mission-wide paused semantics.
     2. Port slot terminals with Starting/Resuming/Stopped/Archiving precedence, duplicate-subject handling, and the inbox-blocked pill over M3's delivery events.
     3. Port the resizable Runners rail and Mission metadata panel, including presence, lead/session metadata, copy, crew link, cwd reveal, and panel toggles.
     4. Port EventFeed's replay/live merge, grouping, avatar/target/time/goal treatment, Markdown bodies, signals, AskHumanCard, mission divider, new-message pill, and autoscroll.
     5. Port MissionInput's IME textarea, channel/direct messages, roster `@` picker, target chip, and send semantics over M3's human-message operation.
     6. Port Start Mission and Reset confirmation with project/default cwd, launchability/session count, grid sizing, exact copy, and current inert Advanced disclosure.
   - **M4.6 — Settings, command palette, and multi-window** (mission split a–f pinned in the [program log](impl_log.md)'s 2026-08-20 entry):
     1. Port the full-window settings takeover, resizable/searchable nav, Back to app, direct routing, and `main`'s ten panes; use `cmp/SettingsNav` styling without copying its stale Chat/omitted Agents/Updates information architecture.
     2. Port General, Appearance, and Terminal with native persistence and live application, including launch auto-resume only after M3's queue consumer is complete.
     3. Port Keyboard Shortcuts as a real action registry with search, record/rebind, conflict handling, unbind, restore/reset, and fixed bindings.
     4. Port Agents and MCP over the ready runtime/MCP operations, then Archived over the ready mission/session restore/delete operations.
     5. Pull the needed Phase 5 services forward, then port Updates/update prompt over Sparkle, Diagnostics over native file logging/reveal, About links over the native opener, and their toast/error states; do not ship inert parity shells. **Runs as one mission, M4.6f (decided 2026-08-21, Jason; the a/b split was folded back together):** the Sparkle updater behind an `updater` feature (pulse's `updater.rs` pattern), the bundle script and build-stamp contract, the `nightly.yml` workflow and rolling `nightly` prerelease feed — all per §Release channels and the cutover bridge — and then the three panes over those services — the Updates pane slim (version, Check, automatic-check toggle, last checked) with Sparkle's standard sheet owning the update flow instead of `main`'s in-pane state machine and prompt card (decision-1 deviation, Jason 2026-08-21, in the register). Exit: a `workflow_dispatch` nightly installs on Jason's machine and a second, higher-stamped run arrives through Sparkle's Updates pane; Diagnostics reveals the real log file; About's links open.
     6. Port Command Palette search/grouping/keyboard navigation for chats, missions, runners, crews, and Settings.
     7. Add native window open/focus/route bootstrap/subject reporting over the existing backend registry, then DuplicateSubjectOverlay and position/state restore. The window adapter and restore service remain Phase 5 prerequisites even though their product surfaces close here.

   **Shell split before tasks 3–7 (inserted 2026-08-20, Jason; runs as M4.6b, palette becomes M4.6c).** After M4.6a the binary is one `NativeRoot` entity with ~70 fields and a single `Render`; every surface under `src/surfaces/` is an `impl NativeRoot` block, so any `cx.notify()` re-renders the whole window and cross-surface coupling is unchecked. Multi-window (task 7) cannot share a window-scoped root, so the split is a prerequisite, not a cleanup: (a) hoist app-scoped data — `AppCore`, terminal bridge, runners/nodes/projects/missions/sessions, session activity, settings — out of `NativeRoot` into one store that surfaces observe; (b) promote the two heaviest surfaces, mission workspace and sidebar, to their own entities with local state and `Render`, leaving `NativeRoot` as the shell (route, tabs, focus, modals, child handles). Zero visual or behavioral change is the bar; the remaining surfaces stay as `impl` blocks until one hurts. The 2026-08-20 directory tidy (`src/surfaces/`, `src/terminal/`) is the mechanical precursor and changed no code.

   - **M4.7 — Terminal selection and copy** (added 2026-08-20 from the technical audit): xterm.js has mouse selection and ⌘C natively; the GPUI terminal has neither — no mouse-down handler on the element, zero use of alacritty's selection API, and `Copy` routes only to `ui/field.rs`. Parity gap and daily-driver blocker: viewport→grid hit-testing, drag selection with autoscroll, selection quads in prepaint, `Copy` in the `Terminal` key context, double/triple-click word/line selection as `main` has. After M4.6b (it touches the pane and workspace files the shell split churns). Landed 2026-08-21 (`e836222`).

   - **M4.8 — Parity fix-ups and the inventory re-audit** (added 2026-08-21, Jason; runs after M4.6f and *is* the M4 exit gate): one mission that closes every small parity gap the log has accumulated as "open nits" / "deferred", then re-audits [m4-surface-inventory.md](m4-surface-inventory.md) row by row against the native app — the status column was last audited at M4 entry — flipping each row to done or recording it as a deviation in the log's register. Known items going in: ⌘-click URL opening in terminals (inventory B8; `open_url` exists, port `main`'s link detection and ⌘-hover treatment); ⌥+letter chords sending ESC plus the accented character so readline M-b/M-f/M-d are broken (`key` over `key_char` when alt is set — `main` has the same bug, so this is a recorded improvement, not parity); the fixed Copy row rendering among rebindable rows in the Shortcuts pane (present it as the fixed New window row is); sidebar drag-handle tooltip and kebab opacity reveal (M4.4); MCP binding-dir tooltip, locale-aware timestamps in Archived, Enter not blurring the Agents override field (M4.6d); the width-chain property that a mission with the Feed tab active forks from the estimate instead of a cached slot measurement (cache the last measurement per mission); the column-select crosshair cursor (port only if cheap, else record). Whatever the re-audit adds joins the list; anything deliberately left becomes a register entry. Backend-side carries (M3.7 `to` validation, the 32 KB human-channel cap, the inert `app/woke`) go to M6.3, not here. **Landed 2026-08-21 (`bfc9eda`); exit gate passed — inventory 75 done / 0 partial / 0 missing. M4 is complete.**

   (Terminal-pane IME, originally M4 scope and a cutover blocker, was pulled forward and shipped 2026-08-18; the start-chat modal + pane controls likewise ran ahead as the UI task closing the M3.5 runtime slice.) Exit gate (made concrete 2026-08-21): M4.8's inventory re-audit is the side-by-side review — every row flipped to done or recorded as a deviation — and the native app is Jason's daily driver for all workflows, not just chats.
6. **M5 — sweep + watermark** (was 0046's M4): **closed 2026-08-21 by decision 13** — `main` froze at `276a3a4`, the backend watermark; the adapter-shaped diff was verified per group as M1 and M3.1–M3.8 landed, so no separate sweep runs. The deviation register in `impl_log.md` stands in for the sweep's output.
7. **M6 — consolidation** (added 2026-08-20): the work beyond parity — verified defects and structural debt from the 2026-08-20 technical audit, plus hook-based session status (#347, reopened 2026-08-20; Jason reversed the 2026-08-01 won't-do: a reliable status detector needs the agents' own turn signals, the byte-flow `IdleDetector` stays only as the fallback tier). Plan: [m6-consolidation.md](m6-consolidation.md); run order (Jason, 2026-08-21): M6.6 terminal resize smoothness first (small; **landed 2026-08-21, `6f020d4`**), M6.8 long-lived terminals (**landed 2026-08-21, `3536c70`**), M6.5 `arch.md` rewrite (**landed 2026-08-22**), M6.10 sidebar overflow + M6.11 first-paint pill (**landed 2026-08-22**), then M6.9 update hint + mission-reset removal (**landed 2026-08-22, `518a7bf`**), the M6.3 mutex item (**landed 2026-08-22, `ba33bd7`**), M6.13 feed text fidelity + the M6.12 scroll-container audit (**landed 2026-08-22, `5b974ed` → `7b12ced`**), M6.1 real input-state tracking (**landed 2026-08-22, `9deca7c`**), M6.15 ⌘1–9 tab switching with hold-to-reveal pills (feature 63, design first; M6.14 settings file, feature 62, dropped 2026-08-22 — already a JSON file nobody hand-edits) and M6.16 descendant sweep by env tag at session stop/startup (added 2026-08-22 after a crew's orphaned stress loops starved the host), M6.17 universal Apple Silicon + Intel builds (added 2026-08-22; the next nightly) M6.18 silent background update checks (**landed 2026-08-23, `6b9bbe4`**) and M6.19 pin the UI font to Inter (added 2026-08-23) before `v0.6.0`; M6.2 hooks, M6.7 terminal render/ingestion performance (added 2026-08-21 from a hot-path audit) and M6.4 ride as post-GA updates. Runs during the Phase 6 daily-drive clock and does not gate cutover. All of it lands on `gpui-nightly` (decision 11); nothing is pulled forward onto `main`.

### Phase 5 — App-shell services (replace what Tauri gave for free)

- Packaging: .app assembly, codesign, notarize, staple, DMG — scripted (`cargo-bundle` or hand-rolled; validated in Phase 1's criterion 5).
- Updater: replace `tauri-plugin-updater` with **Sparkle, via the pulse pattern** (decided 2026-08-17). Pulse is the in-house reference: `SPUStandardUpdaterController` through ~130 lines of `objc2` bindings behind an `updater` feature with a no-op dev fallback (`crates/pulse-app/src/updater.rs`), `Sparkle.framework` embedded by the bundle script (pinned version, SHA-256 checked), appcast served from GitHub Releases (production `SUFeedURL` → `releases/latest/download/appcast.xml`; the nightly channel reads the rolling `nightly` prerelease's `appcast.xml` instead — §Release channels below; EdDSA key in `SUPublicEDKey`). Zed's hand-rolled `auto_update` (~2k lines) was considered and rejected — it earns its complexity from multi-platform + own server infra, neither of which applies here. One-time bridge at cutover: the last Tauri release updates users into the first native release through `tauri-plugin-updater`'s artifact format (same minisign keypair, same bundle id, higher version); Sparkle owns every update after that. Mechanics and the GA runbook: §Release channels and the cutover bridge below. This is real work; do not leave it for cutover week.
- Dialogs → `rfd`; opener → `open` crate; logging → `tracing` + file layer (keep the crash/panic hook); window position restore → reimplement `window_state.rs` on the native windowing layer.
- CI: the nightly workflow on `gpui-nightly` and the tag-driven release workflow (pulse's `release.yml` plus the bridge trio) — §Release channels below.

### Release channels and the cutover bridge (decided 2026-08-21, Jason; decision 12)

Facts this rests on, verified 2026-08-21: Sparkle offers an update only when the appcast item's `sparkle:version` — which `generate_appcast` reads from the DMG's `CFBundleVersion` — is strictly greater than the installed app's; `CFBundleShortVersionString` is display-only. Sparkle 2.x's `SUStandardVersionComparator` stops parsing at a dash and ranks a shorter-with-suffix version lower, so `0.6.0-nightly` < `0.6.0`; numeric components compare numerically. `SUUpdateValidator`/`SUInstaller` require the EdDSA signature to verify against the host's pinned `SUPublicEDKey` and the Developer ID signature to match; they do **not** require the update's bundle identifier to match the host's (the id is only used to locate the app inside a DMG). GitHub's `releases/latest` is a dynamic alias to the newest published release that is neither a prerelease nor a draft; `main`'s `tauri-plugin-updater` polls `releases/latest/download/latest.json` (minisign-verified) and on macOS replaces the running bundle in place from an `.app.tar.gz`, then relaunches. The native release build's data dir is `~/Library/Application Support/com.wycstudios.runner/` (`bootstrap::APP_IDENTIFIER`, `native_paths`), the path Tauri's `app_data_dir()` resolves to — a bridged install opens the user's existing `runner.db`.

**Versions.**

| Field | Nightly | Production | Note |
|---|---|---|---|
| crate version (three crates in lockstep) | `0.6.0-nightly` | `0.6.0`, `0.6.1`, … | never bumped per build; the one hand bump is `0.6.0-nightly → 0.6.0` at cutover |
| `CFBundleVersion` (Sparkle's comparison key) | `20260821.1432` | same scheme | `date -u +%Y%m%d.%H%M`, computed once per CI run; dotted so every component stays well inside 32-bit; permanent — a pulse-style production `CFBundleVersion` of `0.6.0` would compare *older* than every nightly stamp |
| `CFBundleShortVersionString` | `0.6.0-nightly.20260821.1432` | `0.6.0` | what About and the update sheet show |
| in-app display | `0.6.0-nightly.20260821.1432 (abc1234)` | `0.6.0 (abc1234)` | `option_env!("RUNNER_BUILD_STAMP")` / `RUNNER_BUILD_SHA` baked at compile time; `make run` gets neither → `0.6.0-nightly (dev)` with the updater no-op |

No per-build git tags and no version commits on the nightly channel: the stamp plus the sha in the version string is the build→commit record, and every nightly is already a reviewed, human-verified landing. The minor never rolls during the nightly period.

**Nightly channel.** Workflow on `gpui-nightly`: `workflow_dispatch` only (amended 2026-08-21 evening — the original `push` trigger built every landing, including hotfix churn, and could restart the app mid-mission; now `gh workflow run nightly.yml --ref gpui-nightly` after a landing Jason wants to take); the bundle builds in parallel with `ci.yaml` and publishes only after CI is green for the dispatched sha (M4.8); `concurrency: nightly, cancel-in-progress: true`. Each run stamps, builds, signs inside-out (`Sparkle.framework` first), notarizes, staples, produces `Runner-Nightly-<short version>-universal.dmg` (arm64-only before M6.17), downloads the existing nightly assets, prunes to the last ~10, runs `generate_appcast --download-url-prefix https://github.com/yicheng47/runner/releases/download/nightly/`, and overwrites the assets on the single rolling `nightly` release, which is **always a prerelease**. `SUFeedURL` is the fixed `https://github.com/yicheng47/runner/releases/download/nightly/appcast.xml`. Two isolation rules, both load-bearing: a nightly published as a normal release would become `releases/latest` and every Tauri install's update check would land on it; and the nightly feed must not use the `latest` alias, which belongs to the Tauri line until cutover. One EdDSA keypair signs both feeds (`SPARKLE_ED_PRIVATE_KEY`, generated once with `generate_keys`, public half pinned in both bundles' `SUPublicEDKey`); the other secrets are the ones `main`'s `release.yml` already holds (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`). Pulse's `docs/macos-release.md` is the runbook to copy. **Keypair generated 2026-08-21** with Sparkle 2.9.5's `generate_keys` under keychain account `com.wycstudios.runner`: `SUPublicEDKey` is `X2r1GfMmzcCS/9//sSUyyBNxMajjcMqVwQeHHKtAHMs=` (checked in at `packaging/sparkle-public-key`); the private seed lives in Jason's login Keychain, his private backup, and the `SPARKLE_ED_PRIVATE_KEY` repo secret — never in the repository or a workflow log. One key serves both channels for the life of the app; losing it forces users through a manual re-download, leaking it weakens the update chain to Developer ID alone.

**Cutover is one release — GA `v0.6.0` is the last Tauri-format update and the first Sparkle release.** There is no Tauri code to remove afterwards; this tree never had any. Runbook:

1. Preconditions: the Phase 6 criteria, including the `bridge-test` proof below.
2. On `gpui-nightly`: `0.6.0-nightly → 0.6.0` in the three crates.
3. Promotion by tree-swap merge, never a force-push: `git checkout main && git merge -s ours --no-commit gpui-nightly && git read-tree -u --reset gpui-nightly && git commit && git tag v0.6.0`. `main`'s tree becomes `gpui-nightly`'s byte-for-byte with both histories as parents. The tag fires the native `release.yml`, now on `main`.
4. The release workflow builds two artifact sets from one bundle. Sparkle: `Runner-0.6.0-universal.dmg` + `appcast.xml` (`--download-url-prefix …/releases/download/v0.6.0/`, same EdDSA key as nightly). Bridge trio: `tar czf Runner.app.tar.gz Runner.app`, signed with `TAURI_SIGNING_PRIVATE_KEY` through `npx @tauri-apps/cli signer sign` (the CLI emits exactly the signature format the plugin verifies — do not hand-roll with minisign), and `latest.json` with `version: "0.6.0"` and both `platforms.darwin-aarch64` and `platforms.darwin-x86_64` `{url, signature}` pointing at the one universal tarball (M6.17). All five upload to a draft release.
5. Publishing is the switch: `releases/latest` flips to `v0.6.0`; Tauri 0.5.x installs see `0.6.0 > 0.5.2`, download the tar.gz, verify minisign, replace `/Applications/Runner.app` in place, and relaunch into the native app on the same database; the production Sparkle feed `releases/latest/download/appcast.xml` is live from the same release.
6. Nightly users need nothing: under decision 10 they also have the Tauri app side by side, and it bridges itself. No cross-over item in the nightly feed.

After cutover: the nightly channel continues as `main`'s prerelease channel (`0.7.0-nightly`, every landing) — the velocity channel M6 runs on. The signed `Runner.app.tar.gz` + `.sig` are built and uploaded once, on `v0.6.0`. Every later 0.6.x production release re-uploads the same `latest.json` — `version: "0.6.0"`, `platforms.darwin-aarch64.url` / `darwin-x86_64.url` the absolute `releases/download/v0.6.0/Runner.app.tar.gz`, the `v0.6.0` signature string — because the endpoint baked into every shipped 0.5.x build is the `releases/latest` alias, which moves with each release; without the file a dormant Tauri install would get a 404 and never bridge. With it, a dormant install takes two hops: the Tauri updater installs 0.6.0 from the old tarball, relaunches into the native app, and Sparkle carries it to the current release (the plugin checks only `version` > installed and the signature; it does not require the manifest's version to match the current release). Cost per release: one file copied forward in the release workflow. **Hard cutoff at `0.7.0`** (Jason, 2026-08-22): the release workflow stops uploading `latest.json`; any 0.5.x install still dormant by then installs the DMG by hand.

**Proof before the day (`bridge-test`).** The Tauri endpoint is baked into shipped 0.5.x builds, so build one throwaway Tauri app from `main` with `endpoints` pointed at `releases/download/bridge-test/latest.json`, publish a `bridge-test` prerelease carrying the native bridge trio, and run the real hop on a machine with real data: relaunch lands in the native app, `runner.db` opens, sessions resume, settings and window files load; then a second `bridge-test` build with a higher stamp arrives through Sparkle from the bridged install; also test an unwritable `/Applications` (the plugin's admin-authorization fallback). That is the Tauri→native half of "updater proven"; the nightly channel is the native→native half. This proof, the nightly workflow, and the stamping are the deliverable of M4.6f's packaging sub-mission (decision 10's "throwaway internal build").

### Phase 6 — Cutover

Criteria: 2+ weeks daily-driving the native app exclusively, M3–M4 done (M5 closed by decision 13), updater proven both ways — at least one native-to-native nightly update and the `bridge-test` Tauri→native hop — fixture corpus green. Then: `gpui-nightly`'s tree replaces `main` by tree-swap merge, the native binary becomes `Runner.app`, and the version goes `0.6.0-nightly → 0.6.0` (the 0.6 line is the GPUI line, set 2026-08-20). Runbook: §Release channels and the cutover bridge. (The crate renames of decision 7 land earlier, after the M3 session-hardening slice.)

### Post-cutover direction: session daemon (recorded 2026-08-18, Jason)

Live PTY sessions dying with the app is a process-ownership fact in both stacks — the rewrite keeps that behavior identical (parity only; agent-session-key resume remains the continuity story). But the Phase 2 extraction made the fix buildable: `runner-backend` is UI-free, so it can be promoted into a long-lived session daemon (tmux-server model — daemon owns PTYs + seq'd output buffers, the app attaches/detaches over a socket; M3.2/M3.3's replay seams are exactly the attach discipline). The concrete motivator is the nightly/Sparkle loop: every update restarts the app and kills running agents; a daemon makes updates and crashes invisible to live missions. Scope it as the flagship post-cutover project, after M5 — it forks the architecture away from `main`, so it must not happen mid-parity. Known work when designed: ops become RPC, daemon/UI version skew across updates, reaping semantics (M3.1's sweep assumes app-owned processes), MCP moves into the daemon.

## Verification

- Fixture corpus replay green at every milestone (the terminal regression floor).
- `make verify` (check + workspace tests + clippy + fmt-check) per milestone.
- Migration + A/B switch test as in M1 — the seamless-upgrade proof, repeated whenever a migration ports.
- Parity review per M3/M4 slice against the running Tauri app side-by-side: same surface, same behavior, same copy.

## Risks

- **Parity treadmill** — the killer risk: `main` keeps moving while this line chases it (44 backend commits in the first month alone). Containment: the M5 watermark, the schema/protocol-ports-promptly rule, dogfood-ordered slices (pressure to finish is intrinsic), and a soft feature-freeze on new frontend surface on `main`. The real fix remains finishing the milestones. Closed 2026-08-21: `main` froze at `276a3a4` under decision 11 and M5 was retired by decision 13 — the treadmill stopped.
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
