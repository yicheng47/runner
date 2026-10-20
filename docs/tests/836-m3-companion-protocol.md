# 836 m3 — companion protocol v1 validation

Scope: Phase 2 m3 of #836 / #952, on `feat/836-m3-companion-protocol`, based on `nightly-remote` at `3dcc86c1`. The implementation adds only the standalone `runner-core::companion` module and its export; no adapter, transport, client-protocol change, dependency or database migration. The spec records the previously open wire details. No guide page changes because there is no user-visible behavior yet.

## Selected checks

| Changed behavior | Check and reason |
| --- | --- |
| Message names, envelope, fields, nulls and terminal bytes | All 59 golden JSON files decode: 53 serialize to identical JSON values, and six unknown-field compatibility files compare to their known-field forms. Typed overview and feed result payloads decode separately. An inventory assertion prevents an untested fixture. |
| Compatibility | Fixtures prove unknown fields do not change known values (including each known controller), an older v1 message defaults a test-only added field, every wire enum's fallback, unknown push decoding, and unknown request response with its original numeric ID. Malformed known requests must fail decoding instead of turning into unknown requests; array envelopes fail in both directions without panicking. |
| Handshake version choice | Overlapping ranges, no overlap, an empty offer, duplicates and offers out of order; the mismatch error matches its fixture. |
| Status projection | One test for each spec table row, with lifecycle before interactions before activity, all needs-you reasons and unread independently. |
| Terminal data | Standard padded base64, invalid encoding rejected, empty input, exactly 32 KiB and one byte over; 1 MiB limit constant, history default and minimum viewport. |
| Integration/build gates | Required workspace nextest, both Clippy configurations, formatting and whitespace checks, each with a separate log and directly captured exit code. |

No live regression case applies: m3 defines a contract but serves nothing. No app, test chat or mission is launched, and no live check is claimed. The new tests neither sleep nor reach a network. Local build products remain in this worktree's `target/`.

## Results

Local logs and exit files are under `/tmp/runner-836-m3-checks/`; these are machine-local evidence, not repository artifacts. Each command redirects its output to its log and immediately captures `$?` into its `.exit` file, then exits with that value.

| Command | Log | Exit | Result |
| --- | --- | --- | --- |
| `cargo nextest run --locked -p runner-core --cargo-profile ci companion` | `core.log` | 0 | 17 companion tests passed in the initial focused run. |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `workspace-nextest-initial.log` | 0 | 2,508 passed, 6 skipped before the boxed-error adjustment. |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `workspace-nextest.log` | 0 | Before review: 2,508 passed, 6 skipped, including all 17 companion tests and 52 golden files. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `clippy.log` | 0 | Passed after fixing the initial finding. |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `clippy-updater.log` | 0 | Passed. |
| `cargo fmt --all --check` | `fmt.log` | 0 | Passed. |
| `git diff --check` | `diff-check.log` | 0 | Passed. |

The first Clippy attempt exited 101 (`clippy-initial.log` and `.exit`): `VersionRange::negotiate` returned an error of at least 136 bytes. The returned error is now boxed; no lint was suppressed. Both Clippy configurations pass. The existing dependency `block` emits Cargo's future-incompatibility notice, without failing either gate. The six skipped tests are the repository's pre-existing ignored real-runtime smoke/benchmark tests; none is a companion test.

`git fetch origin nightly-remote` confirmed the base remains `3dcc86c1`; no rebase was needed. Implementation changes remained uncommitted throughout working-tree review.

All check processes exited. A final `ps` check found no remaining process running from this worktree's `target/`.

## Review and follow-ups

Working-tree review is required through Runner before any implementation commit, push or PR. The first review found two must-fix issues:

- R1: Serde can decode the header from a two-element array, after which the decoder's object access panicked. Both message directions now reject non-object envelopes before decoding the header. Regression assertions include `["overview",1]`, `["hello",null]`, `["status",null]` and `["welcome",null]`.
- R2: Known controller variants dropped unknown nested fields, contradicting the spec's retention promise. Every known controller variant now retains extensions, and four new `unknown_fields_resized_*.json` files pin both the known variant and its nested metadata. The spec's wire shapes and retention promise stayed the same at this review; Jason's later F4 request below supersedes the retention rule.

