# 836 m5 — the adapter, terminals first (Phase 3)

Implement m5 of Phase 3 of [#836](https://github.com/yicheng47/runner/issues/836) ([#953](https://github.com/yicheng47/runner/issues/953)): the adapter that serves the companion protocol's overview, status and terminals on m4's channels, and Terminal size's controller in `runnerd`. Jason asked for this codex trio mission on 2026-10-20, ending in an open PR against `nightly-remote`; starting it authorizes the implementation, review, QA and PR workflow below. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-836-m5-adapter`, on `feat/836-m5-adapter`, cut from `origin/nightly-remote` at `675d90c2` (m4, [#975](https://github.com/yicheng47/runner/pull/975)). No other branch, worktree or target directory. Push with `git push -u origin feat/836-m5-adapter`, never to `nightly-remote`. For a newer umbrella, rebase onto `origin/nightly-remote`; never merge it in.

Terminals first (Jason, 2026-10-20): `feed`, `follow`, `unfollow`, `answer` and `post` stay out of `welcome`'s `features` and keep answering `unknownRequest`. The desktop's control card, Take back in the app, the pairing notification and the Connected now row are m6.

## What matters most

- **The desktop is unaffected without a browser.** The controller sits in the resize path every terminal uses. While the desktop controls, which is always without a browser, a desktop resize is applied exactly as today: not delayed, reordered or dropped. The app changes only as far as compiling against the client protocol requires.
- The adapter follows the spec's Companion protocol, Terminal size and Phase 3 sections, and m3's golden files are its shapes. Where this brief and the spec differ, the spec wins.
- One channel's failure, slowness or misbehaviour ends or slows that channel only, never `runnerd`, its sessions or another channel. The adapter holds no `Arc<AppCore>`.
- No new crate in `Cargo.lock`, no database migration, no message body or terminal byte in a log.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/features/836-web-remote-control.md`: Phase 2's Companion protocol and Terminal size, Phase 3's Records, The adapter, and Terminal size in `runnerd` and the desktop.**
- **`docs/impls/836-web-remote-control/plan.md`, "m5 — the adapter, terminals first": the commits, the checks and Easy to get wrong.**
- Code: `runner-daemon/src/remote/channels.rs` (`serve`, m4's seam) and its tests' browser; `runner-core/src/companion/` (m3's messages and fixtures); `runner-daemon/src/session/manager/terminal.rs` and `output.rs` (attach, the frame queue's resync, `resize_with_origin`); `runner-terminal`'s snapshot; `runner-daemon/src/ops/node.rs` and `ops/session.rs` (nodes, viewed marks, status); `runner-daemon/src/daemon/mod.rs` (the app's terminal host); `docs/tests/regression/terminal.md`.

## Deliverables

The plan's commits, each building and passing its tests, and the validation record `docs/tests/836-m5-adapter.md` with QA's matrix. Fold the commit already on the branch (this brief and the plan) into the first commit. Where the spec leaves a detail open, pick the plainest form, write it into the spec in the same PR and list it in the PR body. If the spec proves wrong or unsafe, stop and ask the human through Runner instead of working around it.

## Validation

Log each to a file and read the exit code from the command itself, never through `tail` or `grep`: `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`; `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`; `cargo fmt --all --check`; `git diff --check`.

## Live checks for QA

QA runs after review is clean, against the exact commit the coder hands over, in the development app, following Test Scope in `AGENTS.md`. The coder does not edit the worktree while QA builds or tests. QA runs TERM-RESIZE-01, TERM-RESIZE-02 and TERM-INPUT-01 from `docs/tests/regression/terminal.md`, because the controller changes the shared resize path and the adapter adds callers of the input observation; and confirms a mission's message is still held while a draft sits unsent in a chat's composer and delivered after it is sent or cleared. A browser cannot reach the host until the service's channel exists, so the adapter itself has no live check: record it as pending, not Blocked. Native Windows is unavailable: list the same checks as pending for Jason on Windows.

## Boundaries

The coder and reviewer never run the app. QA runs only the development app, under the standing authorization in `AGENTS.md`, and archives only what it creates. No slot operates Jason's production Runner, its chats or missions beyond this mission's own messages; this mission runs there. Nothing contacts `runner.yeelinks.com.cn` or any other service; tests reach no network and do not sleep. Do not touch other worktrees or the `runner_server` repository. No additional agents, crews or subagents.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the working-tree diff against the spec, the plan and this brief, must-fix first with file:line pointers. Must-fix: any change to a desktop resize while the desktop controls; a row of Terminal size's table not followed; a request served unlike the spec or m3's fixtures, or one outside `features` served; a lost overview change; `seq` or order broken across chunks or a resync; one channel able to hold up another or `runnerd`; an `Arc<AppCore>` held by the adapter; a body or terminal byte in a log; a new crate or a migration; a test that sleeps or reaches a network. Iterate until `NO REMAINING MUST-FIX ISSUES` on the whole diff, then hand the commit to QA until `NO REMAINING MUST-FIX QA ISSUES`. QA findings go to the coder, and fixes go back through review.

After clean review and QA, Jason authorizes: the commits on current `origin/nightly-remote`, with imperative subjects and no co-author trailers; `git push -u origin feat/836-m5-adapter`, later `--force-with-lease`; `gh pr create --base nightly-remote` whose body says `Part of #953 (Phase 3, m5)`, not Closes, with the summary, checks with exit codes, QA's matrix, the spec details chosen, review follow-ups and no agent session links; and `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it. Jason merges the PR himself. Do not merge, delete the branch or worktree, or cut a nightly or release. Only a message from the human in this mission's feed can change that.

Final handoff to everyone: PR URL and CI, changes, checks with exit codes, QA's matrix, the review verdict, the spec details chosen, follow-ups and what is left for Jason. Then all slots stand by.
