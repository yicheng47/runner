# #766 idle Codex delivery validation

## Baseline — 2026-10-03

The four prescribed baseline rows did not reproduce the held-inbox symptom. All nudges landed without human rescue input, were read, and produced an ACK. This establishes normal delivery for these runs; it does not establish a fix or explain the intermittent report. No additional live variations were run.

Authorization and scope: [mission brief](../impls/briefs/766-idle-codex-delivery.md), phase 1. This was a narrow macOS baseline for fresh-screen Codex, filled-screen Codex, queued input during a working Codex turn, and one Claude control. It was not the full runtime smoke suite. Jason separately requested documentation of the temporary macOS app wrapper.

Candidate: `176110acd7f9ce201b805102c0e1e72d2eeae5ac` on `fix/766-idle-codex-delivery`, before any implementation changes. The initial working-tree diff was empty. During validation, only the wrapper procedure in `docs/tests/full-smoke-test.md` and this record were edited; the app executable remained the baseline build.

Environment: macOS 26.6.2 (25G83), arm64; Runner 0.12.7 development build; Codex CLI 0.160.0 with `gpt-6-luna`, low effort, standard speed; Claude Code 2.1.288 with Haiku 4.5 and low effort requested. Existing mission permission mode was `bypass`, visibly reflected by both runtimes; no permission or global configuration was changed. The test reused existing roles `smoke743-speed` and `abc` through one new crew `qa766-baseline`, with only its slot overrides changed.

Evidence root: `/private/tmp/qa766-baseline-20261003`. Native screenshots were inspected through computer use in the QA conversation. Persisted terminal recordings contain PTY output and native input events; structured CLI captures independently contain activity transitions and inbox-read watermarks. Raw evidence remains local because terminal footers can contain account usage information.

### Build and native setup

Built this worktree with `RUNNER_RECORD_INPUT_FIXTURE=/private/tmp/qa766-baseline-20261003/rec make run`. No development app was running before the test; the installed app remained running and untouched. The bare debug executable was rejected by computer use as an invalid app, so only the development process created by this run was stopped and replaced by `/private/tmp/qa766-baseline-20261003/QA766 Runner Dev.app` with bundle ID `com.wycstudios.runner.qa766-dev`.

The wrapper executable and `target/debug/Runner` were byte-identical, SHA-256 `b273eea5eae63685693f4aea36d70a0e2f8680e1df7c30aceda238ceea2d8faa`. The development CLI was installed by the initial `make run` app startup. A later phase-5 setup check established that the baseline wrapper's attempted copy of `target/debug/runner` resolved to the app on this case-insensitive filesystem; the wrapper recipe now copies the actual source CLI, `target/debug/runner-agent-cli`. Baseline external test commands used the absolute installed development CLI, which reported `/Users/jason/Library/Application Support/com.wycstudios.runner-dev/mcp.sock`. The wrapper and those commands cleared the four inherited implementation-mission environment variables. See `wrapper-verification.txt`, `dev-status.json`, `app.log`, and `wrapper-app.log` in the evidence root and the reusable [wrapper procedure](full-smoke-test.md#macos-native-control-of-the-debug-executable).

Binding by full app path did not reliably deliver keyboard input. Binding by the distinct bundle ID and clicking the visible composer before each keyboard action worked and was verified through terminal output and recorder input events. The harmless New chat dialog was opened and dismissed without creating a session.

The first Codex setup used retired model `gpt-5.4-mini`, which displayed a migration prompt before executing the goal. That mission was stopped and archived without accepting the prompt. The test crew slot was changed to the available `gpt-6-luna` model, and a new test mission ran the required rows. This setup attempt is not a delivery result. Automatic mission watching was unavailable on this host and was disclosed; explicit CLI state/feed captures were used during validation.

### Baseline matrix

Each message was posted only after the direct turn completed and native UI showed Idle with an empty composer. A nudge still held after 15 seconds would count as held. The times below measure the post-to-router Busy transition and post-to-inbox-read event; the native terminal and recording independently confirmed that the nudge landed. ACK latency is distinct from held delivery: the fresh row completed after 17.740 seconds, but the nudge had already landed immediately.

