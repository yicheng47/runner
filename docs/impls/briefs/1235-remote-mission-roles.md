# #1235 — Remote mission roles

## Starting point

- Issue: [#1235](https://github.com/yicheng47/runner/issues/1235); draft PR [#1461](https://github.com/yicheng47/runner/pull/1461) already open on this branch.
- Branch: `feat/1235-remote-mission-roles`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/feat-1235-remote-mission-roles` (base `origin/main` `64f81420`, its own `target/`; never share `CARGO_TARGET_DIR`).
- Isolation: keep the root checkout on `main` and never touch sibling worktrees. `.worktrees/fix-1395-remote-project-drag` also changes remote-client code; if `main` moves under you with a conflict, rebase onto `origin/main` rather than reaching into that worktree.
- The branch holds the spec commit (`docs(spec): specify remote mission roles (#1235)`) and this brief's commit. Fold this brief into an implementation commit before the final push.

## Problem and outcome

A remote window can follow and operate a mission but cannot change its roster: the rail's **Add role**, each member's **Remove**, and the feed's empty-roster **Add role** card are host-only. Outcome: on a host that advertises the new companion actions, a remote window adds and removes roles with the same Add Role modal and Remove confirmation as the host, the host stays authoritative, failures show clearly and leave the roster unchanged, and older hosts keep the controls hidden.

The spec [`docs/features/1235-remote-mission-roles.md`](../../features/1235-remote-mission-roles.md) is authoritative for behavior, wire shapes, error mapping, copy and checks; no new Pencil frames.

## Read first

- `docs/features/1235-remote-mission-roles.md`
- `crates/runner-core/src/companion/actions.rs` (`ActionRequest`, `field_problem`, `NAMES`), `overview.rs` (`Mission`), `tests.rs`, `fixtures/`
- `crates/runner-daemon/src/ops/mission/roles.rs` (`checked_mission`, `add_role`, `add_checked`, `remove_role`, `removed_handles`), `error.rs`, `ops/mission/feed.rs` (`mission_status`)
- `crates/runner-daemon/src/companion/actions.rs` (`launch_failed`, `failed`, `handle`, `agent`, `speed`, `StartMission`/`ResumeMission` arms), `companion/core.rs` (`overview`), `companion/mod.rs` (`FEATURES`)
- `crates/runner-app/src/surfaces/front_page/client.rs`, `start_chat/add_role.rs`, `mission_workspace/{rail,feed,actions,remote}.rs`, `mission_workspace/remote_tests.rs`, `remote_client/bridge.rs`
- `docs/guide/guides/remote-access.md` (and zh-CN), `docs/tests/regression/remote.md` (`REMOTE-FEED-01`)

## Deliverables

Follow the spec's Scope §1–§5 and Implementation Phases:

1. **Protocol (`runner-core`):** `addMissionRole` (required `cols`/`rows`) and `removeMissionRole`, the additive `Mission.removedHandles`, fixtures and tests.
2. **Daemon (`runner-daemon`):** `add_role_with_size`, `removed_handles` from `mission_status`'s existing scan, `FEATURES` entries, the two action handlers, the typed `RoleRejection` / `Error::RoleRejected` with today's local text unchanged, and `role_failed` mapping exactly as the spec's table. Never match on error text.
3. **App (`runner-app`):** `FrontPageClient` methods; capability-gated rail, remove and empty-card controls (the empty card only after `history_loaded`, cached in `sync_remote_feed`); the transport-neutral Add Role modal with async remote catalog load and handle sets snapshotted at open; remote Remove through `prepare_mission_action`/`complete_mission_action`; bilingual copy; updated `remote_tests.rs`.
4. **Docs:** both remote-access guides, `docs/arch/arch.md`, and the `REMOTE-FEED-01` row.

Non-goals: empty remote mission start, role/crew editing from a remote window, mobile companion UI.

## Validation and handoffs

- **@coder:** run `cargo test -p runner-core`, `cargo test -p runner-daemon`, `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check` and `git diff --check`. Any new test that waits on daemon or remote background work loops with `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'`; state the count. Hand `@reviewer` a local unpushed commit SHA with summary, changed files and checks run.
- **@reviewer:** review the handed-off SHA against the spec. Focus on wire compatibility with older hosts and clients, `RoleRejected` keeping local CLI/app text identical, the companion's privacy (no titles, paths or raw errors), stale-reply guards on the async catalog load and submit, the #1306 anti-flash gate, and test races. Name the candidate SHA and the live checks in the clean review.
- **@qa:** take `script/qa-lock acquire --timeout 540` before building wrappers or launching anything; other missions may hold it, so wait cleanly. Run `RMR-01`, `RMR-02` and `RMR-03` from the spec on the [same-Mac two-host relay fixture](../../tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos), including `RMR-03`'s capture sequence. Record pass/fail per check with screenshots in `docs/tests/runs/1235-remote-mission-roles-mac.md`, tear the fixture down to baseline, archive only QA-created missions, and release the lock. Report the verdict to `@coder`.

## Authorization

Approved crew: `neon trio` (`coder`, `reviewer`, `qa`). After a clean review and qa's verdict, `@coder` may shape the commits, push `feat/1235-remote-mission-roles` with `--force-with-lease`, update PR #1461 (title, body with `Closes #1235` and the live-check results, `risk-medium`, `needs-live-test` while any required check has not passed, keeping `core-review` if the workflow adds it), mark it ready for review, have `@reviewer` apply `reviewed` on the head SHA, and drive CI green on every platform. Do not merge.
