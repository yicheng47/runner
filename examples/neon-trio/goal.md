# Neon Trio — Mission Goal

The author's current best daily delivery team: an Antigravity coder, a Codex reviewer, and a Codex QA engineer running a pipelined branch → review → live-QA loop.

## Roles

Import [`neon-trio.crew.toml`](neon-trio.crew.toml) for all three roles and the team conventions:

- **@coder** (Antigravity `gemini-4-neon`, `xhigh`) — implements on a task branch, runs automated tests, hands off frozen local snapshots, and fixes review/QA findings.
- **@reviewer** (Codex `gpt-6.2-astra`, `xhigh`) — reviews each snapshot in full before QA or push, reports must-fix issues with `file:line` pointers, and selects required live checks.
- **@qa** (Codex `gpt-6.2-sol`, `high`) — builds and validates the reviewed candidate in the live development app and reports concrete findings with evidence.

## Goal (replace this)

Give the crew one concrete feature or bug fix, its acceptance criteria, the required live checks, and explicit Git/PR authorization:

- *"Implement issue #42 — add keyboard navigation to the command list. Keep changes scoped, cover with unit tests, verify live focus/scroll behavior in the dev build, and stop before committing."*
- *"Fix the settings persistence bug in #108. Authorized to commit on a feature branch, push, open a PR, and drive CI green after review and live QA are clean. Do not merge."*
