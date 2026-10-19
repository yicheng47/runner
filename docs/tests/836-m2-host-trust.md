# 836 m2 — host trust validation

Candidate: `feat/836-m2-host-trust`, based on `nightly-remote` at `c5d66b93`. Implementation has passed whole-diff review; this record does not claim a live or full-regression pass.

## Scope and selected checks

The changed behaviors are persistent host identity and legacy adoption, PIN validation and durable lock state, paired-device persistence and revocation, identity reset, and the client protocol and guarded CLI. Settings adds two-step masked six-digit PIN entry and confirmation, Unlock, a paired-device table with Revoke, Reset identity confirmation, a live lock-warning toast, and shared trust snapshots across windows. Jason approved references `eiIy3`, `KNmA3`, `zSJNq`, `TLJNH` and `T0hixs` on 2026-10-19. He also approved using recorded last-seen times and future pairing copy in m2 (Runner message `01M5999KJTHZD8Y3JEKM7A6K6Z`).

- Identity checks restore the public key from the stored private key, adopt a legacy enrollment only when no identity exists, rewrite its credential item without the key, preserve enrollment fingerprints across disconnect and removal, and order reset cleanup before replacement.
- Trust checks cover ASCII digits and leading zeroes, five consecutive failures, successful pairing-result reset, restart persistence, unlock, device round trip and revoke, atomic file replacement and Unix `0600`, and preservation of invalid files as `.bad`.
- Protocol checks exercise the actual client socket, request `Debug` redaction, public snapshots and pushed events without PIN or private key, fast trust reads, remote mutation deadlines covering delayed sign-out and a queued PIN change, and exclusion of all remote requests from `runner call`.
- CLI checks reject sensitive commands without terminals and under `RUNNER_HANDLE`, including a real pseudo-terminal guard test on Unix; validate matching PIN entries without echoing errors; keep remote commands absent from the agent guide; and regenerate the human CLI reference.
- App tests render the no-PIN, populated and locked snapshots, PIN and Reset dialogs, and lock toast in English/Simplified Chinese and Carbon/Runner Light. PIN-input checks cover masked glyphs/native reads, digits-only edits, leading zeroes, paste, deletion and blocked copy/cut; validation checks cover mismatches and exactly six ASCII digits. Lock-transition checks exclude initial snapshots and repeat events. A deterministic production-handler sequence also covers the daemon’s empty fast snapshot followed by restored locked trust on startup/reconnect, then an actual unlocked-to-locked transition that must notify. These checks use GPUI’s test renderer without launching either app.
- The required workspace nextest, Clippy (with and without updater), formatting, whitespace and generated-guide gates cover integration and both feature configurations. Windows is validated by PR CI after the working-tree review.

The live regression catalog has no remote-access cases yet. No unrelated live runtime, IME, session or authentication cases are selected. The mission forbids launching either app; the feature-specific live checks below belong to Jason on `remote-nightly`. Remote-specific tests use memory stores, temporary directories, local fake services, and UUID-namespaced OS secret-store entries with cleanup. The daemon-fixture correction below records an OS secret-store read attempt detected during broader commit-boundary validation and the isolation fix; no real service was contacted.

## Local results

Logs are under `target/validation/836-m2/`. Exit codes are captured from each command directly. Xcode 27.1’s `xcrun --sdk macosx` initially picked an unavailable stub despite an installed Metal compiler. The remaining workspace gates select that existing compiler with the command-local environment `TOOLCHAINS=Metal`; no SDK, global setting or check is changed.

