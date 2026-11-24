# Fix stale Antigravity summary titles reverting session names

## Starting point

- Branch: `fix/agy-stale-summary-title`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-agy-stale-summary-title`
- Base: `origin/main` (`037cae4f`)
- Keep the root checkout on `main` and use this worktree's own Cargo `target/` (never share `CARGO_TARGET_DIR`).
- Sibling in-flight worktrees (do not touch or assume merged): `.worktrees/feat-1235-remote-mission-roles`, `.worktrees/fix-runner-ansi-blue`, `.worktrees/feat-1342-unified-markdown-renderer`, `.worktrees/fix-1395-remote-project-drag`, `.worktrees/test-flaky-process-probe-tests`.

## Problem and outcome

Antigravity CLI (`agy` 1.5.16) runs background summary reconciliation (`store_client.go` / `reconcile_summaries.go`) whenever any `agy` process starts, including Runner's 15-minute macOS `agy -p /usage` probe and any newly launched Antigravity chat or mission session. When `jetbox_summaries_proto.pb` holds an older snapshot of an active conversation, reconciliation overwrites that conversation's row in `~/.gemini/antigravity-cli/conversation_summaries.db` backwards in time (observed live: `title="refactor"`, `step_count=333`, `last_modified_time="2026-11-24 11:28:24.867079+00:00"` rolled back to `title="refacotr"`, `step_count=201`, `last_modified_time="2026-11-24 06:54:13.872081+00:00"`).

Runner's `SummaryTitleSync` polls `SELECT title FROM conversation_summaries WHERE conversation_id = ?1` every 400 ms without checking `last_modified_time` and without coordinating against live `OSC 0/2` titles (`runner hook title`). As a result, a rolled-back summary row immediately overwrites `sessions.live_title` and `TerminalModel.title` with the stale name, oscillating between the stale and current names every time reconciliation runs versus when the live session writes a fresh summary. Since `#1442`, `repo::session::set_live_title` also clears a manual Runner `sessions.title` whenever `live_title` changes, so a stale summary rollback can wipe a manual Runner rename as well.

Intended outcome: `SummaryTitleSync` never applies a summary row older than what Runner has already seen for that `conversation_id` (either from `conversation_summaries.db` or from a live `OSC 0/2` title on the same conversation), while preserving async first-turn titles, `/rename`, `/resume` conversation switches, and `#1442`'s latest-name-wins rules.

## Read first

- `crates/runner-daemon/src/runtimes/antigravity/agy_capture.rs` (`SummaryTitleSync`, `read_summary_title`, `run`, `persist_title`)
- `crates/runner-daemon/src/session/manager/terminal.rs` (`Host::set_live_title`, `apply_provider_title`, `persist_live_title_for_generation`)
- `crates/runner-terminal/src/terminal/model.rs` (`Event::Title`, `ProxyEvent::ApplySessionTitle`, `ProxyEvent::SyncSessionTitle`)
- `crates/runner-daemon/src/repo/session.rs` (`set_live_title`, `set_title`)
- `crates/runner-daemon/src/session/manager/tests/antigravity.rs` (`antigravity_summary_title_updates_attached_terminal_metadata_and_preferred_title_after_osc_title`)

## Deliverables and boundaries

- Extend `read_summary_title` in `agy_capture.rs` to read `last_modified_time` alongside `title` when the column exists in `conversation_summaries` (fall back cleanly when a test fixture or schema omits `last_modified_time`).
- Parse `agy`'s timestamp values into `chrono::DateTime<Utc>` instead of comparing strings lexicographically: real `conversation_summaries.db` values use Go SQLite text format with variable fractional-second precision (for example `2026-11-24 11:01:36.82741+00:00`, `2026-11-24 11:28:24.867079+00:00`, and Go zero time `0001-01-01 00:00:00+00:00`), and RFC 3339 `T`/`Z` forms should also parse cleanly.
- Track the highest observed `last_modified_time` per `conversation_id` inside `SummaryTitleSync` (including from `SummaryTitleSync::for_launch` and `pending_baseline` resolution). Ignore any polled row whose `last_modified_time` is strictly older than that watermark for the same `conversation_id`. Reset per-conversation tracking cleanly when `active_key` switches to a different `conversation_id`.
- Coordinate `SummaryTitleSync` with live `OSC 0/2` titles (`Host::set_live_title` / `Event::Title`) so a stale `conversation_summaries.db` row whose `last_modified_time` predates a live `OSC 0/2` title emitted for the same `conversation_id` cannot overwrite `live_title`, `TerminalModel.title`, or clear a manual Runner `sessions.title`. Scope any `OSC 0/2` freshness floor to the active `conversation_id` so switching to an older conversation (`c1` → `c2`) still adopts `c2`'s summary title.
- Keep `#1442` latest-wins semantics intact: a genuinely newer `last_modified_time` summary title or a new live `OSC 0/2` title still updates `live_title` and replaces an earlier manual Runner `title`, while unchanged or rolled-back summary rows preserve the current name (including any manual Runner `title`).
- Add regression tests in `agy_capture.rs` and `session/manager/tests/antigravity.rs` covering: variable-precision timestamp parsing, backwards `last_modified_time` rollback rejection (both unnamed chats and chats with a manual Runner `title`), live `OSC 0/2` `/rename` protected against an older summary row yet still updated by a genuinely newer summary row, conversation switch from `c1` to an older `c2`, and schema fallback when `last_modified_time` is absent.
- Non-goals: do not change `crates/runner-daemon/src/runtimes/antigravity/usage.rs` (macOS/Linux `agy -p /usage` probing is out of scope), do not read `annotations/*.pbtxt` or mutate any `agy` files, and do not change non-Antigravity runtime title behavior. No user-guide changes are expected.

## Validation and handoffs

- `coder`: implement the fix and tests, run focused `cargo test -p runner-daemon` and `cargo test -p runner-terminal`, stress-loop the relevant polling/integration tests (`cargo nextest run -p runner-daemon --stress-count 20 -E 'test(antigravity) | test(agy_capture)'`), and verify `make clippy` and `make fmt` check. Fold this brief into an implementation commit before final push.
- `reviewer`: review each candidate commit before QA and before push; add `reviewed` and name the reviewed head SHA in a PR comment once clean.
- `qa`: acquire `script/qa-lock` before building/launching the dev app, preserve pre-existing dev sessions untouched, and run targeted macOS live checks against the reviewed candidate commit:
  1. Start a bounded Antigravity chat in a fresh `/tmp` folder (inexpensive model, low effort, tiny prompt), rename via `/rename` twice (A → B), trigger `agy -p /usage` or a second bounded Antigravity launch, and confirm sidebar row and pane header stay on B without reverting (`runner-dev session show <id> --json` + screenshots).
  2. Rename that chat manually in Runner to C, verify C survives a subsequent turn/probe, then run `/rename D` in `agy` and verify D replaces C (`#1442` regression check).
  3. Record results, candidate commit, evidence paths, and platform limits (macOS tested; Windows/Linux covered by CI tests) in `docs/tests/runs/agy-stale-summary-title-mac.md` and summarize in the PR description.

## Authorization

Approved crew: `neon trio` (`coder`, `reviewer`, `qa`; 3 agents). The crew may commit, push `fix/agy-stale-summary-title`, open a pull request against `main`, manage PR labels (`reviewed`, risk label, `needs-live-test`), and drive CI green; do not merge.
