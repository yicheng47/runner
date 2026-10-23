# #836 m7b — client core validation

Candidate: `feat/836-m7b-client-core`, based on `nightly-remote` at `7acf9ab1`, macOS arm64, 2026-10-23. This mission uses the working-tree diff for review before a commit or PR. No app or live remote service is run; Jason owns live acceptance later.

## Selected checks and reasons

| Changed behavior | Selected checks | Reason | Result |
| --- | --- | --- | --- |
| Account below General | Settings navigation unit test | Checks the one changed Settings order without expanding to unrelated Settings behavior | Passed |
| Development secrets in files | File-store independence, mode 0600, unsafe permissions/symlink refusal; platform selection; existing OS-store round trips | Protects separate identity, credentials and client-key items while retaining release macOS and Windows stores | Passed |
| Account directory | Proto-shaped computer rows, directory cache and unauthenticated/resource-exhausted errors | Confirms service IDs map to fingerprint-based pairing and listing limits are respected | Passed |
| Pair and connect | Real host `Channels` and companion adapter through a numbered in-process relay | Exercises client handshakes without contacting any service | 32 focused client/core/store/CLI/IPC checks passed |
| Trust and rejection | noPin, pinLocked, wrongPin, busy, badHandshake; connect after pairing; host Revoke/unknownDevice; changed pinned key; Forget and reset | Covers PIN-attempt preservation and fail-closed identity behavior | Passed |
| Companion overview and pushes | Overview reads, following, overview/status events and owner release | Verifies the m7c-facing client protocol path and subscription lifetime | Passed |
| Remote terminals | `TerminalAttachment` snapshot/output/resize/input, take control and viewport; dropped route produces fresh snapshot and no replay; idle close; no-credit client alongside another client | Covers terminal protocol adaptation, sequence ordering, route recovery and channel isolation | Passed |
| Channel records | Shared record-vector replay from the client; existing credit/oversize tests | Checks client decoding and credit against the frozen wire contract, including the 1 MiB limit | Passed |
| Service transport and redaction | Upgrade request header inspection and refusal mapping; 4401/4404/4409/4426/4429 through duplex WebSockets; private-key Debug canary | Checks that credentials stay only in Authorization and service/peer bodies never become client diagnostics | Passed |
| CLI | Four command help goldens, typed protocol round trips, pair refusal with an agent handle including a real PTY, generated CLI reference | Verifies command surfaces and the human-only PIN entry gate | Passed; goldens regenerated and reference checked, exit 0 |

The new relay and route tests use in-process duplex sockets, fake service implementations and explicit virtual-clock advancement. They have no network calls or sleeps. Existing workspace tests keep their established fixtures. No live regression cases are selected in this mission because its authorization explicitly leaves live checks to Jason; remote terminal acceptance and the development Keychain prompt are recorded below instead. Unchanged IME, agent sign-in and session lifecycle are outside the feature-specific scope; the required workspace suite still runs.

## Required gates at first handoff

Each command writes its output under this worktree's `target/`; the shell captures the command's own exit status before displaying log excerpts.

| Command | Log | Exit |
| --- | --- | --- |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `target/m7b-workspace-final.log` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `target/m7b-clippy-final.log` | 0 |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `target/m7b-clippy-updater-final.log` | 0 |
| `cargo fmt --all --check` | `target/m7b-fmt-final.log` | 0 |
| `make guide-cli-check` | `target/m7b-guide-check-final.log` | 0 |
| `git diff --check` | `target/m7b-diff-check-final.log` | 0 |

Supporting commands: daemon/CLI compile check, exit 0; initial 11 client integration tests, exit 0; expanded 23 focused checks (`target/m7b-focused-stable.log`), exit 0; CLI golden regeneration, exit 0. Initial build attempts found missing protocol round-trip samples and a test fixture enum needing indirection, both fixed. The relay fixture initially failed to close split WebSockets and used an unrelated invalidation name; both fixture issues were corrected before its passing run. Terminal resize sequence advancement was corrected during implementation. The first completed workspace run (`target/m7b-workspace-tests-2.log`, exit 100) ran 2,697 tests: 2,695 passed, with two timeouts in unchanged tests: the runtime-model concurrency fixture and CLI `mission feed --help` during help-golden execution. The runtime-model test passed unchanged in isolation (`target/m7b-runtime-model-rerun.log`, exit 0). The final full run used `NEXTEST_TEST_THREADS=4` to reduce contention and passed all 2,701 tests (one slow help-golden check; six existing skips). The subsequent focused run covers the latest pending-attachment cleanup and added host-reset check. Both earlier timed-out cases passed in the full rerun without changes to those tests. Failed/intermediate logs remain under `target/`.

