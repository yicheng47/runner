# 836 remote live fixes (Phase 3)

Fix the problems Jason finds while live-testing the `remote-nightly` at `b5869c0d` (m7c merged on the [#836](https://github.com/yicheng47/runner/issues/836) umbrella, Phase 3 of [#953](https://github.com/yicheng47/runner/issues/953)). Jason asked for this codex duo mission on 2026-10-23, ending in an open PR against `nightly-remote`; starting it authorizes the implementation, review and PR workflow below. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-836-remote-live-fixes`, on `fix/836-remote-live-fixes`, cut from `origin/nightly-remote` at `b5869c0d`. No other branch, worktree or target directory. Push with `git push -u origin fix/836-remote-live-fixes`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

## How this mission starts

Jason shows the problem himself, in this mission, with what he saw and how to reach it. Until he does, neither slot investigates or changes anything. Then the coder restates the problem in one or two sentences, names the expected behavior from the spec, and asks only if a load-bearing detail is missing. More problems may follow in the same mission; each is fixed and reviewed the same way, in this branch.

## The live setup, which nobody may disturb

Jason is testing with two Runners on this Mac against the same account: the installed `Runner.app` (the `remote-nightly`) and the development app built from `.worktrees/nightly-remote` with `make run`. No slot runs `make run`, starts or stops the development daemon, launches either app, or operates their chats, missions, Settings or pairing. Reading their logs is fine, read only: the development app's under `~/Library/Logs/com.wycstudios.runner-dev/` and its data directory `~/Library/Application Support/com.wycstudios.runner-dev/`, the installed app's under the same paths without `-dev`. Never read or print a secret store, key, PIN or credential, including `remote-secrets.json`, `remote-identity.json` and the Keychain. Nothing contacts `runner.yeelinks.com.cn` or a `runner_server` directly.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- `docs/features/836-web-remote-control.md`: Terminal size, Desktop app, Runner as a client, and the m7a/m7c presentation details.
- `docs/impls/836-web-remote-control/plan.md`: m7b and m7c, including Easy to get wrong.
- `docs/tech/remote-control.md` for the whole path; then the code the problem points at, usually `runner-app/src/surfaces/remote_client/`, `runner-daemon/src/remote/client/` and `runner-daemon/src/companion/`.

## Rules

- Fix the cause, not the symptom, and keep each fix scoped to the problem shown. A fix may touch the app, `runnerd` or the CLI. A change to the companion protocol, the channel's wire format, the client protocol's existing messages or the service contract needs the human's agreement through Runner first.
- Each fix gets a regression test driven by fakes or the in-process relay: no sleeps, no network.
- No key, PIN, credential, message body or terminal byte in a log, error or `Debug` output. No new crate, no migration.
- Update the spec, the guide or `docs/tests/836-remote-live-fixes.md` (the validation record: each problem, its cause, the fix, the checks with exit codes) in the same PR when behavior or a decision changes.

## Validation

Log each to a file under `target/` and read the exit code from the command itself:

- `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`
- `cargo fmt --all --check`
- `make guide-cli-check` when CLI help changes
- `git diff --check`

Run the tests with `CLAUDE_CONFIG_DIR` unset (`env -u CLAUDE_CONFIG_DIR …`): three Claude account tests read it from the environment. `terminal_redraw_tests::terminal_output_reuses_split_siblings_and_title_changes_update_sidebar` is a known flake; a failure of it alone is retried, and reported.

## Review and live checks

The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against the problem as Jason showed it, the spec and this brief, must-fix first with file:line pointers, until `NO REMAINING MUST-FIX ISSUES`. Live checks for this mission are Jason's, in the setup above; the reviewer runs none and says so in its verdict, listing what Jason should recheck.

## Authorization

After clean review, Jason authorizes:

1. Commits on current `origin/nightly-remote`, one per problem unless they are one cause, with imperative subjects and no co-author trailers; this brief folds into the first.
2. `git push -u origin fix/836-remote-live-fixes`, later `--force-with-lease`.
3. `gh pr create --base nightly-remote`. The body says `Part of #953 (Phase 3)`, not Closes, and has each problem, its cause, the fix, the checks with exit codes, and what Jason should recheck. No agent session links.
4. `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it.

Jason merges. Do not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: the PR URL and CI, the problems and their causes, the checks with exit codes, the review verdict and what Jason should recheck. Then both slots stand by.
