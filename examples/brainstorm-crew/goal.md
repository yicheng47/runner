# Brainstorm Crew — Mission Goal

A three-provider architecture and specification brainstorm crew. Three frontier models independently read the codebase, draft competing plans without anchoring on each other, cross-validate every assumption, and converge on one crisp spec plus explicit open tradeoffs for the human.

## Roles

Import [`brainstorm-crew.crew.toml`](brainstorm-crew.crew.toml) for all three planners and the 3-round cross-validation protocol:

- **@planner-neon** (Antigravity `gemini-4-neon`, `xhigh`) — independent Round 1 plan, Round 2 cross-critique, and Round 3 convergence synthesizer.
- **@planner-opus** (Claude Code `claude-opus-5-5`, `xhigh`) — independent Round 1 plan, Round 2 UX/architecture critique, and Round 3 sign-off.
- **@planner-astra** (Codex `gpt-6.2-astra`, `xhigh`) — independent Round 1 plan, Round 2 adversarial systems/correctness critique, and Round 3 sign-off.

## Goal (replace this)

Describe the problem you want to design before writing any code:

- *"Design offline template import and export for our desktop app and CLI. Inspect `src/model/` and `src/storage/`, define the file format, collision rules, error handling, and migration boundaries, and highlight any UX or security tradeoffs that need a human decision. Read-only — do not edit code."*
- *"Compare three options for replacing our background job queue with SQLite-backed persistence. Evaluate crash recovery, concurrency, and schema evolution, then converge on one recommended design."*