## Live checks pending for Jason

Two Runners against a local `runner_server`: list computers; pair; enter one wrong PIN; read the overview; attach a terminal and check output, input and resize; exercise Revoke and Forget; drop the route and verify a fresh snapshot with no replayed input. On macOS, start a development build and confirm that rebuilding/starting it needs no Keychain password prompt. Windows platform behavior is to be checked by CI and later live acceptance. Development Keychain secrets are intentionally not read or migrated; the first file-store run needs sign-in and pairing again, as the guide states.

## Review and handoff

Working-tree review was requested through Runner after local checks. The reviewer reported **NO REMAINING MUST-FIX ISSUES** on the complete tracked and untracked diff in Runner message `01M5JB990R60EVATJGSQ56SA0P`, after all six findings from rounds 1 and 2 were fixed. Review preceded the implementation commit and PR workflow. PR targets `nightly-remote`, with macOS and Windows CI required before handoff. Jason merges; this mission does not merge, cut a nightly/release or remove its branch/worktree.

## Working-tree review round 1

Reviewer findings (Runner message `01M5J93VJTQTVY0Y4PMS185DTW`) required four changes before approval:

- Multiple subscribers now join one immutable snapshot plus a bounded journal of ordered output/resize frames. A join with available replay does not request another host snapshot or resync healthy subscribers. Journal rollover, unsolicited replacement and reconnect replace the baseline once; recovering subscribers reuse that baseline and journal. The targeted checks cover joining after output and resize, journal rollover, and repeated recovery of two subscribers with subsequent shared output.
- Ordinary IPC viewport and take-control requests now update the stored local viewport before forwarding. Recovery uses that latest viewport with `ifFree`. The regression resizes from 80×24 to 120×50 through each path, drops the route and checks the host-side recovery request.
- Daemon `AppEvent` and IPC `ClientEvent` Debug implementations redact remote payloads while serialization preserves them. A canary test formats an actual emitted overview event and its decoded IPC event, and verifies that both wire payloads retain the ask prompt.
- Jason approved a UTF-8-safe pairing label capped at 64 bytes in Runner message `01M5J94VEDNX1DNH68CW2GMC87`, resolving the service's 64-Unicode-scalar limit versus the frozen pairing payload's 64-byte limit. The service name stays unchanged in Account/directory storage. A 22-character Chinese service name pairs successfully using a 21-character label. The plan, spec and guide record this decision; the host and wire contract stay unchanged.

The selected review regression run (`target/m7b-review-focused-2.log`) passed all 29 checks, exit 0. Its first run timed out in the viewport fixture because the fake overview omitted its attached session, so the real host adapter correctly closed it. The fixture now includes that session; diagnostic logs remain under `target/m7b-viewport-diagnostic*.log`. Temporary diagnostics were removed.

The full workspace and both Clippy configurations were rerun because shared event Debug implementations and terminal fanout/recovery changed; formatting, CLI reference and diff checks also repeated. These are automated feature/required gates, not a live-regression claim. No app or live service is run.

