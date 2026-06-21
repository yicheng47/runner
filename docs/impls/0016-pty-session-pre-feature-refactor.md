# PTY Session Layer Pre-Feature Refactor

> Behavior-preserving cleanup of the session/PTY stack before the next round of terminal features. No product-visible change; every step is a refactor or a deletion that keeps the existing tests green.

## Context

The session layer was designed around a tmux-backed runtime (`docs/impls/archive/0004`) and later migrated to an in-process `portable-pty` runtime (`docs/impls/archive/0011`). The migration swapped the implementation but left the *seam* — the `SessionRuntime` trait, `RuntimeSession`, and a large fraction of the comments — shaped around tmux. The result is an abstraction that describes a runtime that no longer exists, plus accumulated dead scaffolding from earlier prompt-delivery and reattach designs.

`src-tauri/src/session/manager.rs` is now 5620 lines (one ~2160-line `impl SessionManager` block plus ~2900 lines of in-file tests), and per-session state is spread across six separate locks. Each new feature has to thread through this, and the misleading tmux vocabulary makes it easy to reason wrong.

This plan does the no-behavior-change cleanup first, so feature work lands on an honest, smaller surface. The two larger architectural ideas surfaced during analysis — a host-side terminal model, and the frontend snapshot/live reconciliation rewrite — are explicit non-goals here and get their own docs if pursued.

## Goal

Leave the session/PTY layer functionally identical but materially smaller and truthful:

- The runtime seam describes the PTY runtime that actually exists, not tmux.
- Dead code (vestigial first-prompt schedulers, unused `OutputStream` API, the uncalled reattach machinery) is gone.
- Per-session state has a single owner instead of six parallel maps.
- `manager.rs` is split into reviewable modules.

## Non-goals

- No host-side terminal/VT model. The raw byte ring buffer + alt-screen escape scanning (`update_alt_screen_state`, the synthetic `seq=0` prepend in `output_snapshot`) stays as-is. A headless emulator is a separate design doc, justified only if a feature needs correct cross-restart replay, scrollback search, or server-side output.
- No frontend reconciliation rewrite. `RunnerTerminal.tsx`'s snapshot/live ref-tangle is left alone; it gets its own doc when terminal UX features next touch it.
- No change to spawn behavior, kill semantics, idle/busy detection, gate timing, event payloads, or DB-stored data that anything reads.

## Decisions

1. **Behavior-preserving only.** Any step that would change a product behavior is out of scope. The test suite is the contract; if a step needs a test changed for reasons other than a renamed symbol or deleted dead path, stop and reassess.
2. **Collapse `RuntimeSession` to `{ runtime: String, session_id: String }`.** Keep `runtime` as a discriminator for a future second implementation (Windows). Drop `socket` (always `""`), `window` (always `"main"`), and `pane` (always `== session_id`).
3. **Leave the dead DB columns as legacy; stop writing them.** `runtime_socket`, `runtime_window`, `runtime_pane` are write-only on the live path — nothing reads them back except the dead reattach reconstructor. No destructive migration: keep the columns (they go NULL on new rows), drop them only from the write statements, and add a schema comment marking them legacy/unused since the PTY-runtime migration. Keep writing `runtime_session` (it stores the session id and the column name is still meaningful).
4. **Delete session reattach; keep resume and startup mission remount. These are different features.** *Session reattach* means re-binding to a still-running agent process that outlived the app — a tmux-era capability (the tmux server held panes across restarts). It is impossible under the in-process PTY runtime: agent processes are children of the app process and die with it. Startup still remounts routers/event buses for missions that are marked `running`, then runs `cleanup_stale_running_rows_on_startup` (demote `running` session rows → `stopped`). The session reattach machinery (`reattach_running_sessions` / `reattach_one` / `attach_existing` / `RowSnap::runtime_session()`) has no production caller and is kept alive only by its own tests and stale comments. *Resume* means re-spawning a fresh PTY and having the CLI agent continue its prior conversation via `agent_session_key` (claude-code `--resume`, codex rollout). That is `SessionManager::resume`, the live feature, which re-spawns and never uses the reattach path. **We support resume only.** Delete all session reattach code; leave `SessionManager::resume` and startup mission router/event-bus remounting intact.
5. **Trim the trait to the real contract.** Remove the trait method `SessionRuntime::resume()` — despite the name it is the runtime-level *reattach* primitive (re-establish liveness from persisted metadata), always errors in `PtyRuntime`, and is unrelated to `SessionManager::resume`. Remove `RuntimeOutput::Replay` (never emitted by `PtyRuntime`; the forwarder's snapshot path is the manager's byte buffer) and `OutputStream::as_receiver` / `try_recv` (unused). Collapse `paste` into `send_bytes` at the runtime level — they are byte-identical (`write_to`), so `SessionManager::inject_paste` keeps its existing byte behavior by calling `send_bytes(payload)` and then submitting Enter.
6. **One PR per step group, ordered by risk.** Steps 1–3 (deletions, seam, comments) are one PR; Step 4 (state consolidation) is its own PR; Step 5 (file split) is its own PR. This keeps each diff reviewable and bisectable.
7. **Per-session locks, not a single global lock.** The consolidated state uses `Mutex<HashMap<String, Arc<Mutex<SessionState>>>>`, not `Mutex<HashMap<String, SessionState>>`. The forwarder's `record_output` runs per output chunk (hot path); a single global lock would make a busy agent's output stream contend with `spawn` / `kill` / `resume` on every other session — a real concurrency regression versus today's independent maps, dressed up as a cleanup. Per-session locks keep the hot path off the global lock; the `Arc` clone per lookup is negligible next to a PTY write. The map lock is held only for membership lookup/insert/remove.

