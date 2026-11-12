# #1248 item 8 — Extract large inline test modules

Jason approved the codex duo on 2026-11-12 for the inline-test extraction slice of [#1248](https://github.com/yicheng47/runner/issues/1248). The reviewer uses Codex `gpt-6-astra`, `xhigh`, standard speed. Work in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/chore-1248-extract-inline-tests`, branch `chore/1248-extract-inline-tests`, based on `origin/main` at `b841b51a63fae25426fd9a36b4da23b927213efa`. Use `/tmp/1248-tests-` for scratch artifacts.

## Scope

Move large inline test modules out of production files into conventional external test-module files. Preserve production behavior, test behavior, test identities and coverage. This is the test extraction part of item 8; splitting production implementations and items 1–5 are separate work.

Start by confirming the inventory on this branch. An initial scan found these 22 production files containing at least 1,000 lines in inline modules named `tests` or ending in `_tests`, counting multiple such modules in a file together:

- `runner-app`: `surfaces/panes/mod.rs`, `surfaces/start_chat/launcher.rs`, `surfaces/settings/page/mod.rs`, `surfaces/settings/remote.rs`, `surfaces/app_shell/mod.rs`, `surfaces/start_chat/launcher/mission.rs`, `keymap.rs`, `surfaces/settings/agents/mod.rs`, `surfaces/update_ui.rs`.
- `runner-daemon`: `session/pty_runtime.rs`, `ops/mission.rs`, `runtimes/claude_code/claude_status.rs`, `ops/session.rs`, `ops/role.rs`, `ops/node.rs`, `ops/mission/report.rs`, `runtimes/codex/codex_status.rs`, `runtimes/copilot/copilot_status.rs`, `ops/skills.rs`.
- `runner-cli`: `command.rs`.
- `runner-terminal`: `input_state.rs`, `terminal.rs`.

Paths above are relative to each crate's `src/`. The original issue's counts are an older snapshot. Correct the inventory if needed and account for each eligible file in the handoff; do not expand into extracting every small test module across the workspace. When a selected file has several inline test modules, extract each while keeping its existing module name and cfg attributes. Follow nearby external-module conventions, using `<module>/tests.rs` or the existing directory's `tests.rs`, and named files for separate modules. Do not merge distinct modules just to put them in one tests file.

Move code mechanically. Necessary module declarations and relative fixture/include path adjustments are in scope. Preserve test functions, assertions, cfg gates, ignores and helper behavior. Avoid production refactors, broad visibility changes, new abstractions, golden updates, test rewrites or unrelated fixes. Test-only helpers already outside the inline module may remain where they are if moving them would require unrelated changes.

Keep Rust module paths stable: Windows self-exec probes in `session/pty_runtime.rs` invoke fully qualified test names with `--exact`; the CLI help golden uses `include_str!`, and PTY tests include fixture JSON. Audit `include_str!`, `include_bytes!`, `file!`, `module_path!`, relative file lookups and any hard-coded test name affected by relocation. Preserve fixture contents and paths where practical; only adjust the source-relative reference when necessary.

## Concurrent work

At setup there are no open GitHub PRs and no active missions in this local Runner instance. Remote branch `fix/1272-remote-sidebar-collapse` has an authorized mission brief for remote sidebar persistence, whose named implementation/tests are outside this inventory. Leave its sidebar, remote-client and app-settings work alone. Recheck open PRs before editing; report and defer a genuinely overlapping file rather than mixing work or touching another worktree.

## Validation and review

Changed production behavior: none. No live checks are required for this scope, because only test source organization changes. Do not launch the app or real agent sessions as validation.

- Establish the discovered test names/counts before extraction and compare after, using the same platform and features. Test identities, ignored status and coverage must remain intact; explicitly audit platform-gated tests that cannot be listed on this host.
- Review the move-aware diff and source equivalence: aside from indentation, module wiring and necessary source-relative paths, test bodies must be unchanged and production logic untouched. The reviewer checks this on the handed-off commit.
- Run `make verify` in this worktree with its own `target/`; extraction spans app, daemon, CLI and terminal crates, so workspace checks cover module resolution, private-item access, fixtures and all moved tests. Log long output and preserve exit codes. Complete the repository's required feature/platform CI gates on macOS, Windows and Linux, especially Windows subprocess probes.
- Do not weaken or disable tests to obtain a pass. Report any pre-existing or external failure with evidence and keep that distinction in the PR. No new behavioral tests are needed for a mechanical extraction.

Reference this as "Part of #1248" in the PR; leave the umbrella issue open. Label the PR `no-behavior-change` and `risk-low`, retaining any automatically applied `core-review` label. Record the inventory, test-identity comparison and checks run in the PR description. The clean review names the final candidate and explains why no live checks are needed.

## Authorization

The approved codex duo may commit, push this branch and open one pull request against `main`. The endpoint is a reviewed PR with passing CI. No merge is authorized. Fold this setup brief into the implementation history as the repository guide requires.

## Rebase and completion update — 2026-11-12

Jason authorized merging PR #1276 and standard post-merge cleanup after main advanced through #1274 and #1275. The crew rebases this existing branch/worktree onto `c2fbafd9b41979d5ff18e9f752672584baa4a955`, preserves every upstream production/test change, and treats that main commit as the new source-equivalence and test-identity baseline. The original baseline and counts above are historical; current evidence belongs in `docs/tests/1248-extract-inline-tests.md`.

The rebased candidate needs fresh independent review, the `reviewed` label removed before its force-with-lease push and restored only after review, and green macOS, Windows, Linux x86_64, and Linux arm64 CI. No live checks are needed while the source-only boundary holds. Once ready, the crew reports to the linked operator and settles; the operator owns the actual merge and mission/worktree cleanup. Any further same-file changes on main require the normal catch-up workflow.
