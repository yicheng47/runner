# 836 m6 — the host desktop (Phase 3)

Implement m6 of Phase 3 of [#836](https://github.com/yicheng47/runner/issues/836) ([#953](https://github.com/yicheng47/runner/issues/953)): the desktop's side of another Runner using this computer, which is the control card with Take back, the pairing notification and Connected now. Jason asked for this codex duo mission on 2026-10-22, ending in an open PR against `nightly-remote`; starting it authorizes the implementation, review and PR workflow below. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-836-m6-host-desktop`, on `feat/836-m6-host-desktop`, cut from `origin/nightly-remote` at `053a2907`. No other branch, worktree or target directory. Push with `git push -u origin feat/836-m6-host-desktop`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

No other computer can connect until m7 builds Runner as a client, so this mission proves itself through the app's tests, and its live checks wait for Jason.

## What matters most

- **Without another Runner nothing changes.** The desktop always controls, and every pane pushes its size exactly as today: not delayed, reordered or dropped.
- **A covered terminal gets nothing from its pane**: no size from any path and no input of any kind.
- The app follows the spec's Terminal size sections and the frames. Where this brief and the spec differ, the spec wins.
- `runnerd` and the client protocol already carry what m6 needs: `TerminalController`, `terminal_take_back`, `remote/paired` and `connected`. Change them only where the spec requires it, and say so in the PR.
- No new crate in `Cargo.lock`, no database migration, no message body or terminal byte in a log.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/features/836-web-remote-control.md`: Phase 2's Terminal size; Phase 3's Terminal size in `runnerd` and the desktop, and The desktop and the CLI.** In the spec, *browser* means a remote Runner client, and the `Browser` controller's `name` is that computer's name.
- **`docs/impls/836-web-remote-control/plan.md`, "m6 — the host desktop": the commits, the checks and Easy to get wrong.**
- **The design**, exported as PNGs in this worktree's ignored `target/design-ref/836-m6/`. Match their layout, hierarchy, spacing and colours with the existing theme tokens; the names are sample data. Do not open or edit `.pen` files, and do not delete the PNGs.
  - `1-controlled.png`: a pane controlled by MacBook Air, with Take back.
  - `2-tab-controlled.png`: a whole split tab under one card.
  - `3-has-left.png`: MacBook Air has left, with Continue here.
  - `4-light-zh.png`: the light theme with the Chinese strings.
  - `5-two-window-overlay.png`: today's two-window overlay, for comparison.
  - `6-notification-macos.png` and `7-notification-windows.png`: the system notification.
  - `8-toast.png`: the in-app toast when system notifications are off.
  - `9-devices-connected-now.png`: Remote access with Connected now. m6 takes the Connected now status and the copy without the browser or the later update; the pane's layout stays as it is, since m7 redesigns it.
- Code: `runner-core/src/protocol/terminal.rs` and `protocol/api.rs`; the app's terminal mirror and pane size push (`pane_layout.rs`, `terminal/`, `surfaces/chat.rs`); `ui/duplicate_subject_overlay.rs`; `notifications.rs` and `show_toast` in `surfaces/app_shell.rs`; `app_store.rs`'s `remote/status` and `remote/trust`; `surfaces/settings/remote.rs`; `i18n/en.toml` and `i18n/zh-hans.toml`; `docs/tests/regression/terminal.md`; `docs/guide/guides/remote-access.md`.

## Deliverables

The plan's commits, each building and passing its tests, and the validation record `docs/tests/836-m6-host-desktop.md`: the changed behaviors, each check with its reason and result, the gates with exit codes, and the live checks pending for Jason. Fold the commit already on the branch (this brief and the plan) into the first commit. Where the spec leaves a detail open, pick the plainest form, write it into the spec in the same PR and list it in the PR body. If the spec proves wrong or unsafe, stop and ask the human through Runner instead of working around it.

## Validation

Log each to a file and read the exit code from the command itself, never through `tail` or `grep`: `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`; `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`; `cargo fmt --all --check`; `git diff --check`.

## Live checks

None run in this mission. Record them as pending for Jason, not Blocked:

- TERM-RESIZE-01, TERM-RESIZE-02 and #838's two-window steps on macOS and Windows, because the pane's size push changed.
- The card, Take back, Continue here, the tab card, the notification, the toast and Connected now, once m7's client can connect.

## Boundaries

The coder and reviewer never run the app. No slot operates Jason's production Runner, its chats or missions beyond this mission's own messages; this mission runs there. Nothing contacts `runner.yeelinks.com.cn` or any other service; tests reach no network and do not sleep. Do not touch other worktrees, the `runner_server` repository or the design repository. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against the spec, the plan, the PNGs and this brief, must-fix first with file:line pointers. Must-fix:

- any change to a pane's size push while the desktop controls;
- a size pushed, or input delivered, to a terminal the desktop does not control;
- Take back not resizing each terminal to its own pane;
- a card state or string that departs from the frames or the spec, or a string missing in `zh-hans.toml`;
- a notification without its toast fallback, or one that does not open Remote access;
- Connected now not following `connected`;
- a new crate or a migration;
- a test that sleeps or reaches a network;
- a body or terminal byte in a log.

Iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff.

After clean review, Jason authorizes:

1. The commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers.
2. `git push -u origin feat/836-m6-host-desktop`, later `--force-with-lease`.
3. `gh pr create --base nightly-remote`. The body says `Part of #953 (Phase 3, m6)`, not Closes, and has the summary, the checks with exit codes, the spec details chosen, review follow-ups and the pending live checks. It has no agent session links.
4. `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it.

Jason merges the PR himself. Do not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: the PR URL and CI, the changes, the checks with exit codes, the review verdict, the spec details chosen, follow-ups, and what is left for Jason. Then all slots stand by.
