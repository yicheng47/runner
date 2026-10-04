# #799 pi draft delivery — macOS QA, 2026-10-04

**NO REMAINING MUST-FIX QA ISSUES for the authorized #799 macOS matrix.** Pi full-draft and single-character protection, clear-to-release, Return/Working preservation, unchanged active-turn delivery, reload, stop/resume and the Codex draft control passed on the reviewed working tree. Jason resolved the active-turn acceptance wording at 05:31 UTC: pi's native steering after the current tool, before `agent_settled`, is the expected behavior from main. The additional Codex startup-setup failure remains excluded and unattributed. Native Windows remains pending Jason.

## Candidate and authorization

The coder requested phase 4 through Runner after the reviewer's clean verdict. Authorization is [the mission brief](../impls/briefs/799-pi-draft-delivery.md#live-test-authorization): this worktree's development app, native computer use, absolute development CLI, existing accounts, one disposable `qa799-` crew and its needed roles, bounded missions, and cleanup of only those objects. No baseline was repeated; [#799](https://github.com/yicheng47/runner/issues/799) and the [#791 record](archive/791-session-state-reducer.md) retain the baseline and attribution.

Branch `fix/799-pi-draft-delivery`; HEAD `2503bc0c59208a308cc4f763588ad950b5b8051b`. The uncommitted implementation, `git diff -- crates`, was SHA-256 `84fcb094314590769ac0c29d43acffe76a4c489a434c4ef619f2cc7619bfd832` before and after QA, matching the reviewed patch. `evidence/candidate.patch`, `git-status.txt` and `final-identity.json` retain identity. QA changed only this record and scratch evidence; no implementation, expectations, authentication, global pi settings/extensions, permission settings, commits, pushes or PRs were changed. Runner's SQLite database was not opened.

Environment: macOS 26.6.2 (25G83), arm64; Runner app/CLI 0.12.8 (dev); pi 1.0.2, `deepseek/deepseek-flash`, low; Codex CLI 0.160.0, `gpt-6-luna`, low, inherited fast. The mission feed reports the existing Bypass permission mode. Both roles used their normal runtime arguments; QA did not set a permission override. Pi's discovered user extensions remained loaded. The native pi terminal retained a package-update notice, and Codex displayed two startup warnings; neither was acted on.

Evidence and canonical scratch root: `/private/tmp/qa799-live-20261004/`. `cli.py` records each exact absolute CLI argument vector, UTC timestamps, stdout/stderr and exit code in `evidence/<label>.json`, removing inherited implementation-mission identity. The endpoint was `com.wycstudios.runner-dev/mcp.sock` with outside-mission identity. Screenshots used native computer use; command/state evidence came independently from the development CLI and only the test objects' status feeds/transcripts. The raw terminal recordings are under `recordings/`.

No development app was running at setup. `env -u NO_COLOR -u RUNNER_CREW_ID -u RUNNER_MISSION_ID -u RUNNER_HANDLE -u RUNNER_EVENT_LOG RUNNER_RECORD_INPUT_FIXTURE=/private/tmp/qa799-live-20261004/recordings/input make run` built and launched this worktree using its own `target/`. Native control rejected the bare executable, so QA stopped only that newly launched process and used `/private/tmp/qa799-live-20261004/Runner QA799 Dev.app`, bundle `com.wycstudios.runner.qa799-dev`. The build succeeded; `make run` ultimately exited 2 after the intentional termination, and the wrapper exited 0 on native Quit. `wrapper-hashes.json` confirms byte-identical app and CLI copies; app SHA-256 `dd78120ab1c96dae4cf9e59fdf73766565f0f1d51d5573c7777fefca19b2cd8d`, CLI `caede4a34aa289f6dc664ef3ebd8f54b0bc7c3c015b3bf829698e86cbdd4e5ba`. The wrapper launch omitted the same environment variables. Native New chat opened/dismissed before terminal input; colors were visible. The installed Runner app was not operated.

Automatic event watching was unavailable on this host, disclosed when both missions started. No manually polled process or unread background file was represented as a notification watch. Required boundaries were captured directly through the native app and CLI.

## Matrix

Evidence filenames below are relative to the scratch root's `evidence/` directory.

| Check | Result | Observation and evidence |
| --- | --- | --- |
| Pi single-lead startup in fresh canonical cwd | Passed | One `QA799-AUTOSTART` line in `pi/startup.txt`, native ACK, captured key and ready/hook/completed before any terminal input. `pi-startup-native.jpg`, `pi-startup-session.json`. |
| Full unsent pi draft holds addressed message beyond the typing guard | Passed | `QA799-UNSENT-DRAFT` remained visible with Inbox waiting; native `drafting=true`, no new turn, no read of that message, and no release file 12 seconds after post. `pi-full-draft-before-message.jpg`, `pi-full-held.jpg`, `pi-full-held-snapshot.json`, `pi-full-held-feed.json`. |
| Backspace clears full draft and releases exactly once | Passed | Eighteen Backspaces, no Return. Clean notification without draft text, one inbox-read event and exactly one marker line; native ACK and Completed. `pi-full-release-native.jpg`, `pi-full-after-clear-snapshot.json`, `pi-final-feed-summary.json`, `pi-turn-order.json`. |
| Single native `x` holds, then one Backspace releases once | Passed | Hold persisted 20 seconds after post. One Backspace released a clean notification, one read and one line in `pi/x-release.txt`, with native ACK/Completed. `pi-x-held.jpg`, `pi-x-held-snapshot.json`, `pi-x-release-native.jpg`, `pi-x-released-state.json`. |
| Normal Return releases native draft state without fabricating idle | Passed | After Return, `editor_draft=false` while Runner remained Working/using_tools through the foreground `sleep 30`. A later steering injection caused another true/false editor pair while status stayed Working. `pi-active-working.json`, `pi-active-working-pi-feed.ndjson`, `pi-active-with-message.jpg`, `pi-active-message-queued-state.json`. |
| Return preserves active-turn delivery from main | Passed | Unchanged from main: notification queued as native pi steering, processed after the current tool and before final `agent_settled`, with both ACKs in the final response and no merged user draft. Jason explicitly accepted this boundary. Main `reserve_delivery` at lines 1023–1036 holds for needs-you, in-flight delivery and draft state, never Working; this function is identical in the candidate. `acceptance-resolution.json`, `pi-turn-order.json`, `pi-active-final-feed-summary.json`, `pi-active-after-turn-pi-feed.ndjson`. |
| Live pi extension reload | Passed | `/reload` emitted shutdown/start with reason reload, one initial false draft report and one subsequent true transition. Native reload confirmation and conversation key unchanged. `pi-reloaded.jpg`, `pi-reloaded-snapshot.json`, `pi-before-stop-pi-feed.ndjson`. |
| Pi stop with native draft, then original-conversation resume | Passed | Test PID exited, feed removed and session Stopped. Resume kept the same key/history, began with `drafting=false`, and the next idle message executed once with no stale hold. `pi-stopped-state.json`, `pi-after-stop-snapshot.json`, `pi-resumed-native.jpg`, `pi-resumed-snapshot.json`, `pi-resume-release-native.jpg`, `pi-resume-completed-state.json`. |
| Sampler programmatic edits, deferred reads, reload/shutdown cleanup, missing APIs | Passed | Independent real-timer harness against the exact embedded extension reported `[false,true,false,true,false,false]`; reload replaced one subscription, shutdown left zero intervals/deferred callbacks, and each missing-API case created no tracking state. Existing embedded extension and reducer checks also passed. `lifecycle-result.json`, `lifecycle-feed.ndjson`, `qa-pi-tests.log`, `qa-native-reducer-tests.log`. |
| Codex startup marker (additional setup observation) | Failed | Native ACK with no startup tool call or marker file. No baseline attribution; this does not count as an automatic-startup pass. `codex-startup-native.jpg`, `codex-startup-snapshot.json`, `codex-test-transcript.jsonl`. |
| Codex draft control | Passed | Native draft and Inbox waiting persisted at least 39 seconds after post, with no marker/read. Fully clearing without Return released a clean notification exactly once, wrote one line in `codex/draft-release.txt`, showed ACK and hook Completed. `codex-held.jpg`, `codex-held-snapshot.json`, `codex-release-native.jpg`, `codex-release-state.json`, `codex-final-feed-summary.json`. |
| Test-object cleanup | Passed | Both missions stopped/archived with zero live sessions; disposable crew/roles deleted under explicit authorization; test rows gone from Recents; only QA's dev app quit. `pi-cleanup-verify.json`, `codex-cleanup-verify.json`, deletion/verification JSON records, `cleanup-recents.jpg`, `app-quit-verify.json`, `final-identity.json`. |
| Native Windows | Skipped | Not accessible in this run; explicitly pending Jason in the brief. |

## Acceptance resolution and other observations

The active-turn prompt was submitted at 05:16:43.726 UTC. The addressed message was posted at 05:16:52.223 while the tool remained active. At 05:17:12.604, both active-turn/after-turn files were absent, native UI showed the steering queue and empty editor, and CLI still reported Working/using_tools. Pi appended the queued notification as a user message at 05:17:14.766 after the foreground command returned. Inbox read occurred at 05:17:17.134; the final response at 05:17:19.316 contained both `ACK QA799-ACTIVE-TURN` and `ACK QA799-AFTER-TURN`. Only then did the agent settle. Both files contain exactly one expected line. No synthetic Ready/Idle publication appeared between submit and final settlement.

QA initially marked a strict after-`agent_settled` interpretation Failed and withheld signoff. Jason clarified through Runner at 05:31:45 UTC (message `01M42PB4ZTWYRVF38P206YB3Y0`) that his "after the turn" wording was imprecise: the requirement is that releasing the native draft hold on Return preserves main's active-turn delivery, without adding a wait for `agent_settled`. He explicitly accepted the observed native steering boundary and requested that the row be recorded Passed, unchanged from main. The observation and initial interpretation remain recorded here; this is an acceptance correction, with no changed implementation or recovery run.

Source comparison establishes the unchanged delivery policy: [main's `reserve_delivery`](https://github.com/yicheng47/runner/blob/8f3b26ae637d47471ce207c88709940565290eb1/crates/runner-backend/src/session/manager/mod.rs#L1023-L1036) gates needs-you, an in-flight delivery/ticket, and `draft_hold`, with no Working-state hold. The entire function is identical in this candidate. `evidence/acceptance-resolution.json` retains that source comparison, main SHA and Jason's decision. No baseline was rerun, as instructed. Strict next-agent-turn scheduling was not demonstrated and is outside the clarified requirement. With this correction, no required accessible QA check remains unresolved.

Codex's startup response said ACK but performed no startup tool call and produced no `startup.txt`; its supplied startup goal is visible in the native terminal and test transcript. That setup check is Failed and has no baseline attribution; it is not counted as an automatic-startup pass. The required Codex draft control subsequently executed the addressed instruction through its tools and passed. `codex-startup-native.jpg`, `codex-startup-snapshot.json`, `codex-test-transcript.jsonl` retain the observation. The first Codex clear attempt moved the caret into the draft and left `AFT`; this was an incomplete QA action, visibly still held. Ctrl+E followed by three Backspaces completed clearing. No human Return was used in that control.

Pi's first-generation recording has 18 edit events for the full draft, one edit for `x`, and no Submit before the deliberately typed active-turn prompt. Its only two native Submit events were the active prompt and `/reload`. The retained pre-lifecycle pi transcript contains three clean inbox notification prompts, never the full unsent draft or `x`. Codex's recording has 17 content keys, 20 edits, one navigation and no Submit. Input-summary JSON files accompany both recordings. The fixture recorder uses `create_new` for a session-named file, so the resumed pi process has no second terminal recording; its native screenshots, new-generation hook feed, unchanged key and original test transcript are retained instead.

## Automated evidence

The coder's required full checks on this exact reviewed diff are recorded in `/private/tmp/qa799-checks/review-fix/results.json`; each exited 0. QA did not repeat the complete suites without a candidate change.

| Command | Result |
| --- | --- |
| `cargo test --locked -p runner-backend --profile ci` | Passed; exit 0, 1,110 tests. |
| `cargo test --locked -p runner-terminal --profile ci` | Passed; exit 0, 103 tests, one ignored. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Passed; exit 0. |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | Passed; exit 0. |
| `cargo fmt --all --check` | Passed; exit 0. |
| `git diff --check` | Passed; exit 0. |

Independent QA commands below ran on the unchanged candidate with golden updates disabled. Exact commands/exit codes are in `evidence/qa-automated-results.json` and `lifecycle-result.json`.

| Command | Result |
| --- | --- |
| `cargo test --locked -p runner-backend --profile ci pi_status::tests` | Passed; exit 0, nine tests. |
| `cargo test --locked -p runner-backend --profile ci native_` | Passed; exit 0, 12 tests. |
| `cargo test --locked -p runner-backend --profile ci session_scenario_goldens` | Passed; exit 0, one Rust test covering the full 142-scenario compact corpus, no golden writes. Only pi `draft-delivery` is modified; the other 141 goldens are byte-identical. |
| `node /private/tmp/qa799-live-20261004/lifecycle.mjs` | Passed; exit 0, real-timer extension harness. |

Phase 1 evidence remains `/private/tmp/qa799-phase1/{findings.md,probe.ts,probe.py,events.ndjson,actions.ndjson,terminal.bin,command.json,api-versions.json}`. It establishes the API timing, programmatic edits without raw terminal callbacks, API availability from 0.52.10 and in 0.99.2, and the unchanged 0.84.4 status floor. Older pi was not launched live; missing-API behavior was covered by the embedded and independent harnesses.

## Cleanup ledger and limits

One crew, `qa799-draft-delivery` (`01M42N7HKPB81RTMM6RABSYY1N`), was reused with one lead slot for pi and then Codex. Roles were `qa799-pi` (`01M42N7HGGTPZNRDY8CWGJ34GF`) and `qa799-codex` (`01M42N7HJ57VT6JXJN5AQSVS0M`). Pi mission/session: `01M42N888JZKP7HPN9KKQN71Q0` / `01M42N888MGWJNE1JJGAJM1X15`. Codex mission/session: `01M42NPX1X3AECND5PT90DWRPR` / `01M42NPX1ZNHMSWQ70747XH851`. `evidence/ledger.json` retains these IDs. Stopped/archived state and feeds were captured before crew/role deletion; deletion removes their archived Runner metadata. Test conversation history, scratch markers, wrapper and evidence are retained.

No other runtimes, unrelated UI workflows, full smoke suite, intentional bridge-failure injection, live older pi, or native Windows were tested. Live exit coverage used normal development CLI stop; direct timer/subscription shutdown coverage used the exact extension in the harness and real pi reload. The required draft behavior is verified live, and the active-turn acceptance row is Passed under Jason's clarification. The excluded Codex startup-setup failure and native Windows pending are retained as limits; neither is claimed resolved by this signoff.
