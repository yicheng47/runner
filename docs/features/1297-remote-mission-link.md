# 1297 — Link and unlink missions from a remote window

Tracking issue: [#1297](https://github.com/yicheng47/runner/issues/1297). Milestone 0.19.

## Problem

A remote window shows a host's linked missions grouped under their chats (#748), but cannot change a link. **Link to chat…**, **Unlink**, and dragging a mission into or out of a chat's link group are host-only. The companion overview already carries each mission's `linked_chat`; the companion actions have no request to change it. So `Sidebar::menu_action_supported` hides both menu items remotely, `FrontPageClient::link_mission` / `unlink_mission` return "Linking is available on the host.", and `sidebar/drag.rs` (`remote_keeps_link`) filters out link changes when remote.

## Design

No new UI. A remote window gets the host window's existing link controls, behaviour and copy; the host keeps owning every link rule.

### Protocol

- Add companion actions `linkMission { mission, session }` and `unlinkMission { mission }` in `crates/runner-core/src/companion/actions.rs`, modelled on `stopMission` / `resumeMission`: parsing, the advertised action list, the id accessors, and JSON fixtures beside `stop_mission.json`.
- The host advertises both in `crates/runner-daemon/src/companion/mod.rs` and handles them in `crates/runner-daemon/src/companion/actions.rs` by calling `ops::mission::link::mission_link` / `mission_unlink` with the same input the host UI passes (including the report flag the host UI uses). The host's checks stay authoritative: direct agent chats only, project match, archived/unknown targets rejected. The linked chat hears about the change exactly as it does for a host-side link.
- Additive only: older peers ignore the new action names, and a remote window connected to a host that does not advertise them behaves as today.

### Remote window

- `menu_action_supported` shows **Link to chat…** and **Unlink** on remote mission rows only when the connected host advertises `linkMission` / `unlinkMission`.
- `FrontPageClient::link_mission` / `unlink_mission` send the companion actions when remote instead of returning the host-only error.
- The link picker lists the host's eligible chats from the remote overview, filtered the same way as on the host.
- Dragging a mission into or out of a chat's link group works remotely as on the host, including the picker for a multi-chat drop target, when the host advertises the actions; otherwise `remote_keeps_link` keeps today's behaviour.
- Grouping changes come only from the host's overview update, after a link change made on either side; the remote window does not regroup optimistically.
- A rejected or failed request surfaces through the remote window's existing action-error path and leaves the grouping unchanged.

### Docs

Update `docs/guide/guides/remote-access.md` and `docs/guide/zh-CN/guides/remote-access.md` together.

## Out of scope

iOS companion UI (#912); any change to host-side link rules, the link picker's design, or #1290's archive prompt.

## Verification

Automated: companion action parse/serialize fixtures and advertised-list tests; daemon handler tests that a remote link/unlink applies the same rules as the host (accepts a direct agent chat, rejects a non-linkable target, unlink clears the link and keeps the mission); remote-client GPUI tests for menu visibility with and without the advertised actions, the drag path, the picker source, and error handling without regrouping.

Live checks, macOS, development app connected to itself or a second host as a remote window:

- **REM-LINK-01** — In a remote window, **Link to chat…** on a mission row links it to a host chat picked from the remote picker; both the remote and the host sidebar regroup it under that chat.
- **REM-LINK-02** — **Unlink** in the remote window removes the mission from the chat's group on both sides.
- **REM-LINK-03** — Dragging a mission into and out of a chat's link group in the remote sidebar links and unlinks it, including the picker for a multi-chat drop target.
- **REM-LINK-04** — A link made on the host regroups in the open remote window without reconnecting.
- **REM-LINK-05** — A rejected link (for example to a chat in another project) shows an error in the remote window and leaves the grouping unchanged.

Connecting to an older host that does not advertise the actions is covered by automated tests; a live check needs an older host build and is optional.
