# M6 — Consolidation

Milestone plan for the work that follows UI parity: the verified defects and structural debt from the 2026-08-20 technical audit, plus the reopened hook-based session status (#347). Sits after M5 (sweep + watermark) in the [program plan](plan.md) and runs during the Phase 6 daily-drive clock. Sizes: S < 50 lines, M 50–300, L > 300 or cross-cutting. Line numbers are as of `gpui-nightly` `9a2b218` and will drift.

## Why a milestone, not a backlog

M3–M5 are parity work: the gate is "same as `main`". Everything here is *beyond* `main` — fixes `main` also has, refactors the agentic build never volunteered, and one new capability. Left as a backlog it does not happen: the audit's own finding is that local patches accumulate and global refactors do not, unless someone schedules them. So M6 is a named stage with ordered missions and gates, like M4.

Two items that look like M6 are deliberately placed elsewhere:

- **Terminal selection and copy is M4.7**, not M6. xterm.js has selection and ⌘C natively, so its absence on the GPUI line is a parity gap and a daily-driver blocker, and it belongs under M4's exit gate.
- **Hook-based session status (#347, feature 52) is M6.1, the first M6 mission** — not pulled forward onto `main`. `main` is feature-frozen (plan decision 11, 2026-08-20): Jason will not carry two versions; the rewrite finishes, then the switch happens, then the work beyond parity starts. Feature 52 was specced against `src-tauri` paths; the port mapping is `src-tauri/src/*` ↔ `crates/runner-backend/src/*`.

## Landing rule

Everything lands on `gpui-nightly`. `main` is feature-frozen as of 2026-08-20 (plan decision 11): no new features, no ports back, one line of work until Phase 6 promotes `gpui-nightly`. Consequences: the M5 watermark records where `runner-backend` diverged from `src-tauri/src` and the diff is allowed to stop being adapter-shaped from M6 on; new migrations (the indexes in M6.2) are allocated on `gpui-nightly` and are deferred to cutover unless the M1 A/B gate (Tauri build and GPUI build alternately opening the same `runner.db` copy) is re-run to prove the bridge still works.

Sequencing with M4.6b (shell split): nothing in the app crate starts until it lands — it churns `main.rs`, `mission_workspace.rs`, `sidebar.rs`. Backend missions have no file overlap and can run alongside.

## Tier 1 — the missions

Ordered by felt impact in the daily two-agent peer-coding loop (Claude Code + Codex in one mission, human typing into both PTYs) against cost.

### M6.1 — Hook-based session status (#347 reopened 2026-08-20; spec `docs/features/52-hook-based-session-status.md`) — L, backend + status consumers, first M6 mission

The one structural fix on the coordination seam. Jason's 2026-08-01 won't-do is reversed: a reliable status detector needs the agents' own turn signals; the byte-flow `IdleDetector` (`session/pty_runtime.rs:37` 750 ms silence, `:49` 500 ms `RESIZE_GRACE`) cannot distinguish a permission prompt, a question, and an idle prompt — they are the same bytes-then-silence — and stays only as the fallback tier.

Scope as specced: spawn-scoped injection, nothing installed in the user's configs — claude-code via `--settings <file-or-json>` carrying a runner-generated hooks config; codex via its hooks system (managed `CODEX_HOME` mirror or `-c` overrides; re-verify against current codex-cli, the 0.145.0 findings are from July). Loopback receiver with per-session bearer token riding the existing env injection, fail-open handler. Per-runtime normalizers → `working` / `waiting` / `done` (+ `interrupted`); `RunnerStatus` gains Waiting and Done with a confidence tier (`hook` vs `heuristic`); freshness decay degrades to the heuristic instead of freezing. Phases: receiver + model → claude-code adapter → codex adapter → interrupt inference + decay.

What it retires downstream, confirmed per phase as it lands: the silence threshold and resize grace become fallback-only; turn-boundary delivery in the router (`DeliveryGate` / `SessionOutbox`, the 80 ms cooldown thread at `router/mod.rs:1179`, the 30 s reconciliation re-nudge at `:844`) keys off `done` instead of inferred idle. What it does **not** retire: `CLAUDE_LAUNCH_GATE_GRACE` 1500 ms, `SUBMIT_DELAY` 80 ms, the 120 ms paste sleep (`output.rs:449`), `RECENT_LOCAL_INPUT_WINDOW` 2 s — those cover input readiness of the agent's TUI, not status, and the renderer swap did not change them. Consumers ready today: sidebar attention rollups (needs-you tier), mission router nudge gating, blocked-inbox indicator, `mission_status` pending asks.

### M6.2 — Backend hygiene bundle — S/M each, one mission

All confirmed at file:line on 2026-08-20.

- `synthesize_wake_busy` (`session/manager/mod.rs:999-1013`) holds the per-session mutex across `try_append_with_retry` (flock + up to 8×5 ms sleeps, `:273-283`); `record_output` (`output.rs:894-901`) needs the same mutex for every 8 KB PTY chunk. Two agents writing `runner_status` to one `events.ndjson` make contention steady state, and the pane you are typing in stutters from the *other* agent's log traffic. Append outside the guard, re-take to set activity. **S — first.**
- `ops::mission::start` (`ops/mission.rs:220-286`) holds a DEFERRED tx across `create_dir_all`, the roster sidecar, `EventLog::open` (flock + tail repair), and two appends; rollback swallows errors with `let _ =` at seven sites. `transaction_with_behavior(Immediate)` (already used at `:1609`) or move filesystem work outside the tx with compensating cleanup. **M.**
- `repo/node.rs:87-92 list_with_repair` takes an IMMEDIATE tx and runs 5+ full scans on every sidebar read. Repair at startup and after structural mutations only. **M.**
- Indexes on `sessions(mission_id)`, `missions(crew_id)`, `nodes(type, ref_id)` — five indexes exist today, none of these; migration allocated on `gpui-nightly`, deferred to cutover unless the A/B gate is re-run (landing rule above). `ops/slot.rs:168-177` one `runner::get` per slot — dedupe. **S.**
- `event_bus/mod.rs:380-405` accepts a lowercase ULID in `inbox_read` (parse is case-insensitive, compare is raw bytes) and pins the watermark above every future event. Normalize through `parse::<Ulid>()?.to_string()`. Only a third-party writer triggers it today; `cli/src/msg.rs:174` emits uppercase. **S.**
- `inject_direct_stdin` (`output.rs:304-313`) `Condvar::wait` with no timeout; only direct chat (`surfaces/chat.rs:515`, `UserInputMode::Inline`) can park on it — mission panes use `Queued`. `wait_timeout` + error. **S.**
- Doc-comment fixes: `runner-core/src/event_log/ulid.rs:69-72` claims `increment()` rolls to the next ms (it returns `None` → hard error); `session/runtime.rs:145-149` claims the stop flag unblocks the reader (checked only at loop top; `runtime.stop` forcing EOF is what works). Stale-cursor guard in `event_bus/mod.rs:310` (reset when `offset > file_len`; latent because `mission_reset` unmounts first). **S.**
- Test: two `EventLog` instances with skewed ULID floors appending concurrently — the app-plus-CLI claim that `log.rs:178-181 raise_floor_from_str` exists for; the only concurrent test today shares one `Arc<EventLog>`. **M.**

### M6.3 — Mission feed incremental append — M, app crate, after M4.6b

`mission_workspace.rs:1060-1064` pushes then `sort_by`s the whole event vec and re-runs `rebuild_event_projection` (`group_feed_blocks` + `project_asks` from scratch) on every append; render clones `feed_blocks` (`:2776`) and materializes every block into a plain `overflow_y_scroll` div (`:2801-2810`). O(n) per event, and two chatty agents append fast. Insert in ULID order (events arrive nearly sorted), project incrementally, render with `uniform_list` / `list(ListState)`. No list in the app uses `uniform_list`; convert the feed and leave the rest unless felt.

### M6.4 — `docs/arch/arch.md` rewrite — S, after M4.6b

Says the app is Tauri + xterm.js (`:7`, `:106`, `:888`) and that "messages do not trigger router actions" (`:630`) while `router/mod.rs:535` dispatches `message_nudge` into the target PTY. Agents read this file as ground truth when planning missions, so it actively misleads. Also record the fsync decision (below) and the landing rule.

### M6.5 — `RESIZE_SETTLE_MS` experiment — S, ten minutes, any time

`session/manager/mod.rs:58` (175 ms trailing debounce) absorbed xterm's fit-addon resize storms; GPUI drives resize from layout and may already coalesce. Runtime-tunable via `set_resize_settle_ms`: set 0, drag a window with Claude Code running, watch for repaint garble; keep, lower, or move to the app layer. Everything else in the timing-constant inventory is agent-TUI-side and survives the renderer swap.

## Tier 2 — schedule explicitly, post-cutover

- **Typed PTY event transport** — M, backend. `output.rs:890` base64-encodes every 8 KB chunk into a `serde_json::Value` `AppEvent` broadcast to five subscribers (`main.rs:269/306/359`, `app_store.rs:140`, `terminal.rs:473`), four of which string-match the name and drop it; only `TerminalBridge` decodes (`terminal.rs:323`). No coalescing. Byte channel keyed by session, or `Arc<[u8]>` payload with per-name routing. The sustained-throughput ceiling when both agents paint TUI frames.
- **Router delayed-work queue** — M (L if the two in-flight flags unify). Five hand-rolled sleep-and-check timer threads (`router/mod.rs:844`, `:1000`, `:1118`, `:1151`, `:1179`) and `DeliveryGateState.in_flight` vs `SessionOutbox.submit_in_flight` reconciled via an event plus the cooldown thread. Sequence **after** M6.1 — hook-driven `done` may remove half of it.
- **`Runtime` enum + `Error` enum** — L. Runtime names are bare strings across 24 non-test files, 30+ dispatch sites (`router/runtime.rs` alone has 48 occurrences of `"claude-code"`). Fights decision 2 until `main` is gone; one mechanical commit at or after cutover.
- **Glyph path pre-parse** — M, opportunistic. `terminal/glyphs.rs:434-436` re-tokenizes path strings per cell per frame, twice (validate + paint); 59 of 80 box-drawing codepoints are on that path. Nanoseconds against quad submission; fix when next in the file.
- **Render-time `.expect()` → early return** — S. Thirteen invariant asserts in render paths (`crews.rs:2047`, `panes.rs:717`, `sidebar.rs:2174`, `mission_workspace.rs:4709`, `glyphs.rs:448/598` on the paint path) are the app's crash surface. The one production `.unwrap()` (`mission_feed.rs:131`) is guarded.
- **Theme scale** — M. `theme.rs` is an `AtomicU8` plus free functions with 16 colour tokens; every spacing and size is a literal at the call site. Add tokens and thread them through as surfaces are touched, not as a sweep.
- **Test determinism** — M. 32 `thread::sleep` sites in `session/manager/tests.rs` and `router/tests.rs`, ~11 sleep-then-assert-negative; ×10 `CI` multiplier in both; `CLAUDE_LAUNCH_GATE_GRACE` zeroed under `cfg(test)`. A clock abstraction turns them into condition waits.
- **Soak mission** — verification, not code: spawn / kill / resume / fork in a loop for an hour; watch for leaked threads, stuck Busy, stalled panes.
- **Comment audit** — periodic mission. Comments are 15–25 % of lines and mostly good, but many still describe React / xterm.js / Tauri; treat agent-written comments as untrusted and re-verify against code.

## Not doing, and why

- **fsync on the event log.** Page-cache durability is the right trade for a local tool; the cost is a hard power loss mid-append, and the log's tail repair already handles a torn line. Recorded as a decision in `arch.md` under M6.4.
- **Renderer damage tracking beyond M4.6b.** PTY bursts are already coalesced (`try_recv` drain), non-terminal routes are gated (`route.terminal_visible()`), and gpui's `LineLayoutCache` reuses shaped lines; M4.6b's per-entity notify removes the remaining whole-window re-render. A dirty-row set out of alacritty is L and nothing currently felt justifies it.
- **A quit path.** The audit listed its absence as a defect; it was refuted — `stop_running_sessions_on_quit` (`bootstrap.rs:194-209`) is wired from ⌘Q, window close, and post-`run()`, kills the process group (SIGHUP → SIGKILL), and joins the forwarder (`lifecycle.rs:96-97`). SIGHUP + the startup orphan sweep is the crash fallback. Only the `OutputStream` doc comment is wrong (M6.2).

## Gates

- Each mission: `make verify` green; reviewer clean on the working-tree diff; backend missions verified by their ported test suites plus the two-instance `EventLog` test once it exists; app missions daily-driven.
- M6.1 specifically: status pills in sidebar and feed switch to hook-sourced for claude-code and codex with the `IdleDetector` demonstrably taking over when a hook is disabled; no nudge regressions in a two-agent mission over a full working day.
- M6 does not gate Phase 6 cutover. Tier 1 runs during the daily-drive clock; anything in it that proves to be a daily-drive blocker is promoted into the cutover criteria explicitly in `plan.md`, not assumed.

## Provenance

Audit and verification record: memory repo `projects/runner/technical_and_product_audit_2026-08.md` (raw audit, with addendum) and `projects/runner/post_audit_worklist_2026-08.md` (verification verdicts). Two independent read-only passes on 2026-08-20 — backend crates and GPUI app — confirmed, partially confirmed, or refuted every claim; this document carries only what survived.
