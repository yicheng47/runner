# 836 m4 — channel validation

Candidate: `feat/836-m4-channel`, based on `nightly-remote` at `3b4eb45c`. Platform: macOS arm64. Scope: Phase 3 m4, ending at companion `hello`/`welcome`; every later valid request receives `unknownRequest`.

## Selected checks

| Changed behavior | Check and reason |
| --- | --- |
| CPace and Noise handshakes | Draft 21 appendix B.3 initiator-responder vector; deterministic pairing, connect, wrong-PIN and unknown-device channel transcripts, reproduced by the Rust browser and host; strict frame sizes, order, canonical points, identity rejection and authenticated payload validation |
| Encrypted records and credit | Exact record bytes; authentication rejection; assembly bounded to 1 MiB; one outgoing message at a time; zero-credit stall/resume; partial-message credit at 64 KiB; invalid/over-granted credit rejected |
| Daemon route and trust | In-process relay and actual daemon WebSocket loop over a duplex byte stream, with real channel tasks, encryption and trust persistence; pairing and connect through `welcome`; unsupported requests; all reject reasons; six concurrent confirmations lock at five failures; Unlock; Revoke; Pause, Disconnect, Reset identity and route loss teardown; 16-channel cap; fake-clock deadline; unknown channel frames dropped |
| Channel isolation | Exhaust a browser's entire receive window without returning credit, pair and welcome another browser, then return credit and resume the first; a blocked trust write does not stall an established channel; delayed snapshots cannot restore revoked devices; pairing completions and provisional authorization survive route loss; stale tasks cannot affect reused channel numbers; overlapping pairings preserve a successful save in either completion order; failed or saturated writes emit no pairing event; malformed transport and trust-save failures do not terminate the remote state machine |
| Secrets and opt-in behavior | Redacted channel/CPace/identity/record `Debug`; source audit of fixed log messages; no channel resources until an enabled relay opens a channel; no session-manager, terminal, database or production app changes |
| Shared compilation and regressions | Required workspace nextest, both workspace Clippy configurations, formatting and diff whitespace gates; public type additions must compile for every workspace client |

No live regression suite rows are selected: m4 has no reachable live channel before the service's browser route exists. The app, its accounts and agents are not launched or operated. New channel tests use only in-process queues and duplex byte streams, temporary files and fake time, with no network sockets or sleeps; pre-existing workspace integration tests retain their existing fixtures.

## Contract decisions

- Jason approved counting every structurally valid `pairFinish` authentication failure, including tampering, on 2026-10-20 (Runner message `01M5CRGN6FH2G9NAF4ZMYRT967`). The original literal real-PIN-mismatch-only rule is unimplementable: wrong PSK and a correct PSK with a flipped confirmation tag both return `snow::Error::Decrypt`. An offline probe exited 0; evidence is `target/m4-logs/confirmation-probe.rs` and `confirmation-probe.log`. Malformed frames, invalid CPace points and successfully decrypted invalid payloads do not count.
- Jason approved the single `connected: false` initializer in the app's existing test fixture (`01M5CRNNF7CWHNZW5RVTRR1J0S`). No production app behavior changes.
- Jason requested the docs-only m5 terminals-first plan amendment (`01M5CRGN5EWCVG7QZC3Y9HPWX8`); credit flow control remains in m4.
- The spec now names the vector JSON layout, zero/over-granted credit rejection, and per-channel queue limits. The starting credit remains 512 KiB; only consumed bytes can be returned.

## Results

Evidence is under this worktree's ignored `target/m4-logs/`; exit codes are read from each command's process result, separately from log inspection.

