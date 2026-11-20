# Codex Trio — Mission Goal

A three-role coder → reviewer → QA pipeline for changes whose proof needs a live run, not just a green diff. The coder ships the change; the reviewer audits the working-tree diff; once review is clean, the coder freezes the source and QA builds that exact revision, exercises it the way a user would, and reports a verdict with evidence. Use the codex duo (`pair-coding/`) when tests alone prove the change.

## Roles

- **@coder** (lead) — implements on a task-specific feature branch, runs the checks, fixes review and QA findings, and owns every commit. See `coder.md`.
- **@reviewer** — reads the working-tree diff after the coder's handoff, reports must-fix issues with file:line pointers, never edits code or runs the tests. See `reviewer.md`.
- **@qa** — verifies the frozen candidate it was handed, runs the required checks against the live app, and writes the result record. Never edits source and never commits. See `qa.md`.

## Slot settings

All three slots run `codex` with `--ask-for-approval on-request --sandbox workspace-write`.

| Slot | Model | Effort | Speed |
| --- | --- | --- | --- |
| @coder | `gpt-6.2-sol` | `xhigh` | fast |
| @reviewer | `gpt-6.2-astra` | `xhigh` | standard |
| @qa | `gpt-6.2-sol` | `xhigh` | fast |

## Team conventions

Use the contents of [`team-conventions.md`](team-conventions.md) as the Crew's team-conventions prompt.

## Goal (replace this)

Drop in one concrete task whose result someone has to see working:

- *"Fix issue #322 — the sidebar section header wraps at narrow widths; truncate with an ellipsis instead. QA: screenshot the sidebar at 280px and 400px."*
- *"Add drag-to-reorder for tabs. QA: reorder three tabs, restart the app, and confirm the order persisted."*
- *"Fix pasting multi-line text into a running session. QA: paste into a Codex and a Claude Code session and confirm each receives one submission."*

Say whether PR mode is authorized. Without it, the crew stops once QA reports and leaves the changes uncommitted.

## How a turn looks

1. Human types the task into the mission input.
2. `@coder` checks repo status, branches, implements, runs the checks, then messages `@reviewer` with the branch name, a summary, changed files, and checks run.
3. `@reviewer` reads the working-tree diff and the blast radius, then messages back: must-fix findings first with file:line pointers, or a clean-review statement. The loop repeats until review is clean.
4. `@coder` freezes the source and hands it to `@qa`: branch, worktree path, `git rev-parse HEAD`, the SHA-256 of `git diff HEAD`, what changed, and which checks apply.
5. `@qa` confirms the candidate matches, builds and launches it, runs the checks, and reports Passed, Failed, Blocked or Skipped for each, with evidence.
6. On failures, `@coder` fixes them, routes the change back through `@reviewer`, and hands the new revision to `@qa` naming the checks it affects.
7. With PR mode authorized, `@coder` commits (including the QA record when asked), pushes, opens the PR, and drives CI green. Nothing is merged unless the human asks.
