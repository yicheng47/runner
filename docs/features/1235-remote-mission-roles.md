# 1235 — Remote mission roles

> Tracking issue: [#1235](https://github.com/yicheng47/runner/issues/1235). Scheduling and priority live on the issue and in the [roadmap](../roadmap.md). Platforms: macOS, Windows and Linux.
> Status: signed-off design spec. Reuses the existing local Add Role modal and Remove confirmation dialog with no new Pencil frames.
> Builds on: [#836](archive/836-web-remote-control.md) (companion protocol and remote mission workspace), [#1106](https://github.com/yicheng47/runner/issues/1106) (remote member tabs and live roster sync, shipped in 0.19.9) and [#1306](https://github.com/yicheng47/runner/issues/1306) (zero-role remote feed loading stability).

## Motivation

A remote window can follow a mission, answer questions, post messages, and stop, resume or restart members, but it cannot add or remove roles in that mission's roster. Both the sessions rail's **Add role** button and each member row's **Remove** button are gated behind `!self.presentation.is_remote()`, and the feed's empty-roster **Add role** card stays host-only. Someone operating a mission from another computer must switch back to the host desktop or CLI just to bring in a reviewer or QA slot, or to remove one.

Most of the client machinery is already transport-ready: the local Add Role modal (`StartChatModal` + `MissionRoleForm`) already operates on `RoleChoice` and `RuntimeChoice`, the host's existing `catalog` RPC already exports saved role settings (`runtime`, `model`, `effort`, `speed`) plus full runtime catalogs (`models`, `supported_efforts`, `efforts`, `speeds`), the Remove confirmation dialog already reads `self.members()`, and `Overview` reconciliation (`reconcile_handles`) already opens tabs for newly added handles and closes tabs for removed ones. What is missing is the companion wire contract for mutating a running mission's roster, host-side capability advertisement and error mapping, and wiring the reused desktop controls to `FrontPageClient`.

## Decisions (2026-11-24)

1. **Reuse the local UI as-is; no new Pencil frames.** Remote missions reuse the exact local **Add role** modal (full form: role, runtime/model/effort/speed overrides, handle and optional task), the rail **Add role** button, the empty-roster feed **Add role** card and the per-member **Remove** icon button with its `ConfirmDialog`.
2. **Additive companion actions with feature gating.** Add `addMissionRole` and `removeMissionRole` to `ActionRequest` and `Welcome.features` without bumping the companion protocol version. Older hosts omit those features and remote windows keep the corresponding controls hidden, matching existing remote actions.
3. **Host remains authoritative.** The host executes `ops::mission::roles::add_role` and `ops::mission::roles::remove_role`, enforces all roster rules, and broadcasts `mission/changed` and `session/updated`. Remote windows never optimistically mutate the roster; they wait for pushed `Overview` and `FeedAppended` updates.
4. **Carry `removedHandles` on `companion::Mission`.** Remote mission feeds are paginated, so older `role_removed` signals may not be loaded in memory when the user opens **Add role**. Adding an optional `removedHandles` list to `companion::Mission` in `Overview`—collected during `mission_status`'s existing single event-log scan with zero extra disk reads—lets the remote modal suggest and validate handles identically to the host while staying backward-compatible with older hosts and clients.
5. **Scope.** This feature covers the companion protocol (`runner-core`), host daemon (`runner-daemon`), desktop remote window (`runner-app`), and documentation. Mobile companions (`runner_ios`, `runner_android`) are out of scope for UI changes here and automatically inherit the new protocol actions when ready. Starting an empty remote mission from the launcher also remains host-only.

## Scope

### 1. Companion wire protocol (`crates/runner-core/src/companion/`)

- Add two `ActionRequest` variants in `crates/runner-core/src/companion/actions.rs` and include both names in `ActionRequest::NAMES`, `ActionRequest::field_problem`, `id()`, and `set_id()`:
  - `AddMissionRole` (`type: "addMissionRole"`):
    - `id: u64`
    - `mission: String`
    - `role: String`
    - `handle: Option<String>`
    - `runtime: Option<String>`
    - `model: Option<String>`
    - `effort: Option<String>`
    - `speed: Option<String>` (`"standard"` or `"fast"`)
    - `task: Option<String>`
    - `cols: u16`
    - `rows: u16` (required, like every other action that spawns a session: `startChat`, `startShell`, `resume`, `restart`, `fork`, `startMission` and `resumeMission`)
  - `RemoveMissionRole` (`type: "removeMissionRole"`):
    - `id: u64`
    - `mission: String`
    - `handle: String`
- Result payload for both requests: `{"mission": "<mission_id>", "handle": "<handle>", "session": "<session_id>"}` (matching `MissionRoleChange` and existing companion camelCase JSON conventions).
- Add an additive field to `companion::Mission` in `crates/runner-core/src/companion/overview.rs`:
  - `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub removed_handles: Vec<String>` (wire name `removedHandles`). Older hosts omit it and deserialize to an empty vec; older clients ignore the unknown field.
- Add wire fixtures under `crates/runner-core/src/companion/fixtures/` (`add_mission_role.json`, `remove_mission_role.json`, `result_mission_role.json`) and cover decoding, field-problem diagnostics (including a missing `cols` or `rows`), `ActionRequest::NAMES`, and `removedHandles` round-trips in `crates/runner-core/src/companion/tests.rs`.

### 2. Host daemon (`crates/runner-daemon/`)

- Advertise `"addMissionRole"` and `"removeMissionRole"` in `companion::FEATURES` (`crates/runner-daemon/src/companion/mod.rs`).
- **Grid size for remotely added workers (`ops/mission/roles.rs`):** Add `add_role_with_size(state, input, grid_hint: Option<(u16, u16)>)` (with existing `add_role` passing `None`, which continues to read `*state.mission_grid_hint.lock().unwrap()`). The companion handler passes `Some` of the request's `cols` and `rows`, clamped with the same bounds as `AttachRequest::clamped_size` (`cols.clamp(20, 512)`, `rows.clamp(8, 256)`) so a newly spawned worker on a headless host starts at the remote viewport size instead of wrapping at 80×24 before attach.
- **Removed handles in `Overview` (`ops/mission/feed.rs`, `companion/core.rs`):** Collect unique handles from human-authored `role_removed` signals during `mission_status`'s existing single `log.read_from_lossy(0)` scan, expose them on `MissionStatusSnapshot` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, and copy them into `companion::Mission.removed_handles` in `companion::core::overview`.
- **Action handlers (`companion/actions.rs`):**
  - `AddMissionRole`: validate optional `runtime` (`agent()`), optional `speed` (`speed()`), optional `handle` syntax when present (`handle()`), verify mission and role exist (`ErrorCode::NotFound` when absent), call `ops::mission::roles::add_role_with_size` and map its error with `role_failed` (below), emit `mission/changed` and `session/updated`, and return `{"mission", "handle", "session"}`.
  - `RemoveMissionRole`: validate `handle` is non-empty (`bad_request()` if blank), verify mission exists (`ErrorCode::NotFound` when absent), call `ops::mission::roles::remove_role` and map its error with `role_failed`, emit `mission/changed` and `session/updated`, and return `{"mission", "handle", "session"}`.
- **Typed roster rejections (`error.rs`, `ops/mission/roles.rs`):** Every rejection in `roles.rs` is plain `Error::Msg` text today, and the companion's `failed()` replaces any untyped error with "The host could not complete the request." Add a fieldless `RoleRejection` enum and an `Error::RoleRejected { kind: RoleRejection, message: String }` variant whose `Display` is `message`, and return it from each rejection in the table below with today's text, so the local app and CLI show exactly what they show now. `checked_mission` raises it for both add and remove. When `slot_handle_error` refuses a handle, `add_checked` picks `HandleInUse` or `HandleRemoved` from the `existing` and `removed` sets it already holds. A failed `ensure_first_turn_fits` is wrapped as `FirstTurnTooLarge` at its `roles.rs` call site, leaving that shared helper and its mission-start and resume callers unchanged. Handle syntax errors stay untyped, because the companion's `handle()` pre-check rejects them first. The companion never matches on error text.
- **Companion error mapping (`companion/actions.rs`):** A `role_failed` helper keeps the companion boundary's privacy discipline: static text only, with no host paths, mission titles or internal errors. It maps `Error::RoleRejected` by `kind` as below, passes `Error::RuntimeVersionGate` to `launch_failed` (keeping the typed `version_gate`), and sends every other error to `failed()`.

| `RoleRejection` | Raised when | `ErrorCode` | Companion message |
| --- | --- | --- | --- |
| `NotRunning` | Add or remove: the mission is stopped or archived (`checked_mission`) | `NotRunning` | The mission is not running. |
| `SpawnPending` | Add or remove: a session of the mission is still starting (`checked_mission`) | `Failed` | A session in this mission is still starting; try again once it is up. |
| `HandleInUse` | Add: the handle belongs to a current member | `Failed` | That handle is already in use in this mission. |
| `HandleRemoved` | Add: the handle was removed from the mission earlier | `Failed` | That handle was already removed from this mission; choose another handle. |
| `FirstTurnTooLarge` | Add: the composed first-turn prompt exceeds the argv ceiling | `Failed` | The mission brief is too large to launch that role. |
| `NotInMission` | Remove: the handle was never in the mission | `NotFound` | That role is not in this mission. |
| `AlreadyRemoved` | Remove: the handle was removed earlier | `NotFound` | That role was already removed from this mission. |
| `NoSession` | Remove: the slot has no unarchived session | `NotFound` | That role has no session in this mission. |
| `Resuming` | Remove: the member is being resumed or restarted | `Failed` | That role is being resumed or restarted; try again once it is up. |

### 3. Front-page client (`crates/runner-app/src/surfaces/front_page/client.rs`)

- Add `mission_add_role(&self, input: MissionAddRoleInput, size: Option<(u16, u16)>) -> ClientResult<MissionRoleChange>` and `mission_remove_role(&self, mission_id: &str, handle: &str) -> ClientResult<MissionRoleChange>` on `FrontPageClient`.
- Local targets delegate to `self.daemon.mission_add_role` and `self.daemon.mission_remove_role`.
- Remote targets call `self.action("addMissionRole", ActionRequest::AddMissionRole { ... })` and `self.action("removeMissionRole", ActionRequest::RemoveMissionRole { ... })`, converting `Runtime` / `CodexSpeed` to wire strings and deserializing `{ mission, handle, session }` into `MissionRoleChange`.

### 4. Desktop UI reuse (`crates/runner-app/src/surfaces/`)

- **Capability-gated entry points in `MissionWorkspace`:**
  - Rail **Add role** button (`surfaces/mission_workspace/rail.rs`, `MISSION_RAIL_ADD_ROLE`): render whenever `mission_slot_actions_available(...)` and `self.remote_supports(&["catalog", "addMissionRole"])` (always true locally).
  - Feed empty-roster **Add role** card (`surfaces/mission_workspace/feed.rs`, `MISSION_NO_ROLES`): render when `self.mission_composer_roster().is_empty() && !self.loading && mission_slot_actions_available(...)` and either local or (`self.remote_supports(&["catalog", "addMissionRole"])` and the remote feed's initial `history_loaded` is true). While initial feed history is still loading or failed to load, or on older hosts lacking `addMissionRole` or `catalog`, `MISSION_NO_ROLES` stays hidden so `#1306`'s anti-flash behavior is preserved. The predicate lives in `feed_item_count(&self)`, which has no `cx`, while `history_loaded` is read through `feed.read(cx)`: copy the feed's `history_loaded` into a workspace field in `sync_remote_feed`, which already runs on every feed notification, so the existing list-count updates see it, rather than threading `cx` through `feed_item_count`'s callers in `state.rs`.
  - Per-member **Remove** button (`surfaces/mission_workspace/rail.rs`, `remove-role-{session_id}`): render when `self.remote_supports(&["removeMissionRole"])` (always true locally).
- **Transport-neutral Add Role modal (`surfaces/start_chat/add_role.rs`, `modal.rs`, `role_setup.rs`):**
  - Replace local-only `(Mission, Vec<SlotWithRole>, Vec<Event>)` inputs to `open_add_role_modal` with a transport-neutral context constructed from `mission_view()`, `members()`, `remote_summary().removed_handles` unioned with loaded `role_removed` signals in `self.events`, `self.client.clone()`, and the workspace's current terminal viewport size (`self.current_mission_terminal_size(window, cx)` on remote). The form's current and removed handle sets are a snapshot taken when the modal opens, as on the host today: later `Overview` pushes do not update an open form, and a handle the host takes meanwhile is caught by the host's `HandleInUse` or `HandleRemoved` rejection on submit.
  - Local path preserves synchronous `role_list()` and `load_selectable_runtimes()` loading.
  - Remote path opens the exact same modal immediately in a loading state (form controls disabled, Submit disabled, Close and Cancel active) while fetching fresh `client.catalog()` on a background thread. When the catalog arrives, guard against modal close, host switch, or epoch/generation change, convert `catalog.roles` via `RoleChoice::from` and `catalog.runtimes` via `RuntimeChoice::remote`, initialize the selects and suggested handle, and focus `role_select`. If the catalog request fails, render the error in the modal's `notice_banner` while keeping Close and Cancel usable.
  - Guard local-only `refresh_start_chat_models` so remote modals never call `self.core(cx).runtime_refresh_models(...)` against the local daemon.
  - When a remote host's catalog contains zero roles, show a host-accurate hint (since the remote Roles page is read-only) in both `en.toml` and `zh-hans.toml`, leaving local copy unchanged.
  - Submit through `client.mission_add_role(input, viewport_size)` on a background thread. Guard completion against host/epoch/modal changes; on success close the modal and restore focus (host pushes update the rail, member tab, composer roster and feed); on error keep the modal open, re-enable controls, and display the error in `notice_banner` without altering the roster.
  - Close an open, non-submitting remote Add Role modal if `sync_remote` observes that its mission vanished or stopped running (`clear_remote_transients` and `select_remote_item` in `remote_client/bridge.rs` already close `start_chat_modal` on host switch, disconnect, return to local, or sidebar item change).
- **Remote Remove Role (`surfaces/mission_workspace/actions.rs`, `remote.rs`):**
  - Update `request_remove_role` to check `self.remote_supports(&["removeMissionRole"])` and validate membership against `self.members()` instead of local-only `self.sessions`.
  - Reuse `render_remove_confirm` (`ConfirmDialog`) unchanged.
  - In `sync_remote`, if `self.remove_confirm` targets a session ID that is no longer in `self.members()`, clear `self.remove_confirm` and trigger `reset_dialog_focus(true)`.
  - Route `remove_role` through `self.prepare_mission_action("removeMissionRole", session_id.to_owned())`, `client.mission_remove_role(&mission_id, &handle)`, and `self.complete_mission_action(...)`. A remote failure surfaces in the workspace error banner and leaves the roster unchanged; a success clears pending state and lets the host's `Overview` and `FeedAppended` pushes remove the rail row, close the member tab (falling back to Feed if active), and update the composer roster and feed.

### 5. Documentation

- Update `docs/guide/guides/remote-access.md` and `docs/guide/zh-CN/guides/remote-access.md` to document remote add/remove role controls, the empty-roster feed **Add role** card on capable hosts, and automatic hiding on older hosts. Replace the statements that the feed **Add role** card stays on the host and that adding and removing roles stay on the host.
- Update `docs/arch/arch.md` to include `addMissionRole`, `removeMissionRole`, and `Mission.removedHandles` in the companion protocol overview.
- Update `REMOTE-FEED-01`'s expected result in `docs/tests/regression/remote.md`, which now says no host-only **Add role** card appears remotely: on a host that offers `addMissionRole` and `catalog`, the card appears once history has loaded and never before, and older hosts still show none. Its no-flash requirement and capture-sequence evidence stay. New lasting checks from RMR-01 to RMR-03 are promoted at the release sweep as usual.

### Non-goals

- Starting an empty remote mission from the launcher (remains host-only).
- Creating, editing, or deleting roles or crews on the host from a remote window.
- Mobile companion (`runner_ios`, `runner_android`) UI changes.

## Implementation Phases

1. **Companion protocol and fixtures (`crates/runner-core`).** Add `AddMissionRole`, `RemoveMissionRole`, `Mission.removed_handles`, field-problem validation, wire fixtures, and unit tests.
2. **Daemon handlers and overview projection (`crates/runner-daemon`).** Add `add_role_with_size`, the typed `RoleRejection` errors in `roles.rs`, `removed_handles` in `mission_status` and `companion::core::overview`, `FEATURES` entries, `DaemonCore::action` handlers with `role_failed` mapping, and daemon/companion tests.
3. **Desktop client and UI reuse (`crates/runner-app`).** Add `FrontPageClient` methods, transport-neutral `AddRole` modal with async remote catalog load, capability-gated rail/feed/remove controls, remote `RemoveRole` action and dialog cleanup, bilingual empty-roles remote copy, and unit/visual tests (updating existing `remote_tests.rs` assertions for capable vs. older hosts).
4. **Docs.** Update English and Simplified Chinese remote-access guides, `docs/arch/arch.md` and the `REMOTE-FEED-01` regression row.

## Verification

### Automated

- `cargo test -p runner-core`: wire round-trip and field-validation tests for `addMissionRole` and `removeMissionRole` (with and without optional fields, and a missing `cols` or `rows` reported as a field problem), `ActionRequest::NAMES` coverage, `Mission.removedHandles` serialization/omission when empty, and backward compatibility with older payloads.
- `cargo test -p runner-daemon`: `DaemonCore::action` happy paths for `AddMissionRole` (with defaults, overrides, custom handle, task and `cols`/`rows`) and `RemoveMissionRole`, `mission/changed` and `session/updated` emissions, `Overview` `removedHandles` population after a removal, and each `RoleRejection` row's `ErrorCode` and message plus missing mission/role and `RuntimeVersionGate`. Existing `roles.rs` tests keep asserting today's local text, which `Error::RoleRejected` must not change.
- `cargo test -p runner-app`:
  - Capable remote host renders `MISSION_RAIL_ADD_ROLE`, member remove buttons, and (after `history_loaded`) `MISSION_NO_ROLES` on zero-role running missions; stopped/archived/secondary missions still hide them.
  - Older host lacking `addMissionRole`, `removeMissionRole`, or `catalog` hides the corresponding controls and refuses dispatch at the client boundary; zero-role missions on capable hosts still do not flash `MISSION_NO_ROLES` or `MISSION_NO_EVENTS` before `history_loaded` or when initial feed load fails (`#1306`).
  - Remote Add Role modal loads host catalog asynchronously, exposes runtime/model/effort/speed overrides, excludes both active members and `Overview.removedHandles` from suggested/valid handles, handles empty host catalog and catalog-fetch failure cleanly, keeps the modal open with inline error on rejected submit (including a handle the host took after the modal opened), closes on success, and discards stale replies on close/switch/mission-stop.
  - Remote Remove Role opens `ConfirmDialog`, dispatches `removeMissionRole`, clears pending state and shows an error banner on rejection without altering the roster, dismisses the confirm dialog if the member disappears remotely before confirm, and updates rail/tabs/composer when pushed `Overview` removes the member.
- Any new app test that waits on background daemon/remote work runs under `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'`.
- Workspace gates: `make clippy`, `cargo fmt --all -- --check`, and `git diff --check`.

### Live checks (macOS development app)

Selected because this change unlocks interactive remote mission-roster controls, opens the Add Role modal over a live remote catalog, and mutates live host sessions over the relay. Runs on the [same-Mac two-host relay fixture](../tests/full-smoke-test.md#same-mac-two-host-relay-fixture-macos) with `qa` holding `script/qa-lock`:

1. **RMR-01 — Add role remotely with overrides and task (anchors: `REMOTE-MIS-06`, `MIS-ADD-01`):** From the remote window of a running bounded mission in `/tmp`, click **Add role** in the sessions rail. Verify the modal loads the host's roles and runtimes, pre-fills an unused `@handle`, and supports runtime/model/effort override plus an optional task. Submit and confirm the new worker joins on both host and remote (rail card, member tab auto-opened remotely, composer roster updated, `role_joined` divider, system intro and directed task in the feed, live terminal attachable). Reason: exercises the end-to-end `catalog` + `addMissionRole` + `Overview`/`FeedAppended` path.
2. **RMR-02 — Remove role remotely and handle reuse guard (anchors: `REMOTE-MIS-06`, `MIS-REM-01`):** From the remote window, click the remove icon on an active member while its tab is active, cancel once (no change), then confirm **Remove**. Verify the host stops the session, both host and remote remove the member card, the remote active tab falls back to Feed, the `role_removed` divider and leave notice appear in the feed, and reopening **Add role** remotely refuses the removed `@handle` and suggests a fresh handle. Reason: verifies `removeMissionRole`, confirmation dialog, tab fallback, and `removedHandles` validation.
3. **RMR-03 — Zero-role empty card and a rejected add (anchors: `REMOTE-FEED-01`, `MIS-ADD-01`):** On the host instance, start an empty mission in a fresh `/tmp` folder with its private-`HOME` CLI (`runner mission start --title <title> --cwd <dir>` with no crew), then open it in the remote window for the first time so its history loads with an empty roster. Record a bounded native capture sequence across that load, as `REMOTE-FEED-01` requires, since stills alone cannot show a flash: neither `MISSION_NO_ROLES` nor **No events yet.** may appear before history has loaded, and the **Add role** card appears once it has. Use the card's **Add role** to add a role. For the rejection, open **Add role** remotely again and type an unused handle such as `@rmr-dup` without submitting; on the host, run `runner mission add-role <mission> <role> --handle rmr-dup` and wait for it to return; then submit the remote modal. Expect the modal to stay open with "That handle is already in use in this mission." in its notice and its controls re-enabled, and both rosters to show only the host's `@rmr-dup`. Reason: validates the empty-roster feed entry point, `#1306` anti-flash stability, and the mapped host rejection reaching the remote modal.

Older-host capability hiding (`REMOTE-MIS-04`) is covered by the automated `remote_tests.rs` suite unless an older-release binary is staged. Record live QA results with screenshots in `docs/tests/runs/1235-remote-mission-roles-mac.md`.
