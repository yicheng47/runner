# #1377 — Remaining daemon, DB, and main-thread stall isolation

Jason approved `carbon trio` on 2026-11-18. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1377-daemon-ui-stall-isolation` on `fix/1377-daemon-ui-stall-isolation` (base `origin/main` `660d3408`, dedicated `target/`). Sibling worktrees `.worktrees/feat-1213-worktrees-app` (#1379) and `.worktrees/test-flaky-process-probe-tests` are active; never touch them or run heavy daemon-spawning test suites concurrently with a sibling run.

## Problem and outcome

In [#1377](https://github.com/yicheng47/runner/issues/1377), companion channels stopped receiving `Welcome` or answering `Ping`, and the macOS app beachballed until force-quit. Fix 1 ([#1380](https://github.com/yicheng47/runner/pull/1380), `v0.20.1`) isolated blocking PTY writes in `input_writer` and dropped `terminal_control` before joining workers. Ranked fixes 2–5 of the [#1377 investigation](https://github.com/yicheng47/runner/issues/1377#issuecomment-6946555850) remain on `main` and still let a slow session, DB lock, or `flock` wait wedge remote channels, Tokio workers, SQLite callers, or the GPUI main thread.

## Read first

- [#1377 investigation](https://github.com/yicheng47/runner/issues/1377#issuecomment-6946555850) (sections 2 & 5) and `remote/channels/tests.rs`.
- Companion & IPC (`crates/runner-daemon/src/`): `remote/channels.rs` (`serve`, `Channels::closed`), `companion/mod.rs` (`Stream::drop`, `Lease::drop`, `Adapter::{initialize,request,viewport,changed}`), `session/manager/terminal.rs` (`release_browser_channel`, `ControlState`), `daemon/server.rs` (`Attachments`, `ConnectionCleanup::drop`, `ConnectionWindows::dispatch`, `wire::REQUEST`).
- Lock, DB & `flock` (`crates/runner-daemon/src/`): `ops/node.rs` (`mark_direct_sessions_viewed`, `record_session_completion`), `companion/core.rs` (`DaemonCore::terminal`), `ops/session.rs` (`live_session_counts`, `session_status_snapshot`), `session/manager/output.rs` (`settle_pending_resize`), `session/manager/attach.rs` (`install_handle`, `install_handle_state`), `ops/mission.rs` and `ops/mission/{feed,report,interrupt}.rs`.
- Desktop UI (`crates/runner-app/src/`): `app_store.rs` (`AppStore::new`, `refresh`, `refresh_activity_inner`, `refresh_render_snapshots`), `surfaces/windowing/mod.rs` (`report_current_subjects`, `sync_window_activation`), `surfaces/sidebar/activation.rs` (`mark_active_tab_viewed`), `surfaces/mission_workspace/input.rs` (`mark_active_session_viewed`), `test_support.rs` (`DrainTasksOnDrop`, `app_store_and_native_root_release_db_pool_on_test_teardown`).

## Deliverables

Pipeline as clean, self-contained commits on `main` (fold this brief into the first commit before pushing) so `@reviewer` reviews each slice independently:

1. **Companion channel isolation & nonblocking drops (Fix 2):**
   - Bound `Adapter::initialize()` before `Welcome` with a deadline so a stalled status/DB read closes the channel cleanly instead of hanging reconnects past `Hello`.
   - Move `Attach`, `Input`, `Viewport`, `TakeControl`, `Answer`, and `Post` off the inline `.await` path in `channels.rs` `serve()` into bounded per-adapter background work polled from `Adapter::changed()` (preserving per-session input/attach/viewport order) so `serve()` keeps pumping `input.recv()` and `Ping` → `Pong` while a core call blocks.
   - Bound queued/in-flight blocking requests per companion channel and per local IPC connection (`daemon/server.rs` `wire::REQUEST`) so timed-out/retried callers cannot exhaust Tokio's `spawn_blocking` pool.
   - Offload `Lease::drop`, `Stream::drop`, `ConnectionCleanup::drop`, and `Attachments::drop` cleanup via `spawn_blocking` when dropped on Tokio workers (inline fallback outside Tokio).
   - Narrow `SessionManager::release_browser_channel` so `ControlState` tracks sessions/groups owned by `channel` and releases only those without locking unrelated `SessionState`s.

2. **DB, `SessionManager`, `EventLog` `flock`, and router-join ordering (Fix 3):**
   - Commit or drop `DbPool` checkouts and `Immediate` transactions before `SessionManager` / `SessionState` locks in `mark_direct_sessions_viewed`, `record_session_completion`, `DaemonCore::terminal`, and `live_session_counts` (snapshot `live_session_ids()` before `db.get()`).
   - Fix reverse `SessionState` -> `pool.get()` + SQLite writes in `settle_pending_resize` (`output.rs`) and `install_handle` / `install_handle_state` (`attach.rs`): drop `terminal_control`, `gate.state`, and `SessionState` before `pool.get()` / `update_last_size`, using a per-session resize stamp so stale writes never overwrite newer sizes or clear `last_requested_size_dirty` for a newer request.
   - Drop `DbPool` connections before `buses.unmount` / `routers.unregister` joins in `mission_archive_impl`, and before shared `EventLog::open` / `read_from_lossy` / `append` `flock` waits in non-transactional `ops/mission*` paths (`post_message`, `feed`, `summaries`, `report`, `interrupt`), preserving atomic rollback transactions in `start_with_files`, `stop`, and `mission_unarchive_impl`.

3. **Offload GPUI main-thread `DaemonClient` calls (Fix 4):**
   - Move steady-state `AppStore::refresh` (`Windows`, `Activity`, `Nodes`, `All`, `Missions` — fix `Missions` using foreground `cx.spawn` instead of `cx.background_spawn` — and `refresh_render_snapshots`) plus `sync_window_activation`, `report_current_subjects`, `mark_active_tab_viewed`, and `mark_active_session_viewed` onto `cx.background_spawn` with coalescing / generation ordering so latest state wins and the UI never blocks on `SocketTransport::call`.
   - Keep `Task` handles entity-owned so `app_store_and_native_root_release_db_pool_on_test_teardown` passes with zero leaked `AppCore` / `DbPool` refs.

4. **Stall diagnostics and main-thread watchdog (Fix 5):**
   - Log slow-stage warnings (`> 2s`) in `runnerd` around `Adapter::initialize`, `session_status_snapshot`, and `ConnectionWindows::dispatch`.
   - Add a main-thread stall detector in `runner-app` (cleanly stopped on teardown, quiet in unit tests unless under test) tracking turn heartbeats and UI/store breadcrumbs, logging a warning to `runner.log` when a turn stalls past 3 s and recovery duration when it finishes (never log a watchdog-thread `Backtrace` as the main thread's stack).

Update `docs/guide/` (`en` & `zh-CN`, `python3 script/check-guide-translations.py`) only if user-visible behavior changes. Non-goals: `#1380`'s `input_writer.rs`, `#1351`'s relay liveness, or SQLite migrations.