| Check | Exit | Evidence |
| --- | --- | --- |
| Identity/core and daemon remote tests, initial compile | 101 | `identity-tests.log`: missing Enrollment key field, corrected |
| Identity tests, first execution | 101 | `identity-tests-2.log`: two obsolete fresh-fingerprint assertions, updated for the new contract |
| Identity tests, rerun | 0 | `identity-tests-3.log` |
| Trust/core and daemon remote tests | 0 | `trust-tests.log` |
| Protocol and CLI remote tests, initial compile | 101 | `protocol-cli-tests.log`: macOS `openpty` pointer type, corrected |
| Protocol and CLI remote tests, rerun | 0 | `protocol-cli-tests-2.log` |
| `make guide-cli` | 0 | `guide-cli.log` |
| CLI help golden generation | 0 | `update-cli-goldens.log`; verified again by the workspace gate |
| Workspace nextest, initial build | 101 | `nextest-workspace.log`: Xcode’s SDK lookup selected a missing Metal driver |
| Workspace nextest with `TOOLCHAINS=Metal`, first execution | 100 | `nextest-workspace-2.log`: 2,472 passed, 2 CLI help golden failures, 6 skipped; snapshots and coverage updated |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` with `TOOLCHAINS=Metal`, rerun | 0 | `nextest-workspace-3.log` and post-review `review-nextest-workspace.log`: 2,474 passed, 6 skipped |
| Workspace Clippy, initial build | 101 | `clippy-workspace.log`: same Metal driver lookup failure |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` with `TOOLCHAINS=Metal` | 0 | `clippy-workspace-2.log`, `clippy-workspace-final.log`, post-review `review-clippy-workspace.log` |
| Same Clippy command with `--features updater` | 0 | `clippy-updater.log`, `clippy-updater-final.log`, post-review `review-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `fmt-check.log`, post-review `review-fmt-check.log` |
| `git diff --check` | 0 | `diff-check.log`, post-review `review-diff-check.log` |
| `make guide-cli-check` | 0 | `guide-cli-check.log`, post-review `review-guide-cli-check.log` |
| Review fix: remote request deadlines and delayed-service socket regression | 0 | `review-timeout-targeted.log`: 2 passed |
| Settings test build, first compile | 101 | `settings-check.log`: GPUI paint arguments, element ID and focus borrow errors; corrected |
| Settings test build, rerun | 0 | `settings-check-2.log` |
| Settings snapshots and PIN-input/validation tests | 0 | `settings-targeted.log`: 5 passed |
| Settings workspace Clippy, first run | 101 | `settings-clippy-workspace.log`: large warning enum and redundant division; corrected |
| Initial whole-diff workspace nextest | 0 | `settings-nextest-workspace.log`: 2,477 passed, 6 skipped |
| Initial whole-diff workspace Clippy | 0 | `settings-clippy-workspace-final.log` |
| Initial whole-diff workspace Clippy with updater | 0 | `settings-clippy-updater.log` |
| Initial whole-diff formatting | 0 | `settings-fmt-check.log` |
| Initial whole-diff whitespace | 0 | `settings-diff-check.log` |
| Initial whole-diff generated CLI reference | 0 | `settings-guide-cli-check.log` |
| Hydration regression and corrected Settings viewports | 0 | `hydration-targeted.log`: 6 passed |
| Workspace nextest after hydration fix | 0 | `hydration-nextest-workspace.log`: 2,478 passed, 6 skipped |
| Workspace Clippy after hydration fix | 0 | `hydration-clippy-workspace.log` |
| Workspace Clippy with updater after hydration fix | 0 | `hydration-clippy-updater.log` |
| Formatting after hydration fix | 0 | `hydration-fmt-check.log` |
| Whitespace after hydration fix | 0 | `hydration-diff-check.log` |
| Generated CLI reference after hydration fix | 0 | `hydration-guide-cli-check.log` |

## Commit boundaries

The four commits were formed only after clean whole-diff review, with the brief folded into commit 1. Each boundary passed `cargo check --locked --workspace --all-targets --profile ci` and formatting (exit 0). The final source files were restored from the reviewed tree and SHA-256 verified; only this validation record gained the boundary evidence. The workspace gate above covers that same final code. The branch is based on current `origin/nightly-remote` at `c5d66b93`; PR CI records macOS and Windows results separately.

| Step | Selected tests | Exit | Evidence |
| --- | --- | --- | --- |
| Identity and legacy upgrade | Core/daemon remote: 35 passed | 0 | `commit-1-build.log`, `commit-1-tests.log`, `commit-1-fmt.log` |
| PIN, lock and devices | Core/daemon remote: 38 passed | 0 | `commit-2-build.log`, `commit-2-tests.log`, `commit-2-fmt.log` |
| Client protocol and CLI | Full CLI plus core/daemon remote: 265 passed; generated guide check | 0 | `commit-3-build.log`, `commit-3-tests.log`, `commit-3-fmt.log`, `commit-3-guide.log` |
| Settings and guide | App trust/input/hydration/rendering: 6 passed | 0 | `commit-4-build.log`, `commit-4-tests.log`, `commit-4-fmt.log` |

## Review and live checks

The first whole-diff review (Runner message `01M599XW07HRHVSV5QACR2H7B5`) found one P2 must-fix: an app starting or reconnecting before asynchronous trust restore finished could read the daemon’s default unlocked snapshot and then mistake restored locked trust for a new lock. The warning now requires the previous snapshot to have a PIN set: before that, a lock cannot be a live failed-PIN transition. The regression feeds startup, reconnect, restored-lock, repeat-event and later real-lock sequences through the production app-store update handler, checks the locked snapshot is retained, and checks notification revisions. The reviewer also noted that `window.resize` does not resize GPUI’s test viewport; the render checks now use `simulate_window_resize` and assert that both dialogs fit the actual viewport. The six targeted tests and all workspace gates pass after the fix. On 2026-10-19, reviewer reported **NO REMAINING MUST-FIX ISSUES** for the complete working-tree diff (Runner message `01M59A7XG6BD2B52HXTPJJE6T0`), independently passing all six focused app tests and `git diff --check` with exit 0. This clears the four-commit PR/CI workflow; At that handoff, Windows CI and Jason’s live checks remained pending.

The first commits 1–3 review found one P2 must-fix: the four new trust mutations inherited the normal 10-second client timeout, shorter than a 15-second sign-out or time spent queued behind a service call. They now use the existing 45-second remote-request budget; status and trust reads retain the normal fast deadline. The deadline assertions and existing CallGate socket regression cover reset, a PIN change queued behind its held sign-out, and a fast trust read during that hold. The two targeted timeout regressions and all required workspace gates pass after the fix. On 2026-10-19, the reviewer reported **NO REMAINING MUST-FIX ISSUES** for the commits 1–3 working-tree scope through Runner (message `01M595CD87ZV1MHJJ6VQZET791`), independently rerunning the two targeted regressions and `git diff --check` with exit 0. The initial review independently passed 52 selected tests plus the CLI guard test. This is a partial-scope verdict: commit 4 now has the approved design, implementation and passing full gates; the complete working-tree diff subsequently passed whole-diff review as recorded above. At that review handoff no implementation commit, push or PR had been made.

Pending with Jason on a `remote-nightly` build: an existing enrolled host retains its fingerprint after upgrade without enrolling again; Disconnect then Connect keeps that fingerprint; Reset identity changes it and removes the host from the web panel; another program’s read of the macOS identity item raises a Keychain prompt. Settings rows, dialogs and the lock toast pass test-renderer snapshot checks in both languages and themes; this does not claim a live Settings check. Actual PIN exchange, device pairing, channels and new-device notifications belong to Phase 3.

## Post-PR corrections

Jason requested two fixes on PR #957 through Runner message `01M59BSY9JT77B3C0TCR283K8V`. Reset identity now saves the replacement identity after sign-out cleanup and before clearing `remote-trust.json`. The state-machine regression checks that exact effect order. An isolated daemon test injects an identity-write failure after enrollment against the loopback fake service: sign-out and enrollment cleanup complete, the request fails, and the old identity and the entire old trust file (devices and locked PIN entry) remain untouched. This prevents a cleared trust file from being left beside an old identity when the secret-store write fails; a crash after a successful identity write leaves the new key with old devices, whose old host key cannot authenticate.

Revoking an unknown fingerprint now returns `No paired device has fingerprint <fp>.` before scheduling any trust-file save. Daemon tests check the refusal with populated and empty lists and the client-socket error, and still verify that a known fingerprint is removed and persisted. A built-CLI test uses the existing isolated daemon fixture to check both outside and inside an agent session: exit 1, the exact refusal on stderr, no success output and no trust file created. Settings retains its existing request-error display path, so it shows the daemon message if a listed device disappears before Revoke reaches it. No new sleeps or real-service calls are introduced; the later daemon-fixture correction below covers the isolation gap in that process setup.

| Check | Exit | Evidence |
| --- | --- | --- |
| Targeted daemon/CLI remote tests | 0 | `post-pr-targeted.log`: 43 passed |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | `post-pr-nextest-workspace.log`: 2,480 passed, 6 skipped |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | `post-pr-clippy-workspace.log` |
| Same Clippy command with `--features updater` | 0 | `post-pr-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `post-pr-fmt-check.log` |
| `git diff --check` | 0 | `post-pr-diff-check.log` |
| `make guide-cli-check` | 0 | `post-pr-guide-cli-check.log` |

