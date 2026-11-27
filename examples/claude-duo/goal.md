# Claude Duo — Mission Goal

A cross-provider two-agent delivery loop pairing Claude Code for implementation with Codex for independent code review — ideal for backend changes, CLI work, refactors, and bug fixes covered by automated tests.

## Roles

Import [`claude-duo.crew.toml`](claude-duo.crew.toml) for both roles and the team conventions:

- **@coder** (Claude Code `claude-opus-5-5`, `xhigh`) — implements on a feature branch, writes and runs tests, hands off frozen local snapshots, and fixes review findings.
- **@reviewer** (Codex `gpt-6.2-astra`, `xhigh`) — audits each snapshot in full, reports must-fix defects with `file:line` pointers, and confirms when no must-fix issues remain.

## Goal (replace this)

Drop in a self-contained implementation or refactor task plus explicit Git/PR boundaries:

- *"Add `--format json` support to the `export` subcommand with schema validation tests. Stop at a clean review with changes uncommitted."*
- *"Refactor the retry loop in `src/http/client.rs` to use exponential backoff with jitter and deterministic unit tests. Authorized to commit, push, and open a PR after clean review; do not merge."*
