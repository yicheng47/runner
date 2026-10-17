# 826 — targeted crew messaging

Implement [#826](https://github.com/yicheng47/runner/issues/826): a message wakes only its recipients, and `runner msg read` shows only what is new. The one-line `[inbox]` notification stays as it is. Jason asked for this `codex trio` mission on 2026-10-17, ending in an open PR; starting it authorizes the implementation, review, live checks and open-PR workflow below. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-826-targeted-messaging`, on `feat/826-targeted-messaging`, based on `main` `045ead84`. No other branch, worktree or target directory. If you need a newer main, rebase onto `origin/main`; never merge it in.

## What matters most

A member is woken only by messages addressed to it or to everyone, and a default read prints only what it has not seen, plus a short digest of others' traffic. No message is lost, no member misses a message addressed to it, and old logs keep parsing and rendering. #907's dispatcher, notification text, readiness, confirmation and redelivery do not change. Everything else follows the plan; edge cases get the plan's handling and no more.

## Read first

- `AGENTS.md`: Worktrees, Test Scope, Crew Missions.
- **`docs/impls/826-targeted-messaging.md`: the contract.** Decisions 1–5, the read-contract table and cursor rules, where each change lands (code line numbers as of `e280932f`), commits, tests, live checks and risks. It wins over this brief and older spec wording on any detail.
- `docs/features/826-targeted-messaging.md`: the spec and its settled questions. Jason settled them on 2026-10-17, including not typing message text into agents; do not reopen them.
- `docs/arch/arch.md` §4.2, §4.3, §8.1, §8.5 and §9.3. Use §8.5's terms in code, comments, tests and commit messages.
- Issue #873: add no rule that depends on a lead.

## Deliverables

One PR against `main` in the plan's three commits, each building and passing its tests, with the spec, the plan and this brief folded into the first:

1. Unread-only `runner msg read`, crew history with `--all`, non-acking filtered reads, the digest, and the typed cursor query with equivalent log fallback and subscriptions from `role_joined`.
2. `Recipients`, the shared resolver, repeatable `--to`, backlogs and notifications only for recipients, the multi-target composer and the feed's rendering.
3. The three Coordination texts, `runner help agents`, arch, the guide pages, `make guide-cli`, QA's test record `docs/tests/826-targeted-messaging.md` and the regression rows whose expected results change.

Easy to get wrong:

- A single recipient still serializes as a string; old single-string lines must parse unchanged.
- The CLI, `mission_post` and the composer resolve addresses through the same function. `--to human` and leading `@human` without explicit recipients are errors. Explicit recipients take precedence over leading mentions.
- The bus, `message_nudge` and `msg read` decide membership through the same `Event::addresses`, so a backlog, an unread count and a read always agree.
- A default read acks fully printed inbox messages. `--since` and `--from` never ack, including with `--all`. `--all` prints all crew messages since joining but only acknowledges the caller's own inbox; other members' traffic and digest entries never advance the ack.
- Cursor offsets point after the acknowledged message, never after its later `inbox_read` signal. Fold newer acks before selecting output; stale cursors, daemon restart and unavailable sockets must preserve the read contract. Messages arriving between the captured read batch and its ack stay unread.
- Multi-target `human_said` conversion and reopen repair preserve the resolved recipients; an array must never become a broadcast.
- Leading mentions address; mid-text mentions do not.
- Gate imports and helpers used only by `#[cfg(unix)]` tests with `#[cfg(unix)]`.

## QA

QA runs the plan's live checks in the development app under the standing authorization in `AGENTS.md`, Test Scope: an inexpensive model, low effort, tiny prompts and a fresh `/tmp` folder per fixture, on Codex, Grok or pi only. It never types into, archives or operates development sessions it did not create, archives its own at the end, and never touches the installed Runner. Agent configuration changes follow [Agent configuration isolation](../../tests/full-smoke-test.md#agent-configuration-isolation).

- On the reviewed candidate, run ADDR-1, COMPOSER-1 and the selected regression rows, recording candidate identity and evidence as the [regression README](../../tests/regression/README.md#run-records-and-evidence) describes.
- Write `docs/tests/826-targeted-messaging.md` with the selected checks, results, failures and gaps. A failure goes to coder as a must-fix with its evidence; QA does not edit code.

## Validation

Log each to a file and read the exit code from the command itself, never through `tail` or `grep`: `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`; `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, and again with `--features updater`; `cargo fmt --all --check`; `make guide-cli-check`; `git diff --check`.

## Boundaries

The coder and reviewer never run the development app; only QA does, as above. No slot operates Jason's installed Runner, its chats or missions; this mission runs there. No additional agents, crews or subagents beyond QA's bounded test missions. The crews' conventions in Jason's Runner are left alone; the plan's After merge step is Jason's.

## Review and authorization

The reviewer waits for an explicit Runner handoff, then reviews the whole diff against the plan and this brief, must-fix first with file:line pointers. Must-fix: anything that can lose a message, wake a member that was not addressed, hide an addressed message from its recipient's read, ack a message that was not printed, break parsing of an old log, change #907's dispatcher behavior, or sleep in a test. Check especially the resolver's agreement across paths, `Recipients`' serde forms against old logs, membership through `Event::addresses`, and the read's ack. Iterate until `NO REMAINING MUST-FIX ISSUES`, then hand the candidate to QA.

After clean review and QA, Jason authorizes: the three commits on current `origin/main` with imperative subjects and no co-author trailers; `git push --force-with-lease` of `feat/826-targeted-messaging`; `gh pr create --base main` with `Closes #826`, the summary, checks with exit codes, QA's results and gaps, review follow-ups and no agent session links; and `gh pr checks <n> --watch` until macOS and Windows are green, folding each fix into the commit it corrects after the reviewer checks it. Do not merge, delete the branch or worktree, or cut a nightly or release.

Final handoff to everyone: PR URL and CI, changes, checks with exit codes, QA's results, the review verdict, follow-ups and what is left for Jason. Then all slots stand by.
