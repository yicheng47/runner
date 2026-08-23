# Mission brief — M6.3 (mutex item): the session lock held across the flocked status append

Drafted 2026-08-22 16:05. Started with `mission_start` on crew `codex peer` (`01K000DEFAULT000PEERCODING01`), project `runner` (`01KZWQA969B26J6WSP68RZP4Y9`), cwd the `runner-gpui` worktree, title `M6.3 — Session lock held across the flocked status append`, the text below as `goal_override`. A nightly (run 32560379906) is building while this runs; no Sparkle update while the crew works.

---

Implement the **M6.3 mutex item** (`docs/impls/gpui-rewrite/m6-consolidation.md` §M6.3, first bullet — the rest of M6.3 is post-GA and out of scope). Backend only (`crates/runner-backend`), one feature branch in this worktree. `gpui-nightly` is at `434bf81`; leave uncommitted docs under `docs/impls/gpui-rewrite/` alone. **`runner-app` is the binary, `runner-backend` the core.** Standing rules in `impl_log.md` apply.

## The defect (re-traced 2026-08-22 on `434bf81`)

`SessionManager` keeps one `Arc<Mutex<SessionState>>` per session (`session/manager/mod.rs:585`, `session_state_or_insert` `:737`). Two paths take it:

- `synthesize_wake_busy` (`mod.rs:988-1003`) — called by the router when it delivers a wake/nudge — locks the session, then **inside the lock** calls `sink.try_append_with_retry(draft)` (`:186-197`): an `EventLog::try_append` (flock on `events.ndjson`) retried up to `RUNNER_STATUS_APPEND_MAX_ATTEMPTS = 8` times with `RUNNER_STATUS_APPEND_RETRY_DELAY = 5 ms` sleeps while contended — up to ~40 ms of sleeping plus the flock wait, all under the per-session mutex — then sets `session.activity = Some(Busy)`.
- `record_output` (`session/manager/output.rs:676-693`) — the PTY ingestion path, once per output chunk — locks the same session to bump `output_seq`. (`ingest_output_chunk` is the caller; the forwarder thread drives it from `output.rs:113`.)

With two agents in one mission both writing `runner_status` to the same `events.ndjson`, flock contention is steady state, so a wake for slot A parks A's ingestion thread behind A's own lock for the retry budget — the pane the human is typing in stutters from the *other* agent's log traffic. The forwarder's own status appends (`output.rs:129` → `try_append_runner_status`) are **not** under the session lock (`note_forwarder_transition` releases it first); `synthesize_wake_busy` is the one offender found by the audit — confirm that with the sweep below.

## Fix

1. In `synthesize_wake_busy`: take the lock only to clone the `mission_status_sink` `Arc` (and whatever else the draft needs), drop it, append outside the lock, then re-take it to set `activity = Busy`. Decide and document the re-take race: between the append and the re-lock the forwarder may have recorded a newer transition (`note_forwarder_transition`) — setting `Busy` over a fresher `Idle` would lie for one cycle. Preferred: a monotonic activity sequence on `SessionState` (or compare the activity observed under the first lock) so the wake only sets `Busy` when nothing newer landed; if you judge the window harmless because the forwarder's next real transition corrects it within milliseconds, say so in the handoff with the reasoning, and keep the behavior identical otherwise (errors on `Contended`/`Failed` unchanged, `activity` untouched on failure).
2. Sweep every `lock()` on a `SessionState` / the `sessions` map in `session/manager/{mod,output,lifecycle,spawn}.rs` for blocking work inside the guard — `try_append`, `append`, `thread::sleep`, DB calls (`state.db.get()`), PTY writes, channel sends that can block, `Condvar` waits. Report each site as clean or fixed; fix any other real one the same way (append/IO outside, state update inside) if it is small, otherwise list it for the post-GA M6.3 remainder. `session_state_or_insert` takes the manager-wide map lock on every chunk — measure whether it matters (it should not; the critical section is a hash lookup) and say so.
3. Test, in `session/manager/tests.rs` next to the existing non-blocking `try_append_runner_status` tests (~3866-3960, which already model a contended log): hold the event log's flock from a helper thread, call `synthesize_wake_busy` on one thread and `record_output`/`ingest_output_chunk` for the same session on another, and assert the ingestion call returns well under the retry budget (e.g. < 5 ms while the wake is parked for ~40 ms) — it must fail on the current code. Plus a test for the re-take race decision from (1).
4. Before/after numbers in the handoff: with the contended-log helper, `record_output` latency p50/p99 over a few hundred chunks while wakes are being synthesized, old vs new.

Out of scope: the other M6.3 bullets (DEFERRED tx in `mission::start`, `list_with_repair`, indexes, `inbox_read` ULID case, `inject_direct_stdin` `Condvar`, archive UX, M3.7/M3.8 carries) — post-GA, per the plan.

## Gates

- `make verify` green; `git diff --check` clean.
- Handoff: the sweep table (site → clean/fixed), the race decision and its reasoning, the numbers from (4), and a daily-drive check: a two-agent mission with both slots streaming while the human types into one — no stutter on wake/nudge delivery.
- **Do not commit, push, open a PR, or merge.** Stop at a clean working-tree review on the feature branch.

reviewer: hunts — (1) the append moved out but the lock re-taken in a way that can deadlock against the forwarder (lock order — **corrected after review, 2026-08-22**: the codebase is consistently **gate → session**; `input_quiescent` at `mod.rs:791-805` drops its session guard, takes `gate.state`, then re-takes the session lock under it, and `inject_reserved` keeps the same order. This brief originally said session → gate, which is backwards. `synthesize_wake_busy` never takes the gate lock, so it adds no cycle either way); (2) `activity` set to `Busy` over a newer forwarder transition with no justification; (3) a second blocking call under a `SessionState` guard the sweep missed (`grep -n "lock()" crates/runner-backend/src/session/manager/*.rs` and read each scope); (4) the new test passing on the old code (it must not); (5) behavior drift on the failure paths — `Contended` still maps to "event log busy", `Failed` still propagates, `activity` untouched on either; (6) the `sessions` map lock held across anything but the lookup.
