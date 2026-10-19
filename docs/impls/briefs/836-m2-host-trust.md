# 836 m2 — host trust (Phase 2)

Implement m2 of Phase 2 of [#836](https://github.com/yicheng47/runner/issues/836) ([#952](https://github.com/yicheng47/runner/issues/952)): the host's trust state. One identity key per host that survives disconnect and enrolling again, a PIN with its lock, the paired-device list and Reset identity, with their client requests, CLI and Settings. Jason asked for this codex duo mission on 2026-10-19, ending in an open PR against `nightly-remote`; starting it authorizes the implementation, review and PR workflow below. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-836-m2-host-trust`, on `feat/836-m2-host-trust`, cut from `origin/nightly-remote` at `c5d66b93`. No other branch, worktree or target directory. The branch has no upstream: push with `git push -u origin feat/836-m2-host-trust`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

Nothing can pair a device yet: the PIN exchange and the channel are Phase 3. This mission builds and tests the state Phase 3 will use.

## What matters most

- Only the person sets or changes the PIN, unlocks PIN entry or resets the identity: from the app, or at an interactive terminal outside an agent session (`login_allowed`'s check). Never through `runner call`, never in the agent guide.
- The PIN and the private key never reach a log, a file outside the secret store, a response, an event, an environment variable or `Debug` output.
- The identity key survives Disconnect, removal and enrolling again, and a host enrolled before this change keeps its fingerprint without enrolling again.
- Remote access never stops, restarts or crashes `runnerd` or its sessions. No database migration.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/features/836-web-remote-control.md`, Phase 2's Client trust and Changes to Phase 1: the contract**, and the Phase 1 sections they change (Host identity, Secrets on the host, Signing out, Platforms, Who may connect a host). The spec wins over this brief on any detail.
- **`docs/impls/836-web-remote-control/plan.md`, "m2 — host trust": the four commits and the checks.**
- Code: `crates/runner-core/src/remote/` (`HostKey`, `fingerprint`); `crates/runner-daemon/src/remote/` (`mod.rs` `start_owned`, `machine.rs`, `store.rs` `Secrets` and `OsStore`, `persistence.rs`); `crates/runner-core/src/protocol/api.rs` (the `remote_*` requests); `crates/runner-cli/src/remote.rs` (`login_allowed`), `client.rs` (`call_daemon`) and `help.rs` (the agent guide); `crates/runner-app/src/surfaces/settings/remote.rs`.

## Deliverables

The plan's four commits, each building and passing its tests: the identity key with the upgrade; the PIN, its lock and the device list; client protocol and CLI; Settings with the user guide section. Update the spec's Phase 1 sections with the code, as Changes to Phase 1 says, and write the validation record `docs/tests/836-m2-host-trust.md`. Fold the commit already on the branch (this brief and the docs) into commit 1.

**Commit 4 waits on the design.** Jason is signing off the Pencil frames for the PIN, its lock, the device list and Reset identity. The human sends reference PNGs into `target/design-ref/836-m2/` with a Runner message when they are ready. Until then, take commits 1–3 through review; if you reach commit 4 first, say so and stand by. Colors come from the app's theme tokens; where a PNG and the spec differ, ask rather than guess.

Easy to get wrong:

- Linux: `OsStore::path` ignores the name today and returns one file. The identity needs its own file, `remote-identity.json`, created `0600`.
- A `v: 1` enrollment item's key is adopted only when no identity exists, and the item is rewritten as `v: 2` without it.
- Logout, removal and the expired-claim cleanup delete only the enrollment item and `remote.json`, never the identity.
- Reset identity runs the sign-out cleanup first when enrolled, then leaves a new key, no PIN, no devices, PIN entry unlocked and state Off.
- The PIN is digits only, six or more, stored as entered. Five failures in a row lock; a success resets the count; the count survives a daemon restart.
- `remote-trust.json` is written like `remote.json`: temporary file and rename, `0600`, a bad file renamed `.bad`.
- Input without echo needs a dependency (`rpassword` proposed), pinned in `Cargo.lock` and built on both platforms.
- Gate imports and helpers used only by `#[cfg(unix)]` tests with `#[cfg(unix)]`.
- Tests use their own secret-store names and delete what they create. No test sleeps or reaches a real service.
- New UI strings in English and Simplified Chinese; the CLI stays English; `make guide-cli` after CLI help changes.

## Validation

Log each to a file and read the exit code from the command itself, never through `tail` or `grep`: `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`; `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`; `cargo fmt --all --check`; `git diff --check`; `make guide-cli-check`. The plan's live checks are Jason's, on a `remote-nightly` build; this mission does not claim them.

## Boundaries

The coder and reviewer never run the app, development or installed. No slot operates Jason's production Runner, its chats or missions; this mission runs there. Nothing contacts `runner.yeelinks.com.cn` or any real service. Never read, write or delete secret-store items other than the test-named ones your tests create. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against the spec, the plan and this brief, must-fix first with file:line pointers. Must-fix: the PIN or a key reaching a log, file, response, event, environment or `Debug`; a path to set the PIN, unlock or reset from an agent session or `runner call`; the identity deleted by logout, removal or enrolling again; the upgrade changing an enrolled host's fingerprint; remote access stopping or crashing `runnerd` or a session; a test that sleeps or touches a real secret or service. Commits 1–3 may be reviewed before 4 exists; iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff.

After clean review, Jason authorizes: the four commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers; `git push -u origin feat/836-m2-host-trust`, later `--force-with-lease`; `gh pr create --base nightly-remote` whose body says `Part of #952 (Phase 2, m2)`, not Closes, with the summary, checks with exit codes, review follow-ups, the live checks pending with Jason and no agent session links; and `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it. Jason squash-merges the PR himself. Do not merge, delete the branch or worktree, or cut a nightly or release.

Final handoff to everyone: PR URL and CI, changes, checks with exit codes, the live checks pending with Jason, the review verdict, follow-ups and what is left for Jason. Then all slots stand by.
