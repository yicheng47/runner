# 791 PR 3 — Fix the four status and key bugs, with a full QA regression

Implement phase 3 of [#791](https://github.com/yicheng47/runner/issues/791): fix [#784](https://github.com/yicheng47/runner/issues/784), [#785](https://github.com/yicheng47/runner/issues/785), [#786](https://github.com/yicheng47/runner/issues/786) and [#781](https://github.com/yicheng47/runner/issues/781), and run a full live regression on the five agent runtimes. Jason requested this mission on 2026-10-03 after PR 1 (#796) and PR 2 (#798) merged. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-791-status-key-bugs`, on the existing branch `fix/791-status-key-bugs`, created from `origin/main` at `e264ed5d`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory.

The spec is [`docs/features/791-session-state.md`](../../features/791-session-state.md). Read it, all four issues, `docs/tests/791-session-state-reducer.md` (PRs 1 and 2, and the QA smoke on PR 1) and `docs/tests/full-smoke-test.md`.

## Phase A: live baseline (QA), before any code change

The coder hands QA the untouched branch first and edits nothing until QA reports; it may read code meanwhile. QA reproduces each bug live, with the issue's steps, and keeps the records the fix needs:
- **#784 pi:** Esc during a bounded `sleep 30` tool call. Keep the pi extension's `message_end` (stopReason, errorMessage) and `agent_settled` reports from the status feed, so a user abort can be told from a provider error. Also run one genuine failure if QA can cause one cheaply, and record it.
- **#785 Claude Code:** start a chat in a `/tmp/...` alias of a scratch directory, recall a marker, stop and resume. Show where the transcript lives (literal versus canonical project path).
- **#786 Copilot:** marker, `/clear`, a new marker, stop and resume. Keep the root `SessionStart` reports with both session ids.
- **#781 Codex:** a test role whose env sets `CODEX_HOME` to a scratch directory, showing from the dev CLI that the chat stays unkeyed. Do not copy or create credentials for that home. If Codex cannot run there without a sign-in, record Blocked; the coder then works from the issue and unit tests.
QA posts the matrix and evidence paths to the coder. Then the coder fixes from that evidence, not from reasoning.

## Phase B: the fixes (coder)

One commit per bug, each flipping its known-wrong golden and only that one:
- **#784** (`pi/escape-reply`, `pi/ctrl-c`): map a user abort to Interrupted from what QA's records show; a genuine provider or tool error stays Failed; a recovery turn clears it.
- **#785** (`claude-code/alias-resume`): the resume probe finds the transcript under the canonical project path when the cwd is an alias, keeps the literal path working, and still starts fresh when no transcript exists.
- **#786** (`copilot/clear-resume`): a new root Copilot `SessionStart` rekeys the active row through the existing guarded rekey path. Subagent sessions, stopped rows and older spawn generations never overwrite it. Resume uses the new key.
- **#781** (`codex/custom-home`): resolve the rollout capture root the way `codex_fork_sessions_root` does (spawn env `CODEX_HOME`, then Runner's, then `~/.codex`, relative against the session cwd), inside `runtimes/codex/`. This changes #777's spawn-effects golden rows for Codex: regenerate those deliberately in the same commit and say which rows. Leave TRAE's root alone unless TRAE documents an equivalent override.

Remove each flipped scenario's known-wrong marker. Every other golden stays byte-identical to `e264ed5d`. New scenarios use the compact text format, and `expectations/` stays under 3,000 lines. The two key write paths (`rekey_agent_session_key`, `capture_agent_session_key`) and their guards stay; merging them is not in this PR. Shell separation is #795.

## Phase C: full regression (QA), on the reviewed branch

After a clean review, QA runs the full regression on Claude Code, Codex, Copilot, pi and Antigravity, following `docs/tests/full-smoke-test.md` (direct-chat matrix and mission startup) plus, on each runtime where the CLI supports it:
1. a turn with a tool; 2. an approval (approve, and once deny); 3. Esc and Ctrl+C mid-reply, then a recovery turn; 4. `/clear` or `/new`, a turn, close and resume to the post-clear conversation; 5. a crew message to an idle slot in a test mission; 6. a typed draft holding a delivery, then clearing it.
Then each bug's phase A reproduction again, which must now pass. Any change from 0.12.7 behavior other than the four fixes is a must-fix. Native Windows is Skipped (Jason).

## Live-test authorization

QA may build this worktree and run its development app with `make run` and the wrapper procedure in full-smoke-test.md, using development data only, with the existing sign-ins. It may create test crews and test roles named with a `qa791p3-` prefix, and bounded test chats and missions using inexpensive models at low effort with throwaway prompts in canonical scratch directories under `/private/tmp/qa791p3/` (the #785 rows use the `/tmp` alias on purpose). A test role may set its own runtime arguments so an approval prompts (for example Codex `--ask-for-approval untrusted`, Claude Code `--permission-mode default`). Do not change global agent configuration, permission settings, authentication or the installed Runner app, do not open Runner's SQLite database, and do not touch pre-existing chats, missions, crews or roles. If native control loses the window, ask Jason through Runner (his screen may have locked). Afterwards, archive QA's test chats and missions, delete the `qa791p3-` crews and roles, quit only the wrapper QA launched, and keep the evidence directory. The coder and reviewer do not run the app. No other agents, crews or subagents.

## Constraints

Never run `target/debug/runner` (the GUI app on this volume). Tests never touch the real `$HOME` or app data. Gate `cfg(unix)`-only test imports with `#[cfg(unix)]`. Tests that build a core or DB pool must not leak threads. Match the surrounding code; keep comments rare.

## Review, checks and authorization

The reviewer waits for an explicit Runner handoff and reviews each fix against its issue and QA's evidence: the fix is the smallest that the evidence supports; exactly one golden flips per bug; no other golden or spawn row changes unexplained; isolation in #786 holds. Iterate until `NO REMAINING MUST-FIX ISSUES`, then QA's phase C; route QA findings back through fix and re-review.

Run and record with exit codes: `cargo test --locked -p runner-backend --profile ci`, `cargo test --locked -p runner-terminal --profile ci`, `cargo test --locked --workspace --no-fail-fast --profile ci`, workspace Clippy with `-D warnings`, the macOS updater Clippy, `cargo fmt --all --check`, `git diff --check`, and `git diff e264ed5d --stat -- crates/runner-backend/src/session/fixtures` showing only the four flipped goldens and any new scenarios.

After clean review and QA, Jason authorizes:
- Four commits on top of current `origin/main`, one per bug, because each fix can be reverted alone (say so in the PR body). Fold this brief and the test record into the first.
- Push `fix/791-status-key-bugs` and open a PR against main. Use `Closes #NNN` only for a bug QA verified fixed live; otherwise `Refs #NNN` with what is left. Add `Closes #791` only if all four are closed.
- Rebase if main moves; never merge main in. Amend fixes into the commit they belong to and push with `--force-with-lease`. Drive CI green on macOS and Windows.
- Do not merge, delete the branch or worktree, or cut a nightly or release.

QA writes a dated "PR 3" section in `docs/tests/791-session-state-reducer.md` with both matrices, CLI versions, candidate SHAs and evidence paths, and nothing private. Final Runner handoff: the PR URL, each bug's cause with evidence and its fix, the flipped goldens, both QA matrices, tests and exit codes, the CI result, the reviewer's verdict, and what Jason should still check (native Windows). Then stand by.
