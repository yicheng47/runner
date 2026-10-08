# 723 — Cursor runtime: review fixes on PR #827

Address Jason's review of [PR #827](https://github.com/yicheng47/runner/pull/827) (Cursor CLI runtime, part of [#723](https://github.com/yicheng47/runner/issues/723)). The PR comes from the fork `ruizer/runner`; Jason took it over on 2026-10-09 and requested this mission on 2026-10-10. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-723-cursor-agent` on `feat/723-cursor-agent`, which tracks `ruizer/feat/723-cursor-agent`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory.

## Starting state

The branch is the PR's single commit (author ruizer) rebased cleanly onto `origin/main` at `d8da3663` (0.14.1), plus this brief; nothing is pushed yet. Cursor CLI `2026.10.01-e373342` is installed at `~/.local/bin/cursor-agent` (`agent` symlinks to it) and signed in to Jason's account.

## Read first

- `AGENTS.md`, including Test Scope, Worktrees and Crew Missions.
- Jason's review, the spec for this mission: `gh pr view 827 --comments` and the inline comments (`gh api repos/yicheng47/runner/pulls/827/comments`).
- `docs/features/723-cursor-agent-runtime.md` and `docs/tests/723-cursor-agent-runtime.md`.
- `crates/runner-daemon/src/runtimes/cursor_agent/` (`conversation.rs`: `config_dir_with`, `chats_dir`, `Watcher::observe`; `mod.rs`: `conversation_exists`; `command.rs`: `ValidatedAlias`), `crates/runner-daemon/src/runtime_status.rs` (`validated_cursor_agent_alias`, `detected_runtime_executable`) and `shell_path.rs` (`DiscoveryState`).

## Deliverables