The focused fix check, `cargo nextest run --locked -p runner-core --cargo-profile ci companion`, exited 0 with 18 tests passed and all 56 golden files (`review-fixes-core.log`). The first workspace rerun exited 100: 2,508 passed, one failed, six skipped (`review-fixes-nextest.log`). The failure was the existing `runner-cli::golden built_cli_goldens::crew_remove` fixture's isolated daemon startup deadline at `golden.rs:217`, before its command body ran; all companion tests passed. Its isolated retry, `cargo nextest run --locked -p runner-cli --test golden --cargo-profile ci -E 'test(=built_cli_goldens::crew_remove)'`, exited 0 (`review-fixes-crew-remove-retry.log`, one passed). No unrelated CLI or daemon code changed during these retries.

The next full workspace run also exited 100: 2,508 passed, one failed, six skipped (`review-fixes-nextest-retry.log`). `crew_remove` passed; the existing `session_show_text_and_json_keep_launch_values_after_role_edit` test instead failed at `golden.rs:1138` because its CLI subprocess returned an unsuccessful status with empty stderr. Its isolated retry with the same workspace command plus `-E 'test(=session_show_text_and_json_keep_launch_values_after_role_edit)'` exited 0 (`review-fixes-session-show-retry.log`, one passed). All companion tests passed in both full runs. These two failures remain in the record; their cause is unconfirmed. The final full run used `NEXTEST_TEST_THREADS=1` to check the same cases without concurrent test execution; no case was removed or newly ignored.

