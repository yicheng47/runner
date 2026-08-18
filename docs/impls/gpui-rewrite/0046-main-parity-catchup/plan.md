# 0046 — Main-parity catchup: backend sync, node tree, terminal split

## Status

Planned. This is the big migration phase for the `gpui-nightly` line: adopt everything `main` shipped since the 2026-07-19 fork point (merge-base `d9483ca`, v0.3.17 → v0.5.2, ~44 backend-touching commits), restructure the terminal layer along Zed's `terminal`/`terminal_view` split, and move the framework dependency to `gpui-ce`. Supersedes the Phase 4 slice list in [impl 0031](../0031-rust-native-ui-rewrite/plan.md) as the working plan; 0031 remains the strategy record.

## Decisions (2026-08-17, Jason)

1. **`main` owns the design.** The whole product design follows `main`, including the node-tree sidebar (Feature 44). The GPUI line does not fork product decisions.
2. **Repo-and-below is identical.** `crates/runner-core`, `cli/`, `db.rs` + migrations, and `repo/` must match `main` line-for-line (modulo file paths: `src-tauri/src/*` ↔ `crates/runner-app/src/*`). Anything GPUI-specific lives in the adapter layer (`ops/`) or above — never in repo/db. Migration numbers are allocated on `main` only; this branch never adds its own.
3. **Product UI never changes; only the tech stack changes.** The GPUI app renders the same surfaces, flows, and copy as `main`'s current React app. Divergence is a bug, not a design opportunity.
4. **Terminal architecture mimics Zed's `terminal` / `terminal_view` crate split** (studied from the local checkout at `~/repos/gui/zed`, tree of 2026-08-17), but on **upstream `alacritty_terminal`** from crates.io — not Zed's fork.
5. **Framework: `gpui-ce`**, the community edition, replaces the stale `gpui` crates.io line. See Workstream D for the evidence.
6. **Updater (Phase 5, decided while this spec was in flight): Sparkle via the pulse pattern** — see the updated Phase 5 bullet in [impl 0031](../0031-rust-native-ui-rewrite/plan.md). Not part of this spec's workstreams; recorded here because pulse also validates gpui-ce at a pinned git rev in production, which de-risks Workstream D's fallback path.

## Ground rules

- **License guardrail:** Zed's `terminal` and `terminal_view` crates are GPL-3.0. They are an architectural reference only — mirror the shape, responsibilities, and data flow; copy no code. (`gpui`/`gpui-ce` are Apache-2.0 and fine as dependencies.)
- **Port unit is `main`'s commit/impl doc, not the diff blob.** Each ported change cites its `main` commit; `main`'s impl docs 0033–0045 are the port guides. Reading the impl doc before porting is the difference between a port and a guess.
- **Seamless upgrade invariant:** a `runner.db` written by any Tauri release must open in the GPUI app and vice versa. This holds automatically iff decision 2 holds — the migration chain is linear (this branch's 0001–0013 is an exact prefix of `main`'s 0001–0020) and must stay that way.
- No commits without human verification per landing step; task branches off `gpui-nightly` as usual.

## Workstream A — Backend catchup (repo-and-below)

### A1. Protocol crates: take `main` wholesale

`crates/runner-core` and `cli/` live at the same paths on both branches and this branch has zero commits on them since the fork — `git checkout origin/main -- crates/runner-core cli` is the entire port. Drift being adopted: `cli/src/msg.rs` payload cap (`c3ea601`), `roster.rs` cleanup, `runner-core` event-log changes, extended roundtrip tests.

### A2. DB layer: migrations 0014–0020 + backfills, verbatim

Copy `main`'s `src-tauri/src/db.rs` and `src-tauri/migrations/` over `crates/runner-app/`'s, adjusting only the module path prefix. New content: `0014_nodes.sql` (nodes table + 1:1 row copy from folders/tabs), `0015_retire_folders.sql` (drops legacy tables), `0016_session_last_size`, `0017_session_resume_on_launch`, `0018_slot_model_override`, `0019_session_agent_options`, `0020_slot_effort_override`, plus the Rust backfill steps (`backfill_0014_nodes`, `backfill_0015_retire_folders`) and the peer-coding-crew seed (`d04ce3f`).

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