## Validation and live checks

- **Automated:** Deterministic tests (`wait` helpers/barriers only, zero fixed sleeps; loop every new concurrency/app test ≥ 20× and state count) for: (1) `initialize` deadline, `Ping`/`Pong` during blocked `Input`/`Attach`/`Viewport`, nonblocking `Lease`/`Stream` drop, channel-scoped `release_browser_channel`; (2) lock-order freedom in `mark_direct_sessions_viewed`, `live_session_counts`, `DaemonCore::terminal`, monotonic `settle_pending_resize` / `install_handle_state`, `mission_archive_impl`; (3) coalesced async `AppStore::refresh` / `sync_window_activation` and `app_store_and_native_root_release_db_pool_on_test_teardown`; (4) slow-stage / stall warnings. Run `cargo test -p runner-daemon`, `cargo test -p runner-app`, `make clippy`, `cargo fmt --all --check`, `git diff --check`.
- **Live QA (`@qa` after clean review):** Hold `script/qa-lock acquire --timeout 540`, preserve existing dev sessions, record screenshots in `docs/tests/runs/1377-daemon-ui-stall-isolation-mac.md`: (1) local `make run` focus/blur, tab switch, unread clearing, and `TERM-INPUT-01` with clean logs; (2) remote access on the **same-Mac two-host relay fixture** (`docs/tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos` — per 2026-11-18 rule, enroll two private-`HOME` `.app` wrappers here with the hosted relay via Chrome's signed-in Runner account; never use a local relay or mark Blocked for a second computer): verify `Welcome` + overview, `REMOTE-TERM-01`, viewport take/release, and reconnect, then revoke/forget only run-owned enrollments and remove `/private/tmp` wrappers.

## Authorization

Approved crew: `carbon trio` (`coder`, `reviewer`, `qa`). Commit, push `fix/1377-daemon-ui-stall-isolation`, and open a PR against `main` (`Fixes #1377`) with `core-review`, `risk-high`, `reviewed` (`@reviewer` on final head), and `needs-live-test` only if Windows native live QA remains unrun. Do not merge.