1. **Windows CI.** Fix `store_path_uses_exact_cwd_bucket_and_role_config_override`: either `#[cfg(unix)]` (the bucket hash is the macOS/Linux contract), or build paths from a tempdir and compute the expected bucket the same way.
2. **Windows resume.** `canonicalize` gives `\\?\C:\…` on Windows, so the bucket can never match. Make `CursorAgent::conversation_exists` return `None` on Windows so `--resume` reports its own error, matching how the Claude Code probe stays permissive when it cannot reproduce the path.
3. **One environment for the config root.** The observer (`Watcher::new`, login-shell vars plus role env) and the resume probe (`conversation_exists`, role env plus the daemon's `std::env`) must resolve the root from the same env: login-shell vars plus role env. Route env reads through the `golden::config_var_os` seam so tests don't see the developer's real env. Before settling the code, probe the real CLI on macOS: does Cursor honour `XDG_CONFIG_HOME`, and `CURSOR_CONFIG_DIR`? Run `cursor-agent` in a canonical scratch cwd with the variable set, in that child's env only, to a scratch directory; send one trivial prompt and see where `chats/<bucket>/<id>/store.db` lands. Resolve the root exactly as Cursor does; if it ignores `XDG_CONFIG_HOME` on macOS, drop that branch on macOS. If moving the config root asks for a sign-in, stop the probe and ask Jason through Runner; never sign in, copy credentials or edit `~/.cursor`. Record the probe in the test record.
4. **Key clearing.** A false clear turns automatic resume into "cannot resume its prior conversation", so it costs more than a stale key. More than one open store means no change. Clear on an empty scan only after it persists across a couple of polls, so a graceful quit that closes the SQLite handle before PTY EOF keeps the key. Decide whether a post-`/clear` empty state clears the key or keeps the old one until the new store appears, and say why in the handoff and the spec. Keep the existing EOF/incarnation guards and the forwarder-survival behavior.
5. **Drop the `agent` fallback.** Detect only `cursor-agent`. Users with only `agent` set the executable override in Settings → Agents, which already wins. Remove `ValidatedAlias`, the `DiscoveryState` field and call sites, the alias tests (including the load-flaky `cursor_alias_rejects_replacement_and_symlink_retargeting`) and alias wording in docs and goldens. Keep a test that the override wins.
6. **README.** Replace the Cursor prose paragraph with a Cursor column in the Supported agents table: resume ✓; conversation-change rekey "macOS/Linux only"; Windows with a footnote that it is unverified; fork —; hooks status — (terminal baseline); needs-you dialogs —; model list ✓; update from Settings —; usage pill —; mission access Bypass; skills pane catalog; Runner skill ✓; fixtures —. Add a Cursor clause to the paragraph below the table and keep details in the spec. Same change in `README.zh-CN.md`.
7. **Docs.** Update the spec for the decisions above. Add a dated section to `docs/tests/723-cursor-agent-runtime.md` for this round: automated results and QA's record.

No Grok, no new Cursor features, no unrelated refactors, no `.pen` edits. Other runtimes' argv, discovery and goldens stay unchanged.

## QA (live)

`qa` tests the frozen candidate in the development app per `docs/tests/full-smoke-test.md` (candidate identity, wrapper bundle, development CLI by absolute path, `RUNNER_*` unset, canonical scratch cwds, ledger, cleanup). Jason authorizes bounded Cursor chats and one small mission on his signed-in account with an inexpensive model. Coder and reviewer never run the dev app. Preserve existing sessions and Cursor's existing chats and config.

Checks, each tied to a fix:

- **Detection (5):** Settings → Agents detects Cursor from `cursor-agent` with its version; an override set to the binary's absolute path starts a chat; restore the setting.
- **Resume (3, 4):** a chat given a marker recalls it after stop and resume.
- **Graceful quit (4):** exiting Cursor from inside the chat keeps the key; a later resume recalls the marker.
- **`/clear` (4):** behavior matches the decided policy; a new marker gets a new key that resumes.
- **Config root (3):** only if Cursor honours a config variable: a role with it in its env resumes.
- **Mission:** a one-slot Cursor mission starts with Bypass, runs one harmless command, survives stop and resume.

Windows checks are Blocked on this Mac, not passed.

## Boundaries

Do not start extra agents, crews or subagents. Do not run `runner` or `runner-dev` commands that stop or kill sessions or the daemon beyond `qa`'s own ledger. Gate imports used only by `cfg(unix)` tests with `#[cfg(unix)]`; Windows CI breaks on them.

## Review, verification and authorization

The reviewer checks the diff against Jason's six comments, must-fix first with file:line pointers, and confirms other runtimes are untouched. Iterate until `NO REMAINING MUST-FIX ISSUES`, then hand the frozen candidate to `qa`.

Run `cargo test --locked -p runner-daemon --profile ci`, the `runner-core`, `runner-app` and `runner-cli` tests that the goldens touch, workspace Clippy with warnings denied plus `--features updater`, `cargo fmt --all --check` and `git diff --check`. Record exact commands and exit codes.

After clean review and QA's verdict, Jason authorizes squashing everything on the branch, this brief included, into one commit on top of current `origin/main` that keeps ruizer as author and the subject `feat(runtime): add Cursor CLI support`. If main has moved, rebase; never merge main into the branch. Push to the contributor's fork, updating PR #827 rather than opening another: `git push --force-with-lease ruizer HEAD:feat/723-cursor-agent`. Rewrite the PR body to the current state (takeover, the six fixes, test and QA evidence, Windows and Linux live gaps, `Part of #723`; never `Fixes #723`). Fork PR runs wait for approval: approve only the CI run whose head SHA is the commit you pushed, with `gh api -X POST repos/yicheng47/runner/actions/runs/<id>/approve`. Drive CI green on macOS and Windows; amend fixes into the same commit and push with lease again, routing non-trivial fixes back through review and the affected QA checks.

Do not merge, delete the branch or worktree, remove the `ruizer` remote, or cut a nightly or release. Final Runner handoff: PR URL, changed files, checks and exit codes, review verdict, QA verdict, CI result, the XDG/config-root finding, and what Jason should still smoke-test. Then stand by.
