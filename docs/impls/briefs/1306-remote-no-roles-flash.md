# Fix `"No roles yet"` flash in remote mission feed (#1306)

## Starting point

- Issue: `yicheng47/runner#1306` (`bug`, `P2`, `backlog`)
- Branch: `fix/1306-remote-no-roles-flash`
- Worktree: `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1306-remote-no-roles-flash`
- Base: `origin/main` at `f06510bf`
- Parallel isolation: sibling worktrees `.worktrees/feat-1295-antigravity-osc7501` and `.worktrees/feat-1297-remote-mission-link` are active in parallel. Stay strictly inside this worktree with its own Cargo `target/` directory; do not touch root `main` or sibling worktrees, and do not assume unmerged PR branches have landed.

## Problem and outcome

When a Runner desktop connects remotely to another Runner desktop and opens a running mission with zero roles, `"No roles yet"` (`MISSION_NO_ROLES` / `render_empty_mission`) flashes and jumps at the top of the mission feed before remote events finish loading.

Root cause across the local vs remote load sequence:

1. `MissionWorkspace::sync_remote` (`remote.rs`) clears `self.loading = false` immediately when `overview` arrives, then calls `bind_feed` (which starts `MissionFeed::refresh` asynchronously with `entries == []` and `loading == true`) and `sync_remote_feed` synchronously while `feed_rows` is still empty. By contrast, local `open_mission` (`attach.rs`) populates `events` and runs `rebuild_event_projection()` before setting `self.loading = false`.
2. `feed_item_count()` (`feed.rs`) appends `render_empty_mission` (`MISSION_NO_ROLES`) whenever `mission_composer_roster().is_empty() && !self.loading && mission_slot_actions_available(...)`, without checking `!self.presentation.is_remote()` or waiting for the initial `MissionFeed` load, even though `render_empty_mission` already hides `"Add role"` remotely.
3. `feed_empty_panel_min_height` (`state.rs`, `feed.rs`) starts at `px(0.)`, expands on a deferred canvas pass in `render_mission_feed`, and resets to `px(0.)` in `rebuild_event_projection()` when feed events arrive, turning the premature render into a visible flash and downward jump.

Decide and implement the clean fix with `reviewer`:

- Prevent the premature empty-feed / `"No roles yet"` flash before remote `MissionFeed` finishes its initial load (avoiding flashing `"No events yet"` + `"No roles yet"` while `MissionFeed` is still loading initial entries).
- Gate or align `MISSION_NO_ROLES` in remote mission views (`!self.presentation.is_remote()`, noting remote windows cannot add roles and `rail.rs` already shows `MISSION_RAIL_NO_ROLES` `"No roles yet."`, or ensure steady-state empty remote rendering does not flash/jump if kept), while preserving the local non-remote empty-mission `"No roles yet"` + `"Add role"` UX.

## Read first

- `crates/runner-app/src/surfaces/mission_workspace/remote.rs` (`sync_remote`, `sync_remote_feed`, `MissionPresentation`)
- `crates/runner-app/src/surfaces/mission_workspace/feed.rs` (`feed_item_count`, `render_mission_feed`, `render_empty_mission`, `feed_empty_panel_min_height`)
- `crates/runner-app/src/surfaces/mission_workspace/state.rs` (`feed_empty_panel_min_height`, `prepare_mission`, `rebuild_event_projection`)
- `crates/runner-app/src/surfaces/mission_workspace/attach.rs` (`open_mission`)
- `crates/runner-app/src/surfaces/mission_workspace/rail.rs` (`MISSION_RAIL_NO_ROLES`, `MISSION_RAIL_ADD_ROLE`)
- `crates/runner-app/src/surfaces/remote_client/feed.rs` (`MissionFeed::new`, `refresh`, `loading`, `history_loaded`)

## Deliverables and boundaries

- Fix the remote initial load / empty-roles feed rendering race in `crates/runner-app/src/surfaces/mission_workspace/`.
- Keep changes tightly scoped; do not alter unrelated remote transport, daemon RPCs, or `start_chat_modal`.
- Add unit/UI tests in `runner-app` covering initial remote sync before vs after `MissionFeed` loads (zero-role running mission with events and with zero events) and verifying local non-remote empty-mission behavior remains intact.

## Validation and handoffs

- **Automated**: `runner-app` tests covering the new regression cases, plus CI-profile checks (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo test -p runner-app`, `cargo fmt --all --check`).
- **Handoffs**: `coder` implements on branch `fix/1306-remote-no-roles-flash` and hands off an unpushed local commit SHA to `reviewer`. `reviewer` reviews the snapshot and names the required live checks for `qa` under `AGENTS.md` Test Scope.
- **Live QA (`qa`)**:
  - Acquire `script/qa-lock acquire --timeout 540` before launching/wrapping dev builds. If another mission holds the lock, wait via bounded `acquire` retries; never bypass the lock or stop unrelated dev sessions.
  - Verify across two connected Runner desktop development instances: open/switch remotely to a running mission with zero roles (both with existing feed events and with zero events), confirming `"No roles yet"` no longer flashes or jumps, and verify local non-remote mission with zero roles still shows `"No roles yet"` + `"Add role"` properly.
  - Keep pre-existing user sessions untouched; archive only QA-created sessions/missions and release `qa-lock` once QA's app exits.

## Authorization

Jason approved the `agy trio` (`coder`, `reviewer`, `qa`) to fix `#1306`, commit, push `fix/1306-remote-no-roles-flash`, open a PR against `main` (`Closes #1306`) with appropriate labels (`frontend-only` if confined to `runner-app`, risk label, and `reviewed` from `reviewer` on the final head SHA), and drive CI green. Fold this brief into the implementation commit per `AGENTS.md` before final push so no standalone brief commit remains in PR history. Stop at the open, reviewed, QA'd, CI-green PR; do not merge or clean up the worktree.
