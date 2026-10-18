# 836 — connect a host (Phase 1)

Implement Phase 1 of [#836](https://github.com/yicheng47/runner/issues/836): a person connects this computer to their account from Settings → Remote access or `runner remote login`, and `runnerd` keeps an outbound connection to the remote access service, whose side went live on 2026-10-16. Jason asked for this codex duo mission on 2026-10-18, ending in an open PR; starting it authorizes the implementation, review and open-PR workflow below. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-836-connect-host`, on `feat/836-connect-host`, based on `main` `f27c74a8`. No other branch, worktree or target directory. The spec's update, its README line and this brief are uncommitted in the worktree; fold them into commit 1. If you need a newer main, rebase onto `origin/main`; never merge it in.

This is a restart. Jason stopped the first run minutes in so the spec could be detailed; it has since been rewritten. That run left an uncommitted draft: `crates/runner-core/src/remote/`, `crates/runner-daemon/src/remote/`, and edits to both crates' `Cargo.toml` and `lib.rs` and to `Cargo.lock`. It was written against the earlier spec. Treat it as a draft you own: keep what matches the spec now, rewrite or delete what does not. In particular, the pending-removal state is gone (Disconnect is local first).

## What matters most

Only the person connects a computer, never an agent. The key and credential live only in the OS secret store, and never in a log, the database, an environment variable or the client protocol. Remote access never stops, restarts or crashes `runnerd` or its sessions. Runner speaks the service's shipped contract exactly. Everything else follows the spec; edge cases get the spec's handling and no more.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/features/836-web-remote-control.md`, Phase 1 in full: the contract.** Talking to the service is the service's wire contract, and its JSON examples are the golden fixtures. Its Implementation section lists the five commits. It wins over this brief on any detail.
- Runner: `crates/runner-core/src/protocol/api.rs` (`daemon_api!`, where requests are declared), `crates/runner-cli/src/client.rs` `call_daemon` (the `runner call` table, which must not gain `remote_*`), `crates/runner-cli/src/env.rs` (`RUNNER_HANDLE`), `crates/runner-daemon/Cargo.toml` (`zed-reqwest`), `crates/runner-app/src/surfaces/settings_page.rs` and `surfaces/settings/notifications.rs` (the pane pattern), `crates/runner-app/src/i18n.rs`.

## Deliverables

The spec's five commits, each building and passing its tests: wire types and identity; the secret store; the remote task with the fake-service integration tests; client protocol and CLI; Settings → Remote access with the user guide page.

The design is settled: Jason chose the status-card layout. Commit 5 follows the reference PNGs in this worktree's `target/design-ref/836/`, from `1-not-connected.png` to `9-stop-reasons.png`, exported from `design/specs/836-web-remote-control.pen` (`main` `d0a4fb73`, not yet pushed). The spec's state table is the content, the PNGs are the layout; where they differ, ask the lead rather than guess. Colors come from the app's existing theme tokens, and the light PNG shows the light theme.

Easy to get wrong:

- The device code, the credential and key material are never logged, printed after enrollment, put in an error message or `Debug` output; anything with the `rhc_` prefix is redacted from logs.
- The upgrade carries no `Origin` header, and checks `Sec-WebSocket-Accept` and `runner-relay.v1`.
- The state machine takes `now` and its random source as inputs and returns its next deadline; tests never sleep. Backoff, the 75 s dead-connection timer, the 60 s reset and the 1–10 s jitter after 1012 are all tested through the fake clock.
- Disconnect deletes the secret-store item and `remote.json` before it calls `SignOutHost`, so a crash or an unreachable service still leaves the computer disconnected; the call is made once and never retried.
- Stopped, removed keeps the key and credential (Try again); Connect again runs the sign-out cleanup first.
- No `unwrap` or panic on anything the service sends; the task's failure is logged and leaves the daemon serving.
- Keychain items are not synchronizable; Windows credentials use `CRED_PERSIST_LOCAL_MACHINE`; development builds use the `runner-dev` names; tests use their own names and delete what they create. If a CI runner cannot reach its secret store, report it in the PR rather than silently skipping the test.
- Golden fixtures are the spec's JSON examples, verbatim.
- `zed-reqwest` gains `macos-system-configuration` for `runner-daemon` only.
- New UI strings get English and Simplified Chinese; the CLI stays English. Regenerate the CLI reference with `make guide-cli`.

## Validation

Log each to a file and read the exit code from the command itself, never through `tail` or `grep`: `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`; `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`; `cargo fmt --all --check`; `git diff --check`; and `make guide-cli-check`. Gate imports and helpers used only by `#[cfg(unix)]` tests with `#[cfg(unix)]`.

Live checks are pending with Jason in a separate session: the spec's Live verification list, against a development service and then the hosted one, on macOS and Windows. This mission does not claim their results.

## Boundaries

The coder and reviewer never run the development app. No slot operates Jason's production Runner, its chats or missions; this mission runs there. Nothing contacts `runner.yeelinks.com.cn` or any real service: every test uses the in-suite fake service. Never read, write or delete secret-store items other than the test-named ones your tests create. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the diff against the spec and this brief, must-fix first with file:line pointers. Must-fix: a secret reaching a log, file, message or environment; an agent path to enrollment (no TTY, `RUNNER_HANDLE`, `runner call`); a path where remote access stops or crashes `runnerd` or a session; a wire difference from Talking to the service; a test that sleeps or touches a real secret or service. Commits 1–4 may be reviewed before 5 exists; iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff, then proceed.

After clean review, Jason authorizes: the five commits on current `origin/main` with imperative subjects and no co-author trailers; `git push --force-with-lease` of `feat/836-connect-host`; `gh pr create --base main` whose body says `Part of #836 (Phase 1)`, not Closes, with the summary, checks with exit codes, review follow-ups, the live checks pending with Jason (including Windows) and no agent session links; and `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it. Do not merge, delete the branch or worktree, or cut a nightly or release.

Final handoff to everyone: PR URL and CI, changes, checks with exit codes, the live checks pending with Jason, the review verdict, follow-ups and what is left for Jason. Then all slots stand by.
