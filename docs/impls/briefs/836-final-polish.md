# 836 final polish (Phase 3)

Polish the remote-control desktop UI with Jason before the [#836](https://github.com/yicheng47/runner/issues/836) umbrella lands on `main` (Phase 3 of [#953](https://github.com/yicheng47/runner/issues/953)). Jason asked for this single-coder mission on 2026-10-25, ending in an open PR against `nightly-remote`; starting it authorizes the implementation and the PR workflow below. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-836-final-polish`, on `fix/836-final-polish`, cut from `origin/nightly-remote` at `ad3ec232`. No other branch, worktree or target directory. The branch has no upstream: push with `git push -u origin fix/836-final-polish`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

## First item: the account circle border

The sidebar's account circle goes back to a 1px border. In `crates/runner-app/src/surfaces/app_shell/account_menu.rs`, the circle around the avatar (about line 191) changes `.border_2()` to `.border_1()` and keeps `theme::faint()` as its color. m9's first PR (`6567c064`) made it 2px; Jason checked it visually and prefers 1px, which lets the button's hover background read better. The button's hover and focus styles, the circle's 28px size and the 26px avatar stay as they are. Make this change, run the validation below, then tell Jason it is ready to check.

## Further items

Jason brings any further polish himself, in this mission, with what he saw and where. Until he does, change nothing beyond the first item. For each one, restate it in one or two sentences, name the code it points at, and ask only if a load-bearing detail is missing. Keep each change to the item shown. Polish means presentation and small app-side fixes: a change to `runnerd`, the companion protocol, the client protocol, the channel's wire format or the service contract, or a new layout that has no Pencil frame, needs Jason's agreement through Runner first.

## The live setup, which nobody may disturb

Jason checks each change himself in the development app built from this worktree. The coder never runs `make run`, starts or stops the development daemon, launches either app, or operates their chats, missions, Settings or pairing. Reading logs is fine, read only: the development app's under `~/Library/Logs/com.wycstudios.runner-dev/`, the installed app's under the same path without `-dev`. Never read or print a secret store, key, PIN or credential, including `remote-secrets.json`, `remote-identity.json` and the Keychain. Nothing contacts `runner.yeelinks.com.cn` or a `runner_server` directly.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Engineering Conventions, Crew Missions.
- `docs/features/836-web-remote-control.md`: Desktop app and Runner as a client.
- The code each item points at.

## Rules

- Follow the surrounding code's patterns. No new crate, no migration, no refactor beyond the item.
- A change a test can drive (fakes or in-process, no sleeps, no network) gets a regression test; a pure style change such as a border width needs none.
- New or changed strings go into `i18n/en.toml` and `i18n/zh-hans.toml` together.
- Update `docs/guide/` when what a person sees or operates changes in a way it describes, and `docs/tests/836-release-checks.md` when a release check changes.
- No key, PIN, credential, message body or terminal byte in a log, error or `Debug` output.

## Validation

Before each handoff to Jason and before each push, log each to a file under `target/` and read the exit code from the command itself:

- `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

Run the tests with `CLAUDE_CONFIG_DIR` unset (`env -u CLAUDE_CONFIG_DIR …`): three Claude account tests read it from the environment. `terminal_redraw_tests::terminal_output_reuses_split_siblings_and_title_changes_update_sidebar` is a known flake; a failure of it alone is retried, and reported.

## Review

This crew has no reviewer: Jason's live check is the review. Changes stay uncommitted until he confirms an item ("this works", "good", "ship it"); an item he rejects is reworked in the working tree.

## Authorization

When Jason says the polishing is done, with every item confirmed, he authorizes:

1. Commits on current `origin/nightly-remote`, one per item unless several are one change, with imperative subjects and no co-author trailers; this brief folds into the first.
2. `git push -u origin fix/836-final-polish`, later `--force-with-lease`.
3. `gh pr create --base nightly-remote`. The body says `Part of #953 (Phase 3)`, not Closes, and lists each item, the change, and the checks with exit codes. No agent session links.
4. `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects.

Jason merges. Do not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to the human: the PR URL and CI, the items and their changes, and the checks with exit codes. Then stand by.