| Command | Exit | Evidence |
| --- | --- | --- |
| Offline Noise confirmation probe | 0 | `confirmation-probe.log` |
| Independent vector-layout and lockfile audits | 0 | `vector-layout-audit.log`, `lockfile-audit.log` |
| Focused channel core tests, seven cases and refreshed golden vectors | 0 | `vector-generation.log` |
| Focused daemon channel tests, 16 cases including the actual socket loop and review regressions | 0 | `review-fixes-daemon.log` |
| Focused rerun of the three runtime discovery timeout failures | 0 | `runtime-timeouts-rerun.log` |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | `nextest.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | `clippy.log` |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | 0 | `clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `fmt.log` |
| `git diff --check` | 0 | `diff-check.log` |
| Each planned commit: `cargo check --locked --workspace --all-targets --profile ci` | 0 | `commit1-check.log`, `commit2-check.log`, `commit3-check.log` |
| First commit: core nextest, 103 passed | 0 | `commit1-tests.log` |
| Second commit: core and daemon nextest, 1,426 passed | 0 | `commit2-tests.log` |
| Commit-split content audit, 31 reviewed files restored exactly | 0 | `commit-split-content-audit.log` |

Final workspace result: 2,553 passed, six pre-existing ignored manual benchmark/real-provider cases skipped; every new channel case ran. No app, account or agent was operated for live validation.

The three planned commits are formed after clean review, with the initial brief folded into the first. Later steps were temporarily reversed using saved patches in this worktree so the first two commits were built and tested in isolation. Restoring the final step reproduced all 31 reviewed files exactly before appending this validation metadata; the full workspace pass above therefore covers the final source, including the CLI. Each step also passed a workspace/all-targets build.

CLI help is unchanged, so `make guide-cli` is not required. The remote access guide documents the CLI's `connected` field and confirmation-failure counting.

Earlier runs: the first focused daemon run failed because the teardown test fixture lacked enrollment state; correcting the fixture produced six passing cases. The first Clippy gate exited 101 for `len_zero` in a test and the intermediate formatting check exited 1 for the newly added queue-byte accessor; both were corrected. Compilation issues while introducing the in-process stream seam were also corrected before the 12-case pass. These failures are recorded here; the command log files contain their latest reruns. Earlier complete workspace passes (2,544, 2,546, 2,548 and 2,549 tests) are retained as `nextest-initial.log`, `nextest-before-writer.log`, `nextest-before-socket-seam.log` and `nextest-before-inbox.log`; the results table records the final implementation's gates.

The first full workspace run after review fixes exited 100: 2,550 passed, three existing runtime discovery subprocess cases failed on timeouts, and six were skipped (`nextest-review-fixes-timeouts.log`). The failures were `update_check_reprobes_installed_version_with_a_cached_latest`, `spawn_failure_timeout_and_cache_write_failure_are_contained` and `concurrent_refreshes_share_one_query_and_obsolete_results_are_discarded`; all three passed together without code changes, exit 0, in 2.692 seconds (`runtime-timeouts-rerun.log`). The fresh full workspace gate then passed all 2,553 tests, exit 0, in 261.886 seconds (`nextest.log`). The last complete pass before review fixes is retained as `nextest-before-review-fixes.log`.

## Review and follow-ups

The first working-tree review identified two must-fix persistence issues: a relay teardown could discard a failed pairing completion and its provisional authorization guard, and `remote/paired` was emitted before persistence succeeded. Save completions and provisional devices now live with the lazy, supervisor-owned trust writer; the remote loop drains completions even without a relay connection. Only a successful, still-current pairing emits the event. Each channel instance is checked before applying delayed work, and channel save permits remain held until completion is consumed. Four deterministic regression cases cover route loss, saturation, reused channel numbers and overlapping pairings. Focused rerun: 16 passed, exit 0; all required local gates passed, exit 0.

Whole-diff re-review returned `NO REMAINING MUST-FIX ISSUES` on 2026-10-20 (Runner message `01M5CWXXS24W5ZWZE24YN5X62J`). The reviewer independently ran the 16 daemon channel tests with `cargo test --offline --locked -p runner-daemon --profile ci --lib remote::channels::tests`, exit 0 (`reviewer-recheck.log`), and checked the whole diff's whitespace, exit 0. The planned commit, push and PR workflow follows this clean working-tree verdict.

Live browser interoperability remains for the service's channel implementation and replay of these vectors. m5 serves overview, status and terminals; later work enables mission feed and messaging without a protocol version change. m6 owns the desktop control card, pairing notification and Connected now row.
