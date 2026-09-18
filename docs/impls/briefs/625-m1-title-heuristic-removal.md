# 625 — Mission 1: remove the title-spinner status heuristic

Jason requested a codex peer crew mission on 2026-09-17. Work in `/Users/jason/repos/yicheng47/runner` on branch `chore/625-title-heuristic-removal`, whose tip is this brief on top of `main` (`be38a8c`). Build on top of it; do not rebase, squash or create another branch.

Read first: this brief; issue [#625](https://github.com/yicheng47/runner/issues/625) (`gh issue view 625`), which is the spec; `docs/arch/arch.md` §5.10; "Source precedence and migration" in `docs/features/archive/347-hook-based-session-status.md`; slice 6 in `docs/impls/archive/347-hook-status/plan.md`.

## Ownership and authorization

The coder owns implementation and checks; the reviewer waits for an explicit Runner handoff, then audits the whole working-tree diff against the branch tip. Iterate through Runner until no must-fix findings remain. No additional crew, nested subagents, new checkout or worktree. Stop at a clean working-tree review with every change uncommitted: no commit, push or PR. The Claude session that started the mission lands it.

## Why

Status has two layers: byte activity, the permanent baseline, and the hook adapters on top. Title-spinner classification is neither. It arms on a braille-prefixed title and then overrides byte activity, reading presentation Runner does not control. The 347 spec named its removal condition: every runtime that animates its title has a hook adapter. Claude Code and Codex have had one on macOS since #589 and on Windows since #610 (PR #627, smoke passed 2026-09-16). After this mission every session without a working hook bridge runs on byte activity alone, labelled estimated as today.

## Deliverable

**1. runner-terminal (`crates/runner-terminal/src/terminal.rs`).** Delete `classify_title`, `TitleStatus`, and the block in the terminal event thread that feeds `title_status.observe` into `core.sessions.report_declared_status`. Everything else on the title path stays byte-identical: `sanitize_title`, `provider_title`, the #587 display and persistence of terminal-provided titles, and their tests (`shell_titles_keep_braille_without_persistence` stays). Delete `title_classifier_only_recognizes_a_leading_spinner`, `title_status_arms_only_on_a_spinner_and_ignores_blanks`, `title_status_debounces_ten_classifications_per_second`, and replace `recorded_title_status_transitions` as described in item 3.

**2. runner-backend session.**

- Remove `SessionManager::report_declared_status`, the `SessionRuntime::note_declared_status` trait method with its default, the `PtyRuntime` implementation, and the `FakeRuntime` implementation in `session/manager/tests.rs`.
- The PTY handle's `status_tx` exists only for declared status; its reap-time release is commented "Release the retained title sender". Remove the field, its construction and that release. EOF must still disconnect the forwarder: once the reader thread and the idle monitor drop their senders, the manager's `recv_timeout` must see `Disconnected`, exactly as today.
- Remove `SessionState::title_status_armed`, its `is_empty` check and both resets in `lifecycle.rs`.
- In `note_forwarder_transition`, `matches!(source, "forwarder" | "title")` becomes `source == "forwarder"` in both places, and the guard `source == "forwarder" && session.title_status_armed` goes. No other source changes behaviour: `forwarder`, `input-submit`, `input-interrupt`, `input-escape`, `agent`, `wake`, `hook`, `spawn`, `fork` and `resume` keep exactly their current gating.
- Tests: delete `title_status_suppresses_only_armed_sessions_forwarder_transitions`, `rejected_declared_status_leaves_byte_detection_active`, `declared_status_uses_existing_direct_and_mission_consumers` in the manager tests, and `declared_status_updates_detector_and_releases_sender_at_eof` in `pty_runtime.rs`. Before deleting the last one, confirm that another test still proves the output stream disconnects after the child exits (`spawn_exit_seven_records_exit_code` drains until `Disconnected`); if none does, keep that half as its own test. In `hook_status_owns_activity_until_teardown_without_changing_submit_or_wake`, the loops over `["forwarder", "title"]` test `forwarder` alone, and the `baseline` session's `"title"` Busy assertion uses `forwarder`. In `assert_status_uses_existing_direct_and_mission_consumers`, drop the `"title"` arm, the `push_status_from(…, "title")` pushes (the forwarder pushes after them already cover the same guard) and the final `title_status_armed` assertion.

When you are done, this search over `crates/` returns nothing: `TitleStatus|classify_title|title_status_armed|declared_status|source: "title"|"forwarder" \| "title"`. The `"title"` keys in `mission_start` payloads and the `title` columns are unrelated and stay.

**3. Fixtures prove the byte baseline alone.** Keep `crates/runner-terminal/fixtures/codex-title-working.ndjson` and `claude-session.ndjson` with their snapshots. Replace `recorded_title_status_transitions` with a replay of each fixture's output timing through the byte idle detector: `on_bytes_at` at every data event, `tick_at` before each event and once more after the last one plus the threshold, collecting the transitions. `IdleDetector` is private to `pty_runtime.rs`, and runner-backend cannot depend on runner-terminal, so put the test in `pty_runtime.rs`'s tests and read the two files by path from `CARGO_MANIFEST_DIR/../runner-terminal/fixtures`. Parse only `ms` and a nonempty `data` from each line with `serde_json`, skipping the header line; keep `IdleDetector` private. Run it, then pin the exact transitions you observe. The Codex fixture must show byte activity staying Busy through its animation tail, where no gap reaches the 2 s threshold, and turning Idle only after its last output; state that in an assertion message. That test plus the search above is the proof that a braille title alone no longer produces Busy while byte activity still does.

**4. Docs, in the same diff.**

- `docs/arch/arch.md` §5.10: the opening no longer says the detector combines title-spinner classification with byte activity. The source list loses the `title` item, and `forwarder` loses its "until title detection arms" and title-guard sentences. "TRAE remains on estimated terminal activity and title status" becomes estimated byte activity. The Copilot clause about the braille prefix goes, since every fallback is byte activity now. "Title and byte activity remain the fallback" becomes "Byte activity remains the fallback".
- `docs/impls/archive/347-hook-status/plan.md`: the slice 6 table row and its section record that #625 removed title classification on both platforms after #610 validated Windows hook coverage, and that byte activity is the only baseline.
- Leave every other archived record and `docs/features/archive/610-windows-hook-status.md` as history. The driving session writes the #625 record at landing.

Out of this mission: the mission-slot spawn seed (#623), shell process detection (#586), the idle threshold and resize grace, anything on the hook adapters, and title display.

## Verification

`cargo test --locked -p runner-backend -p runner-terminal -p runner-app --no-fail-fast --profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check`, `git diff --check`, and the search from item 2. Report each command's exit code in the handoff.

## Handoff

The coder messages the reviewer with the summary, changed files, the pinned fixture transitions, and the checks with exit codes. The reviewer audits the diff against this brief and replies with must-fix findings first, with file:line pointers. When no must-fix remains, the reviewer posts `NO REMAINING MUST-FIX ISSUES` to everyone and both slots stand by.
