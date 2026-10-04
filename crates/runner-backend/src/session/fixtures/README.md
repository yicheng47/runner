# Session state replay corpus

Phase 0 of #791 captures current behavior before the reducer. Each runtime directory contains scripted, sanitized NDJSON scenarios; matching compact text goldens live in `expectations/<runtime>/<name>.txt`. The harness still computes the full normalized JSON timeline before rendering it. No CLI, account, application data or real home is used. The runner uses fake input writes, temporary hook/transcript files and a migrated in-memory connection without an r2d2 pool.

The first line identifies the runtime, scenario, covered rule rows, Motivation bugs and known-wrong bugs. Subsequent lines carry monotonically ordered `ms` timestamps and events: PTY output (including OSC titles), silence ticks, input bytes, composer observations, hook reports, transcript records, bridge failure, delivery reservation/finish/wake, completion/viewing, exit/reattachment, guarded keys and history probes. All prompts, replies, paths and identifiers are throwaway values. Hook/transcript shapes come from the existing watcher tests. The alias probe uses a temporary symlink on Unix and an equivalent parent-traversal spelling on Windows against history stored under the canonical cwd, so the resume check exercises a real filesystem alias.

`manager/tests/replay.rs` feeds `IdleDetector`'s explicit-time methods, each real watcher's `drain_observations`, and the manager's status, draft, delivery, attention and key repository seams. In phase 1, those manager calls reduce events through `session/state/`, and every replay key report uses the same `report_key` / `write_key_effect` seam as production. Initial attachment and reattachment use the same synchronous install/reset implementation as production, with SQLite size persistence supplied by the test connection. Reattachment advances the row start and feed generation, restores the mission sink and creates a fresh watcher. A scoped test clock fixes monotonic time and wall timestamps. Replay forces feed drainage, representing a delivered filesystem notification, so asynchronous OS notification timing does not affect the timeline. It does not start forwarder threads or spawn/resume a CLI; the existing spawn/resume goldens cover launch arguments and file effects.

Local input follows the production PTY path: write bytes, update the idle detector and set a watcher interrupt signal when that runtime has one. The drain callback publishes its observation and ignores its source label, as the idle monitor does. It does not synthesize an interrupt StatusTransition. The separately labeled `legacy-input-escape` and `legacy-input-interrupt` scenarios exercise the manager transition seam described by rule 4; that seam is not emitted by the current PTY input path.

Every step records the current `AgentStatus`, router activity, completion latch, input quiescence, newly published status payloads, delivery notifications, gate decisions (including the remaining recent-input window) and persisted key. The final section records mission `session_status` payloads. Completion consumption and viewing are explicit inputs; `first-turn` and `first-turn-viewed` pin both unread and viewed attention paths.

The four `input-*` scenarios feed composer traces pinned byte-for-byte to #794's recorded terminal replay expectations. `runner-terminal/tests/input_state_replay.rs` independently replays the recordings through the real grid and tracker to verify those producer observations.

## Coverage

All five runtimes have first turn, tool use, approval approved/denied, question answered, Esc during reply/tool, Ctrl+C, approve then cancel, API error, compaction, clear/new then resume, missing history, relaunch, idle crew delivery, typed draft then delivery and synthetic bridge failure scenarios. Codex and Antigravity currently publish no approval/question snapshot; their scenarios preserve that limitation through PTY and local-input evidence. Antigravity has no compaction hook. Codex and Copilot ignore `ErrorOccurred`; their API-error scenarios preserve that limitation. Copilot compaction settles through the subsequent Stop rather than PostCompact. These are evidence boundaries of the current adapters, not new synthesized semantic events.

| Rule | Scenarios |
| --- | --- |
| 1: baseline, title and startup | `first-turn`, Codex `title-authority`, `startup-automatic` |
| 2: hook precedence and quiet output | `first-turn`, `tool-turn`, `compaction`, Codex `rollback-restart` |
| 3: bridge fallback and fresh title | `bridge-failure`, `bridge-before-evidence`, Codex `title-authority` |
| 4: legacy interrupt and provisional Esc | Claude `legacy-input-escape`, `legacy-input-interrupt` (explicit manager seam); native cancels separately use watcher evidence |
| 5: submit, typing suppression and draft gate | `first-turn`, `draft-delivery`, Claude `animated-composer`, recorded `input-*` traces |
| 6: interactions and delivery release | Claude, Copilot and pi `approval-*`, `question-answered`, `approve-cancel` |
| 7: failure timestamp and compaction | `api-error`, `compaction`, Claude `failure-compaction` and `failure-compaction-unviewed` |
| 8: completion and unread | `first-turn`, `tool-turn`, `api-error`, `compaction`, cancels |
| 9: router wake | `crew-idle` |
| 10: exit and ignored late observations | `exit`, `key-guards` |

