# Local Mission Control — Stage 1

**Branch:** `feature/mission-control-board`  
**Worktree:** `/Users/terrynovis/runner-monitor-ui/.worktrees/feature-mission-control-board`  
**Base:** `1b096b1b` (`feat(mission): add review and usage surfaces`)

Implement the approved first slice from
[`local-mission-control-stage1.md`](../../features/local-mission-control-stage1.md).
The native board consumes one typed, read-only `runnerd` snapshot that joins
project-local Markdown tickets and `.crew/triage/` with existing Runner mission
state. The private Pen checkout is unavailable; use AffectUI's agent-component
and design-system pages as visual direction only, while matching existing GPUI
surfaces and retaining the board's strict read-only boundary.

The coder owns the protocol, read-only adapter, daemon join, native route and
focused automated tests. Do not add any workflow mutation, copied ticket text,
web client, Kanbots data store, autopilot or worktree system. Treat bad source
data as source-scoped health warnings with partial board data. Apply the exact
derived-lane precedence in the feature spec; only a real pending Runner
question creates **Needs you**.

The independent reviewer examines the final candidate for protocol stability,
field provenance, lane precedence, malformed/stale-source behavior and scope
containment. It names the required live checks before QA. The QA slot alone
acquires `script/qa-lock`, launches the development app, exercises the selected
ticket-only, active-work, pending-question, empty/malformed-source,
navigation and no-mutation cases, and records results for the reviewed
candidate. Coders and reviewers do not launch the app.

This is the approved **Codex trio**. It may commit, push to `tezmaster/runner`
and open a PR against `main`; use the required `core-review`, `risk-high` and
`needs-live-test` labels until QA passes. Preserve the four-commit rebased base:
do not reset, abort, replay or flatten the Mission Review/Muse dependency.