| Workflow | Result | Delivery and timing | Native pill/status |
| --- | --- | --- | --- |
| Codex fresh screen | Passed | Used `/new`, then typed `Reply exactly QA766_FRESH. No tools.`; after completion posted `QA766_FRESH_NUDGE`. Delivered; Busy after 40.470 ms, inbox read after 10.990 s; visible `ACK QA766_FRESH_NUDGE`. | No Inbox waiting pill observed; Working then Idle, hook Completed. |
| Codex filled screen | Passed | Printed 80 lines `QA766_FILL_01` through `_80` to fill the terminal, then typed `Reply exactly QA766_FILLED. No tools.`; after completion posted `QA766_FILLED_NUDGE`. Delivered; Busy after 48.690 ms, inbox read after 1.774 s; visible correct ACK. | No Inbox waiting pill observed; Working then Idle, hook Completed. |
| Codex prompt queued while working | Passed | Started bounded `sleep 20`; while visibly Working, submitted `Reply exactly QA766_QUEUED. No tools.`. Native UI showed “Messages to be submitted after next tool call,” then the queued prompt and reply. After completion posted `QA766_QUEUED_NUDGE`. Delivered; Busy after 57.860 ms, inbox read after 1.768 s; visible correct ACK. | No Inbox waiting pill observed; Working then Idle, hook Completed. |
| Claude control | Passed | Typed `Reply exactly QA766_CLAUDE. No tools.`; after completion posted `QA766_CLAUDE_NUDGE`. Delivered; Busy after 51.904 ms, inbox read after 3.509 s; visible ACK with marker and event ID. | No Inbox waiting pill observed; Working then Idle, hook Completed. |
| Native Windows | Blocked | No native Windows environment was available to this QA run; the brief assigns the later check to Jason. | No Windows UI result claimed. |
| Draft hold/clear, already-read stale nudge, reviewed-fix reruns | Skipped | Phase 5 awaits a reviewed implementation handoff. | No fix verdict claimed. |

Codex backgrounded the bounded sleep but remained Working while waiting for it; the queued input visibly reached the runtime and was consumed after the tool finished. The runtime answered the queued prompt without printing the original `QA766_WORK_DONE` marker. This did not prevent the required queue-then-idle delivery check. The fresh row read the local Runner skill before reading its inbox; later Codex rows read directly. Claude appended the event ID to its ACK. These runtime details were retained rather than changing settings to normalize them.

Structured timing details are in `baseline-metrics.json`. The four message captures are `codex-fresh-message.json`, `codex-filled-message.json`, `codex-queued-message.json`, and `claude-message.json`; `codex-queued-feed.json` includes all three Codex rows, and `claude-feed.json` includes the control. Final sessions were `activity=idle`, lifecycle Running, observation Ready from Hook with outcome Completed.

### Automated checks

| Command | Result | Evidence |
| --- | --- | --- |
| `cargo test --locked -p runner-terminal --profile ci` | Passed: 103 tests, 1 ignored | `terminal-tests.log` |
| `cargo test --locked -p runner-backend --profile ci` | Passed: 1069 tests | `backend-tests.log` |
| `git diff --check` | Passed after documentation edits | QA command record |

### Object ledger and cleanup

| Object | ID | Final action |
| --- | --- | --- |
| Test crew | `01M3ZTYXP69TXGPBBKP5NDC9YG` | Deleted under the brief's explicit cleanup authorization, after its missions were stopped and archived. |
| Retired-model setup mission / session | `01M3ZTZHZB693621GEWAQNH9BW` / `01M3ZTZHZCZ4BQHV7W9ZQXEHGH` | Stopped and archived. |
| Codex baseline mission / session | `01M3ZV1P9V23DJXCN8PE9PVZQY` / `01M3ZV1P9WN9GZWKGVYE26YJ0M` | Stopped and archived. |
| Claude control mission / session | `01M3ZVNG04JWENTBNRPEGMFFYW` / `01M3ZVNG05MV6MBZ2MWE8NS36C` | Stopped and archived. |

The archive responses recorded Completed status, stopped timestamps, and archive timestamps. Crew deletion then removed these missions' metadata, so later `mission show` returned “mission not found” and live-count verification through that command was unavailable. This consequence was disclosed. Mission event files and rosters remain under the development data directory and were copied to the evidence root. Terminal recordings, final session captures, external agent conversations, and the wrapper were retained; no history files or unrelated objects were deleted. No new role was created.

