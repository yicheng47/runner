# 1188 — a shared lock so concurrent missions' qa take turns

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1188-qa-lock`, on branch `feat/1188-qa-lock`, cut from `origin/main` at `cb3fa7a6`. This brief is the branch's tip. Don't create another branch or checkout, and don't touch the root checkout or another worktree.

## Problem and outcome

[#1188](https://github.com/yicheng47/runner/issues/1188): several trio missions run at once. Each mission's `qa` launches the development app and drives the one screen through computer use. Today qa decides whether the app is free by looking for other Runner processes before launching, and that races. On 2026-11-05 #930's qa launched 44 seconds after #634 m6's qa was cleared to go. Twice m6's qa launched anyway after its own check failed.

Two qa runs can't share the screen, even with separate daemons, so QA runs must take turns. This mission adds a lock that decides whose turn it is, and makes taking it part of every qa's procedure.

## Decisions already made

- **The lock covers the development app and the screen.** It isn't per daemon. Parallel QA and an isolated-data override are out of scope.
- **Only qa agents take it** (operator, 2026-11-05, settling the issue's open question). A person using the development app doesn't. They can see who holds it with `status`.
- **The OS releases it when the holder dies.** Use an advisory lock on an open file (`flock`/`fcntl` on macOS and Linux, `LockFileEx` or `msvcrt.locking` on Windows), not just a file's existence. A crashed qa can't then block the queue. A plain lock file left behind with no live holder must read as free.
- **It lasts across tool calls.** A qa agent works in many separate shell commands, so `acquire` can't simply hold a lock in a process that exits. Design it so the lock survives between commands and is freed when the qa session ends or dies. One way: a small detached holder process that keeps the lock, records the owner, and exits on `release` or when an owner PID passed to it exits. Choose and justify the design.

## Deliverables

1. **The helper:** `script/qa-lock` (Python 3 like the other scripts, or another single dependency-free script), working on macOS, Linux and Windows.
   - `acquire --mission <id> --handle <handle> [--owner-pid <pid>] [--timeout <s>]`: takes the lock or waits up to the timeout. On timeout it exits non-zero and prints the current holder.
   - `release`: frees the lock if this owner holds it; it refuses to release someone else's.
   - `status`: prints free, or the holder's mission, handle, PID and start time.
   - The lock lives in the development data directory: `~/Library/Application Support/com.wycstudios.runner-dev/qa.lock` on macOS, the matching directory on Linux and Windows, as `runner-core`'s `app_paths.rs` derives it.
2. **Conventions:**
   - **AGENTS.md, Test Scope:** qa takes the lock with `script/qa-lock acquire` before building a wrapper or launching the development app. It holds the lock through pauses until cleanup, and releases it after its app and daemon have exited.
   - **On timeout:** a qa that can't get the lock waits and reports the holder through Runner messages. It never launches anyway and never stops another mission's app.
   - **The examples:** reflect the same rule in `examples/codex-trio/qa.md` and `team-conventions.md`.
   - **Live crews:** the operator updates the live crews' conventions after merge, so the crew doesn't edit them.
3. **Tests:** cover acquire, contention with a timeout, release by its owner only, refusing someone else's release, a dead holder freeing the lock, and status. Run them in CI on every platform if there's a natural place, or document how to run them.

## Validation

- Run the script's tests and log their exit codes.
- Run a manual two-shell check on macOS: shell A acquires, shell B times out and prints A as the holder, A releases, then B acquires. Repeat with A's owner process killed instead of releasing.
- No app or daemon code changes and no live checks in the app. If anything outside `script/`, `docs/`, `examples/` and AGENTS.md changes, say why in the handoff.

## Authorization

Approved crew: the claude duo. Once review is clean the crew may:
- commit and push this branch;
- open a PR against `main` titled `feat(validation): a shared lock so qa takes turns on the development app (#1188)`, labelled `risk-low` and `no-behavior-change`;
- drive CI green on every job.

The crew doesn't merge.
