# Codex native input attention validation

Scope: `fix/codex-input-attention`, based on the committed brief at `930580b5` and main at `0696c9e1`. The implementation changes daemon recognition/publication of Codex native input attention and reuses existing attention displays and notifications. No other runtime adapter changes or full regression claim.

## Native contract

Verified online against `openai/codex` tag `rust-v0.163.0` on 2026-10-23:

- [terminal_program_status.rs](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/tui/src/terminal_program_status.rs) emits `ESC ] 7501 ; state=idle|working|blocked:app=codex ESC \\` whenever stdout is a terminal, without a capability probe. Cleanup emits `state=clear` without an app field. The parser also accepts BEL termination and requires a previously recognized Codex report before accepting unscoped clear.
- [status_surfaces.rs](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/tui/src/chatwidget/status_surfaces.rs) chooses blocked before working/idle when action is required.
- [bottom_pane/mod.rs](https://github.com/openai/codex/blob/rust-v0.163.0/codex-rs/tui/src/bottom_pane/mod.rs) derives action-required from unanswered questions or an active view requiring action. This signal cannot identify approval versus question; Runner uses generic Needs you / `WaitReason::Unknown`.

The local CLI reports `codex-cli 0.163.0`. Source verification and the version probe alone do not establish a live app pass.

## Selected checks and reason

- Parser tests cover every split point, BEL/ST, blocked/working/idle/clear, multiple reports, malformed/unrelated-app/oversized/control-containing payloads and opaque terminal strings because PTY chunking and raw control validation determine whether a real report is recognized.
- Detector/reducer tests cover stable interaction identity/age, duplicate suppression, hooks enabled/disabled, compaction/tool/completion/cancellation interleaving, baseline fallback, independent drafts, stale launches and attach/detach/exit because native attention must remain independent of hook work and input.
- Manager/core publication tests cover direct chats and missions, no mounted client, matching mission event/status snapshots, readiness wake and draft holds. An armed-completion callback test ensures native clear cannot create unread completion attention. CoreSessionEvents/database tests cover genuine Completed hooks before/after native clear, repeated reports and Failed/Interrupted outcomes, asserting exactly one successful completion write. Detector tests begin Idle with hooks unavailable and nonquiet response text coalesced with native OSC, and preserve simultaneous readiness/title reports. These tests cover the changed mission path; no additional live mission is needed unless review finds an uncovered delivery path.
- Reviewer live selection: RT-STATUS-12 (replacement for RT-STATUS-06's Codex instance), Codex approval and Plan question with accept/reject/cancel; the affected cancellation variants of RT-STATUS-11; SET-NOTIF-01 adapted to generic Needs you, including one background notification/withdrawal. These check the actual native report and existing sidebar/header/notification consumers. Hooks-off, switch/resume/exit variants remain explicit required observations or gaps. Other agents and unrelated live cases are outside this mission.

## Automated evidence

Commands use this worktree's own target directory and CI profile.

| Check | Result | Evidence |
| --- | --- | --- |
| `cargo test --locked -p runner-daemon codex --profile ci` | Passed, exit 0 | Initial 148 Codex tests; `target/codex-tests.log`. An unused test import reported in that run was removed before backend Clippy. Later added cases are included in the complete daemon rerun below. |
| `cargo test --locked -p runner-daemon -p runner-terminal --profile ci` | Failed, exit 101 | Initial daemon run: 1,324 passed, two pre-existing Claude resume golden failures; `target/daemon-terminal-tests.log`. The same failures reproduce on an unmodified archive of `930580b5` under `target/baseline` (`target/baseline-goldens.log`, exit 101). No goldens were regenerated. |
| `TMPDIR=/tmp cargo test --locked -p runner-daemon -p runner-terminal --profile ci` | Passed, exit 0 | A process-local temporary-directory override avoids the host-specific Claude fixture path-encoding mismatch. Daemon: 1,329 passed. Terminal: 143 passed, one existing ignored test. Results: `target/daemon-terminal-tests-tmp.log`. No global configuration was changed. |
| `TMPDIR=/tmp cargo test --locked -p runner-daemon codex_native --profile ci` | Passed, exit 0 | Pre-review seven attention-specific tests, including outcome/callback and stale-generation cases; `target/native-attention-final.log`. |
| `cargo clippy --locked -p runner-daemon -p runner-terminal --all-targets --profile ci -- -D warnings` | Passed, exit 0 | `target/backend-clippy-final.log`, before first review; correction rerun below. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Blocked, exit 101 | GPUI dependency shader build cannot find `metal` in the active CommandLineTools environment; `target/clippy.log`. The known Xcode app paths from CI/default setup are absent. Workspace app tests/updater checks share this prerequisite; required full gates remain with CI. |
| `cargo nextest --version` | Unavailable, exit 101 | nextest is not installed on this host; local validation uses `cargo test`, and repository CI installs its pinned nextest. |
| `cargo check --locked -p runner-daemon -p runner-terminal --all-targets --profile ci` | Passed, exit 0 | `target/backend-check.log`. |
| `cargo fmt --all --check`, `git diff --check` | Passed, exit 0 | Final formatting and whitespace checks. |

Build logs under `target/` are local ignored artifacts. Committed results retain the failures and retry conditions; native screenshots/session identities belong in the reviewer's dated run record.

## Review and native checks

Working-tree code review is clean after the corrections below; reviewer independently passed all ten attention-specific and four parser tests before any implementation commit. Required macOS/Windows CI [run 39914940481](https://github.com/yicheng47/runner/actions/runs/39914940481) passed on implementation source `254df58b095dc534d207d93b6d2966904e83dbda`, including workspace/updater Clippy and workspace tests.

The original default candidate build exited 101 because GPUI could not find the Metal compiler (`xcrun --find metal`, exit 72). The [initial prerequisite attempt](runs/2026-10-23-macos.md#codex-native-input-attention--prerequisite-attempt-2026-10-23-0657-utc) records those historical Blocked results and no native passes at that stage. A subsequent [runtime-shaders retry](runs/2026-10-23-macos.md#codex-native-input-attention--runtime-shaders-retry-2026-10-23) built the exact implementation source with the dependency's existing `gpui_platform/runtime_shaders` feature. That disposable development wrapper rendered successfully without installing a toolchain or changing product source, dependencies, lockfile or global environment. It is a distinct binary variant from the default CI build.

Reviewer observed real Codex approval and Plan question waits in the existing header/sidebar, stable wait identity/age, controlled rejection and Escape cancellation, question answering, hooks-off native question recognition, and switch/stop/resume/normal exit. Accepted commands also resolved waits, but extra native submit events prevent independent attribution of those acceptance keys to reviewer automation. The hooks-off decision matrix has the same attribution gap. System notification delivery/deduplication/withdrawal remains Blocked: the disposable bundle's permission UI stayed Off and no banner was observed after bounded signing, registration and relaunch remedies. Native Windows checks remain Blocked without a host; CI does not replace them.

Only owned scratch chats and the disposable wrapper were operated and cleaned up. Reviewer removed only the two owned Codex trust tables, verified configuration byte equality with the pre-test baseline, and preserved installed Runner and existing development sessions. Full binary identities, observations, caveats and cleanup evidence are in the dated retry record. Native validation remains incomplete; no complete feature/native regression pass is claimed. The mission stops at an open upstream PR, with no merge/release/implementation-mission or worktree cleanup.

## First review and corrections

Reviewer identified two must-fix findings before any implementation commit:

- P1: excluding attention-only publications from completion was correct for native-only reports, but a genuine Completed hook received while the wait kept the projection Busy lost its completion bookkeeping. The earlier test also asserted the wrong unread expectation after a real Completed hook. Genuine Codex hook completion during a native wait now uses the existing completion callback immediately; native clear still cannot create a completion. Database write-count assertions cover both hook/clear orderings and duplicates, as well as failure and interruption.
- P2: prioritizing attention as the single output event suppressed ordinary nonquiet byte activity in the same chunk. Native attention is now extracted separately, and the reader forwards it alongside the unchanged baseline/title/readiness transition. The regression test starts Idle with unavailable hooks, then coalesces response text with working/idle OSC; simultaneous readiness/title reports are also covered.

Nine correction-focused tests passed (`target/native-review-fixes.log`, exit 0). The expanded full rerun `TMPDIR=/tmp cargo test --locked -p runner-daemon -p runner-terminal --profile ci` passed (exit 0): daemon 1,331, terminal 143 plus one existing ignored test, including the additional combined-title assertion (`target/review-fixes-tests.log`). Backend all-target Clippy with `-D warnings`, formatting and diff checks passed again (exit 0; `target/review-fixes-clippy.log`). At this review stage native checks were pending clean review and the Metal build prerequisite; the later retry is recorded above.

## Second review correction

Reviewer found that the first P2 fix let native-only controls wake ordinary baseline activity and later trigger fabricated completion on the quiet timeout. The direct-event-only test did not cover this path. Recognized native controls now contribute no byte activity; the bounded parser defers candidate bytes across chunks, filters confirmed controls and releases rejected candidates as ordinary byte evidence. Real coalesced text and title/readiness parsing remain intact.

A detector-to-manager test uses real CoreSessionEvents and the database with completion explicitly armed. Native-only blocked/clear/working/idle reports and subsequent timeouts create no unread row, both whole and one byte per chunk. A mixed native report plus ordinary response still wakes baseline Working and records one real output-based completion after quiet. Parser tests also check filtered/retained byte counts across every split. Ten focused tests passed (`target/native-byte-filter-tests.log`, exit 0). The full daemon/terminal command passed again (exit 0): daemon 1,333, terminal 143 plus one existing ignored test (`target/native-byte-filter-full-tests.log`). Backend all-target Clippy with `-D warnings`, check, formatting and diff checks passed (exit 0; `target/native-byte-filter-clippy.log` and `target/backend-check.log`).
