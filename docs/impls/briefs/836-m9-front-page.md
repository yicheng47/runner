# 836 m9 — the front page on another computer (Phase 3)

Implement m9 of Phase 3 of [#836](https://github.com/yicheng47/runner/issues/836) ([#953](https://github.com/yicheng47/runner/issues/953)): a window showing another computer does what the front page does on this one, everything except Settings, on m8's companion requests. Jason made this part of the release on 2026-10-23 and asked Claude Code to drive the program; starting this codex duo mission authorizes the implementation, review and PR workflow below. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-836-m9-front-page`, on `feat/836-m9-front-page`, initially on `origin/nightly-remote` at `69387ef0` (m8 and the remote access switch merged), rebased onto its rewritten `39d957cc` with main 0.17.2 and then `cbb877c4` with the updated Local only spec per Claude Code’s 2026-10-24 mission instructions. No other branch, worktree or target directory. Push with `git push -u origin feat/836-m9-front-page`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

## What matters most

- **One front page, two computers.** The front page's actions go through one target, this computer's `runnerd` or the window's computer, so each surface runs the same code for both. m7c's separate remote views in `surfaces/remote_client/` give way to the local components wherever those can serve both. A parallel remote copy of a local surface is a must-fix unless the plan names why it has to differ.
- **Local windows do not change.** Every local surface behaves exactly as before, apart from Jason’s explicit footer-avatar ring change in Scope; its size and interactions stay unchanged. Local parity is the largest risk in m9, and the reviewer checks it first.
- **m8's protocol as it is.** `remote_request(computer, request)` and the `remote/feed` event carry everything; the companion requests are the spec's Remote actions section and m8's golden files. No `runnerd`, companion or client-protocol change; if one seems needed, stop and ask the human through Runner.
- **The result is not the state.** The remote sidebar, tabs and panes change when the overview and status pushes arrive, never from a request's result.

## Settled before this mission

An earlier mission wrote the plan and Jason archived it on 2026-10-24 after the design changed. What it left is settled; implement it, and do not reopen the design.

- **The plan.** The m9 section of `docs/impls/836-web-remote-control/plan.md` (Settled for m9, the surface table, the retirement inventory, Commits, Checks, Easy to get wrong, Landing) is reviewed by the crew and by Claude Code and committed with this brief. It splits the work into two PRs: chats, tabs, panes, drawer and sidebar first; missions and Start mission second. The first lands before the second starts in this same mission.
- **The host folder menu.** Jason decided on 2026-10-24 that there is no separate folder dialog: New chat and Start mission use their current modals, and in a remote window only the Working directory field's Browse button changes, opening a menu of the host's folders instead of the OS folder window, which cannot see the host's disk. The plan's Host folder menu bullet has the behavior. Jason approved the frames Claude Code drew in `runner/desktop/specs/836-web-remote-control.pen`, journey 3b: `k3GCC`, `QlIfT`, `kYEBx`, `iMPlw`, `hCh3b`, `J0QvG`, `b9rzdK`, `m1P4o`. Read them through the Pencil connector; do not edit the design file.
- **The picker hint.** Jason removed the computer picker's hint line on 2026-10-24 (frame 2.1 `TKakw`). Remove `client.picker_hint` from the picker (`remote_client/picker.rs`) and from both string files, in PR 1.

## Scope

As the spec's Remote actions section and the plan's m9 section lay out: New chat and the start surface with the host's catalog, the host folder menu, stop, resume, restart, fork, rename and archive; new tabs, splitting, filling and closing panes, resizing splits, closing tabs; the terminal drawer's shells; the sidebar's pin, move, reorder and rename; projects; the mission workspace in a remote window (feed, composer, answer cards, stop, resume and archive) and Start mission from an existing crew; remote mission members in the mission's own pane arrangement; the remote tab header from the local one (live-fix Problem 11: no bare title, no empty per-pane bar); the picker hint's removal and accent attention dots on the trigger and remote rows (Jason, 2026-10-24); the footer avatar’s 2 px `theme::faint()` ring, keeping its hit area and interaction styling (Jason’s final selection, mission message `01M5NY7AVERWCA6N7MRNAX7N1A`); PR 2’s first commit replacing the Remote access switch with Local only as the updated spec and approved frames `fUMPv`, `vzS3a`, `ZUHff`, `iAY5X`, `pI2HC` require (Jason, 2026-10-24; not in PR 1); controls hidden when the host lacks a request in `welcome.features`; and the guide. Out: Settings, the Roles and Crews pages, Archived.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- `docs/features/836-web-remote-control.md`: Terminal size, Runner as a client (including the m7c presentation details), and Remote actions with m8's wire details.
- `docs/impls/836-web-remote-control/plan.md`: m7c, m8 and m9 (the m9 section governs the work).
- `docs/tests/836-remote-live-fixes.md` for the presentation-parity work already done (shared sidebar tree, rows, status marks, key and paste handling).
- Code: `runner-app/src/surfaces/remote_client/`, the local surfaces they mirror (`panes.rs`, `chat.rs`, `sidebar/`, `start_chat.rs`, `start_mission.rs`, `mission_workspace/`, `app_shell.rs`), `runner-core/src/companion/` and `runner-core/src/remote/client.rs`.

## Deliverables

The plan's commits, each building and passing its tests, shaped as the crew judges best, and the validation record `docs/tests/836-m9-front-page.md`: the changed behaviors, each check with its reason and result, the gates with exit codes, and the live checks pending for Jason. Fold the commit already on the branch (this brief and the plan) into the first commit. Update the spec's Remote actions section with the client details settled, and `docs/guide/guides/remote-access.md` and `chats-and-windows.md`, in the same PR.

## Validation

Log each to a file under `target/` and read the exit code from the command itself, never through `tail` or `grep`:

- `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

Run the tests with `CLAUDE_CONFIG_DIR` unset (`env -u CLAUDE_CONFIG_DIR …`): three Claude account tests read it from the environment. Known flakes, retried once and reported: `terminal_redraw_tests::terminal_output_reuses_split_siblings_and_title_changes_update_sidebar` and `claude_accounts::tests::add_and_login_keeps_each_request_bound_to_its_created_account`. Tests reach no network and do not sleep; drive the app from fake client-protocol responses and events, including `remote_request` results and `remote/feed`.

## Live checks and boundaries

Live checks are Jason's, on a `remote-nightly` build with two Runners: start, stop, resume, fork, rename and archive a chat on the other computer; split and close panes; drawer shells; pin, move and rename; start a mission with a crew and a host folder chosen from the menu, answer and post; and the host's desktop showing each change at once. The reviewer runs none and lists them. Jason tests with the installed Runner and a development app built from `.worktrees/nightly-remote`: no slot runs `make run`, starts or stops the development daemon, or operates either app, and no slot reads a secret store, key, PIN or credential. Nothing contacts `runner.yeelinks.com.cn` or a `runner_server`. Do not touch other worktrees or the design repository. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff for each PR and reviews the whole working-tree diff against the plan, the spec, the approved frames and this brief, must-fix first with file:line pointers. Must-fix:

- any change in a local window's behavior or layout;
- a parallel remote copy of a local surface the plan did not justify;
- a `runnerd`, companion or client-protocol change;
- remote state changed from a result instead of the overview and status pushes;
- a remote control shown for a request the host does not list in `welcome.features`;
- Settings, Roles, Crews or Archived reachable from a remote window;
- a design state from the approved frames missing, or a string missing in `zh-hans.toml`;
- a new crate, a migration, or a test that sleeps or reaches a network.

Iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff, and recheck the final history (each commit builds) before the push.

After clean reviewer review, post that the whole working tree is ready for Claude Code and wait for his explicit “go” (Jason’s mission gate, 2026-10-24, message `01M5P060G8EQ0NATADTJRECNF8`). His findings return through reviewer review. Only after “go”, Jason authorizes:

1. The commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers.
2. `git push -u origin feat/836-m9-front-page`, later `--force-with-lease`.
3. `gh pr create --base nightly-remote`. The body says `Part of #953 (Phase 3, m9)`, not Closes, and has the summary, the checks with exit codes, the details chosen, review follow-ups and the pending live checks. It has no agent session links.
4. `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it.

Claude Code reviews each PR and merges it once review and CI are clean; after PR 1 merges, rebase onto `origin/nightly-remote` and continue with PR 2 on the same branch. The crew does not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: the PR URL and CI, the changes, the checks with exit codes, the review verdict, the details chosen, follow-ups, and what is left for Jason. Then both slots stand by.