## Step 1: Delete vestigial code

Files: `src-tauri/src/session/manager.rs`, `src-tauri/src/session/runtime.rs`, `src-tauri/src/commands/mission.rs`

- Remove `schedule_mission_first_prompt` and `schedule_direct_first_prompt` (`manager.rs:2866`, `2903`). Since first-turn delivery moved to spawn-time argv (`docs/impls/archive/0007`), both are no-ops whose only effect is a `log::warn!` when argv delivery didn't happen. Inline that one warning at the call sites (or drop it — argv non-delivery for claude-code/codex is already an internal invariant) and delete the functions and their `_mgr`/`mgr` plumbing.
- Remove `OutputStream::as_receiver` and `OutputStream::try_recv` (`runtime.rs`) — no caller in production or tests. Keep `recv_timeout` and `stop_flag`.
- Delete the session reattach machinery: `reattach_running_sessions`, `reattach_one`, `attach_existing`, and `RowSnap::runtime_session()` (the only consumer of the persisted runtime columns), plus their tests (`manager.rs:5034`–`5360` block). Preserve the startup mission router/event-bus remount path, but rename/reword it away from session reattach and remove the failed-id plumbing that only existed for `SessionManager::reattach_running_sessions`.

Validation: code shrinks, `cargo test -p runner` stays green minus the deleted reattach tests.

## Step 2: Collapse `RuntimeSession` and trim the runtime trait

Files: `src-tauri/src/session/runtime.rs`, `src-tauri/src/session/pty_runtime.rs`, `src-tauri/src/session/manager.rs`, `src-tauri/src/db.rs` (schema comment only)

- Reduce `RuntimeSession` (`runtime.rs:70-90`) to `{ runtime: String, session_id: String }`. Update the doc comment to describe the PTY runtime, not tmux sockets/panes.
- Update `PtyRuntime` (`pty_runtime.rs:215-221`) to construct the two-field value; replace every `session.session_name` / `session.pane` usage in `pty_runtime.rs` with `session.session_id`.
- Remove the `runtime.paste` trait method; point `manager::inject_paste` at `runtime.send_bytes(payload)`. Do not add bracketed-paste wrapping in this refactor: the old `PtyRuntime::paste` wrote bytes unchanged, so wrapping would be a behavior change.
- Remove the trait method `SessionRuntime::resume` (the reattach primitive — not `SessionManager::resume`, which stays) and `RuntimeOutput::Replay`. In the forwarder consumer (`manager.rs:1944`) the `Replay(bytes) | Stream(bytes)` arm becomes `Stream(bytes)`.
- Stop writing the dead columns: update the three `UPDATE sessions SET runtime_* = …` sites (`manager.rs:968`, `1335`, `1765`) to persist only `runtime` + `runtime_session`. Leave `runtime_socket` / `runtime_window` / `runtime_pane` in the schema (no migration) and add comments/tests near the migration list and schema guard in `db.rs` marking them legacy/unused since the PTY-runtime migration.

Validation: `cargo test -p runner`, plus a manual mission start + direct chat start to confirm spawn/inject/resize/kill still round-trip.

## Step 3: Strip stale tmux comments

Files: `src-tauri/src/session/*.rs`, `src-tauri/src/commands/session.rs`

