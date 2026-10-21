# 836 m5 — companion adapter validation

Scope: Phase 3 m5 on `feat/836-m5-adapter`, based on m4 (`675d90c2`). The adapter serves overview, status and terminal requests on encrypted channels. The controller gates the shared terminal resize path; desktop resize behavior while the desktop controls must remain unchanged. No production app or external relay is used for validation.

## Selected behaviors and checks

| Changed behavior | Selected check | Reason |
| --- | --- | --- |
| Controller ownership, desktop resize gate, release, take-back and viewport bounds | Controller table unit test, existing terminal resize tests, TERM-RESIZE-01 and TERM-RESIZE-02 | The controller sits in the resize path used by every desktop terminal; browser bounds must leave desktop sizes unchanged |
| Limited history and snapshot state | History-limit test and existing terminal snapshot/replay corpus | The browser snapshot omits old scrollback while preserving screen and modes |
| Overview and status | Golden shape, real core node/mission/status/unread assembly, fake-clock coalescing | A final overview invalidation must be delivered without another event |
| Terminal streams | Encrypted m4 browser test, chunk ordering, fairness, resized/closed, subscriber resync and 1 MiB overflow | Browser credit and terminal output must stay bounded and ordered per channel |
| Browser input observation | Classification and actual daemon input-queue/observation tests, TERM-INPUT-01 and draft-aware mission delivery live check | Browser bytes must use the input queue and the desktop's input observation |
| Compatibility and isolation | Deferred requests return unknownRequest, existing m4 channel isolation tests, weak-core lifetime test | Deferred operations remain unavailable and channels cannot retain the daemon core |

## Automated checks

Initial targeted baseline: `cargo nextest run --locked -p runner-daemon -p runner-core -p runner-terminal --no-fail-fast --cargo-profile ci`, exit 0, 1,544 passed and 1 skipped, log `/tmp/836-m5-targeted.log`. This preceded the added m5-specific tests.

All required automated gates passed on the frozen review-fix source candidate. Matching `.exit` files beside these logs retain each command's own exit code.