Build commands again use command-local `TOOLCHAINS=Metal`. The corrections were uncommitted at the review handoff. On 2026-10-19, the reviewer reported **NO REMAINING MUST-FIX ISSUES** for both corrections and the complete five-file follow-up diff (Runner message `01M59C5PEY634HJAS1R3XM3FEW`), independently passing six daemon/CLI regressions and `git diff --check` with exit 0. The fixes are folded into their owning commits after that clean review; renewed macOS/Windows CI is tracked on PR #957. The live checks above remain pending with Jason.


## Six-digit PIN and two-step dialog

After live testing, Jason requested exactly six ASCII digits and approved PIN dialog option B (Runner message `01M59GWD8X046X3Y43W8HXVG59`). The 15:24 exports on 2026-10-19 are `K2ww03.png` (choose), `VckyU.png` (confirm) and the replacement `TLJNH.png` (step 1 over Settings) under `target/design-ref/836-m2/`. These supersede the old two-field PIN frame. This iteration implements the new references; neither slot launches either app or claims a new live pass.

Daemon, app and CLI reject lengths other than six and non-ASCII digits; leading zeroes remain intact. Socket coverage verifies the daemon's exact refusal for a seven-digit PIN, and CLI validation checks the same error without echoing input. Existing long-PIN fixtures are six digits, with seven- and ten-digit values retained as rejection cases. CLI prompts/help, the generated reference/goldens, Client trust, the Phase 1 CLI/Settings sections, the guide and the m2 plan follow the new contract.

