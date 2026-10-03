# #791 PR 1 validation

Scope: phases 0 and 1 of the session state spec, on `refactor/791-session-state-reducer`. No application, real agent, chat, mission or account is started for validation. Jason performs the live runtime smoke test after the PR is ready.

## Phase 0

The replay corpus contains 124 scenarios: Claude Code 29, Codex 29, Copilot 22, pi 22 and Antigravity 22. The known-wrong scenarios are Claude `alias-resume` (#785), Codex `custom-home` (#781), Copilot `clear-resume` (#786), and pi `escape-reply` and `ctrl-c` (#784). The fixture README maps every rule row and Motivation bug to its scenario and describes the current adapter evidence limits.

Goldens were generated over the current seams with clock injection, test access and a behavior-preserving extraction of the synchronous handle installation/reset implementation. The final standalone replay passes without the update flag. The reviewer reported `NO REMAINING MUST-FIX ISSUES`; phase 0 was checkpointed at `022080e38ce62256562f3a5337b7da64be4492ae`.

| Command | Exit code | Result |
| --- | --- | --- |
| `RUNNER_UPDATE_SESSION_GOLDEN=1 cargo test --locked -p runner-backend --profile ci session_scenario_goldens` | 0 | Generated all 124 timelines |
| `cargo test --locked -p runner-backend --profile ci session_scenario_goldens` | 0 | Frozen-timeline comparison passed |
| `cargo test --locked -p runner-backend --profile ci` | 0 | 1,075 tests passed on the final run |
| `cargo test --locked -p runner-terminal --profile ci` | 0 | 103 passed; one existing ignored test; includes recorded composer replay |
| `cargo clippy --locked -p runner-backend --all-targets --profile ci -- -D warnings` | 0 | Passed |
| `cargo fmt --all --check` | 0 | Passed |
| `git diff --check` | 0 | Passed |

One intermediate backend suite exited 101 at the existing `codex_direct_chat_fork_captures_headless_key_then_resumes_without_watcher` 30-second assertion (the suite took 66 seconds). The focused command `cargo test --locked -p runner-backend --profile ci codex_direct_chat_fork_captures_headless_key_then_resumes_without_watcher -- --nocapture` exited 0 in 1.14 seconds, and subsequent complete backend runs exited 0 in 18.11 and 15.20 seconds. The revised phase 0 full backend runs passed in 17.56 and 17.76 seconds. No fork behavior or expected assertions changed. An initial Clippy run exited 101 on a redundant closure introduced by clock injection; the closure was corrected and Clippy passed.

The first review requested removal of an artificial local-interrupt StatusTransition and replacement of the test-handle shortcut on reattachment. Both harness artifacts were corrected before checkpointing: native cancels now publish only watcher observations, legacy manager interrupt transitions are labeled separately, and initial/renewed attachment shares production reset logic, restores the mission sink and replaces the watcher/generation. Five crash/reattachment scenarios also pin retained unread attention, cleared error/draft state and stale report rejection. Goldens were regenerated while still in phase 0. The second review found that late hook reports were unread after watcher teardown; all five `exit` scripts now also deliver explicitly labeled already-in-flight Working and Ready observations to the manager and attempt completion consumption, pinning unchanged exited state and no new publication or completion.

## Phase 1 and final checks

`session/state/` holds the private `SessionModel`, transitional `SessionEvent`, returned `Effects`, typed `StatusSource`, input types and runtime-neutral cancel constants. The twelve fields leave `SessionState`. Status, draft, completion/attention, attachment/teardown, router wake and every assigned/captured/rekeyed key writer reduce events. The manager retains IO and publication order, including the attention reread after a completion callback, revision validation after the router append, and input rollback under the stdin lock. Key reports use the same production reduction/effect seam in the replay; capture-on-NULL and rekey's running/start-time guards remain separate and unchanged. Pre-attachment key metadata does not expose an extra runtime state or status row.

Existing tests outside the corpus changed only to follow private model getters, reducer-driven test setup, relocated input/cancel types and typed sources. Their expected behavior and serialized strings are unchanged. Added one reducer test for each rule row, a wire-string round trip, and regressions for pre-attachment key visibility and cleanup. The failed-fork test now also verifies map removal. No watcher parsing or outcome rules changed; watchers still return snapshots.

The first phase 1 review found hidden key-only entries surviving rejected persistence and terminal pre-attachment failures. Cleanup now uses raw map lookup without exposing those entries through runtime/status APIs. Rejected/error key effects prune empty key-only entries, explicit forget reaches them, and mission/direct/fork spawn scopes prune them if no runtime attaches. Attachment and measurement retain their state; revealing a hidden entry retries if pruning already removed that allocation. State-to-map lock order is preserved and no lock crosses persistence. Four new tests cover rejected/error writes, explicit forget, attachment/measurement during persistence, failed direct spawn, and cancelled/failed mission spawn; the existing failed-fork test checks removal too. The focused `cargo test --locked -p runner-backend --profile ci pre_attachment` command exited 0 (four selected tests). Its initial compile exited 101 because a test referenced a nonexistent repository list helper; the assertion now queries the row count directly.

Re-review found that startup rollback can discard registered mission spawns before completion begins. `PendingMissionSpawn` now owns the cleanup guard from registration and transfers it into completion, so both later registration errors and bus-mount failures release hidden key-only entries when their pending vectors drop. The guard holds a weak manager reference and performs only map cleanup. A fifth regression drops registered pending work without completion, checking actual removal and preservation of a measured size. The focused `cargo test --locked -p runner-backend --profile ci pre_attachment` command now exits 0 with five selected tests; all required checks below were rerun after this fix.

The frozen comparison command is `git diff 022080e3 --exit-code -- crates/runner-backend/src/session/fixtures/scenarios crates/runner-backend/src/session/fixtures/expectations`; it exits 0 with no output. All 124 timelines match without the update flag. Two intermediate workspace Clippy attempts exited 101: a now-constant fallback needed `unwrap_or`, and an error-side-effect closure needed `inspect_err`. Both were corrected without suppressing a lint.

| Command | Exit code | Result |
| --- | --- | --- |
| `cargo test --locked -p runner-backend --profile ci` | 0 | 1,092 tests passed; final run 17.73 s |
| `cargo test --locked -p runner-terminal --profile ci` | 0 | 103 passed; one existing ignored test |
| `cargo test --locked --workspace --no-fail-fast --profile ci` | 0 | 1,928 passed; three existing ignored tests |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | Passed |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 | Passed |
| `cargo fmt --all --check` | 0 | Passed |
| `git diff --check` | 0 | Passed |
| `git diff 022080e3 --exit-code -- crates/runner-backend/src/session/fixtures/scenarios crates/runner-backend/src/session/fixtures/expectations` | 0 | Empty diff |

One intermediate workspace run exited 101 in three unchanged tests: `runtime_status::models::tests::process_tests::spawn_failure_timeout_and_cache_write_failure_are_contained` (catalog was None), `runtime_status::models::tests::process_tests::concurrent_refreshes_share_one_query_and_obsolete_results_are_discarded` (30-second query deadline), and `usage::tests::codex_rpc_handshake_and_timeout` (CodexNoAnswer). `cargo test --locked -p runner-backend --profile ci runtime_status::models::tests::process_tests` exited 0 (six tests, 5.06 s), and `cargo test --locked -p runner-backend --profile ci usage::tests::codex_rpc_handshake_and_timeout` exited 0 (1.63 s). The subsequent exact workspace command exited 0. These files, test expectations and timeouts were not changed.

Both phase 0 and phase 1 working-tree reviews ended with `NO REMAINING MUST-FIX ISSUES`. The final phase 1 review independently passed the five pre-attachment tests and all 175 manager tests, including all 124 frozen timelines, and confirmed the required-check logs and empty checkpoint diff. The review gate is clear for the authorized single-commit squash and PR against main with `Refs #791`. CI must pass on macOS and Windows. No merge is authorized.

PR #796's first CI run (`37111168066`) passed macOS and failed Windows Clippy: the test-only `SessionModel::hook_status_armed` getter was unused because its two callers are Unix-only tests. Its compilation guard now matches those callers with `cfg(all(test, unix))`; no warning allowance or production behavior changed. The fix is amended into the same mission commit and both platforms must pass the new head before final handoff.

## 2026-10-03 — Compact expectations

Jason authorized a test-only golden format change on #796 before merge. All 124 scenarios remain byte-identical to checkpoint `022080e3`; no production code changed. The harness still computes the full JSON timeline, then renders one text golden per scenario with state deltas, published rows and explicit mission-only rows. Unknown fields are retained generically. QA's dated live-smoke section below is preserved byte-for-byte and remains applicable, including its stated approval-coverage gaps.

The expectations shrink from **57,650 lines / 1,378,031 bytes** in 124 JSON files to **2,148 lines / 114,768 bytes** in 124 LF text files (median line: 44 characters). Mission rows equal published rows after removing the constant session ID in 119 scenarios; the five crew-idle scenarios retain their extra wake rows explicitly. Original JSON files were deleted only after equivalence passed.

Proof command: `RUNNER_SESSION_GOLDEN_CHECKPOINT=022080e3 cargo test --locked -p runner-backend --profile ci session_scenario_goldens -- --nocapture` — **exit 0**. It reads every checkpoint JSON with `git show`, asserts equality with the complete newly computed timeline, renders it with the same Rust renderer, and compares the resulting bytes with the new file. Output: `Checkpoint equivalence: all 124 full JSON timelines and compact golden bytes match`. `git diff 022080e3 --exit-code -- crates/runner-backend/src/session/fixtures/scenarios` also exits 0 with no output.

Renderer regressions cover explicit state resets, repeated result/delivery events, unknown fields at every rendered level, nonconstant session IDs and missing versus null row status. An initial generation attempt exited 101 because the renderer assumed every mission row carried a status; the mission-only wake row's absent status is now printed explicitly and covered by a regression. No expected timeline content was changed.

Compact-format review found collisions between absent or mistyped fields and their omitted defaults. The renderer now validates all known header/step/status fields and required published-row fields before omitting defaults, while mission-row ID/status absence remains valid and distinct from explicit values. A mutation regression deletes every required known field at each rendered level and changes optional values/types, including the reviewer's missing published ID, missing `exit_code` and null `interactions` cases. Each mutation either fails validation or changes the rendering. Only exact null extras and an empty interactions array are omitted. This correction leaves every compact golden byte unchanged. Final compact-format working-tree re-review: **NO REMAINING MUST-FIX ISSUES**. The reviewer independently passed all four renderer regressions and all 124 checkpoint timeline/byte comparisons; scenarios, compact bytes and the QA section remained unchanged.

| Command | Exit code | Result |
| --- | --- | --- |
| `RUNNER_SESSION_GOLDEN_CHECKPOINT=022080e3 cargo test --locked -p runner-backend --profile ci session_scenario_goldens -- --nocapture` | 0 | 124 full timelines and compact files match, also after JSON deletion |
| `cargo test --locked -p runner-backend --profile ci` | 0 | 1,096 passed, including four renderer regressions |
| `cargo test --locked -p runner-terminal --profile ci` | 0 | 103 passed; one existing ignored |
| `cargo test --locked --workspace --no-fail-fast --profile ci` | 0 | 1,932 passed; three existing ignored |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | Passed |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 | Passed |
| `cargo fmt --all --check` | 0 | Passed |
| `git diff --check` | 0 | Passed |
| `git diff 022080e3 --exit-code -- crates/runner-backend/src/session/fixtures/scenarios` | 0 | Empty diff |

## 2026-10-03 — PR 1 live smoke

Verdict: **Blocked by two required approval checks; 10 required runtime checks Passed, none Failed.** No regression was demonstrated by completed checks. This does not support a full pass or a `NO REMAINING MUST-FIX QA ISSUES` verdict. Must-fix implementation findings: none demonstrated; manual approval-wait coverage remains unresolved on both runtimes.

Jason explicitly authorized this macOS run through the `codex qa` mission, superseding the earlier implementation-only scope. Candidate: `aaf38fac05cb30d4d5118939c77514c260491fa7`, branch `refactor/791-session-state-reducer`, this worktree; initial dirty diff empty and final SHA unchanged. Implementation, frozen expectations, authentication, permission settings and global configuration were not changed. Evidence root: `/private/tmp/qa791-pr1/20261003-live/evidence/`; scratch paths are canonical `/private/tmp`. JSON captures include exact absolute CLI commands, timestamps, exit codes and outputs. `ledger.json` records both rounds' test IDs.

Environment: macOS 26.6.2 (25G83), arm64; Rust 1.97.1; Runner app 0.12.7 (dev) and CLI 0.12.7; Codex CLI 0.160.0, GPT-6 Luna/low; Claude Code 2.1.288, Haiku 4.5 with low effort requested. Codex's footer displayed Luna/low; Claude's header displayed Haiku 4.5, with no separate effort readout. An initial GPT-5.4 Mini chat stopped at its retirement notice without a conversation and was archived, excluded from the matrix.

Built with `make run`, stopped only QA's development process, then launched `/private/tmp/qa791-pr1/20261003-live/Runner QA791 Dev.app`, bundle `com.wycstudios.runner.qa791-pr1`. Both wrapper executables were byte-identical to this worktree's build (`wrapper-hashes.txt`); app SHA-256 `a7f08498f47b17bd0e667a33aa48b0b7f5e3268ab9f6e4ffbce75a76e2dd557a`. Wrapper launches and external development CLI calls removed inherited `RUNNER_*` variables. The absolute CLI confirmed `com.wycstudios.runner-dev/mcp.sock` and outside-mission identity (`wrapper-status.json`, `restored-status.json`). New chat was opened/dismissed before terminal testing. Terminal input and screenshots used native computer use, with input recordings enabled. Runner's completed-state pill says **Idle**; CLI observation says `ready`/`completed`.

### Required matrix

Evidence names are relative to the evidence root. Each Passed runtime check includes native inspection and independent CLI status; process lifecycle alone was not counted as activity.

| Check | Codex | Claude Code | Evidence and observation |
| --- | --- | --- | --- |
| Tool turn: Working → Ready/Completed | Passed | Passed | `codex-tool-active.png`, `codex-tool-working.json`, `codex-tool-complete.json`; `claude-approval-waiting.png`, `claude-approval-held.json`, `claude-tool-complete.png`, `claude-tool-completed.json`. Foreground sleep, native Working and hook using_tools, then correct marker and Completed. |
| Approval needed → approved Working → Completed | Blocked | Blocked | `codex-test-permission-context.json`, `codex-approval-waiting.png`; `claude-r2-write-boundary.png`, `claude-r2-write-completed.png`, `claude-r2-write-completed.json`. Harmless probes executed without a manual approval prompt; neither approval boundary was exercised. |
| Esc mid-reply, Interrupted, recovery | Passed | Passed | `codex-escape-retry-streaming.png`, `codex-escape-retry-status.json`, `codex-escape-recovered.json`; `claude-r2-escape-stream-retry-visible.png`, `claude-r2-escape-stream-retry-cancelled.png`, `claude-r2-escape-stream-retry-after.json`, `claude-r2-escape-stream-retry-recovered.png`, `claude-r2-escape-stream-retry-recovered-status.json`. Visible streamed text cancelled; native Interrupted, hook ready/interrupted; following marker and Completed. |
| Ctrl+C mid-reply, Interrupted, recovery | Passed | Passed | `codex-ctrl-c-streaming.png`, `codex-ctrl-c-after.json`, `codex-ctrl-c-recovered.json`; `claude-r2-ctrl-c-streaming.png`, `claude-r2-ctrl-c-cancelled.png`, `claude-r2-ctrl-c-after.json`, `claude-r2-ctrl-c-recovered.png`, `claude-r2-ctrl-c-recovered.json`. Independent active-text cancellation and normal following turn. |
| Clear/new, post-clear turn, close and resume | Passed | Passed | `codex-new-key.json`, `codex-new-stopped.json`, `codex-new-recall-result.png`, `codex-new-resume-status.json`; `claude-r2-clear-command.png`, `claude-r2-new-ack.png`, `claude-r2-new-key.json`, `claude-r2-new-stopped.json`, `claude-r2-new-recall-result.png`, `claude-r2-new-resume-verified.json`. New keys persisted across stop/resume; post-clear history and blind recall returned only new markers. |
| Idle slot: inbox nudge/read within 15 s | Passed | Passed | `qa-r2-codex-idle.png`, `qa-r2-claude-idle.png`, corresponding `*-idle-status.json`, `qa-r2-codex-nudge-completed.png`, `qa-r2-claude-nudge-active.png`, `qa-r2-idle-inbox-feed.json`, `qa-r2-inbox-latency.json`. Actual inbox_read in 2.390 s / 5.002 s; native nudges/ACKs and final hook Completed. |
| Unsent draft: blocked inbox, clear, exactly one delivery | Passed | Passed | `qa-r2-codex-inbox-blocked.png`, `qa-r2-claude-inbox-blocked.png`, corresponding `*-held-status.json`, `qa-r2-held-feed-before-clear.json`, `qa-r2-markers-before-clear.json`, `qa-r2-draft-router.log`, `qa-r2-held-feed-after-clear.json`, `qa-r2-markers-final.json`, `qa-r2-codex-held-completed.png`, `qa-r2-claude-held-completed.png`, corresponding `*-completed-status.json`. Drafts retained, Inbox waiting pill, no reads during hold, then one release/read/marker each. |
| Native Windows | Skipped | Skipped | Jason explicitly excluded Windows; CI is separate evidence. |

Copilot, pi and Antigravity were Skipped as explicitly out of scope. Additional baseline checks Passed on both: fresh ACK, blind second-turn recall, captured key and original-conversation stop/resume with correct original recall (`codex-original-stopped.json`, `codex-original-recall-result.png`, `claude-r2-original-stopped.json`, `claude-r2-original-recall-confirmed.png`, `claude-r2-original-recall-verified.json`).

### Boundaries and evidence limits

Approval reproduction: in a disposable chat under existing settings, request harmless `require_escalated` sleep on Codex or scratch-file Write followed by sleep on Claude. Required: visible approval wait and CLI interaction, manual approval, Working and Completed. Observed: direct execution and completion. Claude's scratch file contained the requested marker. No settings were changed to force a prompt; cause is not established. The Codex fixture README documents approval-observation limits; `codex-watcher-baseline.diff` compares typed-source plumbing with `v0.12.7`. The released 0.12.7 app was not run for live comparison, so this establishes neither a new approval regression nor a live baseline pass.

At approximately 09:33 UTC, native access failed with noWindowsAvailable/cgWindowNotFound while QA wrapper PID 81203 and its socket remained live (`native-window-loss.json`, `wrapper-after-window-loss.json`). Cause is unknown, without evidence of an app crash or reducer regression. After initial cleanup, Jason responded “Restore QA wrapper window.” QA relaunched the same wrapper and completed unfinished checks using a replacement Claude chat and recreated disposable crew. Initial Blocked native coverage was resolved; outage evidence remains preserved.

An initial Codex Esc probe finished after refusing a repetitive request, and one Claude text probe finished before Esc; neither counted as an interruption pass/failure. Claude's first Esc cancelled while thinking; the matrix cites the later visible-text interruption. Native paste once timed out while delivering text; subsequent input used typing and submission inspection. One question entered before resumed-terminal attachment did not land and was re-entered after inspection. The first Codex Ctrl+U cleared only text before the clicked cursor, correctly retaining the hold; Ctrl+E then Ctrl+U cleared the remainder.

All four round-two markers appear exactly once in the canonical mission directory (`qa-r2-markers-final.json`). Agents chose `qa791-Codex-received.txt` and `QA791-received.txt` rather than requested handle-based filenames; this is a test-agent instruction deviation, with correct directory and delivery established independently. Agent timestamps were not used for latency. Router hold intervals were approximately 46 s / 92 s, each followed by one InputCleared release and one inbox_read; neither draft was submitted. Automatic feed delivery while idle is unavailable on this host; no background log counted as a watch. The foreground follow-up was the absolute dev CLI, inherited `RUNNER_*` removed, with `mission feed 01M40JP0918W66T8HB8STA7M2Y --follow --json`.

### Automated checks and cleanup

| Command / gate | Result | Evidence |
| --- | --- | --- |
| `cargo test --locked -p runner-backend --profile ci` | Passed, exit 0; 1,092 tests | `backend-tests.log` |
| `cargo test --locked -p runner-terminal --profile ci` | Passed, exit 0; 103 tests, one existing ignored | `terminal-tests.log` |
| `git diff 022080e3 --exit-code -- crates/runner-backend/src/session/fixtures/scenarios crates/runner-backend/src/session/fixtures/expectations` | Passed, exit 0; empty | `frozen-diff.log` |
| Exact-candidate macOS/Windows CI | Passed | `environment.json`; [CI run 37111596905](https://github.com/yicheng47/runner/actions/runs/37111596905) |

All four direct chats and both rounds' missions were stopped/archived. Missions had zero live sessions before authorized deletion of the disposable crew and roles; only one disposable crew existed at a time. Crew deletion removes Runner mission/session metadata; feeds and archive status were preserved first (`qa-final-feed.json`, `qa-mission-archived-status.json`, `qa-r2-final-feed.json`, `qa-r2-mission-archived-status.json`). `cleanup-verification.json`, `qa-r2-cleanup-verification.json`, `claude-r2-final-archived-status.json` and `qa-r2-native-cleanup-recents.png` verify cleanup and removal from Recents. Child PIDs and remaining test conversation-key processes were absent (`qa-r2-process-cleanup.json`). Only QA's wrapper was quit; dev CLI exit 3 confirmed shutdown (`dev-after-quit.json`, `qa-r2-dev-after-quit.json`). No follower remains. Evidence, scratch artifacts, wrapper and conversation history were retained. Pre-existing objects and installed Runner were not operated.

Required unresolved coverage: both manual approval waits and approved transitions. Only this dated report remains uncommitted; no implementation edit, commit, push, PR creation or merge.
