# #1165 — Agent update notices, version gate and background tasks

## Starting point

Continue [#1165](https://github.com/yicheng47/runner/issues/1165) on the existing branch `feat/1165-agent-cli-version-gate` in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1165-agent-cli-version-gate`. The starting candidate is `3ccfc73673a8d3546603a9966017be640f392b28`, a spec-only commit directly on `origin/main` at `c46354ef2993c562cfefa1d07c1d1da0532151bd`. The branch already has draft [PR #1268](https://github.com/yicheng47/runner/pull/1268); continue that PR rather than opening another. Its implementation-hold text and claude-trio route are superseded by Jason's 2026-11-11 instruction to start the codex trio.

Read `AGENTS.md` and `docs/features/1165-agent-cli-version-gate.md` first. The approved design is `runner/desktop/specs/1165-agent-cli-version-gate.pen` in `/Users/jason/repos/yeelinks/yeelinks_design`, signed off at `8e9db8f`. The spec wins on behavior and the frames win on layout/copy. The issue body's older A/D proposal is superseded by the signed-off B/G design and background tasks. Do not reopen the design choice or implement the discarded alternatives. Preserve unrelated edits in the design repo.

## Outcome

An old Codex must receive a useful version gate before Runner composes an incompatible launch. Available agent updates must be visible where people start work. Running an update must leave the window usable, with a background task pill/list and an input/output dialog opened only on request.

Implement the complete approved spec:

- Catalog minimum version and typed daemon refusal across new chats, resume, fork, mission slots, CLI and remote launch paths; unknown installed versions remain allowed. Launcher B marks only the agent row Update needed; role rows never show update labels or actions, per Jason’s 2026-11-11 scope revision. Starting the agent or a role using it still starts an update and leaves the pane waiting until success. Other refused paths offer the same recovery card.
- Early classified startup failure and stopped-card recovery for Codex config errors, with ordinary exits retaining their current behavior.
- Launcher G's optional versions/Update tag on agent rows only and the Settings gear dot for every enabled updatable runtime; remove the old usage-popover gear dot. Optional updates leave supported launches usable.
- Daemon-owned in-memory tasks: one active update per runtime, joining duplicates, terminal attach, task events, input heuristic, stop/reprobe/reinstall, failure/retry/dismissal, success refresh and five-second cleanup. Implement the approved pill/list priority and hide/reappearance rules, Settings row states, and a hideable dialog that never steals focus automatically.
- Update the three guide pages named by the spec and their Chinese mirrors, plus current architecture/regression contracts directly affected by this change. Record implementation and QA evidence without claiming unavailable platform coverage.

The minimum is an evidence task, not a settled number: confirm the spec's 0.147/0.148/0.149 parsing/execution boundary and whether 0.149–0.163 deliver the hooks Runner needs without the dormant-MCP fix in 0.164. Record the evidence and set the minimum accordingly. Use isolated portable versions under `/tmp`; do not replace the installed Codex or change global agent configuration. If a necessary authenticated probe cannot run safely, disclose the blocker rather than inventing a result.

Automatic updates and their setting belong to #803. Runner's own app updater is outside this change. Remote update tasks do not join the local pill/list; follow the spec's remote gate behavior. There is no database migration or new CLI command.

## Starting code

- `crates/runner-core/src/protocol/runtime.rs` and `runtime_metadata.rs`: catalog/status and the typed protocol surface.
- `crates/runner-daemon/src/runtime_status/versions.rs`, `ops/runtime.rs`, runtime adapters and launch paths: version probing, update PTYs, gate, task ownership and startup classification.
- `crates/runner-app/src/surfaces/start_chat/`, `agent_update.rs`, Settings Agents, sidebar Settings button and stopped-chat cards: launcher, tasks and recovery UI.
- `docs/tests/regression/settings.md`: `SET-UPDATE-01` is the old blocking-modal contract that this feature replaces; retain its ID as retired and add the new behavior per suite conventions. Select only the executable-override portion of `SET-AGENT-02` for fixture setup/restoration, not its unrelated preferences/catalog matrix.

## Validation and handoffs

Before running checks, name the changed behaviors and why each selected check covers them. Coder runs the spec's automated version/gate/classifier/task-state cases, relevant daemon/core/CLI tests for changed paths, `runner-app` tests, workspace Clippy and formatting; include the required updater-feature pass. Preserve existing tests unless the observable contract intentionally changes. Reviewer reviews the frozen candidate and names any directly affected live checks. QA begins after clean review and owns the live app, shared QA lock, fixtures and run record.

QA runs the spec's seven checks in the development app, using scratch executable overrides and backing up/restoring development settings: (1) launcher gate and launch after successful update; (2) resume gate; (3) classified fallback; (4) optional update tags/gear dot and continued launch; (5) Settings background task, usable window, pill/list/hide and five-second success clear; (6) possible-input amber state, no automatic focus change, Answer and Hide; (7) failure/retry, stop with working executable, stop with broken executable/Reinstall and hidden-pill return. The spec gives the reason for each. Include multiple-task priority/count, duplicate joining and continued terminal typing in these checks because those are changed behavior, not a broader smoke run.

Use deterministic scratch updater scripts, never the machine's real agent updater. QA records candidate identity, per-check pass/fail/blocked and screenshot evidence in `docs/tests/runs/2026-11-11-mac.md` (or the actual run date/platform). Record platform gaps explicitly; keep `needs-live-test` while any selected required check remains unpassed. Real CLI compatibility evidence is distinct from native UI evidence. QA fixes return to coder, then reviewer, then only affected live checks rerun.

## Authorization

Jason explicitly approved the **codex trio** (coder, reviewer and qa) on 2026-11-11. The mission may implement, commit, push and update existing PR #1268, including its description, review/QA evidence and applicable labels; convert the draft when the implementation is ready under the repository's gates. This is the go-ahead awaited by the spec-only PR.