Settings shows one six-box row at a time. Tests exercise five-digit refusal for both actions, enabled Continue after the sixth digit, Enter to continue and submit, a mismatch toast with cleared entries, Back clearing both entries, Escape cleanup, focus transfer and Tab wrapping within the modal while skipping the disabled action. Both steps render in English/Simplified Chinese and Carbon/Runner Light; bounds checks cover six equal-width 48 px boxes and dialog containment. The field caps typed input at six, accepts a six-digit paste, removes the last digit on Backspace, refuses other input and keeps native reads masked and copy/cut blocked. Continue/Set PIN explicitly use the shared Primary button, whose fill and label are the theme's accent and accent-ink; the no-PIN row retains that variant. These are test-renderer checks, with no real service or production secret access.

The first focused run caught unequal flex widths at the active box's thicker border; a six-column grid fixes the layout. The interaction test also needed to refocus the input after deliberately clicking disabled Continue before typing the sixth digit. Both failures and the passing reruns are preserved below. Required workspace gates cover the shared client/daemon and app integration after the contract change; selected focused tests cover the new input and state transitions directly.

| Check | Exit | Evidence |
| --- | --- | --- |
| First app test build | 0 | `six-digit-app-build.log` |
| First focused run | 100 | `six-digit-targeted.log`: 7 passed, 2 failed; equal widths and disabled-button click/input focus |
| Grid-layout rerun | 100 | `six-digit-targeted-2.log`: 8 passed, 1 failed; test input needed refocus after disabled click |
| Corrected interaction rerun | 0 | `six-digit-targeted-3.log`: 9 passed |
| Final focused checks, including socket refusal and modal Tab behavior | 0 | `six-digit-targeted-4.log`: 10 passed |
| `make guide-cli` | 0 | `six-digit-guide-cli.log` |
| CLI golden generation | 0 | `six-digit-cli-goldens.log` |
| First workspace nextest after the design change | 0 | `six-digit-nextest-workspace.log`: 2,481 passed, 6 skipped |
| First workspace Clippy | 101 | `six-digit-clippy-workspace.log`: two redundant String conversions in debug selectors, removed |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | `six-digit-clippy-workspace-2.log` |
| Same Clippy command with `--features updater` | 0 | `six-digit-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `six-digit-fmt-check.log` |
| `git diff --check` | 0 | `six-digit-diff-check.log` |
| `make guide-cli-check` | 0 | `six-digit-guide-cli-check.log` |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`, after selector cleanup | 0 | `six-digit-nextest-workspace-2.log`: 2,481 passed, 6 skipped |

