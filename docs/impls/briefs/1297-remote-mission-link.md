# Mission Brief — #1297 Link and unlink missions from a remote window

- **Issue:** [#1297](https://github.com/yicheng47/runner/issues/1297) (feature, P1, milestone 0.19)
- **Spec:** `docs/features/1297-remote-mission-link.md` (authoritative; wins on detail)
- **Starting point:** worktree `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1297-remote-mission-link`, branch `feat/1297-remote-mission-link`, off `origin/main` `ed3de8c0`. Keep the root checkout on `main`; work only in this worktree with its own `target/` (never a shared `CARGO_TARGET_DIR`).
- **Parallel missions:** #1292 (`.worktrees/fix-1292-antigravity-session-title`, PR #1302) and #1295 (`.worktrees/feat-1295-antigravity-osc7501`, PR #1296) run concurrently. Build only against `origin/main`, never touch their worktrees, and expect to share `script/qa-lock` with them.

## Goal

Give a remote window the host's existing **Link to chat…**, **Unlink** and drag-to-link controls by adding additive `linkMission` / `unlinkMission` companion actions that the host executes through its existing link operations. No new UI design: reuse the host window's controls, picker and copy.

## Read First

1. `docs/features/1297-remote-mission-link.md`.
2. `crates/runner-core/src/companion/actions.rs` (+ `fixtures/stop_mission.json`, `resume_mission.json`) — how `stopMission` / `resumeMission` are parsed, advertised and id-mapped.
3. `crates/runner-daemon/src/companion/{mod.rs,actions.rs}` — advertised list and host handlers; `crates/runner-daemon/src/ops/mission/link.rs` (`mission_link`, `mission_unlink`, `ensure_linkable`).
4. `crates/runner-app/src/surfaces/front_page/client.rs` (`link_mission`, `unlink_mission`), `crates/runner-app/src/surfaces/sidebar/model.rs` (`menu_action_supported`), `crates/runner-app/src/surfaces/sidebar/drag.rs` (`remote_keeps_link` and the `is_remote()` branches).
5. `crates/runner-app/src/surfaces/remote_client/` — remote overview, action dispatch and error surfacing; `crates/runner-app/src/surfaces/sidebar/link.rs` and `link_tests.rs` for the host link picker.
6. `docs/guide/guides/remote-access.md` and `docs/guide/zh-CN/guides/remote-access.md`.

## Deliverables And Boundaries

- Additive companion protocol: new actions parse, serialize, are advertised by the host, and older peers keep working; gate remote UI on the host advertising them.
- Host handlers call `mission_link` / `mission_unlink` with the host UI's arguments; host link rules stay the only source of truth.
- Remote menu items, picker (from the remote overview's host chats), drag into/out of link groups (including the multi-chat picker), regroup only from the host's overview update, and visible errors without regrouping.
- Bilingual guide update; automated tests per the spec's Verification section.
- Out of scope: iOS UI (#912), changes to host link rules, picker design, or #1290's archive prompt.

## Validation And Handoffs

1. **Coder:** implement; pass `make fmt`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` (and with `--features updater`), `cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci`, and `make guide-cli` if CLI help changes. Bound every async wait in new tests with a timeout so a regression fails instead of hanging. Do NOT run `make run`. Fold this brief into the implementation commit (no standalone brief commit) and keep the spec. Hand a committed candidate SHA to `reviewer`.
2. **Reviewer:** review the diff against the spec and `AGENTS.md` without running the app (protocol compatibility, host-rule reuse, remote gating, no optimistic regroup, bilingual docs); pass a clean SHA with live checks REM-LINK-01..05 to `qa`.
3. **QA:** `script/qa-lock acquire --timeout 540` (shared with #1292/#1295 — wait politely, report the holder to the coder if blocked), `make run` in this worktree, open a remote window on the development app (to itself or another reachable host), run REM-LINK-01..05 with tiny prompts in fresh `/tmp` folders, screenshot each, preserve every pre-existing development chat and daemon, archive only QA-created chats and missions, release the lock once your app has exited (if pre-existing user chats keep the development daemon alive, release after closing your window and archiving your own sessions). Add a clearly labelled #1297 block to `docs/tests/runs/2026-11-13-mac.md` (append; never overwrite earlier blocks) and record results in the PR. Mark any check whose precondition is unavailable as Blocked with the reason.

## Authorization

The `agy trio` crew is approved by Jason (2026-11-13) for #1297. Once review and macOS QA are clean: commit on `feat/1297-remote-mission-link`, push, open a PR against `main` referencing #1297 with one risk label chosen from the diff (expected `risk-medium`), keep any automatic `core-review`, add `needs-live-test` only if a required check remains unpassed, have the reviewer add `reviewed` on the final head, and drive CI green on all platforms. Stop at the open PR; do not merge.
