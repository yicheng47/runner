# 836 m7a — Account (Phase 3)

Implement m7a of Phase 3 of [#836](https://github.com/yicheng47/runner/issues/836) ([#953](https://github.com/yicheng47/runner/issues/953)): Settings → Account, the sidebar's avatar menu, and Remote access's relay pages with the route choice. Jason asked for this codex duo mission on 2026-10-22, ending in an open PR against `nightly-remote`; starting it authorizes the implementation, review and PR workflow below. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-836-m7a-account`, on `feat/836-m7a-account`, cut from `origin/nightly-remote` at `165ecc0b` (m6, [#989](https://github.com/yicheng47/runner/pull/989)). No other branch, worktree or target directory. Push with `git push -u origin feat/836-m7a-account`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

This is an app-only change. Signing in is Phase 1's enrollment, unchanged, and the service side (`runner_server`'s R2) is already deployed. Nothing here reaches another computer: the computers list and the client are m7b and m7c.

## What matters most

- **Same behaviour, new places.** Sign in, Sign out, Retry, Pause and Reset reuse the client protocol's existing `remote_*` requests as they are. There is no `runnerd`, client-protocol or CLI change, and every Phase 1 and m2 state keeps its behaviour in its new layout.
- **Signed in or not is the one switch.** Account shows the sign-in's state. Remote access shows the route choice while signed out and the relay page while signed in.
- The app follows the spec's Runner as a client section (One route, chosen once; The account; Remote access) and the frames. Where this brief and the spec differ, the spec wins.
- No new crate in `Cargo.lock`, no database migration, no credential, code or fingerprint in a log.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/features/836-web-remote-control.md`**: Phase 1's Desktop app; Phase 3's Runner as a client, especially *One route, chosen once*, *The account* and *Remote access*; and Changes to Phases 1 and 2.
- **`docs/impls/836-web-remote-control/plan.md`, "m7a — Account"**: the commits, the checks and Easy to get wrong.
- **The design**, exported as PNGs in this worktree's ignored `target/design-ref/836-m7a/`. Match their layout, hierarchy, spacing and colours with the existing theme tokens; names and values are sample data. Do not open or edit `.pen` files, and do not delete the PNGs.
  - `01`–`08`: the avatar menu signed out and in; Account signed out, waiting, signed in and removed; Sign out's confirmation; every stop reason.
  - `09`–`15`: Remote access's route choice; the relay page ready, reconnecting and paused; Reset remote access; PIN entry locked; no PIN.
  - The Tailscale pages (`14`, `15`) are for their PIN rows only. The computers list on `10`–`15`, with its Access column and Pair, is m7c's. m7a keeps today's device list with m6's Connected now under the same heading.
- Code:
  - `surfaces/settings/remote.rs`, `surfaces/settings_page.rs`;
  - the sidebar footer in `surfaces/app_shell.rs`, with its layout test;
  - `app_store.rs`'s `remote/status` and `remote/trust`;
  - `runner-core/src/protocol/api.rs`'s `remote_*` requests and `runner-core/src/remote/`;
  - `i18n/en.toml` and `i18n/zh-hans.toml`;
  - `docs/guide/guides/remote-access.md`.

## Deliverables

The plan's commits, each building and passing its tests, and the validation record `docs/tests/836-m7a-account.md`. It lists the changed behaviors, each check with its reason and result, the gates with exit codes, and the live checks pending for Jason. Fold the commit already on the branch (this brief, the plan and the record's spec update) into the first commit. Where the spec leaves a detail open, pick the plainest form, write it into the spec in the same PR and list it in the PR body. If the spec proves wrong or unsafe, stop and ask the human through Runner instead of working around it.

## Validation

Log each to a file and read the exit code from the command itself, never through `tail` or `grep`:

- `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

## Live checks

None run in this mission. Record as pending for Jason, on a `remote-nightly` build on macOS and Windows:

- sign in from Account against the hosted service, and from Remote access's route choice;
- the avatar and its menu in both states;
- sign out and its confirmation;
- pause and resume;
- Reset remote access returning to the route choice.

## Boundaries

The coder and reviewer never run the app. No slot operates Jason's production Runner, its chats or missions beyond this mission's own messages; this mission runs there. Nothing contacts `runner.yeelinks.com.cn` or any other service; tests reach no network and do not sleep. Do not touch other worktrees, the `runner_server` repository or the design repository. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against the spec, the plan, the PNGs and this brief, must-fix first with file:line pointers. Must-fix:

- a change to `runnerd`, the client protocol or the CLI;
- a Phase 1 or m2 state lost, or behaving differently, in its new place;
- Sign out without its confirmation, or Reset not returning to the route choice;
- Tailscale's choice enabled;
- the avatar or menu departing from the frames, or the sidebar footer's layout broken on either platform;
- a string missing in `zh-hans.toml`;
- a credential, code or fingerprint in a log;
- a new crate or a migration;
- a test that sleeps or reaches a network.

Iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff.

After clean review, Jason authorizes:

1. The commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers.
2. `git push -u origin feat/836-m7a-account`, later `--force-with-lease`.
3. `gh pr create --base nightly-remote`. The body says `Part of #953 (Phase 3, m7a)`, not Closes, and has the summary, the checks with exit codes, the spec details chosen, review follow-ups and the pending live checks. It has no agent session links.
4. `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it.

Jason merges the PR himself. Do not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: the PR URL and CI, the changes, the checks with exit codes, the review verdict, the spec details chosen, follow-ups, and what is left for Jason. Then both slots stand by.
