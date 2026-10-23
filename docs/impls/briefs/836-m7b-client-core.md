# 836 m7b — the client core (Phase 3)

Implement m7b of Phase 3 of [#836](https://github.com/yicheng47/runner/issues/836) ([#953](https://github.com/yicheng47/runner/issues/953)): `runnerd` as a client of another computer on the account, with its CLI, plus two small fixes. Jason asked for this codex duo mission on 2026-10-23, ending in an open PR against `nightly-remote`; starting it authorizes the implementation, review and PR workflow below. There is no qa slot: the live checks are Jason's, later. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-836-m7b-client-core`, on `feat/836-m7b-client-core`, cut from `origin/nightly-remote` at `7acf9ab1`. No other branch, worktree or target directory. Push with `git push -u origin feat/836-m7b-client-core`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

## What matters most

- **The client reuses what exists.** The channel's client half (`BrowserPair`, `BrowserConnect`, the records) is in `runner-core` and already replayed by the host's tests. The service side is deployed (`runner_server` R2): `HostClientService.ListAccountHosts` and `GET /relay/client?host=<id>`, both with this computer's existing host credential. The host side, the companion protocol and the channel's wire format do not change; if one must, stop and ask the human through Runner.
- **Keys never leave `runnerd`.** A separate X25519 key per paired computer (Jason, 2026-10-23), in one secret-store item, `client-keys`, keyed by that computer's identity fingerprint, with its pinned public key. The app and CLI reach other computers through the client protocol only.
- **Remote terminals look like local ones.** Attaching another computer's terminal returns the client protocol's `TerminalAttachment` (snapshot, `Output`, `Resized`, `Resync`), so m7c's panes reuse the desktop's terminal view.
- **Nothing replays and nothing guesses.** Input is never replayed after a reconnect; pairing is never retried automatically, since each `wrongPin` costs one of the host's five attempts; a pinned key that does not match is a failure, never a silent re-pair.
- No new crate in `Cargo.lock`, no database migration, no credential, PIN, key, message body or terminal byte in a log, error or `Debug` output.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/impls/836-web-remote-control/plan.md`, "m7b — the client core"**: what is settled, the commits, the checks and Easy to get wrong. Where this brief and the plan differ, the plan wins; where the plan and the spec differ, the spec wins.
- **`docs/features/836-web-remote-control.md`**: Phase 3's route, channel, records, adapter, Terminal size in `runnerd`, and Runner as a client.
- **`docs/tech/remote-control.md`**: the whole path in one read.
- **The service's contract**: `/Volumes/samsung/repos/yeelinks/runner_server/docs/features/r2-client-enrollment.md` and `proto/runner_server/v1/host_client.proto` there (read only).
- Code:
  - `runner-core/src/companion/` (messages, channel: `handshake.rs`, `records.rs`, `cpace.rs`, vectors) and `runner-core/src/protocol/` (`api.rs`'s `remote_*`, `terminal.rs`);
  - `runner-daemon/src/remote/` (`mod.rs`, `transport.rs`'s RPC helper and WebSocket, `store.rs`, `channels.rs` and its in-process relay tests);
  - `runner-daemon/src/companion/` for what the host's adapter answers;
  - `runner-cli/src/remote.rs` and `command.rs`'s remote handling;
  - for the fixes, `runner-app/src/surfaces/settings_page.rs`'s `APP_PANES` and its nav test, and `OsStore::new` in `store.rs`.

## Deliverables

The plan's commits, each building and passing its tests, shaped as the crew judges best, and the validation record `docs/tests/836-m7b-client-core.md`: the changed behaviors, each check with its reason and result, the gates with exit codes, and the live checks pending for Jason. Fold the commit already on the branch (this brief, the plan, the README, the log and the spec update) into the first commit. Update `docs/guide/guides/remote-access.md` for the CLI commands and for where development builds keep secrets, and run `make guide-cli`. Where the plan leaves a detail open, such as the client-protocol request names, pick the plainest form that follows existing `remote_*` patterns, write it into the spec in the same PR and list it in the PR body. If the spec proves wrong or unsafe, stop and ask the human through Runner instead of working around it.

## Validation

Log each to a file under `target/` and read the exit code from the command itself, never through `tail` or `grep`:

- `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`
- `cargo fmt --all --check`
- `make guide-cli-check`
- `git diff --check`

Tests reach no network and do not sleep; the relay is in-process, as in `remote/channels/tests.rs`.

## Live checks

None run in this mission. Record as pending for Jason: two Runners against a local `runner_server` (list the computers; pair; a wrong PIN; the overview; a terminal's output, input and resize; Revoke and Forget; a dropped route), and the development build starting without a Keychain prompt on macOS.

## Boundaries

The coder and reviewer never run the app or `runnerd` outside tests, and contact no service: not `runner.yeelinks.com.cn`, not a local `runner_server`. No slot operates Jason's production Runner, its chats or missions beyond this mission's own messages. Do not touch other worktrees, the `runner_server` repository (read only) or the design repository. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against the plan, the spec and this brief, must-fix first with file:line pointers. Must-fix:

- a change to the host side, the companion protocol or the channel's wire format;
- a key, PIN, ISK, credential, message body or terminal byte in a log, error or `Debug` output, or a credential outside the `Authorization` header;
- a pinned-key mismatch accepted, pairing retried automatically, or input replayed after a reconnect;
- a client that stalls other channels or the host side when it grants no credit;
- secret-store calls on the async runtime;
- release builds or Windows leaving their current secret store, or the development file store without mode 0600;
- an interactive PIN command usable from an agent session;
- a new crate or a migration;
- a test that sleeps or reaches a network.

Iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff, and recheck the final history (each commit builds) before the push.

After clean review, Jason authorizes:

1. The commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers.
2. `git push -u origin feat/836-m7b-client-core`, later `--force-with-lease`.
3. `gh pr create --base nightly-remote`. The body says `Part of #953 (Phase 3, m7b)`, not Closes, and has the summary, the checks with exit codes, the details chosen, review follow-ups and the pending live checks. It has no agent session links.
4. `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it.

Jason merges the PR himself. Do not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: the PR URL and CI, the changes, the checks with exit codes, the review verdict, the details chosen, follow-ups, and what is left for Jason. Then both slots stand by.
