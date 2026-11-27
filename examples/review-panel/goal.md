# Review Panel — Mission Goal

A read-only multi-model code review panel. Two independent reviewers on different providers audit the same diff in parallel without seeing each other's notes, and a review lead verifies every finding against the source before delivering one ranked report.

## Roles

Import [`review-panel.crew.toml`](review-panel.crew.toml) for all three roles and the panel conventions:

- **@lead** (Codex `gpt-6.2-sol`, `high`) — briefs if needed, waits for both independent reports, verifies every must-fix and should-fix against the code, resolves disagreements, and outputs one merged report.
- **@reviewer-opus** (Claude Code `claude-opus-5-5`, `xhigh`) — independent full-diff review covering correctness, architectural fit, and risk.
- **@reviewer-astra** (Codex `gpt-6.2-astra`, `xhigh`) — independent full-diff review covering invariants, concurrency, edge cases, and security boundaries.

## Goal (replace this)

Name the exact branch, commit range, or pull request to audit:

- *"Audit pull request #214 (`origin/main..HEAD`). Focus on transaction atomicity, error recovery, and backward compatibility of the client protocol. Read-only — do not modify files or comment on GitHub."*
- *"Perform a pre-release security and correctness review of `src/auth/` and `src/sync/`."*
