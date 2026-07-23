# Peer Coding — Mission Goal

A two-runner coder/reviewer loop for a single implementation task. The coder ships the change; the reviewer audits it; the coder fixes findings until the review is clean. No architect, no dispatch overhead — just the tightest loop that still has a second pair of eyes.

## Roles

- **@coder** (lead) — implements on a task-specific feature branch, runs the checks, fixes review findings. See `coder.md`.
- **@reviewer** — reads the working-tree diff after the coder's handoff, reports must-fix issues with file:line pointers, never edits code. See `reviewer.md`.

## Team conventions

Paste these into the Crew's team-conventions field:

The Runner CLI is the sole coordination and review record — every handoff (implementation ready, review findings, fixes applied, clean review) is a Runner message, never a PR comment or other remote channel. The coder creates a task-specific feature branch before editing and never implements on `main` or another long-lived branch. Review happens on the local working-tree diff: the reviewer never expects, requests, or waits for a commit, push, or PR. All changes stay in the working tree unless the mission goal explicitly authorizes a specific git side effect. Definition of done: implemented, relevant checks passed, reviewer reports no remaining must-fix issues.

## Goal (replace this)

Drop in one concrete task. The shape fits anything a single focused PR would fit:

- *"Fix issue #322 — the sidebar section header wraps at narrow widths; truncate with an ellipsis instead."*
- *"Add a `--json` flag to the export command, matching the import command's schema."*
- *"Extract the retry logic in `client.ts` into a helper and cover the backoff edge cases with tests."*

## How a turn looks

1. Human types the task into the mission input.
2. `@coder` checks repo status, branches, implements, runs the checks, then messages `@reviewer` with the branch name, a summary, changed files, and checks run.
3. `@reviewer` reads the working-tree diff and the blast radius, then messages back: must-fix findings first with file:line pointers, or a clean-review statement.
4. `@coder` fixes findings and hands back; the loop repeats until the review is clean.
5. The result stays in the working tree for the human to inspect, commit, or discard.
