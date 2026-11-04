# #1151 — Mission members post their result to the feed

Jason requested this mission with the claude duo on 2026-11-04. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1151-mission-result-report` on `fix/1151-mission-result-report`, based on `origin/main` at `268bb29e8a20f890b3755bf6c9693c80e03e1c14`. The root checkout stays on main.

## Problem

Issue [#1151](https://github.com/yicheng47/runner/issues/1151): a Codex member added with `runner mission add-role` finished its task and answered only in its own terminal. It never ran `runner msg post`, so the mission feed had no result until the person asked "why didn't you reply". Runner did not lose a message; the member was never told that the feed, not its terminal, is where a result goes. An `idle` status or a hook's `outcome: completed` marks the end of a turn, not the result.

This matters more now that missions have operators reading the feed: the chat that started a mission, reviewer-only and qa-only missions with no crew, and Runner's report back to a linked chat when a mission settles (#748). A result left in a terminal looks like a stuck member.

## Intended guidance

Every mission member, with or without a crew's conventions and whether it joined at start or through `add-role`, is told: when it finishes a task or a handoff someone is waiting on, it posts the outcome with `runner msg post` before ending its turn — what it did, the key validation, and anything still needing the person or another member — addressed to whoever asked, or without `--to` when the person asked. Its terminal reply is not seen by the operator. This is one post per finished task, not receipt or standby messages, which the existing guidance already forbids; keep that rule intact.

## Scope

Read AGENTS.md, then update the authoritative sources together:

- `crates/runner-daemon/src/router/prompt.rs`: the member first-turn composition every runtime receives (`compose_member_first_turn`), which is the path an `add-role` member without crew conventions gets.
- `crates/runner-core/src/protocol/agent_skill.rs`: both generated skill variants (runner and runner-dev).
- `crates/runner-cli/src/help.rs`: the agent guide, kept consistent with the skill.
- `docs/guide/guides/drive-runner-from-agents.md` where it describes what members do, and the generated `docs/guide/reference/cli.md` via `make guide-cli` if help text changes.
- `examples/pair-coding/team-conventions.md` only if it contradicts the new rule.
- Affected prompt, skill and help assertions and golden snapshots, updated intentionally through their existing workflow.

Keep this an instruction-text change. Non-goals: capturing terminal output into the feed, any router or delivery change, UI changes, rewriting installed skill files or global agent configuration, and the external-session notification gap the issue separates out.

## Validation

Before testing, list the changed behavior and why each check covers it. Run the prompt composition, skill rendering, CLI help and golden tests for the touched crates with locked dependencies and this worktree's own target directory, plus `make guide-cli`'s check, `cargo fmt --all --check`, `git diff --check` and workspace Clippy with warnings denied. Confirm the rule appears for a crewless `add-role` member, a crew member, and both skill variants, and that the no-receipt and no-polling rules still read correctly.

Live checks: the reviewer decides from the diff under Test Scope. The issue's acceptance case is a fresh Codex member given a small task with no reminder to report, whose result then appears in the feed exactly once. If the reviewer requires it, name it in the clean review; the operator adds qa afterwards.

## Authorization and handoff

Jason approved the claude duo (coder and reviewer) for this mission. The crew may commit, push this branch and open one PR against main, with labels picked from the diff under AGENTS.md. Stop at an open PR with green CI, reviewed, and report the PR URL, final SHA, changes and validation through Runner.