Builds use command-local `TOOLCHAINS=Metal` as above. At this handoff the six-digit/dialog changes are uncommitted and await working-tree review, followed by fixup/autosquash and renewed macOS/Windows CI on PR #957. Jason's remaining live checks still apply; this record does not claim a live pass of the new dialog.

The first six-digit working-tree review found one P2 must-fix (Runner message `01M59JAA5M4RE7D0X9YY70FW36`): the Settings confirmation layer had priority 100 and the global toast priority 3, so the PIN scrim covered the mismatch toast and intercepted its close click. Toasts now use priority 101 while the PIN dialog is open in Settings. The production-root mismatch regression clicks `GLOBAL_TOAST_CLOSE`, then checks that the toast and its rendered bounds are gone while cleared step 1 remains open. This regression fails against the old layer order and passes after the scoped fix. The reviewer otherwise found no additional must-fix issue across the initial 14-file follow-up and independently passed its ten focused checks and whitespace check with exit 0. The layering fix adds `app_shell.rs` to the follow-up diff and returns for review before any fixup or push.

| Layering follow-up check | Exit | Evidence |
| --- | --- | --- |
| Close-click regression before the layer fix | 100 | `six-digit-toast-regression-before.log`: the toast remained and its click reached the backdrop |
| Focused app/daemon/CLI checks after the fix | 0 | `six-digit-toast-targeted.log`: 10 passed |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | `six-digit-toast-nextest-workspace.log`: 2,481 passed, 6 skipped |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | `six-digit-toast-clippy-workspace.log` |
| Same Clippy command with `--features updater` | 0 | `six-digit-toast-clippy-updater.log` |
| `cargo fmt --all --check` | 0 | `six-digit-toast-fmt-check.log` |
| `git diff --check` | 0 | `six-digit-toast-diff-check.log` |
| `make guide-cli-check` | 0 | `six-digit-toast-guide-cli-check.log` |

On 2026-10-19, the reviewer reported **NO REMAINING MUST-FIX ISSUES** for the complete 15-file six-digit/two-step follow-up diff (Runner message `01M59JQ1DBHKFCR2W6A0ZK4YM7`). The reviewer independently passed all six focused app checks and `git diff --check`, both exit 0, and confirmed the scoped toast priority and production-root close-click regression resolve the P2. The reviewed changes are folded into their owning commits before renewed macOS/Windows CI on PR #957. Jason authorized merging that PR into `nightly-remote` once those checks are green; live checks remain pending with him.