Native UI confirmed the QA rows disappeared from Recents. Only this run's wrapper was quit; process verification showed its app PID and both active test-agent PIDs gone, while the installed Runner PID remained. `ledger.json`, the stop/archive/delete captures, and `cleanup-processes.txt` retain cleanup evidence. The terminal recordings are `rec.01M3ZTZHZCZ4BQHV7W9ZQXEHGH.ndjson`, `rec.01M3ZV1P9WN9GZWKGVYE26YJ0M.ndjson`, and `rec.01M3ZVNG05MV6MBZ2MWE8NS36C.ndjson`.

No must-fix delivery issue was observed in the completed macOS baseline matrix. The reported intermittent defect remains unexplained, Windows remains unresolved, and this record provides no approval of a future fix. Additional live variations require Jason's direction under phase 1 of the brief.

## Recording replay and implementation — 2026-10-03

The coder replayed both baseline recordings through the untouched `InputTracker` in a temporary integration test, then removed the helper. Command: `cargo test --locked -p runner-terminal --profile ci --test qa766_replay_scratch -- --nocapture`, exit 0. The replay evidence is `/private/tmp/qa766-baseline-20261003/input-tracker-replay.txt` and `replay-*.json`. Both recordings ended `Idle`, with a visible composer. Fresh Codex cleared at 235846 ms, filled Codex at 507806 ms, queued Codex at 565363 ms, and Claude at 37348 ms. Codex's short `Submitted` → `Drafting` → `Idle` redraw sequence lasted at most 1 ms; Claude's lasted 15 ms. These transitions do not establish the persistent false draft reported in #766. Direct chunk replay also observes intermediate redraws; the live delivery matrix remains the independent behavioral evidence.

The coder reported this diagnosis to Jason through Runner before changing delivery behavior. Per the brief's unreproduced-baseline path, the detector and its existing golden transitions stay unchanged. The implementation logs changed hold reasons and release triggers, retains composer visibility in backend observations for diagnostics, and skips an inbox nudge when the recipient's watermark covers its newest message ID. Coalesced nudges retain the newest ID; relays still flush in order. The PR must use `Refs #766`, because the original intermittent hold was not reproduced or fixed live.

Four new fixtures were cut from the baseline recordings, with a composer seed projected from the recorded grid and only the submit redraw window retained. The seed preserves the recorded row, colors and styles; the remaining PTY frames and classified input events retain their relative ordering and timing. Unrelated screen content is omitted from the seed, and event IDs, timestamps and Claude's repeated coordination boilerplate are blanked without changing cursor commands or relevant composer text. No existing replay expectation was updated. Existing backend assertions were changed only to distinguish `LocalInputPending` from `Drafting { composer_visible }`, which previously shared `PendingInput`; both still hold delivery.

| Fixture | Original recording window | Final expected state |
| --- | --- | --- |
| `input-codex-fresh-submit` | Codex, 235622–235846 ms | Idle, visible |
| `input-codex-filled-submit` | Codex, 507573–507806 ms | Idle, visible |
| `input-codex-queued-submit` | Codex, 565134–565363 ms | Idle, visible |
| `input-claude-control-submit` | Claude, 37122–37348 ms | Idle, visible |

Backend regression checks cover deduplicated hold logs, visible/hidden draft reasons, the fallback latch and other hold reasons, the `InputCleared` release trigger, no Enter or Busy wake-up for a stale nudge, relays behind a stale nudge, a partially read coalesced nudge, and independent broadcast-recipient watermarks. Review and live phase 5 are still pending; these automated checks do not establish that the intermittent hold is fixed.

### Implementation checks

Candidate: uncommitted working-tree implementation over `176110ac` on `fix/766-idle-codex-delivery`. Logs and exact command results are under `/private/tmp/qa766-implementation-checks/`, in `check-0.log` through `check-5.log` and `results.json`. The first terminal run exited 101 because the four new fixtures lacked required grid snapshots; new snapshots were added without changing existing expectations. The first workspace Clippy run exited 101 for an unused test binding introduced when splitting hold reasons; that binding was corrected. The final results below supersede those failed attempts.

| Command | Exit code | Result |
| --- | --- | --- |
| `cargo test --locked -p runner-terminal --profile ci` | 0 | Passed: 103 tests, 1 ignored; includes all four new fixture transitions and snapshots |
| `cargo test --locked -p runner-backend --profile ci` | 0 | Passed: 1073 tests |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | Passed |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 | Passed |
| `cargo fmt --all --check` | 0 | Passed |
| `git diff --check` | 0 | Passed |

