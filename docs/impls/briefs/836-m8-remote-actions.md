# 836 m8 — remote actions on the host (Phase 3)

Implement m8 of Phase 3 of [#836](https://github.com/yicheng47/runner/issues/836) ([#953](https://github.com/yicheng47/runner/issues/953)): the companion requests that let a paired Runner do on this computer what its front page does, everything except Settings, served by the host and carried through `runnerd`'s client. Jason made this part of the release on 2026-10-23 and asked Claude Code to drive it; starting this codex duo mission authorizes the implementation, review and PR workflow below. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-836-m8-remote-actions`, on `feat/836-m8-remote-actions`, cut from `origin/nightly-remote` at `864bc82b` (the live fixes). No other branch, worktree or target directory. Push with `git push -u origin feat/836-m8-remote-actions`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

## What matters most

- **One operation, two callers.** Each request calls the `AppCore` operation the matching client-protocol request calls, so a remote action has the desktop's validation, events and side effects. A handler that writes state itself is a must-fix.
- **Mirrors, not passthrough.** Each action is its own companion request with camelCase fields and a golden file, listed in `welcome.features`. The client protocol is never carried over the channel.
- **The allowlist is the spec's.** Nothing in Settings, no role or crew editing, no unarchive or delete, and nothing that changes pairing, the PIN, the identity or remote state.
- **No app UI.** The app's side is m9's. Existing companion messages and the channel's wire format stay unchanged.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/impls/836-web-remote-control/plan.md`, "m8 — remote actions on the host"**: what is settled, the commits, the checks and Easy to get wrong. Where this brief and the plan differ, the plan wins; where the plan and the spec differ, the spec wins.
- **`docs/features/836-web-remote-control.md`**: Companion protocol (messages, versions, errors, fixtures), Terminal size, Runner as a client, and Remote actions.
- Code:
  - `runner-core/src/companion/` (messages, overview, fixtures) and `protocol/api.rs`, whose operations the requests mirror;
  - `runner-daemon/src/companion/` (the adapter's dispatch and `core.rs`), `runner-daemon/src/remote/client/` (m7b's client), and the `ops` each mirrored request calls;
  - `docs/tech/remote-control.md` for the whole path.

## Deliverables

The plan's commits, each building and passing its tests, shaped as the crew judges best, and the validation record `docs/tests/836-m8-remote-actions.md`: the changed behaviors, each check with its reason and result, and the gates with exit codes. Fold the commit already on the branch (this brief, the spec section and the plan) into the first commit. The spec's request table is a proposal: settle each request's fields in its golden file, pick the plainest form where the plan leaves a detail open, write it into the spec's Remote actions section as a wire-details bullet, and list it in the PR body. If the spec proves wrong or unsafe, stop and ask the human through Runner instead of working around it.

## Validation

Log each to a file under `target/` and read the exit code from the command itself, never through `tail` or `grep`:

- `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`
- `cargo fmt --all --check`
- `make guide-cli-check` if CLI help changes
- `git diff --check`

Run the tests with `CLAUDE_CONFIG_DIR` unset (`env -u CLAUDE_CONFIG_DIR …`): three Claude account tests read it from the environment. `terminal_redraw_tests::terminal_output_reuses_split_siblings_and_title_changes_update_sidebar` is a known flake; a failure of it alone is retried, and reported. Tests reach no network and do not sleep; drive the host and client over the in-process relay with fakes.

## Live checks

None. m8 has no UI; m9 brings the actions to the app and its live checks to Jason. The reviewer's verdict says so.

## Boundaries

The coder and reviewer never run the app or `runnerd` outside tests, and contact no service, including `runner.yeelinks.com.cn` and any `runner_server`. Jason is testing with the installed Runner and a development app built from `.worktrees/nightly-remote`: no slot runs `make run`, starts or stops the development daemon, or operates either app, and no slot reads a secret store, key, PIN or credential. Do not touch other worktrees, the `runner_server` repository or the design repository. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against the plan, the spec and this brief, must-fix first with file:line pointers. Must-fix:

- a handler that bypasses the mirrored operation, or an action outside the spec's allowlist;
- a change to an existing companion message, the channel's wire format or an existing client-protocol message;
- a request missing its golden file or its `features` entry;
- `catalog` or `folders` carrying a credential, a configuration path, an environment value or a file name;
- a remote request reachable through `runner call`;
- a credential, PIN, message body or terminal byte in a log, an error or `Debug`;
- a new crate or a migration;
- a test that sleeps or reaches a network.

Iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff, and recheck the final history (each commit builds) before the push.

After clean review, Jason authorizes:

1. The commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers.
2. `git push -u origin feat/836-m8-remote-actions`, later `--force-with-lease`.
3. `gh pr create --base nightly-remote`. The body says `Part of #953 (Phase 3, m8)`, not Closes, and has the summary, the checks with exit codes, the details chosen and review follow-ups. It has no agent session links.
4. `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it.

Claude Code reviews the PR and merges it once review and CI are clean; it may post findings to this mission. The crew does not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: the PR URL and CI, the changes, the checks with exit codes, the review verdict, the details chosen and follow-ups. Then both slots stand by.