## Isolated daemon startup correction

After the six-digit review, rechecking each rewritten commit exposed a broader daemon-test fixture entering the real OS secret store. The commit-3 core/daemon/CLI run reached 1,601 passing tests, with `daemon::server::tests::client_accept_errors_back_off_and_preserve_live_sessions` blocked in teardown. A one-second stack sample showed its remote startup worker inside `Identity::load` → `OsStore::get` → `SecItemCopyMatching`, waiting in Keychain. This corrects the earlier blanket isolation claim: that fixture attempted a read of the normal development identity item. No prompt was approved, no secret content was retrieved or printed by the agent, and no item was written or deleted. The test server intercepted SIGTERM, so the same identified test process was killed with SIGKILL; it had no child processes. The nextest command itself returned 100, recorded in `six-digit-boundary-1bc56c75.log`; the sample is `six-digit-boundary-accept-hang.sample.txt`.

An initial correction skipped the remote actor for isolated daemons. Its focused checks passed, but the full workspace run returned 100 because the built-CLI unknown-revocation regression then received “Remote access task is unavailable” instead of the real command refusal (2,480 passed, 1 failed, 6 skipped; `isolated-daemon-nextest-workspace.log`). The final correction instead passes `Config::isolated` to remote startup and selects the existing `MemoryStore` for isolated daemons, keeping their remote commands functional while normal daemons use `OsStore`. The existing server regression asserts empty initial trust, successfully sets a test PIN through the isolated socket, verifies a fingerprint and no devices, and checks that no remote identity, credential or trust file was written, before completing its session-preservation and shutdown checks. Remote-specific socket tests still inject their memory-store actor directly. Normal app/daemon startup is unchanged. This correction remains uncommitted for working-tree review before fixup/autosquash, push, renewed CI and the authorized umbrella merge. The first new test build returned 101 because its assertions called typed methods directly on `SocketTransport`; the corrected test uses its typed client from `spawn_blocking` (`isolated-daemon-targeted.log`, `isolated-daemon-targeted-2.log`).

| Final isolation check | Exit | Evidence |
| --- | --- | --- |
| Focused isolated server, remote state/socket and built-CLI refusal checks | 0 | `isolated-daemon-targeted-3.log`: 44 passed |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | 0 | `isolated-daemon-nextest-workspace-2.log`: 2,481 passed, 6 skipped |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | 0 | `isolated-daemon-clippy-workspace-2.log` |
| Same Clippy command with `--features updater` | 0 | `isolated-daemon-clippy-updater-2.log` |
| `cargo fmt --all --check` | 0 | `isolated-daemon-fmt-check-2.log` |
| `git diff --check` | 0 | `isolated-daemon-diff-check-2.log` |
| `make guide-cli-check` | 0 | `isolated-daemon-guide-cli-check-2.log` |

All commands capture their own exit code; builds again use command-local `TOOLCHAINS=Metal`. The preceding six-digit autosquash preserved all 35 reviewed file contents exactly, with four commits on `c5d66b93`. Its identity, trust and app boundary checks passed; its broader commit-3 run exposed the isolation defect above. The final isolation correction adds `daemon/server.rs` to the feature and is submitted as an uncommitted three-file follow-up diff. No app was launched and no real service was contacted. The merge remains pending clean review and renewed CI on the corrected head.

On 2026-10-19, the reviewer reported **NO REMAINING MUST-FIX ISSUES** for the complete three-file isolation follow-up (Runner message `01M59MVRX08FPZQP1DEVDRPMR6`). The reviewer traced the sole production startup caller and the isolated CLI path, confirmed the same in-memory store is reused across actor restarts and normal daemons still use the OS store, and independently passed the isolated server, typed trust socket and built-CLI unknown-revoke regressions (3 passed, exit 0), plus `git diff --check` (exit 0). This clears fixup/autosquash and renewed macOS/Windows CI before the authorized merge into `nightly-remote`. Live checks remain with Jason.