### Review correction

The first working-tree review found one must-fix deadlock in the stale-prune path: a concurrent flush could reserve delivery after another flush removed its now-stale outbox, then invoke `finish_delivery` while retaining the router state lock. The synchronous `DeliveryFinished` listener attempted to lock that same state. Both missing-outbox and empty-queue early returns now release the state lock before finishing the reservation.

The deterministic `concurrent_stale_prune_releases_reservation_without_deadlocking_later_delivery` regression pauses the first flush at reservation, advances the read watermark, prunes from a second flush, and then resumes the reservation. It requires the callback to return, confirms no stale bytes were written and no reservation remains held, and verifies a subsequent relay is submitted once. Command: `cargo test --locked -p runner-backend --profile ci concurrent_stale_prune_releases_reservation_without_deadlocking_later_delivery -- --nocapture`; exit 101 before the correction and 0 after it. Logs: `/private/tmp/qa766-race-regression-before.log` and `/private/tmp/qa766-race-regression-after.log`. The reviewer's independent reproducer and initial verdict remain in Runner and `/private/tmp/qa766-review-race-result.txt`.

After this correction, all six exact commands in the implementation-check table were rerun and each exited 0. Terminal results remain 103 passed and 1 ignored; backend results are now 1074 passed. Current logs and exact results are `/private/tmp/qa766-review-fix-checks/check-0.log` through `check-5.log` and `results.json`. The branch remains uncommitted pending re-review and live verification.

The reviewer posted `NO REMAINING MUST-FIX ISSUES` through Runner on 2026-10-03. It independently reran the concurrent stale-prune regression (exit 0) and `git diff --check` (exit 0), and its public-API reproducer passed against the corrected backend (exit 0), recorded at `/private/tmp/qa766-review-race-fixed-result.txt`. Live phase 5 remains the publication gate.

## Reviewed-candidate verification attempt — 2026-10-03

Phase 5 was requested through Runner after clean review. QA rebuilt the exact uncommitted working tree over `176110acd7f9ce201b805102c0e1e72d2eeae5ac` on `fix/766-idle-codex-delivery`. No development app was running before this attempt. The initial dirty diff and manifests, including untracked fixture files, are preserved under `/private/tmp/qa766-reviewed-20261003` as `candidate.diff`, `candidate-files.json`, `candidate-source-files.json`, and `candidate.json`.

Candidate SHA-256 identifiers: tracked dirty diff `06f1a7237c26cd9718978de3b4c7f962e518134bb1603f4ad8c7c398a65abce2`; all changed-file manifest `564558b20586f878387ed444652fd6ce7e579193bace693ee39a69f944ca0c9a`; source/fixture manifest `76005fa24165ed48f5227d719e499191a50e6e491642962abce75f411cf11aed`. Manifest digests use compact sorted JSON mapping relative paths to content SHA-256. Source/fixture hashes were rechecked after the automated validation and remained identical. QA changed only this record and the source-side CLI name in the wrapper procedure.

Environment remained macOS 26.6.2 arm64, Runner 0.12.7 development build, Codex CLI 0.160.0, and Claude Code 2.1.288. `make run` built the CLI and app successfully. Only the development process created by this attempt was stopped to replace it with `/private/tmp/qa766-reviewed-20261003/QA766 Reviewed Dev.app`, bundle ID `com.wycstudios.runner.qa766-reviewed-dev`, retaining `RUNNER_RECORD_INPUT_FIXTURE=/private/tmp/qa766-reviewed-20261003/rec` and clearing inherited mission identity. Its app executable matched this worktree byte-for-byte, SHA-256 `d78e81786941148ca85c117a29b3428b30aaa91fafb3f47f71fb426145c12cd3`. The absolute development CLI reported the expected `com.wycstudios.runner-dev/mcp.sock` endpoint and matched `target/debug/runner-agent-cli`, SHA-256 `4764f2abdd38a4562399b6f0694542c0e5e7e09f305d3839509610a6d54d3400`. Evidence: `app.log`, `wrapper.log`, `wrapper-hashes.json`, and `dev-status.json`.

### Native-control blocker