| Command | Exit | Result / log |
| --- | --- | --- |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | 2,573 passed, 6 skipped; `/tmp/836-m5-rereview-workspace.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | `/tmp/836-m5-rereview-clippy.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 | `/tmp/836-m5-rereview-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `/tmp/836-m5-rereview-fmt.log` |
| `git diff --check` | 0 | `/tmp/836-m5-rereview-diff.log` |

The pre-review targeted m5 run passed 67 tests, exit 0 (`/tmp/836-m5-final-targeted4.log`), including encrypted channels, controller bounds, real node/mission assembly, resync, coalescing, slow-channel isolation, recovery after a missed exit event and the real daemon input path. After review fixes, 15 focused adapter and input tests passed, exit 0 (`/tmp/836-m5-review-fixes-targeted.log`). The input test holds the terminal grid while input begins, proving observation precedes enqueueing, then processes echo and verifies draft delivery readiness. Real backend move/archive coverage checks current pane membership, and held-frame tests cover output draining before exit, archive and status-recovery closure.

Earlier validation found and fixed a stale queue-boundary expectation after adding 64 bytes of per-frame accounting (workspace run exit 100, `/tmp/836-m5-workspace-final.log`), a nested database-pool checkout in mission overview assembly (targeted run exit 100, `/tmp/836-m5-final-targeted2.log`), and test lock scopes flagged by Clippy (exit 101, `/tmp/836-m5-clippy.log`). The same earlier workspace run also hit a 10-second CLI-help timeout for `crew remove --help`; a subsequent run hit the same deadline at `crew lead --help` (`/tmp/836-m5-workspace-candidate.log`). The first frozen-candidate run passed the help test in 58.332 seconds overall; the review-fix gate above passed it in 58.458 seconds, without changing any deadline or expectation. The mission assembly test now passes with a one-connection pool. These failures are retained; the current candidate gates above supersede them.

## Commit boundary validation

The controller/history commit, `471a89ce`, folds in the initial brief and plan and was validated without the adapter source enabled: `cargo nextest run --locked -p runner-daemon -p runner-core -p runner-terminal --no-fail-fast --cargo-profile ci`, exit 0, 1,549 passed and 1 skipped (`/tmp/836-m5-controller-nextest-retry.log`); workspace all-target Clippy, CI profile, exit 0 (`/tmp/836-m5-controller-clippy.log`); staged diff check, exit 0 (`/tmp/836-m5-controller-diff.log`).

The first boundary-test attempt exited 100 because the unchanged model-discovery test `concurrent_refreshes_share_one_query_and_obsolete_results_are_discarded` exceeded its 30-second wait (`/tmp/836-m5-controller-nextest.log`). The test passed alone in 0.539 seconds, exit 0 (`/tmp/836-m5-controller-refresh-retry.log`), and the same full crate selection then passed without source, deadline or expectation changes. After restoring the adapter patch, the complete source matched the reviewed candidate's SHA256; only this validation record was updated with the verdict and commit-boundary evidence.

## QA matrix

Jason removed QA from m5 on 2026-10-21 (mission feed, human message `01M5D0ZCSNF8DTSZ1YZ05ZPN5J`). There is no QA handoff or live validation in this mission. After clean review, the authorized workflow proceeds directly to commits, push, PR against `nightly-remote` and CI. The selected live checks remain pending for Jason on both platforms.

| Platform | Check | Status | Evidence |
| --- | --- | --- | --- |
| macOS development app | TERM-RESIZE-01 | Pending for Jason | QA removed on 2026-10-21 |
| macOS development app | TERM-RESIZE-02 | Pending for Jason | QA removed on 2026-10-21 |
| macOS development app | TERM-INPUT-01 | Pending for Jason | QA removed on 2026-10-21 |
| macOS development app | Mission message held during unsent composer draft, delivered after send/clear | Pending for Jason | QA removed on 2026-10-21 |
| Native Windows development app | TERM-RESIZE-01 | Pending for Jason | QA removed; no native Windows host in this mission |
| Native Windows development app | TERM-RESIZE-02 | Pending for Jason | QA removed; no native Windows host in this mission |
| Native Windows development app | TERM-INPUT-01 | Pending for Jason | QA removed; no native Windows host in this mission |
| Native Windows development app | Mission message held during unsent composer draft, delivered after send/clear | Pending for Jason | QA removed; no native Windows host in this mission |
| Browser adapter live | End-to-end service channel | Pending | The service channel is not available; this is not a blocked m5 check |

## Review and follow-ups

The first working-tree review requested three fixes: observe input before enqueueing it, read pane membership from current slots rather than saved tree leaves, and let the subscription producer finish forwarding frames before marking a stream ended. All three are fixed with regression coverage. Reviewer reported **NO REMAINING MUST-FIX ISSUES** on the whole working-tree diff (mission message `01M5D1AHJAE8J4D7GWRA9KAB3R`), verified all 25 diff files matched the frozen patch, and independently reran the four relevant regression tests: 4 passed, exit 0 (`/tmp/836-m5-reviewer-fixes.log`). Reviewer diff check also passed. The reviewed patch SHA256 is `8f2a1245abeed5e7d2c749482e93d5e79b1c5f556efa4e49b8cab3eb43ad2268`.

m6 owns the desktop controller card, Take back/Continue here UI, pairing notification and Connected now row. Feed/follow/unfollow/answer/post remain unavailable. Browser live validation waits for the service channel. Jason owns the pending native live checks and final PR merge.

## Spec details chosen

Jason approved clamping every browser `attach`, `takeControl` and `viewport` to 20–512 columns and 8–256 rows, lowering large sizes rather than rejecting them; desktop sizes remain unchanged (mission feed, 2026-10-20, human message `01M5CZD1VQGRETTJ4HQ666BGRW`). This is written into Terminal size's bounds sentence and must be listed in the PR body.

The spec also records per-channel subscriptions after hello, trailing overview coalescing, replacement attach and idempotent detach, attachment requirements for input/control/viewport, the initial resized push, a per-companion-stream output sequence across chunks and resync, and opaque channel identity in the internal client-protocol controller. Normal/crashed process exits use exited, intentional stops use stopped, and already-recorded archives use archived.

## PR #976 terminal follow-up — 2026-10-21

Jason authorized the Codex duo follow-up on the existing `feat/836-m5-adapter` branch and PR #976 against `nightly-remote`. Starting implementation was `7020a599`; the brief-only startup commit is `b2b76458`. The umbrella remains `675d90c2`, with no catch-up needed. This follow-up has coder and reviewer only. No app, real agent, live relay or browser service was launched. Native checks remain pending for Jason; browser/service integration remains pending until its channel exists. The user guide continues to describe browser terminals as a later update; these changes affect the unreleased companion wire contract, whose spec and decoder expectations are updated here.

### Changed behaviors and selected checks

| Changed behavior | Selected check | Connection to the change |
| --- | --- | --- |
| Backlogged stream resync after process exit or intentional stop | `companion_backlog_sends_final_snapshot_after_exit_for_both_queue_limits` | Deterministically force subscriber overflow and adapter byte overflow using normal 8 KiB output chunks exceeding 1 MiB; stop before draining; require a multi-chunk final snapshot with FINAL-MARKER, matching resized and then closed, monotonic sequences and final state release; stopped sessions still reject attach |
| Final state ownership and bounded history | `companion_final_snapshot_outlives_registry_without_retaining_the_terminal` | Release the registry and terminal model, consume the stream-owned final snapshot once, enforce the history limit, and prove its subscription is closed |
| Companion REP and unfinished-parser metadata | `snapshot_continuation_survives_chunking_attach_and_resync`, `companion_snapshot_rep_continuation_fixtures_match_host`, `snapshot_continuation_defaults_and_validation` | Verify metadata across attach/resync and a chunk boundary within unfinished bytes; bind shared wire fixtures to the daemon's real parser; validate defaults and malformed fields |
| Intended xterm replay | `script/test-companion-snapshot.cjs` with `@xterm/headless` 6.0.0 | Reproduce byte-only REP loss, then verify the reference decoder's REP hook and restore order on attach/resync fixtures, partial CSI/UTF-8, cursor moves, null continuation and older messages; this uses the same parser as browser xterm without claiming a browser/service live pass |
| Unchanged native replay, resize, sequence and channel isolation | Existing focused command below | Existing snapshot corpus, parser continuation, terminal history/resize fan-out and encrypted-channel isolation cover the code paths shared with these fixes; the live regression suite's TERM-RESIZE-01/02 and TERM-INPUT-01 remain pending for Jason, as authorized in the mission |

Each attached companion stream owns a resync source with only a weak reference to session state. The session registry holds weak source registrations. On exit the source receives one history-limited final snapshot, and resync consumes it into an already closed subscription. Resync no longer requires a new generally attachable stopped session. Delivery/closure, detach and channel drop release the source and final bytes. The native attachment and snapshot replay path is unchanged.

Companion snapshot chunks now repeat `precedingChar` and `unfinishedLen`, defaulting to null and zero. A decoder assembles the snapshot, replays its body, restores REP continuation, then replays unfinished bytes before output. The reference xterm decoder tracks host printable scalars through the print path and uses a CSI REP hook, since xterm's built-in REP resets on controls and reads the previous screen cell. Printing from that hook uses xterm's internal InputHandler; the service integration must retain this behavior or an equivalent and rerun the shared fixtures when changing xterm. This dependency is explicit in the spec. No native wire fields or native restoration order changed.

### Reproduction and failed attempts

Both supplied temporary reproductions failed before the fixes: `cargo nextest run --locked -p runner-daemon --cargo-profile ci -E 'test(review_976_)' --no-fail-fast`, exit 100, two failures, `/tmp/836-m5-followup-reproduce.log`. The reviewer's earlier failure evidence remains `/tmp/pr-976-reproductions.log` and `/tmp/pr-976-reproductions.patch`; its original selected run passed 116 tests in `/tmp/pr-976-review-tests.log`. The temporary tests were replaced by the durable cases above.

Intermediate attempts are retained: an Arc receiver compile error (exit 101, `/tmp/836-m5-followup-exit-first.log`); a test-fixture double-lock that was terminated and corrected (exit 100, `/tmp/836-m5-followup-regressions-first.log`, two other regressions passed); an accidental scripted edit to an earlier test section, fully restored before the successful retry (exit 101, `/tmp/836-m5-followup-focused.log`); and a missing explicit parser generic in the added cleanup test (exit 101, `/tmp/836-m5-followup-focused-final.log`). The xterm check first failed because its screen comparison retained serialized blank cells (exit 1, `/tmp/836-m5-followup-xterm.log`); trimming the comparison fixed that test expectation. These are implementation/test-setup failures, not hidden retries or a claim of platform validation.

### Follow-up results before working-tree review

| Command | Direct exit | Result / log |
| --- | --- | --- |
| `cargo nextest run --locked -p runner-daemon -p runner-core -p runner-terminal --cargo-profile ci -E 'test(companion) \| test(remote::channels) \| test(session::manager::terminal) \| test(snapshot)' --no-fail-fast` | 0 | 121 passed, 1,449 skipped; `/tmp/836-m5-followup-focused-final-retry.log` |
| `node script/test-companion-snapshot.cjs /tmp/836-xterm/node_modules/@xterm/headless` | 0 | REP and continuation cases passed; `/tmp/836-m5-followup-xterm-retry.log` |

The headless dependency was installed outside the checkout with `npm install --prefix /tmp/836-xterm --no-audit --no-fund @xterm/headless@6.0.0` (exit 0, `/tmp/836-m5-xterm-install.log`). It adds no repository dependency or crate. Workspace tests, both workspace Clippy variants, formatting and diff checks are recorded below on the final reviewed source before the authorized update to PR #976.


### Working-tree follow-up review

Reviewer reported **NO REMAINING MUST-FIX ISSUES** on the follow-up and its interactions with the complete PR against `675d90c2` (Runner message `01M5E36C6MNG4FDVQ3BF0NFKRZ`). Reviewer confirmed the exact frozen working-tree patch SHA256 `6432300c40d78d22e5e4c6ef641765d6f75fbcb2a00c9d06c3fb84c68a06e2b1`, independently passed 121 focused tests with 1,449 skipped (`/tmp/836-m5-reviewer-focused.log`), and passed the xterm harness and diff check, each exit 0. No source edits followed this verdict. Final gate results and this review record are evidence-only additions to the reviewed candidate.


### Final reviewed-source gates

The source diff under `crates/` and `script/` remained identical to the reviewed candidate, SHA256 `68a3e32d6f714152aa1846b3bfb8361bc1b81b10e48f93dd602ad768dfe0f119`; only validation evidence was appended after review. Each command below recorded its own direct exit code in the matching `.exit` file. No source changes or CI fixes were needed for these gates.

| Command | Direct exit | Result / log |
| --- | --- | --- |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | 2,578 passed, 6 skipped; `/tmp/836-m5-followup-workspace.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | `/tmp/836-m5-followup-clippy.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 | `/tmp/836-m5-followup-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `/tmp/836-m5-followup-fmt.log` |
| `git diff --check` | 0 | `/tmp/836-m5-followup-diff.log` |

The reviewed implementation and startup brief are folded into the existing adapter implementation commit; the original controller/history commit stays intact. PR #976 remains Part of #953, open against `nightly-remote` for Jason's merge. Current pushed candidate identity and both platform CI results are recorded in the PR description. Native and browser live checks remain pending as above.
