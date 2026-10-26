# Linux remote sign-in validation

Candidate: `fix/823-linux-remote-sign-in`, based on Linux umbrella `f2bc95662e608bbce052e23736ecf7b3c22a20a8`. Local checks ran on macOS ARM64 on 2026-10-26, using this worktree's own `target/` and the locked `ci` profile. No app was launched.

## Selected scope

- A user-owned `0775` data root must permit new secrets and preserve an existing identity and enrollment after repair. Unix `FileStore` tests exercise those paths and retain refusal of world-writable roots, symlinked roots and unsafe files. A bounded subprocess test plants a FIFO before repairing a 0775 root and verifies nonblocking open rejects it before reading.
- Startup errors and preparation panics must reach logs, status events and queued requests with credential redaction. A failing-store test queues Start and Logout before preparation, submits more commands during backoff and a deliberately blocked retry, then recovers and verifies the abandoned Start never created an identity.
- Cancel must immediately clear busy/opening state while the sign-in request is pending. App settings tests exercise late success and failure, failed cleanup, suppression of the abandoned browser launch, an immediate second Sign in with successful delayed cleanup, and Cancel before Start dispatch. Dispatched attempts finish their ordered cleanup before a new Start; undispatched attempts are skipped.
- Missing native roots must not prevent constructing the HTTP client. A Linux-only subprocess test sets `SSL_CERT_FILE` and `SSL_CERT_DIR` to empty fixtures without mutating the parent test environment. The pinned `zed-reqwest` builder permits zero loaded certificates; HTTPS verification still requires trusted roots.
- Required gates: both affected crates' tests, formatting, diff whitespace and workspace Clippy. CI covers Linux x86_64/aarch64, macOS and Windows; its PR trigger now includes the Linux umbrella target.

## Results

| Check | Result |
| --- | --- |
| Before-fix `cargo test --locked -p runner-daemon --profile ci group_writable_data_root_restores_existing_identity_and_enrollment -- --exact remote::store::file_tests::group_writable_data_root_restores_existing_identity_and_enrollment` | Exit 101: a valid stored identity/enrollment under `0775` fails restore with `Secret directory must be owned by this user and not writable by others`. Confirms the suspected startup path; the VM's original directory mode was not independently measured. |
| `cargo test --locked -p runner-daemon --profile ci remote:: -- --test-threads=4` | Exit 0: 113 passed before the additional recovery-gate test assertions. |
| First full affected-crate nextest run | 2,439 passed, one new Cancel test failed because its fixture displayed Chat instead of Settings; three existing skips. Corrected the fixture route. |
| `cargo nextest run --locked -p runner-daemon -p runner-app --no-fail-fast --cargo-profile ci` | Exit 0: 2,440 passed, three existing skips. Includes the blocked-retry assertions and corrected Cancel fixture. |
| Before FIFO fix, `cargo test --locked -p runner-daemon --profile ci fifo_secret_in_group_writable_root_is_rejected_without_blocking -- --exact remote::store::file_tests::fifo_secret_in_group_writable_root_is_rejected_without_blocking` | Exit 101: FIFO blocked startup; the bounded subprocess was killed and reaped after two seconds. |
| After review fixes, `cargo nextest run --locked -p runner-daemon -p runner-app --no-fail-fast --cargo-profile ci` | Exit 0: 2,442 passed, three existing skips. Includes ordered cleanup/immediate retry, cancel-before-dispatch and FIFO regressions. |
| `cargo fmt --all --check` and `git diff --check` | Exit 0, including review fixes. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Exit 0, including review fixes. |
| `cargo clippy --locked -p runner-app --features updater --all-targets --profile ci -- -D warnings` | Exit 0, including review fixes. |

Review-fix raw logs: `/tmp/823-fifo-before-fix.log`, `/tmp/823-review-tests.log`, `/tmp/823-review-clippy.log` and `/tmp/823-review-updater-clippy.log`. Local initial raw logs: `/tmp/823-daemon-remote-tests.log`, `/tmp/823-affected-tests.log` (first full run), `/tmp/823-affected-tests-final.log`, `/tmp/823-startup-test.log` and `/tmp/823-clippy.log`. The Linux-only empty-roots test is covered by Linux CI, not the macOS result above. PR checks are the authority for cross-platform results.

## Human live check — not run

Jason owns the UTM Ubuntu ARM64 check after a later `linux-nightly` cut. Adapt `REMOTE-ACCOUNT-01` to Linux: start with a user-owned `0775` Runner data directory, choose Sign in to Runner from both Settings → Remote access and Account, verify the browser opens and approval completes, then verify Cancel during a new pending Sign in immediately restores the button. Immediately Sign in again and verify the abandoned attempt neither opens an extra browser nor cancels the fresh enrollment. Check that the repaired root has no group/other write bits and that an existing identity keeps its fingerprint. If startup is deliberately made inaccessible in an owned test root, verify the Remote access card shows the actual error promptly. These checks cover the changed browser handoff, button states, automatic permissions repair and visible startup failure. The codex duo has no QA slot and neither coder nor reviewer runs the app.

Clean working-tree review received through Runner on 2026-10-26 after both review findings were fixed. The reviewer requires the three human checks above (permission repair/browser approval and preserved identity; immediate Cancel/retry ordering; visible startup error), and no broader live regression for this diff. All remain not run.
