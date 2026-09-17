# 632 — Rename the `runner_status` signal to `session_status`

Tracking issue: [#632](https://github.com/yicheng47/runner/issues/632). Follow-up left on the [604 role rename record](./604-role-rename/README.md) (decision 6). Chore, P2. Implemented inline on 2026-09-17 on `chore/632-session-status-signal` over `main` at `dc28cdc`.

## Why

The 604 rename made the stored entity a role and kept the product name Runner, but the busy/idle signal on the mission log was still called `runner_status`, its enum `RunnerStatus`, and the `mission_status` field `latest_runner_status_by_handle`. The thing that is busy or idle is a session, so the term was the last place where "runner" still named a domain concept instead of the product.

## What ships

- **Writers emit `session_status`.** The forwarder's mission-log rows and the hook-observation rows (`ForwarderEmitCtx::session_status_draft`), the router's synthetic wake-busy row, and the CLI's `runner status busy|idle`; the verb stays (604 decision 4). `KnownSignalType::RunnerStatus` is `SessionStatus`.
- **Readers accept both.** `KnownSignalType::from_name("runner_status")` resolves to `SessionStatus` and `SignalType::is_session_status` covers both spellings, so persisted logs written before the rename still project: router replay and dispatch, the `mission_status` projection, mission activity from the log (`ops/mission.rs`), the app's mission workspace projection (`project_session_statuses`), the feed's hidden-signal filter and the store refresh filter. `runner signal runner_status` from an old brief validates and is written under the new name.
- **One busy/idle enum.** `RunnerStatus` is gone; `SessionActivityState` moved to `session::runtime` with its serde derive and is re-exported from `session::manager` and `router` where the old names were imported. `RunnerStatusSnapshot` is `SessionStatusSnapshot`; `latest_runner_status_by_handle` is `latest_session_status_by_handle`.
- **Prompt and comments.** The first-turn prompt says "the lead of crew" instead of "lead runner in crew"; the remaining "per-runner", "spawns the runners" comments and the CLI's "runner rail" hint now say handle, session, slot or role. Left as they are on purpose: the `runners` table in the 0023 migration test SQL, the `runners/` window-route compat and the `runner_handle` not-persisted assertions, all read compatibility.
- **Docs.** `docs/arch/arch.md` §5.1 diagram and §5.5 (with the compat note on the handler row), `docs/product/vision.md`; mission-watch's jq filter and SKILL.md in the memory repo accept both types. Archived records keep their historical wording.

## Verification

`cargo test --workspace` (814 backend tests, every app suite, the CLI roundtrip asserting `session_status`), workspace Clippy with `-D warnings`, `cargo fmt --check`, `git diff --check`. Legacy-row tests: `reconstruct_and_dispatch_read_legacy_runner_status_rows` (router), `event_projection_reads_legacy_runner_status_rows` (MCP `mission_status`), the pre-#632 row at the end of the mission activity test (`ops/mission.rs`), `session_status_projection_reads_legacy_runner_status_rows` (app), and the `from_name` / `is_session_status` cases in runner-core.

## Log

- 2026-09-17 — Implemented inline after #624 landed: a mechanical rename pass over the workspace, then the read-compatibility helpers in runner-core, the four reader sites, the CLI canonicalization, the prompt wording, the docs and the mission-watch filter. Design canvas labels, the third 604 follow-up, stay open.