Feature 44 on `main` replaced `folders` + `tabs` + pin flags with one `nodes` tree (`folder` | `project` | `tab` | `mission`, `parent_id` + `position`, `pinned_position` for the pinned section). This branch's Phase 4 direct-chats sidebar is built on the retired model and must be rebased onto nodes, matching `main`'s current sidebar exactly: node tree with projects/folders, pinned section (`a172ca6`), tab attention watermarks carried on the node row, drag-reorder/rename/archive semantics as `main` ships them. This is UI work, but it is parity work — the target rendering is `main`'s sidebar, not a new design.

## Workstream C — Terminal split (`runner-terminal` / view side)

Mirror Zed's two-crate shape (reference: `~/repos/gui/zed/crates/terminal` and `crates/terminal_view`), adapted to Runner's ownership model:

| Zed | Responsibility | Runner target |
|---|---|---|
| `terminal/src/terminal.rs` | UI-agnostic model: `Term` behind `FairMutex`, VTE `Processor`, internal event queue, `last_content` renderable snapshot, selection/scroll state, search | new `crates/runner-terminal/src/terminal.rs` (grown from today's `runner-native/src/terminal.rs`) |
| `terminal/src/alacritty.rs` + `alacritty/hyperlinks.rs` | isolation of `alacritty_terminal` API surface: grid iteration, selection types, event glue | `runner-terminal/src/alacritty.rs` |
| `terminal/src/mappings/{keys,mouse,colors}.rs` | keystroke→escape encoding, mouse reports, color conversion | `runner-terminal/src/mappings/` (today this logic is scattered in `runner-native`) |
| `terminal/src/pty_info.rs` | foreground-process info | **not ported** — process identity is `SessionManager`'s domain |
| `terminal_view/src/terminal_element.rs` | GPUI `Element`: layout → batched text runs/rects/cursor, paint, IME `InputHandler` | stays in `runner-native/src/terminal_element.rs` (keeps the per-cell-origin alignment strategy) |
| `terminal_view/src/terminal_view.rs` | focus, key dispatch, blink, context menu | `runner-native` chat/pane views |
| `terminal_view/src/{terminal_panel,persistence,scrollbar,path_like_target}.rs` | workspace integration | `runner-native` pane layout / future slices |

**Deliberate deviations from Zed:**

- **The model crate does not own the PTY.** Zed's `Terminal` runs alacritty's `event_loop` + `tty` and owns the child process. In Runner, `SessionManager` (in `runner-app`, on `portable-pty`) owns spawn/resume/kill/resize because sessions outlive views and are mission-coordinated. `runner-terminal` stays a pure byte-consumer: `SessionManager` output events in, renderable content + encoded input bytes out. This keeps the fixture corpus a pure model-level replay harness.
- **Upstream `alacritty_terminal`, not Zed's fork.** Zed pins `zed-industries/alacritty`; we stay on crates.io (`0.26.0`, current as of 2026-04). Upstream is the contract; if we hit something Zed's fork patches, we work around it inside `runner-terminal` rather than adopting a fork.
- The fixture corpus (`fixtures.rs`, `replay.rs`) moves into `runner-terminal` as its test suite — the corpus is a property of the model, not the renderer.

## Workstream D — Framework dependency: `gpui-ce`

Evidence (checked 2026-08-17):

- Zed's crates.io `gpui` is frozen at 0.2.2, published 2025-10-22 — ten months stale, while Zed's in-tree gpui (our architectural reference) has moved continuously.
- `gpui-ce` is a maintained community fork explicitly designed as a drop-in (`gpui = { package = "gpui-ce", version = "0.3" }`): crates.io 0.3.3, repo actively synced with Zed upstream (last upstream sync commit covers Zed's 2026-07-30 tree; commits within the last week), recommended by Zed developers, 1:1 compatible with the surrounding ecosystem.

Decision: switch to `gpui-ce`. Start on crates.io 0.3.3; if reference patterns from Zed's current tree need newer API, pin a git rev of `gpui-ce/gpui-ce` and bump deliberately. Exit hatch: because it's a drop-in, reverting to mainline `gpui` (or a future Zed publish) is a one-line change. The swap lands first and alone, gated on the fixture corpus and the app running clean — API churn between 0.2.2 and 0.3.x is expected in element/input APIs and must not be entangled with parity changes.

## UI reference: pulse (2026-08-18, Jason)

For M3+ native UI work, `~/repos/yicheng47/pulse` (our own pure gpui-ce app, in production) is the reference for window-level operations — hidden-native-titlebar setup (`TitlebarOptions`, traffic-light positioning in `pulse-app/src/main.rs`), titlebar drag areas and double-click zoom (`shell.rs`), app menus (`menu.rs`), and window-bounds handling. Its `components.rs`, `text_input.rs`, `theme.rs`, and settings surfaces are candidate reusable components/patterns. Check pulse before hand-rolling any window-level UI; it already anchors the Phase 5 Sparkle updater decision. Zed remains the reference for terminal/editor-grade patterns (architecture only — GPL guardrail above); pulse is the reference we can copy from directly.

## Sequencing

Each milestone is a task branch off `gpui-nightly`, human-verified before merge:

1. **M0 — framework swap**: `gpui` → `gpui-ce 0.3`; fix API churn; fixture corpus green; app daily-drivable. No behavior change.
2. **M1 — repo-and-below parity** (A1–A3 + minimal B): protocol crates wholesale; db + migrations + repo verbatim; `ops/` adapter updated; sidebar rewired onto nodes far enough to compile and function (parity polish deferred to M3). Gate: migration test on a *copy* of a production `runner.db` (never the live file), then the A/B switch test — Tauri `main` build and GPUI build alternately opening the same copy.
3. **M2 — terminal split** (C): extract `runner-terminal`, consolidate input encoding into `mappings/`, move the fixture corpus. No rendering change; corpus green.
4. **M3 — parity slices** (A4 + B polish, dogfood order): session hardening → runtimes/model-effort pickers → node sidebar polish + pinned section → mission feed (read-mostly + channel composer) → pagination/update checks/misc. Revised from 0031 Phase 4; each slice daily-driven before the next.
5. **M4 — sweep**: diff `crates/runner-app` against `main`'s `src-tauri/src` module-by-module; the diff should be adapter-shaped only. Record the synced `main` SHA in this doc as the watermark; subsequent `main` backend commits port promptly (schema/protocol) or per-slice (features).

## Verification

- Fixture corpus replay green at every milestone (the terminal regression floor).
- `make test` + workspace clippy per milestone.
- Migration + A/B switch test as in M1 — this is the seamless-upgrade proof, repeated whenever a migration ports.
- Parity review per M3 slice against the running Tauri app side-by-side: same surface, same behavior, same copy.

## Risks

- **The node-tree rebase is the long pole** — it touches schema, repo, and the only UI this branch has shipped (Phase 4 direct chats). Mitigated by landing it in M1 at "functional" depth and polishing in M3.
- **`gpui-ce` is community-run.** Mitigated by drop-in reversibility, its upstream-tracking discipline, and the M0 gate keeping the swap isolated.
- **Parity treadmill** (0031's killer risk) continues until cutover: `main` moved 44 backend commits in a month. The M4 watermark plus the schema/protocol-ports-promptly rule is the containment; the real fix remains finishing the phases.

## Non-goals

- Any product/UX change relative to `main` (divergence is recorded as an issue, not shipped).
- Porting Tauri app-shell services (updater, packaging, dialogs) — that remains 0031 Phase 5.
- Adopting Zed's alacritty fork, or copying GPL code from Zed's terminal crates.