| Review-round command | Log | Exit |
| --- | --- | --- |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` with `NEXTEST_TEST_THREADS=4` | `target/m7b-review-workspace.log` | 0 — 2,708 passed, six existing skips |
| Selected client/core/store/CLI tests on the final route code | `target/m7b-review-focused-final.log` | 0 — 29 passed |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `target/m7b-review-clippy-final.log` | 0 |
| Same Clippy command with `--features updater` | `target/m7b-review-clippy-updater-final.log` | 0 |
| `cargo fmt --all --check` | `target/m7b-review-fmt-final.log` | 0 |
| `make guide-cli-check` | `target/m7b-review-guide-final.log` | 0 |
| `git diff --check` | `target/m7b-review-diff-final.log` | 0 |

After the workspace suite compiled, the two queued-command failure paths were aligned with normal forwarding: an accepted viewport updates the stored recovery size even when its request fails on a dropped route. Input is still discarded. The subsequent 29 focused checks and both Clippy configurations checked that final source. The workspace run passed all 2,708 cases, including the six added review regressions and the previously timed-out unchanged cases; one help-golden test was slow. All exit codes above were captured directly from the command.

## Working-tree review round 2

Reviewer findings (Runner message `01M5JA8CTCAFTDBEBM5HJK2WH0`) identified a second queue bound and final-snapshot ownership across IPC:

- The replay journal now has the same 64-frame limit as SocketTransport, in addition to its existing 1 MiB byte limit. An oversized journal is discarded and the next attachment obtains a fresh host snapshot. Existing healthy subscriptions continue receiving output. The shared frame limit prevents a replacement from repeatedly preloading more frames than the IPC client can hold.
- The IPC server retains a remote resync subscription until the client replaces/detaches it or disconnects. A host `Closed` marks its stream closed and closes its queues while preserving its final snapshot and valid journal for those existing subscribers. Their reattachment returns that baseline and then closure, without an Attach to the stopped host. The last detach removes the cached stream. Local terminal forwarding retains its existing ownership behavior.

The selected checks now include the real SocketTransport and daemon IPC bridge, reusing the existing test harness. A test-only event after each IPC terminal frame is written provides an ordered barrier: the client receives that event after it has processed the terminal frame. The regression holds the terminal reader through two bursts of 80 alternating tiny output/resize frames, verifies overflow and one recovery per burst, checks the full fresh snapshot, and receives later output on both subscribers. The final-screen regression holds its resync until after the host's replacement Snapshot + Resized + Closed, then verifies the final bytes and closure, with the stopped host refusing new attaches and its Attach count unchanged.

All 32 selected checks passed (`target/m7b-round3-focused-2.log`, exit 0), including the two new IPC regressions and the existing IPC overflow test. The first compile attempt needed the Core trait imported into the expanded fixture (`target/m7b-round3-focused.log`, exit 101); it is corrected, and the failed log is retained. The relay remains in-process; IPC uses only the test Unix socket or Windows named pipe. No network, sleep, app or live service is used by these regressions.

The full workspace and required gates passed on the final source for this round. They repeat because terminal forwarding and subscription ownership changed; this remains automated feature validation, with live checks left to Jason.

| Review-round command | Log | Exit |
| --- | --- | --- |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` with `NEXTEST_TEST_THREADS=4` | `target/m7b-round3-workspace.log` | 0 — 2,710 passed, six existing skips |
| Selected client/core/store/CLI and IPC overflow tests | `target/m7b-round3-focused-2.log` | 0 — 32 passed |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `target/m7b-round3-clippy.log` | 0 |
| Same Clippy command with `--features updater` | `target/m7b-round3-clippy-updater.log` | 0 |
| `cargo fmt --all --check` | `target/m7b-round3-fmt.log` | 0 |
| `make guide-cli-check` | `target/m7b-round3-guide.log` | 0 |
| `git diff --check` | `target/m7b-round3-diff.log` | 0 |

Each `.exit` file records the command's actual status. One unchanged CLI help-golden check was slow; all tests passed. The clean working-tree review below authorizes the mission's commit and PR steps.

## Clean working-tree review

The reviewer independently passed both new IPC bridge regressions (2/2, exit 0, `target/m7b-reviewer-ipc.log`) and `git diff --check` (exit 0), and verified the final 2,710 workspace/32 focused test results and all gate exit codes. Runner message `01M5JB990R60EVATJGSQ56SA0P` records **NO REMAINING MUST-FIX ISSUES** on the entire diff, with all six findings resolved and the host/channel contract unchanged. The reviewer edited no source and ran no app or live service. This is a clean code review and macOS automated validation; Windows CI and Jason's live acceptance are separate gates.