| Motivation bug | Scenario |
| --- | --- |
| #459, #738 | Claude and Codex `clear-resume` |
| #583 | Claude `animated-composer` |
| #623, #659, #687 | `first-turn`, Claude `clear-resume`, Codex startup/title scenarios |
| #670, #736 | `relaunch`, `resume-missing`, Codex `rollback-restart`, `clear-resume` |
| #753, #766 | `crew-idle`, `draft-delivery`, recorded `input-*` traces |
| #783 | `escape-reply`, `escape-tool`, `ctrl-c` |
| #781 (known wrong) | Codex `custom-home`: the real capture-root selector ignores CODEX_HOME, and scanning that root leaves the key NULL |
| #784 | pi `escape-reply`, `ctrl-c`: the live error-shaped abort message settles Interrupted; other API errors remain Failed |
| #785 | Claude `alias-resume`: history stored under the physical cwd is found through the alias spelling |
| #786 | Copilot `clear-resume`: root SessionStart replaces the persisted key for the new conversation |

`exit` tears down the watcher, then separately delivers already-in-flight Working and Ready snapshots through the manager observation seam. Both leave lifecycle, attention and activity unchanged, publish no status or mission row, and cannot rearm or consume completion.

`crash-reattach` retains unread attention across a crash while reattachment clears error/draft state, rejects old-generation reports and resumes publishing mission rows.

`key-guards` additionally pins capture-on-NULL, replacement by rekey, stale-start rejection and stopped-row rejection. Codex SessionStart keys run through the rekey guard. Coverage tests require all ten rule rows and every Motivation bug, including the remaining known-wrong markers.

## Running

```sh
cargo test --locked -p runner-backend --profile ci session_scenario_goldens
cargo test --locked -p runner-terminal --profile ci
```

PR 3 changes only the five bug goldens listed above, with pi's errorMessage supplied from QA's live abort feed. Every other golden remains byte-identical to `e264ed5d`. During the reducer refactors, a changed timeline remains a refactor defect. Never use `RUNNER_UPDATE_SESSION_GOLDEN=1` to accept reducer drift. Jason authorized the JSON-to-text format change on 2026-10-03; all 124 full timelines and their compact renderings were compared with the phase 0 checkpoint before removing the JSON files. The compact corpus is 2,148 lines / 114,768 bytes, replacing 57,650 lines / 1,378,031 bytes.

## Reading the compact goldens

The header lists runtime, name, rules, bugs and known-wrong bugs. `session_status_rows=published` means mission rows equal the ordered published rows after removing the constant `session_id="scenario-session"`; this holds for 119 scenarios. The five `crew-idle` scenarios instead list every mission row, including the mission-only wake row.

Each step begins with its `ms` timestamp. The first step shows all state fields; later steps show only changed fields, including explicit `null` resets. A timestamp alone means no state change. Non-null `result` and nonempty `delivery` events are always shown, even when identical to the previous step. Their omitted defaults are `null` and `[]`. Published rows are indented beneath the step as `state@source [status]`; no row is omitted. The constant session ID is implicit, while any different ID is printed.

Status is `lifecycle/activity/source`, followed by non-null outcome, detail, interaction and attention/exit fields. Empty interactions and null extras have their normal empty/null defaults. Known field presence and types are validated before rendering; only the exact documented defaults can disappear, and missing or mistyped known fields fail the test. A printed status replaces the whole preceding status, so omitted extras clear rather than inherit. Published rows require session ID and status fields. Mission rows may omit them; a missing mission-row status is `[<absent>]`, distinct from `[null]`, and any supplied mission ID is printed even when it equals the published default. Unknown header, step, status, observation and row fields are rendered as compact JSON; unknown status/observation fields retain their prefixes, and a removed unknown step field is `<absent>`. JSON quoting preserves whitespace and newlines within values.

With the local checkpoint object available, this command uses the same Rust renderer on every original JSON golden and asserts both full-timeline equality and byte-identical compact files, without updating files:

```sh
RUNNER_SESSION_GOLDEN_CHECKPOINT=022080e3 cargo test --locked -p runner-backend --profile ci session_scenario_goldens -- --nocapture
git diff 022080e3 --exit-code -- crates/runner-backend/src/session/fixtures/scenarios
```

Both commands exited 0 for the authorized format change. Normal replay/CI compares the full timeline's compact rendering with the text golden and does not require the local checkpoint Git object.