- Rewrite or delete the comments that narrate tmux mechanics for a PTY runtime: `kill()`'s "tmux's pipe-pane cleanup chain", `SessionHandle.stop`'s "kill-session → cat dies → FIFO POLLHUP → tx drops" (`manager.rs:392-398`, `2256-2260`), the `send-keys -l --` references, and the ~59 `tmux` mentions across `session/`. Describe the real mechanism: drop master PTY → child SIGHUP → reader `read()` returns 0 → forwarder winds down; `kill`'s explicit stop-flag breaks the consumer out within ~500ms if the disconnect path stalls.
- This is comments-only; no code changes. Doing it as its own commit keeps Step 2's diff focused on behavior-bearing edits.

Validation: none beyond `cargo fmt --check` and a read-through.

## Step 4: Consolidate per-session state

Files: `src-tauri/src/session/manager.rs`

- Replace the four `Mutex<HashMap<String, _>>` (`sessions`, `output_buffers`, `output_seq`, `alt_screen_on`) and the two `Mutex<HashSet<String>>` (`killed`, `resuming_claims`) with a single `Mutex<HashMap<String, Arc<Mutex<SessionState>>>>`, where `SessionState` owns `handle: Option<SessionHandle>`, output buffer, seq counter, alt-screen flag, and per-session status flags. The outer map lock guards membership only; the inner mutex guards a session's mutable state. See Decision 7 for why per-session locks rather than a single global lock.
- Keep `pending_mission_cancels` and `claude_launch_gate` as-is — they are keyed by `mission_id` / global, not by `session_id`, so they don't belong in the per-session map.
- Preserve the kill state machine's contract without losing retained output: the live `handle` stays present until `runtime.stop` succeeds; then only `handle` is cleared and the forwarder is joined. The `SessionState` entry itself remains so `output_snapshot` can replay stopped-session output and `output_seq` continuity survives resume, and is removed only by the existing purge/forget paths. Lookups (`record_output`, `inject_stdin`, `resize`, `output_snapshot`) take the outer lock just long enough to clone the `Arc`, then operate under the inner lock — so the forwarder hot path never contends with `spawn` / `kill` / `resume` on a global lock.

Validation: `cargo test -p runner` (the existing kill/resume/output_snapshot tests exercise this), plus a manual concurrent-kill check (Stop a mission with multiple live slots).

## Step 5: Split `manager.rs` into modules

Files: `src-tauri/src/session/manager/` (new directory)

- Convert `session/manager.rs` into `session/manager/mod.rs` plus focused submodules, each with its own `impl SessionManager` block:
  - `mod.rs` — struct definition, `new`, `runtime`, the consolidated `SessionState`.
  - `spawn.rs` — `base_spawn_spec`, `apply_runtime_args`, `register_mission_session`, `complete_mission_session_spawn`, `spawn`, `spawn_direct*`, `enter_claude_launch_gate`.
  - `lifecycle.rs` — `kill`, `kill_all_for_mission`, `kill_all_for_runner`, `forget`, the pending-mission-cancel helpers.
  - `output.rs` — `start_forwarder_thread`, `record_output`, `output_snapshot`, `purge_*`, `update_alt_screen_state`, `inject_stdin`, `inject_paste`, `resize`.
- Move the ~2900 lines of tests to `session/manager/tests.rs`.
- Make struct fields `pub(super)` (or add thin accessors) as needed so the submodules can reach shared state. Step 4's single state map makes this cleaner — one field to expose instead of six.

Validation: `cargo test --workspace` (pure code-move; behavior unchanged), `cargo fmt`, `cargo clippy`.

## Open questions

1. **Step 5 timing.** The file split is pure code-move with zero behavior change, but reshuffling a 5600-line file guarantees merge conflicts with any in-flight feature branch. Recommendation: ship Steps 1–4 regardless, and only run Step 5 when the feature queue is quiet enough that no large branch is open against `manager.rs`.

Resolved:

- **Reattach.** Not a supported feature — resume only. The reattach machinery is deleted outright; see Decision 4.
- **Lock granularity.** Per-session `Arc<Mutex<SessionState>>`, not a single global lock; see Decision 7.
- **Dead DB columns.** No destructive migration — keep `runtime_socket` / `runtime_window` / `runtime_pane` as legacy columns, stop writing them, and comment them in the schema; see Decision 3.

## Validation (whole effort)

- `pnpm exec tsc --noEmit`
- `pnpm run lint`
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets`
- `cargo test --workspace`
- Manual smoke: start a mission (multi-slot), inject input, resize, Stop; start a direct chat, send a prompt, resize, Stop; restart the app and confirm prior `running` rows show stopped (no reattach regression).
