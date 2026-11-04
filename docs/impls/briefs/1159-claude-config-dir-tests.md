# #1159 — Claude launch tests ignore the shell's CLAUDE_CONFIG_DIR

Jason requested this mission with the claude duo on 2026-11-04. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1159-claude-config-dir-tests` on `fix/1159-claude-config-dir-tests`, based on `origin/main` at `268bb29e8a20f890b3755bf6c9693c80e03e1c14`. The root checkout stays on main.

## Problem

Issue [#1159](https://github.com/yicheng47/runner/issues/1159): four `runner-daemon` tests in `session::manager::tests::launch_definition` read the test process's own `CLAUDE_CONFIG_DIR`, so they fail whenever `cargo test` runs in a shell that sets it, as Jason's Claude Code sessions do (`CLAUDE_CONFIG_DIR=/Users/jason/.claude-work-3`). CI does not set it and stays green. Agents validating locally see four fake regressions; the #1155 crew had to unset the variable.

- `claude_accounts_follow_active_selection_and_resume_and_fork_from_origin`
- `claude_rekey_after_account_switch_resumes_and_forks_the_new_conversation`
- `claude_single_account_and_explicit_pins_keep_their_launch_inputs`
- `claude_trust_uses_the_effective_launch_account_and_environment`

Reproduce: `CLAUDE_CONFIG_DIR=/tmp/somewhere cargo test --locked -p runner-daemon launch_definition` fails 4 of 15; with `env -u CLAUDE_CONFIG_DIR` all 15 pass. Your own shell likely sets it already, which is the point.

## Read first

- `crates/runner-daemon/src/claude_accounts.rs`: `config_pin` falls back to `std::env::var("CLAUDE_CONFIG_DIR")`.
- `crates/runner-daemon/src/runtimes/claude_code/trust.rs`: reads the inherited variable the same way.
- `crates/runner-daemon/src/runtimes/claude_code/mod.rs`: `npm_dist_tag` already overrides the process value under `#[cfg(test)]` with `crate::runtimes::test_home()`; follow that existing pattern.
- `crates/runner-daemon/src/session/manager/tests/launch_definition.rs`: the four tests.

## Deliverables

The tests are hermetic: their results do not depend on the developer's `CLAUDE_CONFIG_DIR`. Production behaviour of honouring an inherited `CLAUDE_CONFIG_DIR` stays exactly as it is in release and dev builds; the isolation is test-only. Do not mutate the process environment from tests (parallel tests share it). Look for other tests that read the same variable and fix them the same way if they exist. Non-goals: any change to account selection, trust seeding or launch behaviour.

## Validation

Run `cargo test --locked -p runner-daemon` both with your shell's `CLAUDE_CONFIG_DIR` set to a non-default folder and with it unset, and show both pass; plus `cargo fmt --all --check`, `git diff --check` and workspace Clippy with warnings denied. No live checks: this changes test isolation only.

## Authorization and handoff

Jason approved the claude duo (coder and reviewer) for this mission. The crew may commit, push this branch and open one PR against main, labelled `no-behavior-change` and `risk-low`. Stop at an open PR with green CI, reviewed, and report the PR URL, final SHA, changes and validation through Runner.
