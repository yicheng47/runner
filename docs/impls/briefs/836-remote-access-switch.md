# 836 remote access switch (Phase 3)

Three Settings changes Jason asked for on 2026-10-23 while live-testing the `remote-nightly`, moved out of the live-fix mission ([#1010](https://github.com/yicheng47/runner/pull/1010)) so it could land: Account loses its "What the account is for" section, Remote access loses the Tailscale route choice, and Remote access gains the switch that keeps Runner a local app until a person turns remote control on. Part of Phase 3 of [#836](https://github.com/yicheng47/runner/issues/836) ([#953](https://github.com/yicheng47/runner/issues/953)). Jason asked Claude Code to drive this; starting this codex duo mission authorizes the implementation, review and PR workflow below. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-836-remote-access-switch`, on `feat/836-remote-access-switch`, cut from `origin/nightly-remote` at `864bc82b` (the live fixes). No other branch, worktree or target directory. Push with `git push -u origin feat/836-remote-access-switch`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

## The changes

1. **Account's "What the account is for" section goes.** Remove the whole section, the relay row and "Sync roles and crews · Later", and the footer "Tailscale needs no account…". Signed in, Account shows the status card (account, service, this computer, fingerprint) and Sign out on this computer, nothing else; later account features add their own section when they ship. Drop the unused strings in both locales.
2. **Remote access has one route.** Tailscale moved out of the release on 2026-10-23 into its own future feature, [#954](https://github.com/yicheng47/runner/issues/954). Remove the Tailscale card and the choose-a-route step: a first visit, and Reset remote access, open on the relay's signed-out state. The choice comes back with Tailscale.
3. **The Remote access switch** (private beta). Runner never pushes remote control on someone who only wants a local app. The switch is the first row of Settings → Remote access, which stays in the Settings list either way so people find it: Remote access, with the existing Private beta badge, a one-line description (it lets your computers open each other through Runner's relay, invite-only for now) and the switch.
   - **Off** is the default for a new install while the relay is invite-only. The pane then shows only that row and a short explanation of what turning it on adds: the computer picker, the account and pairing. No window shows the computer picker, the sidebar footer shows the plain Settings gear instead of the account avatar or person icon, and Account leaves the Settings list.
   - **On** shows those surfaces again and the pane continues as today: the relay's sign-in when signed out, then the relay page. Turning it on does not start sign-in.
   - **Never hidden while signed in.** An install that is already signed in reads as on, including after `runner remote login` from the CLI. Turning the switch off while signed in asks first with the Sign out confirmation and signs the computer out; the identity, PIN and paired computers stay, as Sign out keeps them.
   - Stored with the existing app settings; no migration, and no `runnerd`, client-protocol or companion change.

## Design first

Pull `~/repos/yeelinks/yeelinks_design`, then draw in `runner/desktop/specs/836-web-remote-control.pen`: the Remote access pane with the switch off and on (signed out), the sidebar footer with the switch off, and the updated Account and first-visit Remote access frames for changes 1 and 2, in the file's journey layout. Commit and push the design repo when the session ends. Post the frame ids to the human through Runner and implement change 3 only after the human approves them; changes 1 and 2 may proceed with their frames updated.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- `docs/features/836-web-remote-control.md`: Runner as a client (The account, Remote access, m7a and m7c presentation details, the signed-out picker).
- Code: `runner-app/src/surfaces/settings/remote.rs` and `remote/account.rs`, the settings navigation, the sidebar footer and the computer picker in `surfaces/remote_client/`, the app settings store, and `i18n/en.toml` and `i18n/zh-hans.toml`.

## Deliverables

One commit per change, each building and passing its tests, with a fake-driven regression test for each behavior; the spec's Account and Remote access bullets and `docs/guide/guides/remote-access.md` (and the guide page that lists Settings panes, if any) updated in the same commits; and the validation record `docs/tests/836-remote-access-switch.md`: each change, each check with its reason and result, the gates with exit codes, and what Jason should recheck. Fold the commit already on the branch (this brief) into the first commit.

## Validation

Log each to a file under `target/` and read the exit code from the command itself:

- `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

Run the tests with `CLAUDE_CONFIG_DIR` unset (`env -u CLAUDE_CONFIG_DIR …`). `terminal_redraw_tests::terminal_output_reuses_split_siblings_and_title_changes_update_sidebar` is a known flake; a failure of it alone is retried, and reported. Tests reach no network and do not sleep.

## Live checks and boundaries

Live checks are Jason's; the reviewer runs none and lists what he should recheck. Jason is testing with the installed Runner and a development app built from `.worktrees/nightly-remote`: no slot runs `make run`, starts or stops the development daemon, or operates either app, and no slot reads a secret store, key, PIN or credential. Nothing contacts `runner.yeelinks.com.cn` or a `runner_server`. Do not touch other worktrees. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against this brief, the spec and the approved frames, must-fix first with file:line pointers, until `NO REMAINING MUST-FIX ISSUES`. Must-fix: a remote surface hidden while this computer is signed in; turning the switch off without the Sign out confirmation; a `runnerd`, protocol or companion change; a migration or new crate; a string missing in `zh-hans.toml`; a test that sleeps or reaches a network.

After clean review, Jason authorizes:

1. The commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers.
2. `git push -u origin feat/836-remote-access-switch`, later `--force-with-lease`.
3. `gh pr create --base nightly-remote`. The body says `Part of #953 (Phase 3)`, not Closes, and has each change, the checks with exit codes and what Jason should recheck. No agent session links.
4. `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it.

Claude Code reviews the PR and merges it once review and CI are clean. The crew does not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: the PR URL and CI, the changes, the checks with exit codes, the review verdict and what Jason should recheck. Then both slots stand by.

## Added change 4 — picker availability (Jason, 2026-10-23)

Runner human message `01M5MQ3Q8R1BDCFM5XK974E83B` adds one app-only change under the same review and PR authorization: every remote computer in the picker shows directory online/offline status, including last seen when available, whether paired or not. An unpaired row appends Not paired after the status, for example Online · Not paired. Preserve its row layout, window button and ⋯. Check and update journey frame 2.1 before implementation. One commit includes the change, a fake-driven regression for online and offline unpaired computers, both locales and the spec's picker wording.

Frame `TKakw` was updated and pushed in design commit `a99aeb3`. The human authorized preserving and including the design file's pre-existing saved edits in message `01M5MQ8W18EGE53F5X63K5V122`. The switch frames were separately approved in message `01M5MQ4W8VJEQ4SP6Q5DG9RM16`.