Computer use could not bind the running reviewed wrapper: both its bundle ID and exact `.app` path repeatedly returned `Computer Use server error -10005: cgWindowNotFound`. The native app inventory showed the exact wrapper running. Resetting the computer-use session and rebinding returned the same error. Finder initially exposed its Desktop, but requesting its path dialog and rebinding also returned `cgWindowNotFound`; no terminal test input was sent. Optional native `launch_app` and `listWindows` APIs were absent on this Mac host. App logs recorded a restored window, and the development endpoint was independently reachable; these are not evidence of accessible native UI. The cause of inaccessible windows is not established. See `native-control-blocker.txt` and the native tool results in the QA conversation.

No phase-5 crew, role, mission, or agent was created while native control was unavailable. The prepared scratch directories and goal/conventions files were retained. This is an unresolved environment limitation, not an observed delivery failure or a live QA pass.

### Reviewed-candidate matrix

| Required check | Result | Evidence / remaining requirement |
| --- | --- | --- |
| Rebuild and identify reviewed candidate | Passed | `make run` compiled both artifacts; byte-identical wrapper, current source hashes, and development endpoint verified. |
| Codex fresh-screen repeat | Blocked | Native wrapper window inaccessible; no live turn or nudge attempted. |
| Codex filled-screen repeat | Blocked | Native wrapper window inaccessible. |
| Codex queued-prompt repeat | Blocked | Native wrapper window inaccessible. |
| Claude control repeat | Blocked | Native wrapper window inaccessible. |
| Real Codex unsent draft holds with pill and `Drafting`/visibility log | Blocked | Requires native input and pill observation; no live result claimed. |
| Clearing draft releases once with trigger logged | Blocked | Requires the live held draft; no live `runner.log` assertion claimed. |
| Already-read held nudge never lands | Blocked | Required live read-watermark scenario was not attempted. Automated coverage passed separately below. |
| Native Windows | Blocked | No native Windows environment available; remains assigned to Jason by the brief. |

### Independent automated checks

| Exact QA command | Exit code | Result / evidence |
| --- | --- | --- |
| `cargo test --locked -p runner-backend --profile ci router::tests::nudges:: -- --nocapture` | 0 | Passed: 18 tests, including stale-nudge pruning, coalescing, recipient watermarks, and the concurrent reservation regression. `qa-nudge-tests.log`. |
| `cargo test --locked -p runner-backend --profile ci hold_logs_dedupe_retries_and_name_reasons_and_release_trigger -- --nocapture` | 0 | Passed: reason-change logging and release-trigger assertions. `qa-hold-log-tests.log`. |
| `cargo test --locked -p runner-terminal --profile ci --test input_state_replay` | 0 | Passed: fixture replay integration test, including the four recorded submit scenarios. `qa-replay-tests.log`. |
| `git diff --check` | 0 | Passed after the QA documentation update. |

These checks supplement the coder's six passing commands at `/private/tmp/qa766-review-fix-checks/results.json`; they do not replace the blocked live rows. No implementation code, tests, frozen expectations, agent configuration, or permission settings were changed.

### Attempt cleanup and gate

The reviewed wrapper could not be quit through native control, so QA sent SIGTERM only to its own process. Process verification confirmed this attempt's development app processes exited and the pre-existing installed Runner process remained. No test agents or Runner objects needed stopping or archiving. Raw evidence and the wrapper remain under the attempt's evidence root; no data was deleted.

The required macOS live matrix remains unresolved. QA cannot report `NO REMAINING MUST-FIX QA ISSUES` or approve publication from this attempt. Resume phase 5 when the exact development window is accessible through native control; use a new dated attempt while retaining this blocked record. The intermittent baseline symptom remains unreproduced, so `Refs #766` remains the appropriate issue linkage.

## Human-operated mission with CLI probes — 2026-10-03

Jason supplied development mission `01M405E2YYFTP35JGYK2MY1RM4` and explicitly authorized the coder to test it through the CLI. The coder did not start or control the app, create agents or missions, or operate a native window. The supplied mission belongs to the existing Peer coding crew, with Codex `coder` and pi `reviewer`; the test addressed only Codex. Jason's running development app and the built executable match the reviewed app SHA-256 `d78e81786941148ca85c117a29b3428b30aaa91fafb3f47f71fb426145c12cd3`; the source/fixture manifest matches the clean-reviewed candidate. The absolute development CLI reported the expected development socket, with the implementation mission's identity removed from its environment.

