# 459 — Rekey a claude-code session when `/clear` changes its identity

Tracking issue: [#459](https://github.com/yicheng47/runner/issues/459). Bug, P1.

## Problem

`/clear` inside a claude-code pane ends the current conversation and starts a new one with a new UUID. Runner never learns the new id: for claude-code the key is pre-assigned at spawn (`--session-id <uuid>`, `crates/runner-backend/src/router/runtime.rs:687`) and nothing re-captures it afterwards — `capture_agent_session_key` (`crates/runner-backend/src/repo/session.rs:316`) is write-once by design (`agent_session_key IS NULL` guard) and is only ever called by the codex rollout watcher. Result: stop/resume runs `--resume <old-uuid>` and silently restores the pre-`/clear` conversation; fork (`--resume <old-uuid> --fork-session`) forks the pre-clear history the same way.

## Investigation findings (verified 2026-09-01)

- **Relaxing the SQL guard alone fixes nothing for claude-code.** No code path calls capture after spawn for this runtime; the guard change without a detection mechanism is dead code.
- **The codex watcher shape does not transfer.** `codex_capture.rs` attributes rollouts to panes primarily by pid-held file descriptors (`lsof`). claude-code does **not** hold its transcript `.jsonl` open — checked three live claude processes, zero open fds under `~/.claude/projects/` — so a transcript-directory watcher can only attribute by cwd + timing, which is ambiguous whenever two panes share a cwd (routine for split chats). Rejected.
- **Runner already injects `--settings` into every claude-code spawn** (`claude_settings_args`, `router/runtime.rs:160`, currently the static `{"tui":"fullscreen"}`), skipped only when the runner row's own args already carry `--settings`.
- **`SessionStart` hooks give exact attribution.** Verified empirically with `claude -p … --settings '{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"cat > /tmp/…json"}]}]}}'` — the hook command receives on stdin:

  ```json
  {"session_id":"e279062d-…","transcript_path":"/Users/…/e279062d-….jsonl","cwd":"…","hook_event_name":"SessionStart","source":"startup"}
  ```

  `SessionStart` fires on `startup`, `resume`, `clear`, and `compact` sources. On `/clear` it fires inside the same claude process with the **new** `session_id` — the hook runs in the pane whose identity changed, so attribution is exact even with same-cwd siblings.

## Design

Per-spawn `SessionStart` hook reports the session identity to Runner through a watched drop directory.

1. **Settings injection** (`router/runtime.rs`): `claude_settings_args` becomes per-spawn. Alongside `"tui":"fullscreen"`, inject a `hooks.SessionStart` entry whose command writes stdin to a per-session drop file, e.g. `cat > "<app_data_dir>/session-keys/<runner_session_id>.json"`. The command must stay silent on stdout (SessionStart hook stdout is appended to the conversation context) — `cat` with stdout redirected qualifies. Keep the existing skip when the runner row already passes `--settings` (same tradeoff as fullscreen today; note it in the plan). The `expect("static Claude settings must serialize")` and its tests assume a static object — rework for the per-spawn shape.
2. **Drop-dir watcher** (runner-backend): watch `<app_data_dir>/session-keys/` with `notify` (same machinery as the event logs). On create/modify: parse the JSON, require `session_id` to be a valid UUID, map the filename back to a sessions row, and rekey when the incoming key differs from the stored one. Delete the file after processing; clear leftovers at startup. Treat file content as untrusted input — malformed JSON or an unknown filename is ignored, never an error path that can wedge the watcher.
3. **Guarded rekey** (`repo/session.rs`): new `rekey_agent_session_key(conn, id, new_key, expected_row_started_at)` — `UPDATE sessions SET agent_session_key = ?2 WHERE id = ?1 AND started_at = ?3 AND status = 'running'`. The `started_at` equality keeps the stale-writer protection the existing comment defends; dropping the `IS NULL` clause is what allows a legitimate mid-session change. The existing NULL-guarded `capture_agent_session_key` stays untouched for the codex watcher.
4. **Event out**: on an actual rekey, emit `SessionUpdatedEvent` so the side panel's session-key display refreshes. A report whose key equals the stored key is a no-op (this is the common case: `startup` and `resume` sources re-confirm the assigned key and double as self-healing).

Fork staleness needs no separate fix: `spawn_fork` reads the row's `agent_session_key`, so a fresh key fixes it automatically.

## Non-goals

- Codex parity: `/new` in a codex pane has the same class of exposure, and the 30 s post-spawn watcher won't see it. Out of scope — file a follow-up issue instead of widening this change.
- PTY/grid scraping of the `/clear` command (rejected in #455) and transcript-directory watching (attribution, above).
- Any UI work beyond the existing `SessionUpdatedEvent` refresh.

## Verification

- Unit tests: settings-args composition (hook entry present, per-spawn path, runner-supplied `--settings` still skips injection); `rekey_agent_session_key` guards (`started_at` mismatch rejected, stopped row rejected, differing key on a running row accepted); drop-file handling (valid report rekeys once, same-key report emits no event, malformed JSON and unknown filenames ignored).
- Headless hook check is fine (`claude -p … --settings` as above); do **not** launch the Runner app — the human smoke-tests the real flow: `/clear` in a claude pane → side-panel key flips to the newest transcript; stop/resume restores the post-clear conversation; fork after `/clear` forks post-clear history.
- `cargo test -p runner-backend`, `make clippy`, `make fmt`.
