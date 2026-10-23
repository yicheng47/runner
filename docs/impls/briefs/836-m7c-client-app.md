# 836 m7c — the client app (Phase 3)

Implement m7c of Phase 3 of [#836](https://github.com/yicheng47/runner/issues/836) ([#953](https://github.com/yicheng47/runner/issues/953)): the computers list, the PIN dialog, the computer picker, and a window that becomes another computer's Runner with live terminals. Jason asked for this codex duo mission on 2026-10-23, ending in an open PR against `nightly-remote`; starting it authorizes the implementation, review and PR workflow below. There is no qa slot: the live checks are Jason's. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-836-m7c-client-app`, on `feat/836-m7c-client-app`, cut from `origin/nightly-remote` at `12b9b80a` (m7b). No other branch, worktree or target directory. Push with `git push -u origin feat/836-m7c-client-app`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

## What matters most

- **App only, on m7b as it is.** `remote_computers`, `remote_pair`, `remote_forget`, `remote_overview`, `remote_follow`, `remote_take_control`, `attach_remote` and the `remote/overview`, `remote/session-status` and `remote/connection` events. No `runnerd`, client-protocol, companion or CLI change; if one seems needed, stop and ask the human through Runner.
- **A remote terminal is a terminal.** Remote panes reuse the desktop's terminal view through the `TerminalAttachment` m7b returns, with input and resize through `RemoteTerminal::address()`.
- **Local and remote never mix.** A window showing another computer shows only its overview; local tabs, panes and sessions are untouched and come back when the window returns to this computer.
- **Nothing guesses.** One `remote_pair` per PIN submit, never retried; nothing typed while reconnecting is replayed.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/impls/836-web-remote-control/plan.md`, "m7c — the client app"**: what is settled, the commits, the checks and Easy to get wrong. Where this brief and the plan differ, the plan wins; where the plan and the spec differ, the spec wins.
- **`docs/features/836-web-remote-control.md`**: Terminal size, Desktop app, and Runner as a client (Remote access, The app, the m7b protocol details).
- **The design**, exported as PNGs in this worktree's ignored `target/design-ref/836-m7c/`, named by journey step. Match their layout, hierarchy, spacing and colours with the existing theme tokens; names and values are sample data. Do not open or edit `.pen` files, and do not delete the PNGs.
  - `1.7`, `5.1`, `5.2`: Remote access's computers list, ready, paused and reconnecting.
  - `2.1`–`2.7`: the picker and the PIN dialog's states.
  - `3.1`–`3.5`: switching, another computer's Runner, a terminal connecting, live, and taken back.
  - `4.1`–`4.3`: reconnecting, can't reach, revoked.
  - `host-side-m6/` is what m6 built on the host, for reference only.
- Code:
  - `runner-core/src/remote/client.rs`, `protocol/api.rs`'s `remote_*`, `protocol/client.rs`'s `attach_remote`, `protocol/terminal.rs`;
  - `runner-app`'s `app_store.rs` (`remote/*` events), `surfaces/settings/remote.rs`, `surfaces/app_shell.rs` and the sidebar, the terminal view and `terminal/control.rs` from m6;
  - `i18n/en.toml` and `i18n/zh-hans.toml`; `docs/guide/guides/remote-access.md`.

## Deliverables

The plan's commits, each building and passing its tests, shaped as the crew judges best, and the validation record `docs/tests/836-m7c-client-app.md`: the changed behaviors, each check with its reason and result, the gates with exit codes, and the live checks pending for Jason. Fold the commit already on the branch (this brief, the plan, the README and the log) into the first commit. Where the plan leaves a detail open, pick the plainest form, write it into the spec in the same PR and list it in the PR body. If the spec proves wrong or unsafe, stop and ask the human through Runner instead of working around it.

## Validation

Log each to a file under `target/` and read the exit code from the command itself, never through `tail` or `grep`:

- `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

Tests reach no network and do not sleep; drive the app's state from fake client-protocol responses and events.

## Live checks

None run in this mission. Record as pending for Jason, on a `remote-nightly` build, macOS and Windows: two Runners against a local `runner_server`, then the hosted service: pair from the list and from the picker; a wrong PIN; open the other computer; open a terminal, type and resize; the host taking it back; a dropped route; Revoke; and m6's card, notification and Connected now on the host.

## Boundaries

The coder and reviewer never run the app or `runnerd` outside tests, and contact no service. No slot operates Jason's production Runner, its chats or missions beyond this mission's own messages. Do not touch other worktrees, the `runner_server` repository or the design repository. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against the plan, the spec, the PNGs and this brief, must-fix first with file:line pointers. Must-fix:

- a change to `runnerd`, the client protocol, the companion protocol or the CLI;
- local and remote state mixing in a window, or local tabs changed by visiting another computer;
- remote input going anywhere but its remote address, or replayed after a reconnect;
- pairing retried automatically, or the PIN in a log, event or `Debug`;
- a design state from the PNGs missing, or the sidebar's layout broken on either platform;
- a string missing in `zh-hans.toml`;
- a new crate or a migration;
- a test that sleeps or reaches a network.

Iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff, and recheck the final history (each commit builds) before the push.

After clean review, Jason authorizes:

1. The commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers.
2. `git push -u origin feat/836-m7c-client-app`, later `--force-with-lease`.
3. `gh pr create --base nightly-remote`. The body says `Part of #953 (Phase 3, m7c)`, not Closes, and has the summary, the checks with exit codes, the details chosen, review follow-ups and the pending live checks. It has no agent session links.
4. `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it.

Jason merges the PR himself; Claude Code also reviews it for him and may post findings to this mission. Do not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: the PR URL and CI, the changes, the checks with exit codes, the review verdict, the details chosen, follow-ups, and what is left for Jason. Then both slots stand by.

## Review follow-up (2026-10-23)

Claude Code reviewed PR [#1005](https://github.com/yicheng47/runner/pull/1005) at `114d5cec` for Jason, who asked for this follow-up codex duo mission on the same branch. It runs in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-836-m7c-client-app`, which replaces the path above; everything else above still holds unless this section says otherwise. App only, as before: no `runnerd`, protocol, companion or CLI change.

Fix, each with a fake-driven test where the behavior is testable:

1. **Revoke can reach another computer.** `remote_client/model.rs:446` and `picker.rs:273-280` match paired devices to account computers by name alone, so two computers with the same name both show ↙ and Revoke on either revokes both (`mod.rs:133-141`). A device name that matches more than one account computer is ambiguous: list those devices as unmatched rows, each with its own Revoke, and attribute ↙ to neither computer.
2. **Mission panes split unevenly.** `workspace.rs:56-66` folds members into nested 50/50 splits, so three members get 25/25/50 and four 12.5/12.5/25/50. Members share the width equally.
3. **One slow computer blocks every request.** `model.rs:121-179` runs directory refreshes, the overview of every paired online computer, follows, retries and forgets in one serial loop, so opening the picker can delay choosing a computer or Forget behind a slow overview. Visiting, retrying and forgetting must not wait for another computer's overview; keep one `remote_pair` per submit and no retries.
4. **A local Forget reads as a revocation.** `Update::Forgotten` sets `Connection::Revoked` (`model.rs:267-276`), so pairing again says the other computer revoked this one (`pairing.rs:69`) and an open window shows the revoked card. Forgetting here is not a revocation; use the plain pairing copy and an accurate state.
5. **Small ones.** A mission with no shells draws an empty strip (`workspace.rs:724-731`); a column split's second pane gets a left border instead of a top one (`workspace.rs:812-820`); a failed Take control is silently dropped (`workspace.rs:989-991`) and should say so in the pane; choosing the computer a window already shows retakes control of every terminal (`workspace.rs:92-106`) and should keep the window as it is unless its connection has failed.

Not in scope, waiting on Jason: whether signed-off frame 3.5's typing-disabled behavior is his call (keep the current behavior), and whether the computer picker should show when signed out (keep it as is). The design PNGs are not on this machine; do not open `.pen` files.

Validation: the gates above, logged with their exit codes, and the validation record updated with this round. Review follows the reviewer role's live-validation rule and the current `AGENTS.md` Test Scope, which replace this brief's earlier "no live checks" and "never run the app" lines for the reviewer only; the coder still never runs the app. Pairing, remote terminals and anything else that needs a second computer remain Jason's and are recorded as left to him. Nothing contacts `runner.yeelinks.com.cn`, and no slot starts or contacts a `runner_server`.

Authorization is as above: fold the fixes into the branch's commit, `git push --force-with-lease`, update the PR body (review follow-ups, checks, pending live checks), and drive macOS and Windows CI green. Jason merges.