Local evidence is `/private/tmp/qa766-manual-cli-20261003`: before-state snapshots, posts, bounded feed snapshots, read/ACK timing calculations, delivery logs and reviewed-source comparison. The active terminal recordings are `/private/tmp/qa766-manual-rec.01M405E2Z4S1HAV1BY7ZFA3HTM.ndjson` and `/private/tmp/qa766-manual-rec.01M405E2Z7ESVD48E1A14KQZ0Y.ndjson`. No private terminal content was copied into this record. The supplied mission and installed app remain untouched apart from the explicitly requested test messages.

Both probes used development `msg post --mission 01M405E2YYFTP35JGYK2MY1RM4 --to coder` with unique markers and a bounded instruction to read the inbox, ACK and remain idle without editing files or performing other work. Codex posted its ACK to the crew channel, which also woke the existing pi slot; pi acknowledged and both slots returned idle. These CLI observations establish message receipt, but do not establish native composer contents or pill behavior.

| Probe | Result | Evidence |
| --- | --- | --- |
| `QA766_CLI_PROBE_01`, Codex initially Working | Passed, CLI receipt | Posted 06:07:32.156944Z; read watermark covered the message after 2.408 s; ACK after 7.306 s. No new hold logged. |
| `QA766_CLI_IDLE_02`, Codex confirmed Idle before post | Passed, CLI receipt | Posted 06:08:33.259544Z; router Busy after 49.212 ms, read after 4.765 s, ACK after 8.440 s. No new hold logged. |
| `QA766_EMPTY_03`, Codex Idle with empty composer in Jason's screenshot | Passed, CLI receipt with human native-state evidence | Posted 06:11:18.642373Z; router Busy after 28.153 ms, read after 2.950 s, then Idle/Completed. No new hold logged and no crew ACK broadcast; this probe requested a terminal-only ACK. |
| `QA766_DRAFT_04`, Jason confirmed unsent `draft` before post, then cleared it | Passed, native hold/pill, release/log and once-only receipt | Posted 06:13:03.493860Z; one hold logged at 06:13:03.515727Z with `Drafting { composer_visible: true }`. At 21.785 s no covering inbox read and Codex remained Idle. Jason's screenshot confirms `draft` and the Inbox waiting pill. Clearing released at 06:15:16.445208Z with `trigger=InputCleared`; one router Busy event, one covering inbox read 2.956 s later, then Idle/Completed. Jason confirmed “done, it works.” |
| `QA766_STALE_05`, held message read while the draft remains unsent, then cleared | Passed, already-read nudge suppressed | Held at 06:23:12.784832Z with visible `Drafting`; gated inbox read at 06:23:26.419085Z covered the message. At 06:23:44.238353Z the router logged one stale skip, then release with `trigger=InputQueueDrained`. No new router Busy delivery event or additional inbox read, final Idle/Completed; Jason confirmed no `[inbox]` nudge after clearing. |
| Fresh/filled/queued Codex scenario reruns and Claude control | Pending | Requires Jason's native actions and confirmation or restored QA computer use. These manual probes do not replace those phase-5 rows. |

No rescue input was sent by the coder, and no test mission was stopped or archived because this mission was supplied by Jason. Publication remains gated on the remaining required live checks.

Jason's screenshot before the third probe showed the earlier ACK exchange and an empty Codex composer displaying its placeholder. This supplies native empty-composer evidence for that probe; it does not show the mission workspace's blocked-inbox pill area. The screenshot remains outside the repository and its account-usage footer is not transcribed. `probe-03-session-before.json`, `probe-03-post.json`, `probe-03-feed.json`, `probe-03-result.json` and `probe-03-session-after.json` retain the CLI boundaries and timing under the manual evidence root. Real draft hold/clear, already-read suppression and the other required phase-5 scenarios remain pending.

Jason's second screenshot visibly confirms the real draft and the Inbox waiting pill for `QA766_DRAFT_04`, and shows the previous `ACK QA766_EMPTY_03` in the terminal. The native hold/pill component is therefore passed; the draft-clear release and stale-nudge scenarios remain pending. Raw screenshots stay outside the repository; no account-usage figures are transcribed.