| Final fix check | Log | Exit | Result |
| --- | --- | --- | --- |
| `NEXTEST_TEST_THREADS=1 cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `review-fixes-nextest-serial.log` | 0 | 2,509 passed, 6 skipped; all 18 companion tests and 56 golden files. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `review-fixes-clippy.log` | 0 | Passed. |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `review-fixes-clippy-updater.log` | 0 | Passed. |
| `cargo fmt --all --check` | `review-fixes-fmt.log` | 0 | Passed. |
| `git diff --check` | `review-fixes-diff.log` | 0 | Passed. |

On 2026-10-20, the initial working-tree re-review returned **NO REMAINING MUST-FIX ISSUES** through Runner. The reviewer independently reproduced all four array-envelope cases without panics, verified all known controller variants retain metadata, ran the focused suite (18 passed, all 56 golden files covered; exit 0), and checked the final gate evidence. Both original findings are resolved. Commit `815fc228` then passed both default-concurrency CI jobs on PR #966: macOS in 7m47s and Windows in 15m48s; the checks watch exited 0. The concurrent local CLI failures above remain a caveat rather than a confirmed diagnosis.

## Jason's PR follow-ups

Jason requested F1–F3 through the mission feed on 2026-10-20, after reviewing PR #966. These fixes stay in `runner-core::companion` and the docs and remained uncommitted throughout another working-tree review, before being folded into the implementation commit for the same PR.

| Follow-up and changed behavior | Selected check and reason |
| --- | --- |
| F1: malformed known bodies lost the already decoded request ID | The host-facing `ClientMessage::decode` returns a boxed wire `ErrorMessage`; both it and Serde use one dispatch path. A malformed attach with ID 7 produces `error_bad_request_body.json` without reparsing, and invalid/missing type preserves a readable numeric ID. Missing/invalid ID, arrays, null and invalid JSON produce `badRequest` with `id:null`. Every valid client golden also runs through the new byte decoder. |
| F2: push dispatch wrongly validated `id` as a correlation number | The shared header now reads only `type`; result/error bodies still validate their own IDs. New unknown-push and known-status goldens retain string `id` fields. Assertions pin both variants and confirm malformed reply IDs are still rejected. |
| F3: a required `cwd` string could not represent the existing nullable model | `protocol/model.rs:176` and `repo/session.rs:139` store `Option<String>`; `session_from_row_db` passes `row.cwd` through unchanged. Companion `Session.cwd` is now optional before v1 ships. All overview goldens pin an unavailable directory as `cwd:null`, and a focused test checks null, missing and known directories. |

| Follow-up check | Log | Exit | Result |
| --- | --- | --- | --- |
| `cargo nextest run --locked -p runner-core --cargo-profile ci companion` | `followups-core.log` | 0 | 23 passed; all 59 golden files. |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `followups-nextest.log` | 0 | Default concurrency: 2,514 passed, 6 pre-existing ignored tests skipped. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `followups-clippy.log` | 0 | Passed. |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `followups-clippy-updater.log` | 0 | Passed. |
| `cargo fmt --all --check` | `followups-fmt.log` | 0 | Passed. |
| `git diff --check` | `followups-diff.log` | 0 | Passed. |

Every exit was captured directly from its command. All check processes exited; a final `ps` check found no process running from this worktree's `target/`. No live check applies. On 2026-10-20 the follow-up working-tree review returned **NO REMAINING MUST-FIX ISSUES** on the whole diff. The reviewer independently ran all 23 focused tests covering 59 golden files, reproduced F1 correlation, F2 push/reply ID handling and F3 null/missing directories with a compiled API probe, and checked every final gate result; all exited 0. The amended PR must pass fresh macOS and Windows CI before final handoff.

Phase 3 owns serving these types from `runnerd`, transport size enforcement, stream ordering/resync and live terminal control. No lasting live regression case is promoted from this automated-only milestone.

## Jason's F4–F5 follow-ups

Jason requested two more fixes on PR #966 through the mission feed. F4 removes extension maps from every known message, overview object, feed variant, pane and controller. Unknown variants retain their raw payloads. The defined `unsupportedVersion.versions` field is an explicit `Option<VersionRange>`, omitted on other errors as the spec requires, rather than an unknown-field map. The spec and m3 check plan now distinguish exact golden round trips from unknown-field compatibility checks.

The following six files keep their JSON and wire meaning unchanged, and move from exact round trips to decode-and-compare checks. Both decoded equality and serialization to the known-field form are checked:

| Compatibility file | Known-field form |
| --- | --- |
| `unknown_field.json` | `hello.json` |
| `unknown_field_status_id.json` | `status.json` |
| `unknown_fields_resized_you.json` | `resized.json` |
| `unknown_fields_resized_desktop.json` | `resized_desktop.json` |
| `unknown_fields_resized_browser.json` | `resized_browser.json` |
| `unknown_fields_resized_none.json` | `resized_none.json` |

F5 makes malformed bodies report the known request, field and missing/expected type in bounded English, while preserving the request ID. Diagnostics never copy the Serde error or a client value, including terminal data. Missing-field cases cover every request with required body fields and the handshake; mistyped numbers, nullable fields, arrays, strings and base64 include large private-value sentinels and assert exact messages of at most 120 bytes. `error_bad_request_body.json` changes only its English message to `attach: missing field cols.`; its `badRequest` code, ID and wire shape stay unchanged. All other fixture JSON remains unchanged.

Before these changes, CI for `1c19c6da` passed Windows, but the first macOS attempt exited 100 on the pre-existing CLI test `cat_echo_and_da1_reply_once_with_zero_and_two_mirrors`: unexpected lag at `daemon_process.rs:409`, then an output deadline disconnected. It passed 2,513 tests, including all 23 companion tests, with one failed and six ignored. An isolated retry of the failed case on that same commit exited 0 (`followups-ci-cat-retry.log` and `.exit`). The complete macOS job passed on attempt 2 of that same commit; both platforms were green before the F4–F5 amendment. No CLI or daemon code was changed or test excluded.

| Final F4–F5 check | Log | Exit | Result |
| --- | --- | --- | --- |
| `cargo nextest run --locked -p runner-core --cargo-profile ci companion` | `followups45-final-core.log` | 0 | 25 passed; all 59 files covered: 53 exact round trips and six known-field comparisons. |
| `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci` | `followups45-final-nextest.log` | 0 | Default concurrency: 2,516 passed, 6 pre-existing ignored tests skipped. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | `followups45-final-clippy.log` | 0 | Passed. |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | `followups45-final-clippy-updater.log` | 0 | Passed. |
| `cargo fmt --all --check` | `followups45-final-fmt.log` | 0 | Passed. |
| `git diff --check` | `followups45-final-diff.log` | 0 | Passed. |

Every exit was captured directly from its command. The earlier F4–F5 gate pass also exited 0; the final commands above reran after simplifying the known version-range field. All local check processes exited. The local watch for the superseded commit's remote CI retry was explicitly stopped; no local helper or worktree test process remains. On 2026-10-20 the whole-diff working-tree re-review returned **NO REMAINING MUST-FIX ISSUES**, including F4–F5 and the previously reviewed implementation. The reviewer independently ran 25 focused tests and a compiled API probe for missing/mistyped diagnostics without value leakage, correlated errors, nested unknown-field discard, unknown-controller payload retention and version-range serialization; both exited 0. Independent whitespace checking and every final gate log/exit were verified. The amended PR needs fresh macOS and Windows CI before final handoff.