Jason cleared the real draft and confirmed success. `probe-04-released-feed.json`, `probe-04-released-session.json`, `probe-04-released-log.txt` and `probe-04-released-summary.json` retain the release evidence. The hold lasted until the native clear action, with exactly one hold log and one release log. The feed shows one router delivery turn, one covering inbox read and a final Idle/Completed observation. Already-read suppression and the remaining scenario reruns are still required; this result alone does not approve publication.

Jason directed that his manual testing replace a separate QA run. QA acknowledged that it is idle and will perform no further review, app operation or live testing. Before stopping, QA verified that the source/fixture manifest remained unchanged and read the provided captures/logs, but did not complete an independent evidence assessment or edit the manual matrix. The passed human-operated draft hold/clear case remains recorded above; already-read suppression and the requested scenario reruns remain pending manual verification.

For `QA766_STALE_05`, a bounded gate-read command kept Codex working while Jason typed an unsent draft. The coder confirmed a visible `Drafting` hold and no covering inbox read before creating the temporary gate file. Codex then read the inbox once through the absolute development CLI while the draft remained present. Jason waited for the read ACK, cleared the draft without Enter, and confirmed no late inbox nudge. The recorded watermark covers message `01M406WMVJ49EY2VPBP59TZCG3`; the router logged exactly one stale skip and one release, and no new delivery turn occurred. `probe-05-*` under the manual evidence root retain the setup, boundaries, gate, feed, logs and result. This passes the required live stale-nudge scenario; the remaining baseline reruns and native Windows retain their stated gaps. No implementation changes or git writes were made.

### Human-operated filled-screen repeat

Jason typed the requested 80-line prompt directly into the reviewed Codex terminal and confirmed that the long reply finished. Before `QA766_FILLED_06`, the development CLI confirmed Idle. The message was posted at 2026-10-03T06:26:04.110543Z; one router Busy event followed after 48.618 ms, one covering inbox read after 2.875 s, and the session returned Idle/Completed with no new hold log. Jason confirmed the terminal ACK without rescue input. This passes the reviewed-candidate filled-screen repeat. Evidence: `probe-06-*` under the manual evidence root and the active input recording. Fresh-screen, queued-prompt and Claude-control repeats remain pending.

## Accepted manual verification and publication — 2026-10-03

After the filled-screen pass, Jason accepted stopping manual testing and explicitly requested a PR and merge. His manual results replace the separate QA live gate under this direction. No queued-test setup or new agent was launched after he stopped that work. The original intermittent false-draft hold remains unreproduced, the input detector is unchanged, and the PR uses `Refs #766`.

| Final reviewed-candidate check | Result | Evidence / accepted gap |
| --- | --- | --- |
| Empty-composer Codex delivery | Passed | `QA766_EMPTY_03`; native empty-composer screenshot, one delivery/read and visible terminal ACK. |
| Filled-screen Codex delivery | Passed | `QA766_FILLED_06`; locally submitted 80-line prompt, one delivery/read, no hold and human-confirmed terminal ACK. |
| Real unsent draft hold, pill and reason log | Passed | `QA766_DRAFT_04`; native screenshot, unread hold and visible `Drafting` log. |
| Clear draft, release log and once-only receipt | Passed | `QA766_DRAFT_04`; human confirmation, `InputCleared` release, one delivery turn and inbox read. |
| Already-read held nudge suppression | Passed | `QA766_STALE_05`; covering watermark while draft remained, stale-skip log, no delivery turn and human-confirmed absence of a late nudge. |
| Fresh-screen Codex repeat | Skipped | Baseline passed; reviewed-candidate ordinary empty-composer case passed, but a new fresh-screen session was not recreated. Jason accepted stopping further manual reruns. |
| Queued-prompt Codex repeat | Skipped | Baseline passed; reviewed-candidate rerun stopped under Jason’s direction before setup. |
| Claude control repeat | Skipped | Baseline passed; reviewed-candidate rerun skipped under Jason’s direction. |
| Native Windows live testing | Blocked | No native Windows environment; remains assigned to Jason. Windows CI must pass before merge. |

The source/fixture manifest still matches the clean-reviewed candidate. All six required local checks passed after the reviewed race correction, and QA’s independent automated checks passed before the native-control blocker. The reviewer’s `NO REMAINING MUST-FIX ISSUES` verdict remains applicable to the unchanged implementation. No claim is made that the intermittent false-draft cause was reproduced or corrected. The supplied development mission and app are retained; no branch or worktree deletion, nightly or release is authorized.
